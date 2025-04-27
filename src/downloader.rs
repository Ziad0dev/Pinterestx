use anyhow::{anyhow, Result};
use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use url::Url;

pub struct DownloadConfig {
    pub url: Url,
    pub genre: String,
    pub query: String,
    pub quality: String,
}

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("Network request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to create directory: {0}")]
    DirCreation(#[from] tokio::io::Error),
    #[error("Failed to parse URL: {0}")]
    UrlParseError(String),
    #[error("Invalid selector: {0}")]
    Selector(String),
    #[error("Failed to parse JSON: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("Image not found in expected JSON structure")]
    JsonStructure,
    #[error("Invalid image quality specified: {0}")]
    InvalidQuality(String),
    #[error("HTTP error: {0}")]
    Http(reqwest::StatusCode),
    #[error("Could not find Pictures directory")]
    PicturesDirNotFound,
    #[error("Generic error: {0}")]
    Any(#[from] anyhow::Error),
    #[error("Failed to fetch Pinterest content: {0}")]
    FetchError(String),
    #[error("Failed to extract images: {0}")]
    ExtractionError(String),
    #[error("Failed to download images: {0}")]
    DownloadError(String),
}

pub async fn download_pinterest_images(config: DownloadConfig) -> Result<(), DownloadError> {
    println!("Attempting to download images from: {}", config.url);
    println!(
        "Classifying under Genre: '{}', Query: '{}', Quality: '{}'",
        config.genre,
        config.query,
        config.quality
    );

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .build()?;

    let html_content = fetch_page(&client, &config.url).await?;
    println!(
        "Successfully fetched page content ({} bytes).",
        html_content.len()
    );

    let image_urls = extract_image_urls(&html_content, &config.quality)?;
    println!("Found {} potential image URLs.", image_urls.len());

    if image_urls.is_empty() {
        println!("No images found to download.");
        return Ok(());
    }

    let base_output_dir = get_output_dir(&config.genre, &config.query)?;

    // Create directories if they don't exist
    fs::create_dir_all(&base_output_dir).await?;
    println!("Saving images to: {}", base_output_dir.display());

    let mut download_count = 0;
    let total_images = image_urls.len();

    for (index, img_url_str) in image_urls.iter().enumerate() {
        match Url::parse(img_url_str) {
            Ok(img_url) => {
                let filename = generate_filename(&img_url, index + 1)?;
                let dest_path = base_output_dir.join(&filename);

                println!(
                    "Downloading [{}/{}] {} to {} ...",
                    index + 1,
                    total_images,
                    img_url.as_str(),
                    dest_path.display()
                );
                match download_image(&client, &img_url, &dest_path).await {
                    Ok(_) => {
                        println!(" -> Success");
                        download_count += 1;
                    }
                    Err(e) => println!(" -> Failed: {}", e),
                }

                // Add a small delay to be polite
                if index < total_images - 1 {
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

fn get_output_dir(genre: &str, query: &str) -> Result<PathBuf, DownloadError> {
    let pictures_dir = dirs::picture_dir().ok_or(DownloadError::PicturesDirNotFound)?;
    Ok(pictures_dir.join("Pinterestx").join(genre).join(query))
}

fn generate_filename(url: &Url, index: usize) -> Result<String, DownloadError> {
     // Try to get filename from URL path segment
     let filename_from_url = Path::new(url.path())
        .file_name()
        .and_then(|name| name.to_str());

    // Fallback to sequential naming if no filename in URL or it's generic
    if let Some(name) = filename_from_url {
        if !name.is_empty() && name.contains('.') { // Basic check
             return Ok(name.to_string());
        }
    }

    // Fallback: sequential naming
    let extension = Path::new(url.path())
        .extension()
        .and_then(|os_str| os_str.to_str())
        .unwrap_or("jpg"); // Default to jpg
    Ok(format!("image_{:04}.{}", index, extension))
}


async fn fetch_page(client: &Client, url: &Url) -> Result<String, DownloadError> {
    let response = client.get(url.clone()).send().await?;

    if !response.status().is_success() {
        return Err(DownloadError::Http(response.status()));
    }

    let body = response.text().await?;
    Ok(body)
}

fn extract_image_urls(html_content: &str, quality: &str) -> Result<Vec<String>, DownloadError> {
    let document = Html::parse_document(html_content);
    let mut urls = HashSet::new();

    // Strategy 1: Look for JSON data embedded in <script type="application/json" id="__PWS_INITIAL_PROPS__">
    // This is often the most reliable way, but requires inspecting the page source
    // to find the correct script tag and the JSON path to the image URLs.
    let json_selector = Selector::parse("script#initial-state, script[type='application/json']") // Common patterns
        .map_err(|e| DownloadError::Selector(format!("{:?}", e)))?;

    for script_element in document.select(&json_selector) {
        let json_text = script_element.inner_html();
        if let Ok(json_data) = serde_json::from_str::<Value>(&json_text) {
            // Example: Search for relevant keys within the JSON structure.
            // This part is highly dependent on Pinterest's current API/structure.
            // You would typically use json_data.pointer("/path/to/images") or similar.
            // For now, we just search recursively for likely image URLs.
            find_urls_in_json(&json_data, &mut urls);
        }
    }

    // Strategy 2: Fallback to scraping <img> tags
    if urls.is_empty() {
        println!("No URLs found in JSON, falling back to <img> tag scraping...");
        let img_selector =
            Selector::parse("img[src]").map_err(|e| DownloadError::Selector(format!("{:?}", e)))?;
        for element in document.select(&img_selector) {
            if let Some(src) = element.value().attr("src") {
                if src.contains("pinimg.com")
                    && (src.ends_with(".jpg") || src.ends_with(".png") || src.ends_with(".webp"))
                {
                    urls.insert(src.to_string());
                }
            }
        }
    }

    // Apply quality preference (simple replacement)
    let processed_urls: Vec<String> = urls.into_iter().map(|url| adjust_quality(&url, quality)).collect();

    if processed_urls.is_empty() {
        println!("Warning: No image URLs found using any method. The HTML structure might have changed.");
    }

    Ok(processed_urls)
}

// Recursive helper to find URLs within JSON
fn find_urls_in_json(data: &Value, urls: &mut HashSet<String>) {
    match data {
        Value::String(s) => {
            // Basic check for potential pinimg URLs
            if s.contains("pinimg.com") && (s.ends_with(".jpg") || s.ends_with(".png") || s.ends_with(".webp")) {
                if let Ok(parsed_url) = Url::parse(s) {
                    // Avoid adding thumbnail-like URLs directly if possible
                    if !parsed_url.path().contains("/avatar/") && !parsed_url.path().contains("/user/") {
                        urls.insert(s.clone());
                    }
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                find_urls_in_json(item, urls);
            }
        }
        Value::Object(map) => {
            for (_, value) in map {
                find_urls_in_json(value, urls);
            }
        }
        _ => {}
    }
}

fn adjust_quality(url: &str, quality: &str) -> String {
    // Simple quality adjustment based on common patterns.
    // Could be made more robust.
    let quality_segment = match quality {
        "original" | "originals" => "/originals/",
        "736x" => "/736x/",
        "474x" => "/474x/",
        "236x" => "/236x/",
        _ => "/736x/", // Default fallback
    };

    // Attempt to replace known resolution paths with the desired one
    url.replace("/236x/", quality_segment)
       .replace("/474x/", quality_segment)
       .replace("/736x/", quality_segment)
       .replace("/originals/", quality_segment)
}


async fn download_image(client: &Client, url: &Url, dest_path: &Path) -> Result<(), DownloadError> {
    let response = client.get(url.clone()).send().await?;

    if !response.status().is_success() {
        return Err(DownloadError::Http(response.status()));
    }

    let mut file = File::create(dest_path).await.map_err(DownloadError::DirCreation)?;
    let content = response.bytes().await?;
    file.write_all(&content).await.map_err(DownloadError::Io)?;

    Ok(())
} 