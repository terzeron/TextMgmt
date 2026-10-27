# Rust backend build. The PDFium wheel supplies the shared library used by pdfium-render.
FROM python:3.13-slim AS pdfium
RUN python -m pip install --no-cache-dir --only-binary=:all: pypdfium2==5.12.1

FROM rust:1.91-slim-bookworm AS builder
ARG APT_REFRESH=0
RUN : "apt refresh ${APT_REFRESH}" && \
    apt-get update && apt-get upgrade -y && \
    apt-get install -y --no-install-recommends build-essential cmake pkg-config libssl-dev ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock /app/
COPY .cargo /app/.cargo/
COPY backend /app/backend
COPY libs /app/libs
COPY utils /app/utils
RUN cargo build --locked --release -p backend -p loader

FROM debian:bookworm-slim AS epubcheck
ARG EPUBCHECK_VERSION=5.3.0
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl unzip && \
    curl -fsSL "https://github.com/w3c/epubcheck/releases/download/v${EPUBCHECK_VERSION}/epubcheck-${EPUBCHECK_VERSION}.zip" -o /tmp/epubcheck.zip && \
    unzip -q /tmp/epubcheck.zip -d /opt && \
    rm -rf /tmp/epubcheck.zip /var/lib/apt/lists/*

# Runtime includes only the Rust server, its loader subprocess, PDFium, and the
# external tools still required for office conversion and EPUB validation.
FROM debian:bookworm-slim
ARG APT_REFRESH=0
ARG EPUBCHECK_VERSION=5.3.0
ARG USER_ID=1000
ARG USER_NAME=terzeron

RUN : "apt refresh ${APT_REFRESH}" && \
    apt-get update && apt-get upgrade -y && \
    apt-get install -y --no-install-recommends \
        libreoffice-writer-nogui \
        default-jre-headless \
        poppler-utils \
        ca-certificates \
        libssl3 && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --uid "$USER_ID" --create-home "$USER_NAME" && \
    mkdir -p /app /books /comics /home/"$USER_NAME"/.cache /home/"$USER_NAME"/.config && \
    chown -R "$USER_ID:$USER_ID" /app /books /comics /home/"$USER_NAME"

COPY --from=builder /app/target/release/backend /usr/local/bin/backend
COPY --from=builder /app/target/release/loader /usr/local/bin/loader
COPY --from=pdfium /usr/local/lib/python3.13/site-packages/pypdfium2_raw/libpdfium.so /usr/local/lib/libpdfium.so

COPY --from=epubcheck /opt/epubcheck-${EPUBCHECK_VERSION} /opt/epubcheck-${EPUBCHECK_VERSION}
RUN \
    printf '#!/bin/sh\nexec java -jar /opt/epubcheck-%s/epubcheck.jar "$@"\n' "$EPUBCHECK_VERSION" > /usr/local/bin/epubcheck && \
    chmod +x /usr/local/bin/epubcheck

ENV PDFIUM_LIB_PATH=/usr/local/lib/libpdfium.so
ARG TM_BACKEND_DEFAULT_PORT=8020
ENV TM_BACKEND_PORT=$TM_BACKEND_DEFAULT_PORT

WORKDIR /app
VOLUME /books
VOLUME /comics
EXPOSE $TM_BACKEND_PORT
USER $USER_NAME
CMD ["sh", "-c", "exec backend serve --addr 0.0.0.0:${TM_BACKEND_PORT}"]
