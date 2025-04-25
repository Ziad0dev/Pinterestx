# PinterestX

A high-performance image downloader for Pinterest written in Rust.

## Overview

PinterestX is a tool for downloading high-quality images from Pinterest boards, pins, and user profiles. It provides both a command-line interface and a web-based UI, allowing for flexible usage based on your preferences.

## Features

- Download images from Pinterest boards, pins, and user profiles
- Organize images in customizable folder structures by genre and collection
- Multiple image quality options (original, large, medium, small)
- Web-based UI with modern design for easy use
- Command-line interface for automation and scripting
- Concurrent downloads for better performance
- Automatic error handling and retry mechanisms

## Requirements

- Rust 1.70 or higher
- Cargo package manager
- Internet connection
- Linux, macOS, or Windows operating system

## Installation

### From Source

1. Clone the repository:
```bash
git clone https://github.com/yourusername/Pinterestx.git
cd Pinterestx
```

2. Build the project:
```bash
cargo build --release
```

3. The compiled binary will be available at `target/release/pinterest_downloader`

### Using Cargo

```bash
cargo install pinterest_downloader
```

## Usage

### Web Interface

1. Start the web server:
```bash
./target/release/pinterest_downloader serve
```
Or if installed via Cargo:
```bash
pinterest_downloader serve
```

2. Open your browser and navigate to http://127.0.0.1:3000
3. Enter a Pinterest URL and optionally specify genre and collection name
4. Select your preferred image quality
5. Click "Download Images" to start the download process

### Command Line

```bash
# Basic usage
pinterest_downloader download --url "https://www.pinterest.com/username/boardname/"

# Specify genre and collection name for organization
pinterest_downloader download --url "https://www.pinterest.com/username/boardname/" --genre "Wallpapers" --query "Nature"

# Choose image quality
pinterest_downloader download --url "https://www.pinterest.com/username/boardname/" --quality "original"
```

Available quality options:
- `original`: Highest quality available
- `736x`: Large size (default)
- `474x`: Medium size
- `236x`: Small size

## Image Organization

Downloaded images are saved to your Pictures directory in the following structure:
```
~/Pictures/Pinterestx/<Genre>/<Collection>/image_001.jpg
```

If genre and collection name are not specified, they default to "Uncategorized" and "Pinterest" respectively.

## Maintenance and Troubleshooting

### Updating the Application

To update to the latest version:

```bash
git pull
cargo build --release
```

Or with Cargo:
```bash
cargo install --force pinterest_downloader
```

### Common Issues

1. **No images found**: Pinterest occasionally updates their HTML structure. If no images are being found, file an issue on the GitHub repository.

2. **Rate limiting**: Pinterest may block your IP if you download too many images in a short time. Use the tool responsibly and consider adding delays between downloads.

3. **Invalid URL**: Ensure you're using the full URL including the "https://" prefix.

### Log Files

The application logs important events to the console. For web server mode, all download operations are logged to help with troubleshooting.

## Project Structure

- `src/`: Main source code
- `templates/`: HTML templates for the web interface
- `src/main.rs`: Entry point and CLI implementation
- `templates/index.html`: Main web interface template

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## Acknowledgments

- Built with [Rust](https://www.rust-lang.org/)
- Web interface powered by [Axum](https://github.com/tokio-rs/axum) and [HTMX](https://htmx.org/)
- Template rendering with [Tera](https://tera.netlify.app/)
