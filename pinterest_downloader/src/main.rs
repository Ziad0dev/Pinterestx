use clap::{Parser, Subcommand};
use anyhow::Result;
use reqwest::Client;
use url::Url;
use scraper::{Html as ScraperHtml, Selector};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use std::net::SocketAddr;
use std::io::{Read, Write};
use reqwest::cookie::Jar;
use std::sync::Arc;
use std::time::SystemTime;
// Additional imports for web server
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Json,
    Router,
};
use serde::Deserialize;
use tera::{Context, Tera};
use tower_http::trace::TraceLayer;
use tracing::{info, error};
use once_cell::sync::Lazy;
use rust_embed::RustEmbed;
use std::sync::Mutex as StdMutex;
use fnv::FnvHasher;
use std::hash::Hasher;

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
    /// Start a web server interface
    Serve,
    /// Clear stored cookies
    ClearCookies,
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
    
    /// Maximum number of images to download (0 = unlimited)
    #[arg(short, long, default_value = "0")]
    max_images: usize,
}

// Web-specific structs
#[derive(RustEmbed)]
#[folder = "templates/"]
struct Templates;

#[derive(Clone)]
struct AppState {
    // Empty struct since we don't need state for our simple app
}

#[derive(Deserialize, Debug, Clone)]
struct DownloadRequest {
    url: String,
    genre: Option<String>,
    query: Option<String>,
    quality: String,
    max_images: Option<usize>,
}

impl From<DownloadRequest> for DownloadArgs {
    fn from(request: DownloadRequest) -> Self {
        Self {
            url: request.url,
            genre: request.genre,
            query: request.query,
            quality: request.quality,
            max_images: request.max_images.unwrap_or(0),
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

/// Cookie consent request from the web interface
#[derive(Deserialize, Debug)]
struct CookieConsentRequest {
    consent: bool,
}

/// Handler for cookie consent from the web interface
async fn cookie_consent_handler(
    Json(payload): Json<CookieConsentRequest>,
) -> impl IntoResponse {
    info!("Received cookie consent: {}", payload.consent);
    
    if payload.consent {
        // Create consent file
        let consent_path = get_cookie_consent_path();
        match std::fs::File::create(&consent_path) {
            Ok(mut file) => {
                let timestamp = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                    Ok(duration) => duration.as_secs(),
                    Err(_) => 0,
                };
                let _ = writeln!(file, "Cookie consent granted at: {}", timestamp);
                info!("Created cookie consent file at {:?}", consent_path);
            },
            Err(e) => {
                error!("Failed to create cookie consent file: {}", e);
            }
        }
    } else {
        // Remove any cookie consent file and cookies
        let consent_path = get_cookie_consent_path();
        if consent_path.exists() {
            let _ = std::fs::remove_file(&consent_path);
        }
        
        // Also clear any cookies
        let _ = clear_cookies();
    }
    
    // Return success
    StatusCode::OK
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // If no subcommand is provided, use the default 'download' command with interactive prompts
    match cli.command {
        Some(Commands::Download(args)) => {
            // Check cookie consent before proceeding
            if !has_cookie_consent() {
                request_cookie_consent()?;
            }
            // Run the download command with the provided arguments
            download_images(&args).await?;
        }
        Some(Commands::Serve) => {
            // Check cookie consent before starting server
            if !has_cookie_consent() {
                request_cookie_consent()?;
            }
            // Run the web server
            run_server().await?;
        }
        Some(Commands::ClearCookies) => {
            // Clear stored cookies
            clear_cookies()?;
            println!("Pinterest cookies have been cleared successfully.");
        }
        None => {
            // Default behavior if no subcommand is provided - show help
            println!("Error: A subcommand is required.");
            println!("Use 'pinterest_downloader download --help' to see download options.");
            println!("Use 'pinterest_downloader serve' to start the web interface.");
            println!("Use 'pinterest_downloader clear-cookies' to remove saved Pinterest cookies.");
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Checks if the user has previously given cookie consent
fn has_cookie_consent() -> bool {
    let consent_file_path = get_cookie_consent_path();
    std::fs::metadata(consent_file_path).is_ok()
}

/// Gets the path to the cookie consent file
fn get_cookie_consent_path() -> PathBuf {
    let mut path = get_app_data_dir();
    path.push("cookie_consent");
    path
}

/// Gets the path to store cookies
fn get_cookies_path() -> PathBuf {
    let mut path = get_app_data_dir();
    path.push("pinterest_cookies.json");
    path
}

/// Gets the application data directory
fn get_app_data_dir() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| {
            println!("Could not determine home directory. Using current directory for data storage.");
            ".".to_string()
        }))
    });
    
