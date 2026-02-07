#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../.env"

NS="textmanager"
MAX_RETRIES=30
RETRY_INTERVAL=5

# ES API 호출 재시도 함수
es_curl() {
  local user="$1"; shift
  for i in $(seq 1 $MAX_RETRIES); do
    if response=$(curl -sf -u "$user" "$@" 2>&1); then
      echo "$response"
      return 0
    fi
    echo "  retry $i/$MAX_RETRIES..." >&2
    sleep $RETRY_INTERVAL
  done
  echo "ERROR: ES API 호출 실패 - $*" >&2
  return 1
}

echo "=== [1/6] Helm repo 설정 ==="
helm repo add elastic https://helm.elastic.co 2>/dev/null || true
helm repo update

echo "=== [2/6] ECK CRD & Operator 설치 ==="
kubectl apply --server-side -f https://download.elastic.co/downloads/eck/3.1.0/crds.yaml
CRD_COUNT=$(kubectl get crd | grep -c k8s.elastic.co)
echo "  CRD 개수: $CRD_COUNT (10개 이상이면 정상)"

helm upgrade --install elastic-operator elastic/eck-operator \
    -n "$NS" \
    --create-namespace \
    --version 3.1.0 \
    --set installCRDs=false
kubectl -n "$NS" rollout status sts/elastic-operator --timeout=3m

echo "=== [3/6] K8s Secret 생성 ==="
kubectl -n "$NS" create secret generic tm-es-cred \
  --from-literal=username="$TM_ES_USER" \
  --from-literal=password="$TM_ES_PASSWORD" \
  --dry-run=client -o yaml | kubectl apply -f -

echo "=== [4/6] Elasticsearch 클러스터 배포 ==="
helm upgrade --install es-kb-quickstart elastic/eck-stack \
    -n "$NS" \
    --create-namespace \
    -f "$SCRIPT_DIR/es-values.yml"

echo "=== [5/6] Gateway API 설정 ==="
kubectl apply -f "$SCRIPT_DIR/traefik-gateway-rbac.yml"
kubectl apply -f "$SCRIPT_DIR/gateway.yml"

echo "=== [6/6] ES 준비 대기 및 사용자 설정 ==="
echo "  pod ready 대기 중..."
kubectl -n "$NS" wait --for=condition=ready pod/elasticsearch-es-default-0 --timeout=5m

# elastic 관리자 비밀번호 조회
ELASTIC_PASSWORD=$(kubectl get secret elasticsearch-es-elastic-user -n "$NS" -o jsonpath='{.data.elastic}' | base64 -d)

# ES API가 실제로 응답할 때까지 대기
echo "  ES API 응답 대기 중..."
es_curl "elastic:$ELASTIC_PASSWORD" "$TM_ES_URL/_cluster/health?pretty"

# 역할 생성
echo "  tm_role 생성 중..."
es_curl "elastic:$ELASTIC_PASSWORD" \
  -X PUT "$TM_ES_URL/_security/role/tm_role" \
  -H 'Content-Type: application/json' \
  -d '{ "cluster": ["monitor", "manage_index_templates"], "indices": [{"names": ["tm", "tm-*"], "privileges": ["all"]}] }'

# appuser 생성/업데이트
echo "  appuser 생성 중..."
es_curl "elastic:$ELASTIC_PASSWORD" \
  -X PUT "$TM_ES_URL/_security/user/appuser" \
  -H 'Content-Type: application/json' \
  -d "{\"password\":\"$TM_ES_PASSWORD\",\"roles\":[\"superuser\"]}"

# appuser로 접속 테스트
echo ""
echo "=== appuser 인증 테스트 ==="
es_curl "$TM_ES_USER:$TM_ES_PASSWORD" "$TM_ES_URL/_cluster/health?pretty"
echo ""
echo "=== ES 설정 완료 ==="
