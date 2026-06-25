# syntax=docker/dockerfile:1
FROM node:22.16.0 AS build

WORKDIR /app

COPY frontend/package.json frontend/package-lock.json ./
RUN npm install -g npm@10.9.2 && npm ci && npx browserslist@latest --update-db

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
