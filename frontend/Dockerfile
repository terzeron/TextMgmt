# syntax=docker/dockerfile:1
FROM node:22.23.2 AS build

WORKDIR /app

COPY frontend/package.json frontend/package-lock.json frontend/.npmrc ./
# 이미지 빌드는 락파일을 그대로 재현하기만 한다. caniuse-lite 갱신은 레포에서
# `npm run deps:browsers` 로 수행하고 package-lock.json 변경을 커밋한다.
# (빌드 중 원격 패키지를 받아 락파일을 바꾸면 빌드 재현성이 깨진다.)
RUN npm install -g npm@10.9.2 && npm ci

COPY frontend/ .
RUN npm run build


FROM nginxinc/nginx-unprivileged:1.28

# apt 보안 패치 레이어의 캐시 키. build.sh가 ISO 주차를 넘겨 주 1회 재실행시킨다.
# 이 값이 고정이면 아래 apt-get upgrade가 캐시에서 재사용되어 패치가 적용되지 않는다
# (2026-08 점검: 이미지는 10분 전 재빌드인데 apt 레이어만 2개월 경과).
ARG APT_REFRESH=0

USER root
RUN : "apt refresh ${APT_REFRESH}" && \
    apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*
USER 101

COPY --from=build /app/dist /usr/share/nginx/html
COPY frontend/default.conf /etc/nginx/conf.d
COPY frontend/security-headers.conf /etc/nginx/security-headers.conf

EXPOSE 8080

CMD ["nginx", "-g", "daemon off;"]