    path.push("PinterestX");
    
    // Create directory if it doesn't exist
    if !path.exists() {
        let _ = std::fs::create_dir_all(&path);
    }
    
    path
}

/// Requests cookie consent from the user
fn request_cookie_consent() -> Result<()> {
    println!("\n===== COOKIE CONSENT NOTICE =====");
    println!("PinterestX would like to store Pinterest session cookies to:");
    println!("1. Improve image extraction capabilities");
    println!("2. Bypass anti-scraping measures");
    println!("3. Access content that requires authentication");
    println!();
    println!("These cookies will be stored locally on your computer and are not sent to any third parties.");
    println!("You can clear the cookies at any time using the 'clear-cookies' command.");
    println!();
    
    // Prompt for consent
    print!("Do you consent to storing Pinterest cookies? (yes/no): ");
    std::io::stdout().flush()?;
    
    let mut response = String::new();
    std::io::stdin().read_line(&mut response)?;
    
    let response = response.trim().to_lowercase();
    if response == "yes" || response == "y" {
        // Save consent
        let consent_path = get_cookie_consent_path();
        let mut file = std::fs::File::create(consent_path)?;
        let timestamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
        writeln!(file, "Cookie consent granted at: {}", timestamp)?;
        println!("Thank you. Cookie consent has been granted.");
        return Ok(());
    } else {
        println!("Cookie consent declined. PinterestX will function with limited capabilities.");
        println!("Note: Some Pinterest content may not be accessible without cookies.");
        println!("You can enable cookies later by rerunning the application.");
        return Ok(());
    }
}

/// Clears any stored Pinterest cookies
fn clear_cookies() -> Result<()> {
    let cookies_path = get_cookies_path();
    if cookies_path.exists() {
        std::fs::remove_file(cookies_path)?;
        println!("Pinterest cookies have been deleted.");
    } else {
        println!("No Pinterest cookies found to delete.");
    }
    
    Ok(())
}

