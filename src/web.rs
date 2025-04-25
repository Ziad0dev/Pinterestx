use crate::downloader::{download_pinterest_images, DownloadConfig, DownloadError};
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Form,
    Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tera::{Context, Tera};
use tokio::sync::Mutex;
use tower_http::trace::TraceLayer;
use tracing::info;
use url::Url;
use once_cell::sync::Lazy;
use rust_embed::RustEmbed;
use std::sync::Mutex as StdMutex;

// Shared application state
#[derive(Clone)]
struct AppState {
    tera: Arc<Mutex<Tera>>,
}

#[derive(Deserialize, Debug)]
struct DownloadRequest {
    url: String,
    genre: String,
    query: String,
    quality: String,
}

#[derive(RustEmbed)]
#[folder = "templates/"]
struct Assets;

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
    for filename in Assets::iter() {
        if let Some(file) = Assets::get(&filename) {
            if let Ok(content) = std::str::from_utf8(file.data.as_ref()) {
                if tera.add_raw_template(&filename, content).is_err() {
                    eprintln!("Failed to load embedded template: {}", filename);
                }
            } else {
                 eprintln!("Failed to read embedded template as UTF-8: {}", filename);
            }
        } else {
            eprintln!("Failed to get embedded template file: {}", filename);
        }
    }
    tera.autoescape_on(vec![".html"]); // Adjust as needed
    StdMutex::new(tera)
});

pub async fn run_server() -> anyhow::Result<()> {
    // Initialize tracing (logging)
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false) // Don't print module paths
        .init();

    // Trigger lazy initialization of templates to catch errors early
    if TEMPLATES.lock().is_err() {
        return Err(anyhow::anyhow!("Failed to initialize template engine"));
    }

    let app_state = AppState {
        // Tera instance is globally initialized via Lazy, no need to clone into state
        // If state needed specific Tera config later, adjust this.
        tera: Arc::new(Mutex::new(Tera::default())), // Placeholder, actual rendering uses static TEMPLATES
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/download", axum::routing::post(start_download))
        .layer(TraceLayer::new_for_http()) // Apply logging
        .with_state(app_state); // Pass state if needed by handlers

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("Web server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> impl IntoResponse {
    let context = Context::new();
    HtmlTemplate("index.html".to_string(), context)
}

async fn start_download(
    State(_state): State<AppState>, // Access state if needed later
    Form(payload): Form<DownloadRequest>,
) -> impl IntoResponse {
    info!("Received download request: {:?}", payload);
    let mut context = Context::new();
    context.insert("url", &payload.url);

    match Url::parse(&payload.url) {
        Ok(url) => {
            let config = DownloadConfig {
                url,
                genre: payload.genre.clone(),
                query: payload.query.clone(),
                quality: payload.quality.clone(),
            };

            // Spawn the download task in the background
            tokio::spawn(async move {
                info!("Starting background download for {}", config.url);
                match download_pinterest_images(config).await {
                    Ok(_) => info!("Background download completed successfully."),
                    Err(e) => {
                        eprintln!("Background download failed: {}", e); // Log error server-side
                        // TODO: Implement a way to notify the user about background failures
                        // (e.g., WebSockets, status endpoint, database flag)
                    }
                }
            });

            context.insert("success", &true);
        }
        Err(e) => {
            context.insert("success", &false);
            context.insert("error_message", &format!("Invalid URL: {}", e));
        }
    }

    // Return an immediate response indicating the task was started (or failed validation)
    HtmlTemplate("partials/download_results.html".to_string(), context)
} 