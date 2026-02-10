FROM python:3.13-slim

# root
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    libpoppler-dev libpoppler-cpp-dev libheif-dev libcairo2-dev \
    libqpdf-dev \
    libxml2-dev libxslt-dev \
    libreoffice-writer-nogui \
    default-jre-headless \
    unzip \
    curl ca-certificates && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

# epubcheck 설치
ARG EPUBCHECK_VERSION=5.1.0
RUN curl -L https://github.com/w3c/epubcheck/releases/download/v${EPUBCHECK_VERSION}/epubcheck-${EPUBCHECK_VERSION}.zip \
    -o /tmp/epubcheck.zip && \
    unzip /tmp/epubcheck.zip -d /opt && \
    rm /tmp/epubcheck.zip && \
    printf '#!/bin/sh\nexec java -jar /opt/epubcheck-%s/epubcheck.jar "$@"\n' "${EPUBCHECK_VERSION}" > /usr/local/bin/epubcheck && \
    chmod +x /usr/local/bin/epubcheck

ARG USER_ID=1000
ARG USER_NAME=terzeron
RUN useradd -u $USER_ID $USER_NAME && \
    mkdir -p /app /app/backend /app/utils /books /home/$USER_NAME/.cache /home/$USER_NAME/.config /home/$USER_NAME/.local && \
    chown -R $USER_NAME /app /books /home/$USER_NAME

RUN curl -LsSf https://astral.sh/uv/install.sh | sh && \
    cp /root/.local/bin/uv /usr/local/bin/

# user(1000)
USER $USER_NAME
WORKDIR /app

ENV UV_SYSTEM_PYTHON=1 UV_LINK_MODE=copy UV_PYTHON=python3.13
COPY pyproject.toml uv.lock /app/
RUN HOME=/tmp XDG_CACHE_HOME=/tmp/.cache UV_CACHE_DIR=/tmp/uv-cache \
    uv sync --locked --no-editable --no-dev

COPY logging.conf /app
COPY backend /app/backend
COPY utils /app/utils

VOLUME /books

ENV PATH="/app/.venv/bin:/bin:/sbin:/usr/bin:/usr/sbin:/usr/local/bin:/usr/local/sbin:/home/$USER_NAME/.local/bin:/app/utils"
ENV PYTHONPATH="/app"
ARG TM_BACKEND_DEFAULT_PORT=8020
ENV TM_BACKEND_PORT=$TM_BACKEND_DEFAULT_PORT

EXPOSE $TM_BACKEND_PORT

CMD ["sh", "-c", "uvicorn backend.main:app --workers 2 --host 0.0.0.0 --port ${TM_BACKEND_PORT} --log-config /app/logging.conf"]
