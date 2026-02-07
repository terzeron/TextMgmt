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

# 2. cert-manager 설치 (TLS Certificate 생성에 필요)
echo ""
echo ">>> Installing cert-manager"
if kubectl get namespace cert-manager &>/dev/null; then
    echo "cert-manager already installed, skipping"
else
    kubectl apply -f https://github.com/cert-manager/cert-manager/releases/latest/download/cert-manager.yaml
    echo "Waiting for cert-manager to be ready..."
    kubectl wait --for=condition=Available deployment --all -n cert-manager --timeout=180s
fi

# 3. Gateway API CRD 설치 + RBAC + Gateway/HTTPRoute 설정
echo ""
echo ">>> Installing Gateway API"
kubectl apply -f https://github.com/kubernetes-sigs/gateway-api/releases/latest/download/standard-install.yaml
kubectl apply -f "$SCRIPT_DIR/traefik-gateway-rbac.yml"
kubectl apply -f "$SCRIPT_DIR/gateway.yml"

# 4. Traefik v3 업그레이드 + Gateway API provider 활성화
echo ""
echo ">>> Configuring Traefik for Gateway API"
kubectl -n kube-system set image deploy/traefik traefik=traefik:v3.3
CURRENT_ARGS=$(kubectl get deploy -n kube-system traefik -o jsonpath='{.spec.template.spec.containers[0].args}')
if ! echo "$CURRENT_ARGS" | grep -q "kubernetesgateway"; then
    kubectl -n kube-system patch deploy traefik --type=json -p='[
      {"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--providers.kubernetesgateway"}
    ]'
fi
kubectl rollout status deploy/traefik -n kube-system --timeout=120s

# 5. PersistentVolume & PersistentVolumeClaim 생성
echo ""
echo ">>> Creating PersistentVolume and PersistentVolumeClaim"
kubectl apply -f "$SCRIPT_DIR/books-volume.yml"

# 6. TLS Certificate 생성 (cert-manager 필요)
echo ""
echo ">>> Creating TLS Certificate"
kubectl apply -f "$SCRIPT_DIR/tm-certificate.yml" || echo "Warning: TLS Certificate creation failed (cert-manager ClusterIssuer may not exist)"

# 7. Google OAuth Secret 생성
echo ""
echo ">>> Creating Google OAuth Secret"
if [ -z "$TM_GOOGLE_CLIENT_ID" ] || [ -z "$TM_GOOGLE_CLIENT_SECRET" ]; then
    echo "Warning: TM_GOOGLE_CLIENT_ID or TM_GOOGLE_CLIENT_SECRET not set in .env"
else
    kubectl -n $NAMESPACE create secret generic tm-google-cred \
      --from-literal=client-id="$TM_GOOGLE_CLIENT_ID" \
      --from-literal=client-secret="$TM_GOOGLE_CLIENT_SECRET" \
      --dry-run=client -o yaml | kubectl apply -f -
fi

# 8. Elasticsearch 설치
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

# 9. MySQL 설치
echo ""
read -p "Install MySQL? (y/N): " install_mysql
if [ "$install_mysql" = "y" ] || [ "$install_mysql" = "Y" ]; then
    cd "$SCRIPT_DIR"
    bash "$SCRIPT_DIR/mysql.sh"
    cd - > /dev/null
else
    echo "Skipping MySQL installation"
fi

# 10. Application Deployment 적용
echo ""
echo ">>> Applying application deployments"
kubectl apply -f "$SCRIPT_DIR/tm-deployment.yml"

# 11. 상태 확인
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
