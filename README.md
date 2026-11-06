# Introduction
This software is for management of text files. It is optimized and tested for ten thousand files. It supports for EPUB, PDF, HTML, image and text files.

# Prerequisites

## backend

### Local setup
* Install Rust 1.91 or newer, LibreOffice Writer, Java, and Noto CJK fonts.
  PDF는 MuPDF Rust library와 lopdf로 처리한다. `fonts-noto-cjk`와 native build headers가 필요하다.
* Build the isolated PDF worker with `cargo build -p pdf_engine --bin tm-pdf-worker` before running individual backend tests.
  전체 빌드(`cargo build --workspace`)에는 worker가 포함된다. 자세한 패키지는 [PDF engine](docs/pdf-engine.md)을 본다.
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
