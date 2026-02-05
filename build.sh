#!/bin/bash

TARGET=${1:-all}

rm -rf */{nohup.out*,run.log*,.mypy_cache,__pycache__,.idea,.git}

build_backend() {
    echo "=== Building backend ==="
    docker build -f backend/Dockerfile --build-arg TM_BACKEND_PORT="$TM_BACKEND_PORT" -t terzeron/tm_backend . && \
    docker tag terzeron/tm_backend:latest registry.terzeron.com/terzeron/tm_backend:latest && \
    docker push registry.terzeron.com/terzeron/tm_backend:latest
}

build_frontend() {
    echo "=== Building frontend ==="
    docker build -f frontend/Dockerfile -t terzeron/tm_frontend . && \
    docker tag terzeron/tm_frontend:latest registry.terzeron.com/terzeron/tm_frontend:latest && \
    docker push registry.terzeron.com/terzeron/tm_frontend:latest
}

rollout_backend() {
    echo "=== Rolling out backend ==="
    kubectl rollout restart deployment tm-backend -n textmanager
}

rollout_frontend() {
    echo "=== Rolling out frontend ==="
    kubectl rollout restart deployment tm-frontend -n textmanager
}

case "$TARGET" in
    backend)
        build_backend && rollout_backend
        ;;
    frontend)
        build_frontend && rollout_frontend
        ;;
    all|"")
        build_backend && build_frontend && rollout_backend && rollout_frontend
        ;;
    *)
        echo "Usage: $0 [backend|frontend]"
        echo "  backend  - Build and rollout backend only"
        echo "  frontend - Build and rollout frontend only"
        echo "  (no arg) - Build and rollout both"
        exit 1
        ;;
esac

echo ""
echo "Done! If this is a fresh install, run: ./k8s/k8s_init.sh"
