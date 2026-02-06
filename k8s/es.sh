#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../.env"

# helm repo
helm repo add elastic https://helm.elastic.co
helm repo update
# crd
kubectl create -f https://download.elastic.co/downloads/eck/3.1.0/crds.yaml
kubectl get crd | grep k8s.elastic.co | wc -l    # 10개 이상 나오면 정상
# operator
helm install elastic-operator elastic/eck-operator \
    -n textmanager \
    --create-namespace \
    --version 3.1.0 \
    --set installCRDs=false
kubectl -n textmanager rollout status sts/elastic-operator
# secret
kubectl -n textmanager create secret generic tm-es-cred \
  --from-literal=username="$TM_ES_USER" \
  --from-literal=password="$TM_ES_PASSWORD" \
  --dry-run=client -o yaml | kubectl apply -f -

# elasticsearch
helm install es-kb-quickstart elastic/eck-stack \
    -n textmanager \
    --create-namespace \
    -f es-values.yml

# ES를 위한 gateway-api 설정
kubectl apply -f gateway.yaml -n textmanager
hostname=$(kubectl get ingress es-ingress -n textmanager -o json | jq -r ".spec.rules[0].host")
ip=$(kubectl get ingress es-ingress -n textmanager -o json | jq -r ".status.loadBalancer.ingress[0].ip")
echo "$hostname $ip"

# ready될 때까지 대기한 후에 endpoint 접근 테스트
echo "waiting for ready state..."
kubectl -n textmanager wait --for=condition=ready pod/elasticsearch-es-default-0 --timeout=5m && \
curl -u "$TM_ES_USER:$TM_ES_PASSWORD" $TM_ES_URL/_cluster/health\?pretty

# appuser에게 역할 부여
ELASTIC_PASSWORD=$(kubectl get secret elasticsearch-es-elastic-user -n textmanager -o jsonpath='{.data.elastic}' | base64 -d)

# 역할 생성
curl -u "elastic:$ELASTIC_PASSWORD" -X PUT "$TM_ES_URL/_security/role/tm_role" \
  -H 'Content-Type: application/json' \
  -d '{ "cluster": ["monitor", "manage_index_templates"], "indices": [{"names": ["tm", "tm-*"], "privileges": ["all"]}] }'

# appuser에 역할 할당
curl -u "elastic:$ELASTIC_PASSWORD" -X PUT "$TM_ES_URL/_security/user/appuser" \
  -H 'Content-Type: application/json' \
  -d "{\"password\":\"$TM_ES_PASSWORD\",\"roles\":[\"superuser\"]}"
