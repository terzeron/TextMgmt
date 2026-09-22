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

echo "=== [1/6] 호스트 전제조건: vm.max_map_count ==="
# es-values.yml 에서 node.store.allow_mmap 우회책을 제거했으므로 ES 는 mmapfs 로
# 세그먼트를 읽는다. 이 값이 리눅스 기본값(65530)이면 bootstrap check 에서 기동을
# 거부한다. 배포판이 /usr/lib/sysctl.d 로 넣어주는 값은 OS 업그레이드로 바뀔 수
# 있어서, 우선순위가 높은 /etc/sysctl.d 에 고정한다.
SYSCTL_FILE="/etc/sysctl.d/99-elasticsearch.conf"
REQUIRED_MAP_COUNT=262144
CURRENT_MAP_COUNT=$(sysctl -n vm.max_map_count)
echo "  현재 vm.max_map_count: $CURRENT_MAP_COUNT (필요: $REQUIRED_MAP_COUNT 이상)"

if [ -f "$SYSCTL_FILE" ]; then
  echo "  $SYSCTL_FILE 이미 존재 - 건너뜀"
elif sudo -n true 2>/dev/null; then
  echo "  $SYSCTL_FILE 생성 중..."
  sudo tee "$SYSCTL_FILE" >/dev/null <<'SYSCTL'
# Elasticsearch bootstrap check: vm.max_map_count >= 262144
# k8s/es-values.yml 이 mmapfs 를 쓰므로 이 값이 낮으면 ES 가 기동하지 않는다.
vm.max_map_count=1048576
SYSCTL
  sudo sysctl -q --system
  echo "  적용 후: $(sysctl -n vm.max_map_count)"
else
  echo "  경고: sudo 에 비밀번호가 필요해 $SYSCTL_FILE 을 만들지 못했다."
  echo "  현재 값으로는 동작하지만, OS 업그레이드 후 ES 가 기동하지 않을 수 있다."
  echo "  다음을 직접 실행한다:"
  echo "    echo 'vm.max_map_count=1048576' | sudo tee $SYSCTL_FILE && sudo sysctl --system"
  if [ "$CURRENT_MAP_COUNT" -lt "$REQUIRED_MAP_COUNT" ]; then
    echo "  ERROR: 현재 값이 필요값보다 낮아 ES 가 기동하지 못한다. 위 명령을 먼저 실행한다." >&2
    exit 1
  fi
fi

echo "=== [2/6] Helm repo 설정 ==="
helm repo add elastic https://helm.elastic.co 2>/dev/null || true
helm repo update

echo "=== [3/6] ECK CRD & Operator 설치 ==="
kubectl apply --server-side -f https://download.elastic.co/downloads/eck/3.1.0/crds.yaml
CRD_COUNT=$(kubectl get crd | grep -c k8s.elastic.co)
echo "  CRD 개수: $CRD_COUNT (10개 이상이면 정상)"

helm upgrade --install elastic-operator elastic/eck-operator \
    -n "$NS" \
    --create-namespace \
    --version 3.1.0 \
    --set installCRDs=false
kubectl -n "$NS" rollout status sts/elastic-operator --timeout=3m

echo "=== [4/6] K8s Secret 생성 ==="
kubectl -n "$NS" create secret generic tm-es-cred \
  --from-literal=username="$TM_ES_USER" \
  --from-literal=password="$TM_ES_PASSWORD" \
  --dry-run=client -o yaml | kubectl apply -f -

echo "=== [5/6] Elasticsearch 클러스터 배포 ==="
helm upgrade --install es-kb-quickstart elastic/eck-stack \
    -n "$NS" \
    --create-namespace \
    -f "$SCRIPT_DIR/es-values.yml"

echo "=== [6/6] ES 준비 대기 및 사용자 설정 ==="
echo "  pod ready 대기 중..."
kubectl -n "$NS" wait --for=condition=ready pod/elasticsearch-es-default-0 --timeout=5m

# elastic 관리자 비밀번호 조회
ELASTIC_PASSWORD=$(kubectl get secret elasticsearch-es-elastic-user -n "$NS" -o jsonpath='{.data.elastic}' | base64 -d)

# ES API가 실제로 응답할 때까지 대기
echo "  ES API 응답 대기 중..."
es_curl "elastic:$ELASTIC_PASSWORD" "$TM_ES_URL/_cluster/health?pretty"

# 역할 생성 (최소 권한: superuser 대신 book/comics 인덱스 한정)
#   - cluster: monitor (es.info() 연결 확인용. manage_index_templates는 앱 미사용이라 제거)
#   - indices: manage(인덱스 생성/삭제/put_mapping/refresh) + read + write 만 부여
echo "  tm_role 생성 중..."
es_curl "elastic:$ELASTIC_PASSWORD" \
  -X PUT "$TM_ES_URL/_security/role/tm_role" \
  -H 'Content-Type: application/json' \
  -d "{\"cluster\":[\"monitor\"],\"indices\":[{\"names\":[\"${TM_ES_BOOK_INDEX:-book}\",\"${TM_ES_COMICS_INDEX:-comics}\"],\"privileges\":[\"manage\",\"read\",\"write\"]}]}"

# appuser 생성/업데이트
echo "  appuser 생성 중..."
es_curl "elastic:$ELASTIC_PASSWORD" \
  -X PUT "$TM_ES_URL/_security/user/appuser" \
  -H 'Content-Type: application/json' \
  -d "{\"password\":\"$TM_ES_PASSWORD\",\"roles\":[\"tm_role\"]}"

# appuser로 접속 테스트
echo ""
echo "=== appuser 인증 테스트 ==="
es_curl "$TM_ES_USER:$TM_ES_PASSWORD" "$TM_ES_URL/_cluster/health?pretty"
echo ""
echo "=== ES 설정 완료 ==="
