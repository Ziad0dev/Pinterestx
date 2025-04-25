use clap::{Parser, Subcommand};
use url::Url;
use anyhow::Result;

mod downloader;
mod web;
use downloader::{DownloadConfig, DownloadError};

#[derive(Parser, Debug)]
#[command(author, version, about = "CLI and Web UI for downloading Pinterest images.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the downloader directly from the command line.
    Download(DownloadArgs),
    /// Start the web server interface.
    Serve(ServeArgs),
}

/// Downloads images from a Pinterest URL (board, pin, or user)
/// and saves them to ~/Pictures/Pinterestx/<Genre>/<Query>/
#[derive(Parser, Debug)]
struct DownloadArgs {
    /// The Pinterest URL to download images from.
    #[arg(short, long)]
    url: String,

    /// The genre to classify the downloaded images under (e.g., "Landscapes", "SciFi").
    #[arg(short, long)]
    genre: String,

    /// The query name to classify the downloaded images under (e.g., "Mountains", "Cyberpunk").
    #[arg(short, long)]
    query: String,

    /// Desired image quality.
    /// Examples: "original", "736x", "474x", "236x". Defaults to "736x".
    #[arg(short, long, default_value = "736x")]
    quality: String,
}

#[derive(Parser, Debug)]
struct ServeArgs {
    // Placeholder for potential server configuration args later (e.g., port)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Download(args) => {
            // Validate URL early
            let url = Url::parse(&args.url)
                .map_err(|e| anyhow::anyhow!("Invalid URL '{}': {}", args.url, e))?;

            let config = DownloadConfig {
                url,
                genre: args.genre,
                query: args.query,
                quality: args.quality,
            };

            println!("Starting CLI download...");
            if let Err(e) = downloader::download_pinterest_images(config).await {
                eprintln!("Error during download process: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Serve(_args) => {
            println!("Starting web server...");
            if let Err(e) = web::run_server().await {
                eprintln!("Web server failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
} 