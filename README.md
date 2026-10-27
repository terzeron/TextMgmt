# Introduction
This software is for management of text files. It is optimized and tested for ten thousand files. It supports for EPUB, PDF, HTML, image and text files.

# Prerequisites

## backend

### Local setup
* Install Rust 1.91 or newer, LibreOffice Writer, Poppler utilities, Java, and PDFium.
* Copy `.env.example` to `.env` and configure the backend environment variables.
* Start the Rust API server from the repository root:
  * `cargo run --release -p backend -- serve --addr 0.0.0.0:8020`
* `backend/startup.sh` builds and starts the Rust server in the background.

## frontend
* `cd frontend`
* `cp .env.production.example .env.production`
* check .env.production and modify environment variables.
* `npm install`
* `npm run build`

# License
[LICENSE](LICENSE)
