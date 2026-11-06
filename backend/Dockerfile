# syntax=docker/dockerfile:1
FROM rust:1.91-slim-bookworm AS builder
ARG APT_REFRESH=0
RUN : "apt refresh ${APT_REFRESH}" && \
    apt-get update && apt-get upgrade -y && \
    apt-get install -y --no-install-recommends build-essential cmake pkg-config libssl-dev libclang-dev libjpeg62-turbo-dev zlib1g-dev libfreetype6-dev libharfbuzz-dev libopenjp2-7-dev libfontconfig1-dev ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock /app/
COPY .cargo /app/.cargo/
COPY backend /app/backend
COPY libs /app/libs
COPY utils /app/utils
# registry와 target을 cache mount에 둬 소스 변경 시 의존 crate 재컴파일을 피한다.
# cache mount는 이미지 레이어에 남지 않으므로 바이너리를 /out으로 복사한다.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --locked --profile image -p backend -p loader -p pdf_engine && \
    mkdir -p /out && \
    cp target/image/backend target/image/loader target/image/epubveri target/image/tm-pdf-worker /out/

# Runtime includes only the Rust server, its loader/PDF worker subprocesses and the
# external tools still required for office conversion.
FROM debian:bookworm-slim
ARG APT_REFRESH=0
ARG USER_ID=1000
ARG USER_NAME=terzeron

RUN : "apt refresh ${APT_REFRESH}" && \
    apt-get update && apt-get upgrade -y && \
    apt-get install -y --no-install-recommends \
        libreoffice-writer-nogui \
        fonts-noto-cjk \
        libjpeg62-turbo zlib1g libfreetype6 libharfbuzz0b libopenjp2-7 libfontconfig1 \
        ca-certificates \
        libssl3 && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --uid "$USER_ID" --create-home "$USER_NAME" && \
    mkdir -p /app /books /comics /home/"$USER_NAME"/.cache /home/"$USER_NAME"/.config && \
    chown -R "$USER_ID:$USER_ID" /app /books /comics /home/"$USER_NAME"

COPY --from=builder /out/backend /usr/local/bin/backend
COPY --from=builder /out/loader /usr/local/bin/loader
COPY --from=builder /out/tm-pdf-worker /usr/local/bin/tm-pdf-worker
COPY --from=builder /out/epubveri /usr/local/bin/epubveri
COPY backend/fontconfig/99-textmgmt-cjk.conf /etc/fonts/conf.d/99-textmgmt-cjk.conf

ARG TM_BACKEND_DEFAULT_PORT=8020
ENV TM_BACKEND_PORT=$TM_BACKEND_DEFAULT_PORT

WORKDIR /app
VOLUME /books
VOLUME /comics
EXPOSE $TM_BACKEND_PORT
USER $USER_NAME
CMD ["sh", "-c", "exec backend serve --addr 0.0.0.0:${TM_BACKEND_PORT}"]
