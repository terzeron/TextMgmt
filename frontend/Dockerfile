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

USER root
RUN apt-get update && apt-get upgrade -y && rm -rf /var/lib/apt/lists/*
USER 101

COPY --from=build /app/dist /usr/share/nginx/html
COPY frontend/default.conf /etc/nginx/conf.d
COPY frontend/security-headers.conf /etc/nginx/security-headers.conf

EXPOSE 8080

CMD ["nginx", "-g", "daemon off;"]
