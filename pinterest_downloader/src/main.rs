use clap::{Parser, Subcommand};
use anyhow::Result;
use reqwest::Client;
use url::Url;
use scraper::{Html as ScraperHtml, Selector};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use std::sync::Arc;
use std::net::SocketAddr;
// Additional imports for web server
use axum::{
    extract::Query,
    http::StatusCode,
    response::{Html, IntoResponse, Response, Redirect},
    routing::{get, post},
    Form,
    Router,
};
use serde::Deserialize;
use tera::{Context, Tera};
use tokio::sync::Mutex;
use tower_http::trace::TraceLayer;
use tracing::{info, error};
use once_cell::sync::Lazy;
use rust_embed::RustEmbed;
use std::sync::Mutex as StdMutex;
use std::collections::HashMap;
use axum::http::header::LOCATION;
use handlebars::Handlebars;

/// Pinterest image downloader application
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Download images from a Pinterest URL
    Download(DownloadArgs),
    /// Start a web server interface (placeholder for now)
    Serve,
}

/// Arguments for the download command
#[derive(Parser, Debug, Clone)]
struct DownloadArgs {
    /// The Pinterest URL (e.g., a board or pin) to download images from.
    #[arg(short, long)]
    url: String,

    /// The genre to classify the downloaded images under (optional).
    #[arg(short, long)]
    genre: Option<String>,

    /// The query name to classify the downloaded images under (optional).
    #[arg(short, long)]
    query: Option<String>,

    /// Image quality to download (original, 736x, 474x, 236x)
    #[arg(short, long, default_value = "original")]
    quality: String,
}

// Web-specific structs
#[derive(RustEmbed)]
#[folder = "templates/"]
struct Templates;

#[derive(Clone)]
struct AppState {
    tera: Arc<Mutex<Tera>>,
}

#[derive(Deserialize, Debug, Clone)]
struct DownloadRequest {
    url: String,
    genre: Option<String>,
    query: Option<String>,
    quality: String,
}

impl From<DownloadRequest> for DownloadArgs {
    fn from(req: DownloadRequest) -> Self {
        DownloadArgs {
            url: req.url,
            genre: req.genre,
            query: req.query,
            quality: req.quality,
        }
    }
}

struct HtmlTemplate(String, Context);

impl IntoResponse for HtmlTemplate {
    fn into_response(self) -> Response {
        let Self(template_name, context) = self;
        // Lock the globally initialized Tera instance
        match TEMPLATES.lock() {
            Ok(tera) => {
                match tera.render(&template_name, &context) {
                    Ok(html) => Html(html).into_response(),
                    Err(err) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to render template '{}':\n{}", template_name, err),
                    ).into_response()
                }
            }
            Err(poisoned) => {
                // Handle the case where the mutex is poisoned
                 (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Template engine lock poisoned: {}", poisoned),
                ).into_response()
            }
        }
    }
}

