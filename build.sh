#!/bin/bash

rm -rf */{nohup.out*,run.log*,.mypy_cache,__pycache__,.idea,.git}

docker build -f Dockerfile.backend --build-arg TM_BACKEND_PORT="$TM_BACKEND_PORT" -t terzeron/tm_backend . && \
docker tag terzeron/tm_backend:latest registry.terzeron.com/terzeron/tm_backend:latest && \
docker push registry.terzeron.com/terzeron/tm_backend:latest

(cd frontend && npm install -y > /dev/null && \
    npm run build > /dev/null) && \
docker build -f Dockerfile.frontend -t terzeron/tm_frontend . && \
docker tag terzeron/tm_frontend:latest registry.terzeron.com/terzeron/tm_frontend:latest && \
docker push registry.terzeron.com/terzeron/tm_frontend:latest

