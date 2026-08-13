FROM python:3.13-slim

# apt 보안 패치 레이어의 캐시 키. build.sh가 ISO 주차를 넘겨 주 1회 재실행시킨다.
# 이 값이 고정이면 아래 apt-get upgrade가 캐시에서 재사용되어 패치가 적용되지 않는다
# (2026-08 점검: 이미지는 재빌드되는데 apt 레이어만 2개월 경과).
ARG APT_REFRESH=0

# root
RUN : "apt refresh ${APT_REFRESH}" && \
    apt-get update && \
    apt-get upgrade -y && \
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
# 5.1.0의 번들 jar에 취약점이 있었다: jackson-mapper-asl 1.9.12(CVE-2019-10202
# CRITICAL, upstream 수정판 없음), xercesImpl 2.9.1(XML DoS — 신뢰 불가 EPUB을
# 파싱하므로 도달 가능). 5.3.0은 전자를 제거하고 후자를 2.12.2로 올렸다.
# --json 출력 스키마는 5.1.0과 동일함을 실물 비교로 확인(정상/손상 EPUB 2건).
ARG EPUBCHECK_VERSION=5.3.0
RUN curl -L https://github.com/w3c/epubcheck/releases/download/v${EPUBCHECK_VERSION}/epubcheck-${EPUBCHECK_VERSION}.zip \
    -o /tmp/epubcheck.zip && \
    unzip /tmp/epubcheck.zip -d /opt && \
    rm /tmp/epubcheck.zip && \
    printf '#!/bin/sh\nexec java -jar /opt/epubcheck-%s/epubcheck.jar "$@"\n' "${EPUBCHECK_VERSION}" > /usr/local/bin/epubcheck && \
    chmod +x /usr/local/bin/epubcheck

ARG USER_ID=1000
ARG USER_NAME=terzeron
RUN useradd -u $USER_ID $USER_NAME && \
    mkdir -p /app /app/backend /app/utils /books /comics /home/$USER_NAME/.cache /home/$USER_NAME/.config /home/$USER_NAME/.local && \
    chown -R $USER_NAME /app /books /comics /home/$USER_NAME

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
VOLUME /comics

ENV PATH="/app/.venv/bin:/bin:/sbin:/usr/bin:/usr/sbin:/usr/local/bin:/usr/local/sbin:/home/$USER_NAME/.local/bin:/app/utils"
ENV PYTHONPATH="/app"
ARG TM_BACKEND_DEFAULT_PORT=8020
ENV TM_BACKEND_PORT=$TM_BACKEND_DEFAULT_PORT

EXPOSE $TM_BACKEND_PORT

CMD ["sh", "-c", "uvicorn backend.main:app --workers 2 --host 0.0.0.0 --port ${TM_BACKEND_PORT} --log-config /app/logging.conf"]