/// Save cookies to disk
fn save_cookies(cookie_content: &str) -> Result<()> {
    // Only save cookies if we have consent
    if !has_cookie_consent() {
        return Ok(());
    }
    
    let cookies_path = get_cookies_path();
    let mut file = std::fs::File::create(cookies_path)?;
    file.write_all(cookie_content.as_bytes())?;
    println!("Pinterest cookies saved successfully.");
    
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

    let app_state = AppState {};  // Now an empty struct

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/download", post(download_handler))
        .route("/cookie-consent", post(cookie_consent_handler))
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
    
    // Also add the max_images to the context for display
    if let Some(max) = payload.max_images {
        if max > 0 {
            context.insert("max_images", &max);
        }
    }

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
    
    // Print max images limit if set
    if args.max_images > 0 {
        println!("Will download at most {} images", args.max_images);
    }

    let url = Url::parse(&args.url)?;
    
    // Check if URL is a search page
    let is_search_page = url.path().contains("/search/") || 
                         url.as_str().contains("q=") || 
                         url.as_str().contains("query=");
    
    // Check if it's a modern search page with source_module_id                    
    let is_modern_search = url.as_str().contains("source_module_id");
                        
    if is_search_page {
        println!("Detected Pinterest search page, extracting search results...");
        if is_modern_search {
            println!("Detected modern Pinterest search format with source_module_id");
        }
    }
    
    // Try multiple approaches to get image URLs
    let mut image_urls = Vec::new();
    
    // Approach 0: Direct Pinterest data extraction (2024 method) - Try this first
    println!("Attempting direct Pinterest data extraction (2024 approach)...");
    match try_direct_pinterest_extraction(&url).await {
        Ok(direct_urls) => {
            println!("Successfully extracted {} images with direct method", direct_urls.len());
            image_urls = direct_urls;
        },
        Err(e) => {
            println!("Direct extraction failed: {}. Trying other methods.", e);
        }
    }
    
    // Approach 1: For modern search URLs, try the specialized method
    if is_modern_search {
        println!("Attempting specialized modern search approach...");
        match try_fetch_from_modern_search(&url).await {
            Ok(modern_urls) => {
                println!("Successfully fetched {} images using modern search approach", modern_urls.len());
                image_urls = modern_urls;
            },
            Err(e) => {
                println!("Modern search approach failed: {}. Trying other methods.", e);
            }
        }
    }
    
    // Approach 2: Try fetching directly from Pinterest API if it's a search URL
    if image_urls.is_empty() && is_search_page {
        println!("Attempting to fetch images via Pinterest API...");
        match try_fetch_from_pinterest_api(&url).await {
            Ok(api_urls) => {
                println!("Successfully fetched {} images from Pinterest API", api_urls.len());
                image_urls = api_urls;
            },
            Err(e) => {
                println!("API approach failed: {}. Falling back to HTML parsing.", e);
            }
        }
    }
    
    // Approach 3: If other methods didn't work, use regular HTML parsing
    if image_urls.is_empty() {
        let html_content = fetch_page(&url).await?;
        println!("Successfully fetched page content ({} bytes).", html_content.len());
        image_urls = extract_image_urls(&html_content, is_search_page)?;
    }
    
    println!("Found {} unique image URLs.", image_urls.len());

    if image_urls.is_empty() {
        println!("No images found to download.");
        return Ok(());
    }
    
    // Apply max_images limit if set
    if args.max_images > 0 && image_urls.len() > args.max_images {
        println!("Limiting to {} images as requested.", args.max_images);
        image_urls.truncate(args.max_images);
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
    
    // Track which images we've downloaded already using a hash of the file content
    let mut downloaded_hashes = HashSet::new();
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
                match download_image_with_deduplication(&client, &img_url, &dest_path, &mut downloaded_hashes).await {
                    Ok(true) => {
                        println!(" -> Success");
                        download_count += 1;
                    },
                    Ok(false) => {
                        println!(" -> Skipped (duplicate of already downloaded image)");
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

    println!("\nFinished downloading {} unique images.", download_count);

    Ok(())
}

/// Try to fetch images directly from Pinterest API for search pages
async fn try_fetch_from_pinterest_api(url: &Url) -> Result<Vec<String>> {
    // Extract search query from the URL
    let search_query = url.query_pairs()
        .find_map(|(key, value)| {
            if key == "q" {
                Some(value.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("No search query found in URL"))?;
    
    println!("Extracted search query: {}", search_query);
    
    // Try to construct an API URL 
    // Note: This is a best-guess approach as Pinterest doesn't have a public API
    let api_url = format!(
        "https://www.pinterest.com/resource/BaseSearchResource/get/?source_url=/search/pins/?q={}&data=%7B%22options%22%3A%7B%22query%22%3A%22{}%22%2C%22scope%22%3A%22pins%22%2C%22filters%22%3A%7B%7D%7D%2C%22context%22%3A%7B%7D%7D",
        search_query, search_query
    );
    
    println!("Trying Pinterest API URL: {}", api_url);
    
    // Setup a Pinterest-like request
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()?;
    
    let response = client
        .get(&api_url)
        .header("Accept", "application/json")
        .header("Referer", "https://www.pinterest.com/")
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await?;
    
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("API request failed: {}", response.status()));
    }
    
    // Try to parse the JSON response
    let json_text = response.text().await?;
    println!("Got API response ({} bytes)", json_text.len());
    
    // Extract image URLs from the response
    let mut urls = HashSet::new();
    find_image_urls_in_text(&json_text, &mut urls);
    
    // Process URLs for highest quality
    let mut processed_urls: Vec<String> = urls.into_iter()
        .map(|url| improve_image_quality(&url))
        .collect();
    
    // Remove duplicates
    processed_urls.sort();
    processed_urls.dedup();
    
    Ok(processed_urls)
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
/// Updated to handle modern Pinterest HTML structure.
fn extract_image_urls(html_content: &str, is_search_page: bool) -> Result<Vec<String>> {
    let document = ScraperHtml::parse_document(html_content);
    let mut urls = HashSet::new(); // Use HashSet to avoid duplicates

    // Strategy 1: Look for JSON data in scripts - usually most reliable
    println!("Searching for image URLs in embedded JSON data...");
    let script_selector = Selector::parse("script").map_err(|e| anyhow::anyhow!("Invalid selector: {}", e))?;
    
    for script in document.select(&script_selector) {
        let script_content = script.inner_html();
        // Check for various JSON structures containing image data
        if script_content.contains("\"images\"") || 
           script_content.contains("\"image_url\"") || 
           script_content.contains("\"orig\"") ||
           script_content.contains("\"url\":") ||
           // Pinterest search specific JSON data
           (is_search_page && (
               script_content.contains("\"resource_response\"") || 
               script_content.contains("\"data\"") || 
               script_content.contains("\"results\"") ||
               script_content.contains("\"pins\"") ||
               script_content.contains("\"items\"")
           )) {
            // Try to find JSON-like content
            if let Some(json_start) = script_content.find('{') {
                let potential_json = &script_content[json_start..];
                // Very basic approach - find URL patterns in the script content
                find_image_urls_in_text(potential_json, &mut urls);
            }
        }
    }

    // Strategy 2: Direct raw search for image URL patterns in the HTML - often works well with search pages
    if urls.is_empty() || is_search_page {
        println!("Performing raw HTML search for Pinterest image URLs...");
        // Directly search for Pinterest image URL patterns in the raw HTML
        let raw_patterns = [
            "https://i.pinimg.com/originals/",
            "https://i.pinimg.com/736x/",
            "https://i.pinimg.com/474x/",
            "https://i.pinimg.com/236x/"
        ];
        
        for pattern in raw_patterns {
            let mut start_idx = 0;
            while let Some(idx) = html_content[start_idx..].find(pattern) {
                let idx = start_idx + idx;
                // Find the end of the URL (usually a quote, space, or closing bracket)
                if let Some(end_idx) = html_content[idx..].find(|c| c == '"' || c == '\'' || c == ' ' || c == ')' || c == '}') {
                    let url = &html_content[idx..(idx + end_idx)];
                    if is_pinterest_image_url(url) {
                        urls.insert(url.to_string());
                    }
                }
                // Move past this URL for the next iteration
                start_idx = idx + pattern.len();
                if start_idx >= html_content.len() {
                    break;
                }
            }
        }
    }

    // Strategy 3: Look for modern Pinterest image containers
    if urls.is_empty() || is_search_page {
        println!("Searching for images in container elements...");
        // Multiple selector types to try to capture different Pinterest layouts
        let selectors = [
            // Search page specific selectors (2023-2024 versions)
            ".GrowthUnauthPinImage img", 
            ".PinImage img",
            ".pinWrapper img",
            "div[data-test-id=\"pin\"] img",
            ".searchImgContainer img",
            ".gridCentered img",
            // Generic img tags with source set
            "img[src*=\"pinimg.com\"]",
            "img[data-src*=\"pinimg.com\"]",
            // Search page specific selectors
            ".SearchPageContent img",
            ".Grid__Item img",
            "div[role='list'] div[role='listitem'] img",
            // Fallback selectors for various Pinterest layouts
            "div.Pin img",
            ".pinHolder img",
            ".pinImageWrapper img",
            "[data-test-id=\"pinrep-image\"]",
            ".GrowthPinImage img"
        ];

        for selector_str in selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                for img in document.select(&selector) {
                    // Try multiple attribute names Pinterest might use for image URLs
                    let src_attributes = ["src", "data-src", "srcset", "data-srcset"];
                    for attr in src_attributes {
                        if let Some(src) = img.value().attr(attr) {
                            // For srcset, we need to extract the URL from the format
                            if attr == "srcset" || attr == "data-srcset" {
                                for srcset_part in src.split(',') {
                                    if let Some(url) = srcset_part.split_whitespace().next() {
                                        if is_pinterest_image_url(url) {
                                            urls.insert(url.to_string());
                                        }
                                    }
                                }
                            } else if is_pinterest_image_url(src) {
                                urls.insert(src.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Strategy 4: Fallback to basic image search
    if urls.is_empty() {
        println!("Falling back to basic image tag search...");
        let img_selector = Selector::parse("img").map_err(|e| anyhow::anyhow!("Invalid selector: {}", e))?;

        for element in document.select(&img_selector) {
            for attr_name in ["src", "data-src", "srcset", "data-srcset"] {
                if let Some(src) = element.value().attr(attr_name) {
                    if is_pinterest_image_url(src) {
                        urls.insert(src.to_string());
                    } else if src.contains("pinterest") || src.contains("pinimg") {
                        // Check for partial URLs that might be Pinterest related
                        println!("Found potential Pinterest-related image: {}", src);
                    }
                }
            }
        }
    }
    
    // Process image URLs to get highest available quality
    let mut processed_urls: Vec<String> = urls.into_iter()
        .map(|url| improve_image_quality(&url))
        .collect();
    
    if processed_urls.is_empty() {
        // Add more detailed diagnostic information
        println!("Warning: No image URLs found using current selectors. The HTML structure might have changed.");
        println!("Consider examining the Pinterest page source to update the selectors.");
        println!("DEBUG: Saving a sample of the HTML for analysis...");
        
        // Save a sample of the HTML for debugging (first 2000 chars)
        if html_content.len() > 100 {
            let sample_size = std::cmp::min(2000, html_content.len());
            let sample = &html_content[0..sample_size];
            println!("HTML Sample (first {} chars): {}", sample_size, sample);
        }
    } else {
        // Remove duplicates that might have been created during quality improvement
        processed_urls.sort();
        processed_urls.dedup();
        println!("After processing, found {} unique image URLs", processed_urls.len());
    }

    Ok(processed_urls)
}

/// Finds image URLs in text/JSON content
fn find_image_urls_in_text(text: &str, urls: &mut HashSet<String>) {
    // Common Pinterest image URL patterns
    let patterns = [
        "https://i.pinimg.com/originals/",
        "https://i.pinimg.com/736x/",
        "https://i.pinimg.com/474x/",
        "https://i.pinimg.com/236x/",
        "https://www.pinimg.com/",
        "https://pin.it/"
    ];
    
    for pattern in patterns {
        let mut start_idx = 0;
        while let Some(idx) = text[start_idx..].find(pattern) {
            let idx = start_idx + idx;
            // Find the end of the URL (usually a quote or closing bracket)
            if let Some(end_idx) = text[idx..].find(|c| c == '"' || c == '\'' || c == ' ' || c == ')' || c == '}') {
                let url = &text[idx..(idx + end_idx)];
                if is_pinterest_image_url(url) {
                    urls.insert(url.to_string());
                }
            }
            // Move past this URL for the next iteration
            start_idx = idx + pattern.len();
            if start_idx >= text.len() {
                break;
            }
        }
    }
}

/// Checks if a URL is likely a Pinterest image URL
fn is_pinterest_image_url(url: &str) -> bool {
    let url = url.trim();
    
    // More permissive check for Pinterest image URLs
    (url.contains("pinimg.com") || url.contains("pin.it")) &&
    (url.ends_with(".jpg") || url.ends_with(".jpeg") || 
     url.ends_with(".png") || url.ends_with(".webp") || 
     url.ends_with(".gif") || url.contains("/originals/") ||
     url.contains("/736x/") || url.contains("/474x/") || url.contains("/236x/") ||
     // Additional checks for newer URL formats
     url.contains("images.") || url.contains("media-amazon") ||
     url.contains("i.pinimg.com"))
}

/// Attempts to improve the quality of an image URL
fn improve_image_quality(url: &str) -> String {
    // Try to replace lower quality URL paths with originals
    let improved = url
        .replace("/236x/", "/originals/")
        .replace("/474x/", "/originals/")
        .replace("/736x/", "/originals/");
    
    // If the URL doesn't already have quality indicators, check if we can add them
    if !improved.contains("/originals/") && !improved.contains("/736x/") {
        // Add originals path if it looks like a Pinterest image URL pattern
        if improved.contains("pinimg.com") && improved.contains("/") {
            // Try to insert originals into the path
            let parts: Vec<&str> = improved.split('/').collect();
            if parts.len() >= 4 {
                // Very basic attempt to insert quality - would need better path analysis for production
                let mut result = String::new();
                let mut inserted = false;
                for (i, part) in parts.iter().enumerate() {
                    if !inserted && i > 2 && i < parts.len() - 1 {
                        result.push_str("/originals/");
                        inserted = true;
                    } else {
                        result.push_str("/");
                        result.push_str(part);
                    }
                }
                if inserted {
                    return result.trim_start_matches('/').to_string();
                }
            }
        }
    }
    
    improved
}

/// Downloads an image from a URL, checks for duplicates, and saves it to a destination path.
/// Returns Ok(true) if image was downloaded, Ok(false) if it was a duplicate, or an Error.
async fn download_image_with_deduplication(
    client: &Client, 
    url: &Url, 
    dest_path: &Path,
    downloaded_hashes: &mut HashSet<u64>
) -> Result<bool> {
    let response = client.get(url.clone()).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Failed to download image: {}", response.status()));
    }

    let content = response.bytes().await?;
    
    // Compute a simple hash of the image content to detect duplicates
    // We use a simple FNV hash here, but you could use a more sophisticated image hash
    let mut hasher = FnvHasher::default();
    hasher.write(&content);
    let content_hash = hasher.finish();
    
    // If we've already downloaded this image (by content), skip it
    if downloaded_hashes.contains(&content_hash) {
        return Ok(false);
    }
    
    // Otherwise, save the image and record its hash
    downloaded_hashes.insert(content_hash);
    let mut file = File::create(dest_path).await?;
    file.write_all(&content).await?;

    Ok(true)
}

/// Try to fetch images from modern Pinterest search pages with source_module_id
async fn try_fetch_from_modern_search(url: &Url) -> Result<Vec<String>> {
    println!("Parsing modern Pinterest search URL: {}", url.as_str());
    
    // Extract the module ID 
    let module_id = url.query_pairs()
        .find_map(|(key, value)| {
            if key == "source_module_id" {
                Some(value.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("No source_module_id found in URL"))?;
    
    // Extract the search query
    let search_query = url.query_pairs()
        .find_map(|(key, value)| {
            if key == "q" {
                Some(value.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("No search query found in URL"))?;
    
    println!("Module ID: {}", module_id);
    println!("Search query: {}", search_query);
    
    // Create a more refined client with necessary headers
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()?;
    
    // First, fetch the search page to get any necessary cookies/tokens
    println!("Fetching search page to initialize session...");
    let initial_response = client
        .get(url.as_str())
        .header("Accept", "text/html,application/xhtml+xml,application/xml")
        .header("Referer", "https://www.pinterest.com/")
        .send()
        .await?;
    
    if !initial_response.status().is_success() {
        return Err(anyhow::anyhow!("Failed to fetch search page: {}", initial_response.status()));
    }
    
    let html_content = initial_response.text().await?;
    println!("Got initial page ({} bytes), extracting from HTML...", html_content.len());
    
    // Extract image URLs directly from the HTML content
    let mut urls = HashSet::new();
    
    // Look for image URLs in the HTML content
    let raw_patterns = [
        "https://i.pinimg.com/originals/",
        "https://i.pinimg.com/736x/",
        "https://i.pinimg.com/474x/",
        "https://i.pinimg.com/236x/"
    ];
    
    for pattern in raw_patterns {
        let mut start_idx = 0;
        while let Some(idx) = html_content[start_idx..].find(pattern) {
            let idx = start_idx + idx;
            // Find the end of the URL (usually a quote or closing bracket)
            if let Some(end_idx) = html_content[idx..].find(|c| c == '"' || c == '\'' || c == ' ' || c == ')' || c == '}') {
                let url = &html_content[idx..(idx + end_idx)];
                if is_pinterest_image_url(url) {
                    urls.insert(url.to_string());
                }
            }
            // Move past this URL for the next iteration
            start_idx = idx + pattern.len();
            if start_idx >= html_content.len() {
                break;
            }
        }
    }
    
    // Also try to extract JSON data that might contain image URLs
    println!("Extracting from JSON data in scripts...");
    let document = ScraperHtml::parse_document(&html_content);
    let script_selector = Selector::parse("script").map_err(|e| anyhow::anyhow!("Invalid selector: {}", e))?;
    
    for script in document.select(&script_selector) {
        let script_content = script.inner_html();
        if script_content.contains("\"sourceUrl\"") || 
           script_content.contains("\"images\"") || 
           script_content.contains("\"image_url\"") {
            find_image_urls_in_text(&script_content, &mut urls);
        }
    }
    
    // Process URLs for highest quality
    let mut processed_urls: Vec<String> = urls.into_iter()
        .map(|url| improve_image_quality(&url))
        .collect();
    
    // Remove duplicates
    processed_urls.sort();
    processed_urls.dedup();
    
    println!("Found {} unique images from modern search page", processed_urls.len());
    Ok(processed_urls)
}

/// Most effective way to extract Pinterest images in 2024 by directly accessing their internal
/// data structure from within the page HTML
async fn try_direct_pinterest_extraction(url: &Url) -> Result<Vec<String>> {
    println!("Using direct Pinterest data extraction method for URL: {}", url);
    
    // Create a client with cookie store support
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .cookie_store(true)
        .build()?;
    
    // First GET request to get cookies and any initial data
    println!("Making initial request to Pinterest...");
    
    // Apply cookies manually if we have consent
    let mut request = client.get(url.as_str());
    
    if has_cookie_consent() {
        // Try to load cookies from file
        let cookies_path = get_cookies_path();
        if cookies_path.exists() {
            if let Ok(cookie_content) = std::fs::read_to_string(&cookies_path) {
                // Apply cookies to the request
                request = request.header(reqwest::header::COOKIE, cookie_content);
                println!("Applied stored cookies to request");
            }
        }
    }
    
    // Send the request
    let response = request.send().await?;
    
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Failed to access Pinterest: {}", response.status()));
    }
    
    // Save the cookies if we have consent
    if has_cookie_consent() {
        // Extract cookies from response
        let cookie_headers: Vec<_> = response.headers().get_all("set-cookie").iter().collect();
        if !cookie_headers.is_empty() {
            println!("Received cookies from Pinterest. Saving them for future requests.");
            let mut all_cookies = String::new();
            for header in cookie_headers {
                if let Ok(cookie_str) = header.to_str() {
                    all_cookies.push_str(cookie_str);
                    all_cookies.push(';');
                }
            }
            
            if !all_cookies.is_empty() {
                if let Err(e) = save_cookies(&all_cookies) {
                    println!("Warning: Failed to save cookies: {}", e);
                }
            }
        }
    }
    
    let html_content = response.text().await?;
    println!("Received page content ({} bytes)", html_content.len());
    
    // Extract image URLs using three different methods
    let mut all_urls = HashSet::new();
    
    // Method 1: Search for specific image constants in initial_state data
    if let Some(pos) = html_content.find("\"initial_state\"") {
        // Find the actual JSON object start
        if let Some(start) = html_content[pos..].find('{') {
            let json_start = pos + start;
            
            // Try to find the end of this JSON object - this is tricky
            let mut bracket_count = 1;
            let mut json_end = json_start + 1;
            
            for (i, c) in html_content[json_start+1..].char_indices() {
                if c == '{' {
                    bracket_count += 1;
                } else if c == '}' {
                    bracket_count -= 1;
                    if bracket_count == 0 {
                        json_end = json_start + i + 2;
                        break;
                    }
                }
            }
            
            if json_end > json_start {
                let json_data = &html_content[json_start..json_end];
                println!("Found initial_state data ({} bytes)", json_data.len());
                
                // Look for all URLs in the JSON data
                find_image_urls_in_text(json_data, &mut all_urls);
            }
        }
    }
    
    // Method 2: Look directly for "original": { "url": "https://... patterns
    // This is specific to Pinterest's image data structure
    println!("Searching for Pinterest's original image URL patterns...");
    let mut start_pos = 0;
    let pattern = "\"original\"";
    
    while let Some(pos) = html_content[start_pos..].find(pattern) {
        let pos = start_pos + pos;
        start_pos = pos + pattern.len();
        
        // Look for "url": nearby
        if let Some(url_pos) = html_content[pos..pos+100].find("\"url\"") {
            let url_pos = pos + url_pos;
            
            // Find the URL value
            if let Some(quote_pos) = html_content[url_pos..url_pos+50].find(':') {
                let url_value_start = url_pos + quote_pos;
                
                // Skip whitespace and get to the quote
                let mut i = url_value_start + 1;
                let chars: Vec<char> = html_content[url_value_start+1..].chars().collect();
                let mut char_idx = 0;
                
                // Skip whitespace characters
                while char_idx < chars.len() && (chars[char_idx] == ' ' || chars[char_idx] == '\n') {
                    char_idx += 1;
                    i += chars[char_idx].len_utf8();
                }
                
                // Check if we have a quote character
                if char_idx < chars.len() && chars[char_idx] == '"' {
                    i += 1; // Skip the opening quote
                    let url_start = i;
                    
                    // Find the closing quote
                    if let Some(end_quote) = html_content[url_start..].find('"') {
                        let url_end = url_start + end_quote;
                        let url = &html_content[url_start..url_end];
                        
                        if is_pinterest_image_url(url) || url.contains("pinimg.com") {
                            all_urls.insert(url.to_string());
                        }
                    }
                }
            }
        }
    }
    
    // Method 3: General image URL pattern search throughout the entire HTML
    let image_patterns = [
        "https://i.pinimg.com/originals/",
        "https://i.pinimg.com/736x/",
        "https://i.pinimg.com/474x/",
        "https://i.pinimg.com/236x/"
    ];
    
    for pattern in image_patterns {
        let mut start_idx = 0;
        while let Some(idx) = html_content[start_idx..].find(pattern) {
            let idx = start_idx + idx;
            
            // Find the end of the URL
            if let Some(end_idx) = html_content[idx..].find(|c: char| c == '"' || c == '\'' || c == ' ' || c == ')' || c == '}' || c == '\\') {
                let url = &html_content[idx..(idx + end_idx)];
                if is_pinterest_image_url(url) {
                    all_urls.insert(url.to_string());
                }
            }
            
            // Move past this URL for the next iteration
            start_idx = idx + pattern.len();
            if start_idx >= html_content.len() {
                break;
            }
        }
    }
    
    // Process the URLs to get the highest quality versions
    let mut processed_urls: Vec<String> = all_urls.into_iter()
        .map(|url| improve_image_quality(&url))
        .collect();
    
    // Remove duplicates and sort
    processed_urls.sort();
    processed_urls.dedup();
    
    if processed_urls.is_empty() {
        return Err(anyhow::anyhow!("Could not extract any image URLs"));
    }
    
    println!("Direct extraction found {} unique images", processed_urls.len());
    Ok(processed_urls)
}
