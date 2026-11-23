# Introduction
This software is for management of text files. It is optimized and tested for ten thousand files. It supports for EPUB, PDF, HTML, image and text files.

# Prerequisites

## backend

### Local setup
* Install Rust 1.95 or newer (pinned in `rust-toolchain.toml`), LibreOffice Writer, Java, and Noto CJK fonts.
  PDF는 MuPDF Rust library와 lopdf로 처리한다. `fonts-noto-cjk`와 native build headers가 필요하다.
* Build the isolated workers with `cargo build -p pdf_engine --bin tm-pdf-worker` and `cargo build -p file_indexer --bin tm-index-worker` before running individual backend or loader tests.
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

# 배포 절차

이 절차에서 development는 리뷰 전 임시배포, production은 리뷰·커밋 후 배포를 뜻한다.

1. 사람이 눈으로 확인할 변경은 관련 검증 후 저장소 루트에서 `./build.sh`를 실행하여 development 버전을 임시배포한다.
   rollout 결과와 배포 버전을 확인하고, 확인할 URL을 사용자에게 전달하여 리뷰를 받는다.
2. 리뷰가 끝나고 변경을 커밋한 뒤에는 `deploy_and_build`를 실행하여 production 버전을 배포한다.
   배포 후에도 rollout 결과와 배포 버전을 확인한다.

`npm run build`는 로컬 빌드만 수행한다. 사람이 확인할 development 배포를 대신하지 않는다.

# License
[LICENSE](LICENSE)