// Initialize Tera from embedded assets
static TEMPLATES: Lazy<StdMutex<Tera>> = Lazy::new(|| {
    let mut tera = Tera::default();
    // Iterate over embedded files and add them to Tera
    for file_path in Templates::iter() {
        let path = file_path.as_ref();
        if let Some(file) = Templates::get(path) {
            if let Ok(content) = std::str::from_utf8(file.data.as_ref()) {
                if tera.add_raw_template(path, content).is_err() {
                    eprintln!("Failed to load embedded template: {}", path);
                }
            } else {
                 eprintln!("Failed to read embedded template as UTF-8: {}", path);
            }
        } else {
            eprintln!("Failed to get embedded template file: {}", path);
        }
    }
    tera.autoescape_on(vec![".html"]); // Adjust as needed
    StdMutex::new(tera)
});

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // If no subcommand is provided, use the default 'download' command with interactive prompts
    match cli.command {
        Some(Commands::Download(args)) => {
            // Run the download command with the provided arguments
            download_images(&args).await?;
        }
        Some(Commands::Serve) => {
            // Run the web server
            run_server().await?;
        }
        None => {
            // Default behavior if no subcommand is provided - show help
            println!("Error: A subcommand is required.");
            println!("Use 'pinterest_downloader download --help' to see download options.");
            println!("Use 'pinterest_downloader serve' to start the web interface.");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_server() -> Result<()> {
    // Initialize tracing (logging)
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false) // Don't print module paths
        .init();

    info!("Starting web server...");

    // Trigger lazy initialization of templates to catch errors early
    if TEMPLATES.lock().is_err() {
        return Err(anyhow::anyhow!("Failed to initialize template engine"));
    }

    let app_state = AppState {
        tera: Arc::new(Mutex::new(Tera::default())), // Placeholder, actual rendering uses static TEMPLATES
    };

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/download", post(download_handler))
        .layer(TraceLayer::new_for_http()) // Apply logging
        .with_state(app_state); // Pass state if needed by handlers

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("Web server listening on {}", addr);
    info!("Open http://127.0.0.1:3000 in your browser to access the Pinterest Downloader.");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root_handler() -> impl IntoResponse {
    let context = Context::new();
    HtmlTemplate("index.html".to_string(), context)
}

async fn download_handler(
    Form(payload): Form<DownloadRequest>,
) -> impl IntoResponse {
    info!("Received download request: {:?}", payload);
    let mut context = Context::new();
    context.insert("url", &payload.url);

    match Url::parse(&payload.url) {
        Ok(_) => {
            let args = DownloadArgs::from(payload.clone());

            // Spawn a background task to process the download
            tokio::spawn(async move {
                info!("Starting background download for {}", args.url);
                match download_images(&args).await {
                    Ok(_) => info!("Download completed successfully."),
                    Err(e) => error!("Download failed: {}", e),
                }
            });

            context.insert("success", &true);
        }
        Err(e) => {
            context.insert("success", &false);
            context.insert("error_message", &format!("Invalid URL: {}", e));
        }
    }

    HtmlTemplate("partials/download_results.html".to_string(), context)
}

/// The main function to download images based on the provided arguments
async fn download_images(args: &DownloadArgs) -> Result<()> {
    println!("Attempting to download images from: {}", args.url);
    println!("Using quality: {}", args.quality);
    
    // Get the genre/query if provided, otherwise use defaults
    let genre = args.genre.as_deref().unwrap_or("Uncategorized");
    let query = args.query.as_deref().unwrap_or("Pinterest");
    
    println!("Classifying under Genre: '{}', Query: '{}'", genre, query);

    let url = Url::parse(&args.url)?;
    let html_content = fetch_page(&url).await?;
    println!("Successfully fetched page content ({} bytes).", html_content.len());

    let image_urls = extract_image_urls(&html_content)?;
    println!("Found {} potential image URLs.", image_urls.len());

    if image_urls.is_empty() {
        println!("No images found to download.");
        return Ok(());
    }

    // Construct base output path
    let base_output_dir = match dirs::picture_dir() {
        Some(pictures_dir) => pictures_dir.join("Pinterestx").join(genre).join(query),
        None => {
            // Fallback if picture_dir is not available
            PathBuf::from(std::env::var("HOME")?)
                .join("Pictures")
                .join("Pinterestx")
                .join(genre)
                .join(query)
        }
    };

    // Create directories if they don't exist
    fs::create_dir_all(&base_output_dir).await?;
    println!("Saving images to: {}", base_output_dir.display());

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .build()?;

    let mut download_count = 0;
    for (index, img_url_str) in image_urls.iter().enumerate() {
        match Url::parse(img_url_str) {
            Ok(img_url) => {
                // Generate filename (e.g., image_001.jpg)
                let extension = Path::new(img_url.path())
                    .extension()
                    .and_then(|os_str| os_str.to_str())
                    .unwrap_or("jpg"); // Default to jpg if no extension
                let filename = format!("image_{:03}.{}", index + 1, extension);
                let dest_path = base_output_dir.join(&filename);

                println!("Downloading {} to {} ...", img_url.as_str(), dest_path.display());
                match download_image(&client, &img_url, &dest_path).await {
                    Ok(_) => {
                        println!(" -> Success");
                        download_count += 1;
                    },
                    Err(e) => println!(" -> Failed: {}", e),
                }
                
                // Add a small delay between downloads to be polite to the server
                if index < image_urls.len() - 1 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                }
            }
            Err(e) => {
                println!("Skipping invalid URL [{}]: {}", img_url_str, e);
            }
        }
    }

    println!("\nFinished downloading {} images.", download_count);

    Ok(())
}

/// Fetches the HTML content of a given URL.
async fn fetch_page(url: &Url) -> Result<String> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .build()?;

    let response = client.get(url.clone()).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Failed to fetch URL: {}", response.status()));
    }

    let body = response.text().await?;
    Ok(body)
}

