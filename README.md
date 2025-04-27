# PinterestX

A versatile Pinterest image downloader tool written in Rust. PinterestX allows you to easily download images from Pinterest boards, pins, or search results both through a command-line interface and a web interface.

## Features

- Download images from any Pinterest URL (including search result pages)
- Automatically detect and extract highest quality image versions
- Smart duplicate detection to avoid downloading the same image multiple times
- Limit the number of images to download with the max-images parameter
- Organize downloads by genre and query
- Command-line interface for scripting and automation
- Simple web interface for easy use without coding knowledge
- Fast and efficient with minimal resource usage

## Installation

### Prerequisites

- Rust and Cargo (install from [rust-lang.org](https://www.rust-lang.org/tools/install))

### Building from Source

Clone the repository and build with Cargo:

```bash
git clone https://github.com/Ziad0dev/Pinterestx.git
cd Pinterestx
cargo build --release
```

The compiled binary will be available at `./target/release/pinterest_downloader`.

## Usage

### Command Line Interface

Download images from a Pinterest URL:

```bash
# Basic usage
./pinterest_downloader download --url "https://www.pinterest.com/username/boardname/"

# With genre and query for better organization
./pinterest_downloader download --url "https://www.pinterest.com/username/boardname/" --genre Art --query Landscapes

# Specify image quality
./pinterest_downloader download --url "https://www.pinterest.com/username/boardname/" --quality original

# Download from a search URL (up to 100 images)
./pinterest_downloader download --url "https://se.pinterest.com/search/pins/?q=dark%20gothic%20art%20wallpaper" --max-images 100
```

### Web Interface

Start the web server:

```bash
./pinterest_downloader serve
```

Then open `http://localhost:3000` in your browser to access the web interface.

## Image Organization

Images are saved to your Pictures directory under the following structure:

```
Pictures/
└── Pinterestx/
    └── [genre]/
        └── [query]/
            ├── image_001.jpg
            ├── image_002.jpg
            └── ...
```

## Project Structure

```
Pinterestx/
├── pinterest_downloader/
│   ├── src/
│   │   └── main.rs         # Main application code
│   ├── templates/          # Web templates
│   │   └── index.html      # Main web interface
│   │   └── partials/       # Partial templates
│   └── Cargo.toml          # Project dependencies
└── README.md
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

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
