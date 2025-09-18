#!/bin/bash

rm -rf */{nohup.out*,run.log*,.mypy_cache,__pycache__,.idea,.git}

docker build -f Dockerfile.backend --build-arg TM_BACKEND_PORT="$TM_BACKEND_PORT" -t terzeron/tm_backend . && \
docker tag terzeron/tm_backend:latest registry.terzeron.com/terzeron/tm_backend:latest && \
docker push registry.terzeron.com/terzeron/tm_backend:latest

docker build --build-arg VITE_FACEBOOK_APP_ID=$VITE_FACEBOOK_APP_ID --build-arg VITE_API_URL_PREFIX=$VITE_API_URL_PREFIX --build-arg VITE_ADMIN_EMAIL=$VITE_ADMIN_EMAIL -f Dockerfile.frontend -t terzeron/tm_frontend . && \
docker tag terzeron/tm_frontend:latest registry.terzeron.com/terzeron/tm_frontend:latest && \
docker push registry.terzeron.com/terzeron/tm_frontend:latest

echo 'You might deploy the containers;'
echo 'kubectl apply -f ~/k8s/textmanager/tm-deployment.yml'
echo 'kubectl rollout restart deployment tm-backend -n textmanager'
echo 'kubectl rollout restart deployment tm-frontend -n textmanager'
