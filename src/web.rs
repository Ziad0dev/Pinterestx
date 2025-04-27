use crate::downloader::{download_pinterest_images, DownloadConfig, DownloadError};
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
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
use tower_http::services::ServeDir;

// Shared application state
#[derive(Clone)]
struct AppState {
    tera: Arc<Mutex<Tera>>,
}

#[derive(Deserialize, Debug)]
pub struct DownloadRequest {
    pub url: String,
    pub genre: String,
    pub query: String,
    pub quality: String,
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
        .route("/", get(index_handler))
        .route("/download", post(download_handler))
        .layer(TraceLayer::new_for_http()) // Apply logging
        .with_state(app_state) // Pass state if needed by handlers
        .nest_service("/static", ServeDir::new("static"));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("Web server listening on {}", addr);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn index_handler() -> Html<&'static str> {
    Html(include_str!("../templates/index.html"))
}

async fn download_handler(Form(form): Form<DownloadRequest>) -> impl IntoResponse {
    match Url::parse(&form.url) {
        Ok(url) => {
            let config = DownloadConfig {
                url,
                genre: form.genre,
                query: form.query,
                quality: form.quality,
            };

            // Spawn a task to handle the download
            tokio::spawn(async move {
                match download_pinterest_images(config).await {
                    Ok(_) => println!("Download completed successfully"),
                    Err(e) => eprintln!("Download failed: {}", e),
                }
            });

            Html("<div class='success'>Download started!</div>")
        }
        Err(e) => Html(format!("<div class='error'>Invalid URL: {}</div>", e).as_str()),
    }
} 