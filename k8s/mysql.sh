#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../.env"

# secret
kubectl -n textmanager create secret generic tm-mysql-cred \
  --from-literal=mysql-root-password="$TM_MYSQL_ROOT_PASSWORD" \
  --from-literal=mysql-user="$TM_MYSQL_USER" \
  --from-literal=mysql-password="$TM_MYSQL_PASSWORD" \
  --dry-run=client -o yaml | kubectl apply -f -

# mysql
kubectl apply -f mysql-deployment.yml

# ready될 때까지 대기한 후에 연결 테스트
echo "waiting for ready state..."
kubectl -n textmanager wait --for=condition=ready pod -l app=tm-mysql-app --timeout=5m
sleep 10  # MySQL 초기화 대기
kubectl -n textmanager exec deploy/tm-mysql -- mysql -h 127.0.0.1 -u"$TM_MYSQL_USER" -p"$TM_MYSQL_PASSWORD" -e "SELECT 1 AS test;" textmanager
