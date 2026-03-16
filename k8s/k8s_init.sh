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
LIMA_VM="docker"
K3D_CLUSTER="dev"

echo "=== K8s Initialization for TextManager ==="

# 0-1. Lima VM 생성 (Docker runtime용)
echo ""
echo ">>> [0-1] Setting up Lima VM: $LIMA_VM"
if limactl list --format '{{.Name}}' 2>/dev/null | grep -qx "$LIMA_VM"; then
    STATUS=$(limactl list --format '{{.Status}}' --filter "Name=$LIMA_VM" 2>/dev/null || true)
    if [ "$STATUS" = "Running" ]; then
        echo "Lima VM '$LIMA_VM' already running, skipping"
    else
        echo "Lima VM '$LIMA_VM' exists but stopped, starting..."
        limactl start "$LIMA_VM"
    fi
else
    echo "Creating Lima VM '$LIMA_VM' from lima-docker.yaml..."
    limactl create --name="$LIMA_VM" "$SCRIPT_DIR/lima-docker.yaml"
    limactl start "$LIMA_VM"
fi

# Docker 소켓 경로 설정
export DOCKER_HOST="unix://$HOME/.lima/$LIMA_VM/sock/docker.sock"
echo "DOCKER_HOST=$DOCKER_HOST"

# Docker 접속 확인
echo "Waiting for Docker daemon..."
for i in $(seq 1 30); do
    if docker info &>/dev/null; then
        echo "Docker is ready"
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "Error: Docker daemon not responding"
        exit 1
    fi
    sleep 2
done

# 0-2. k3d 클러스터 생성
echo ""
echo ">>> [0-2] Setting up k3d cluster: $K3D_CLUSTER"
if k3d cluster list 2>/dev/null | grep -q "$K3D_CLUSTER"; then
    echo "k3d cluster '$K3D_CLUSTER' already exists, skipping"
else
    echo "Creating k3d cluster '$K3D_CLUSTER'..."
    k3d cluster create "$K3D_CLUSTER" \
        --servers 1 \
        --agents 1 \
        --port "80:80@loadbalancer" \
        --port "443:443@loadbalancer" \
        --port "30300-30310:30300-30310@loadbalancer" \
        --wait
fi

# kubectl context 설정
kubectl config use-context "k3d-$K3D_CLUSTER"
kubectl cluster-info

# 1. Namespace 생성
echo ""
echo ">>> Creating namespace: $NAMESPACE"
kubectl create namespace $NAMESPACE --dry-run=client -o yaml | kubectl apply -f -

# 2. Gateway API CRD 설치 + HTTPRoute 설정
echo ""
echo ">>> Installing Gateway API"
kubectl apply -f https://github.com/kubernetes-sigs/gateway-api/releases/latest/download/standard-install.yaml
kubectl apply -f "$SCRIPT_DIR/httproute.yml"

# 3. Traefik v3 업그레이드 + Gateway API provider 활성화
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

# 4. PersistentVolume & PersistentVolumeClaim 생성
echo ""
echo ">>> Creating PersistentVolume and PersistentVolumeClaim"
kubectl apply -f "$SCRIPT_DIR/books-volume.yml"

# 5. Google OAuth Secret 생성
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

# 6. Elasticsearch 설치
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

# 7. MySQL 설치
echo ""
read -p "Install MySQL? (y/N): " install_mysql
if [ "$install_mysql" = "y" ] || [ "$install_mysql" = "Y" ]; then
    cd "$SCRIPT_DIR"
    bash "$SCRIPT_DIR/mysql.sh"
    cd - > /dev/null
else
    echo "Skipping MySQL installation"
fi

# 8. Application Deployment 적용
echo ""
echo ">>> Applying application deployments"
kubectl apply -f "$SCRIPT_DIR/tm-deployment.yml"

# 9. 상태 확인
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
