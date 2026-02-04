#!/bin/bash

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# .env 파일 로드
if [ -f "$PROJECT_ROOT/.env" ]; then
    source "$PROJECT_ROOT/.env"
else
    echo "Error: .env file not found at $PROJECT_ROOT/.env"
    exit 1
fi

NAMESPACE="textmanager"

echo "=== K8s Initialization for TextManager ==="

# 1. Namespace 생성
echo ""
echo ">>> Creating namespace: $NAMESPACE"
kubectl create namespace $NAMESPACE --dry-run=client -o yaml | kubectl apply -f -

# 2. PersistentVolume & PersistentVolumeClaim 생성
echo ""
echo ">>> Creating PersistentVolume and PersistentVolumeClaim"
kubectl apply -f "$SCRIPT_DIR/books-volume.yml"

# 3. TLS Certificate 생성 (cert-manager 필요)
echo ""
echo ">>> Creating TLS Certificate"
kubectl apply -f "$SCRIPT_DIR/tm-certificate.yml"

# 4. Elasticsearch 설치
echo ""
echo ">>> Installing Elasticsearch (this may take a while)"
read -p "Install Elasticsearch? (y/N): " install_es
if [ "$install_es" = "y" ] || [ "$install_es" = "Y" ]; then
    cd "$SCRIPT_DIR"
    bash "$SCRIPT_DIR/es.sh"
    cd - > /dev/null
else
    echo "Skipping Elasticsearch installation"
fi

# 5. Application Deployment 적용
echo ""
echo ">>> Applying application deployments"
kubectl apply -f "$SCRIPT_DIR/tm-deployment.yml"

# 6. 상태 확인
echo ""
echo "=== Deployment Status ==="
kubectl get all -n $NAMESPACE

echo ""
echo "=== Initialization Complete ==="
echo ""
echo "Next steps:"
echo "  - Build and push Docker images: ./build.sh"
echo "  - Check pod status: kubectl get pods -n $NAMESPACE"
echo "  - Check logs: kubectl logs -f deployment/tm-backend -n $NAMESPACE"