/// Extracts potential image URLs from HTML content.
/// This might need adjustments based on Pinterest's current HTML structure.
fn extract_image_urls(html_content: &str) -> Result<Vec<String>> {
    let document = ScraperHtml::parse_document(html_content);
    let mut urls = HashSet::new(); // Use HashSet to avoid duplicates

    // Strategy 1: Look for <img> tags with specific attributes
    // Pinterest might use data attributes or specific class names.
    // This selector might need refinement by inspecting actual Pinterest HTML.
    let img_selector = Selector::parse("img[src]").map_err(|e| anyhow::anyhow!("Invalid selector: {}", e))?;

    for element in document.select(&img_selector) {
        if let Some(src) = element.value().attr("src") {
            // Basic filtering: Check if it looks like a plausible image URL
            // Pinterest often uses `i.pinimg.com` for images.
            if src.contains("pinimg.com") && (src.ends_with(".jpg") || src.ends_with(".png") || src.ends_with(".webp")) {
                // Try to find higher resolution versions if available (common pattern)
                let high_res_url = src.replace("/236x/", "/originals/") // Example replacement pattern
                                      .replace("/474x/", "/originals/")
                                      .replace("/736x/", "/originals/");
                urls.insert(high_res_url);
            }
        }
    }

    // Strategy 2: Look for JSON data embedded in <script> tags (More robust potentially)
    // This requires inspecting the page source to find the right script tag and JSON structure.
    // Example (needs adaptation):
    // let script_selector = Selector::parse("script[type='application/json']").map_err(|e| anyhow::anyhow!("Invalid selector: {}", e))?;
    // for script in document.select(&script_selector) {
    //     let json_text = script.inner_html();
    //     // Attempt to parse json_text and extract image URLs from the structure
    //     // Use serde_json::from_str here
    // }

    if urls.is_empty() {
        println!("Warning: No image URLs found using current selectors. The HTML structure might have changed.");
    }

    Ok(urls.into_iter().collect())
}

/// Downloads an image from a URL and saves it to a destination path.
async fn download_image(client: &Client, url: &Url, dest_path: &Path) -> Result<()> {
    let response = client.get(url.clone()).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Failed to download image: {}", response.status()));
    }

    let mut file = File::create(dest_path).await?;
    let content = response.bytes().await?;
    file.write_all(&content).await?;

    Ok(())
}

async fn handle_index(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let mut ctx = HashMap::new();

    // Add error/success messages from URL parameters if they exist
    if let Some(error) = params.get("error") {
        ctx.insert("error", error.to_string());
    }
    if let Some(success) = params.get("success") {
        ctx.insert("success", success.to_string());
    }

    // Default to empty strings for optional fields
    ctx.insert("url", params.get("url").unwrap_or(&String::new()).to_string());
    ctx.insert("genre", params.get("genre").unwrap_or(&String::new()).to_string());
    ctx.insert("query", params.get("query").unwrap_or(&String::new()).to_string());
    ctx.insert("quality", params.get("quality").unwrap_or(&"original".to_string()).to_string());

    let template = Templates::get("templates/index.html").unwrap();
    let contents = std::str::from_utf8(template.data.as_ref()).unwrap();
    
    // Render template and handle potential errors
    match Handlebars::new().render_template(contents, &ctx) {
        Ok(rendered) => Html(rendered).into_response(),
        Err(e) => {
            eprintln!("Template rendering error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!(
                    "<h1>Internal Server Error</h1><p>Failed to render template: {}</p>",
                    e
                )),
            )
                .into_response()
        }
    }
}

async fn download(
    Form(download_req): Form<DownloadRequest>,
) -> impl IntoResponse {
    let args = DownloadArgs {
        url: download_req.url.clone(),
        genre: download_req.genre.clone(),
        query: download_req.query.clone(),
        quality: download_req.quality,
    };

    // Validate the URL
    if args.url.trim().is_empty() {
        return Redirect::to("/?error=URL cannot be empty").into_response();
    }

    // Start download in separate task to not block response
    tokio::spawn(async move {
        if let Err(e) = download_images(&args).await {
            eprintln!("Download error: {}", e);
        }
    });

    // Get folder names for success message
    let genre = args.genre.as_deref().unwrap_or("Uncategorized");
    let query = args.query.as_deref().unwrap_or("Pinterest");
    let success_msg = format!("Download started! Images will be saved to Pictures/Pinterestx/{}/{}", genre, query);
    
    // Redirect with success message and preserve form data for potential re-use
    let path = format!(
        "/?success={}&url={}&genre={}&query={}",
        encode_uri_component(&success_msg),
        encode_uri_component(&args.url),
        encode_uri_component(&genre),
        encode_uri_component(&query)
    );
    
    Redirect::to(&path).into_response()
}

// Utility function to URL encode parameters
fn encode_uri_component(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
