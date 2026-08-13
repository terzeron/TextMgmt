#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/.env"

TARGET=${1:-all}

REGISTRY=registry.terzeron.com/terzeron
NAMESPACE=textmanager

rm -rf */{nohup.out*,run.log*,.mypy_cache,__pycache__,.idea,.git}

# 이미지 태그를 커밋 SHA로 고정한다. :latest만 쓰면 클러스터에 어느 빌드가 떠 있는지
# 추적할 수 없다(trivy KSV013). 배포 자체는 아래에서 digest로 고정한다.
# 정리 작업 뒤에 계산해야 잔여 파일 때문에 -dirty가 붙지 않는다.
TAG="$(git -C "$SCRIPT_DIR" rev-parse --short HEAD)"
[[ -z "$(git -C "$SCRIPT_DIR" status --porcelain)" ]] || TAG="${TAG}-dirty"

# --pull: FROM을 로컬 캐시에 고정하지 않고 base image 갱신분을 받는다.
# APT_REFRESH: Dockerfile의 apt-get upgrade 레이어 캐시 키(ISO 연-주). 이게 없으면
#   이미지를 재빌드해도 apt 레이어가 캐시에서 재사용되어 보안 패치가 적용되지 않는다.
BUILD_FLAGS=(--pull --build-arg "APT_REFRESH=$(date +%G%V)")

# push 후 repo digest만 표준출력으로 돌려준다(호출부가 명령 치환으로 받으므로
# push 출력은 버린다). 배포는 태그가 아니라 이 digest로 한다.
push_image() {
    local name=$1
    docker tag "terzeron/$name:$TAG" "terzeron/$name:latest" && \
    docker tag "terzeron/$name:$TAG" "$REGISTRY/$name:$TAG" && \
    docker tag "terzeron/$name:$TAG" "$REGISTRY/$name:latest" && \
    docker push -q "$REGISTRY/$name:$TAG" >/dev/null && \
    docker push -q "$REGISTRY/$name:latest" >/dev/null && \
    docker inspect --format='{{range .RepoDigests}}{{println .}}{{end}}' "$REGISTRY/$name:$TAG" | grep "^$REGISTRY/$name@" | head -1
}

# digest가 이미 배포된 것과 같으면 재시작하지 않는다(내용이 동일하다는 뜻).
deploy_image() {
    local deploy=$1 container=$2 image=$3
    if [ -z "$image" ]; then
        echo "digest를 확인할 수 없어 배포를 중단한다: $deploy" >&2
        return 1
    fi
    local current
    current="$(kubectl get deployment "$deploy" -n "$NAMESPACE" -o jsonpath="{.spec.template.spec.containers[?(@.name=='$container')].image}")"
    if [ "$current" = "$image" ]; then
        echo "  동일 digest가 이미 배포됨 — 재시작 생략 ($image)"
        return 0
    fi
    kubectl set image "deployment/$deploy" "$container=$image" -n "$NAMESPACE" && \
    kubectl rollout status "deployment/$deploy" -n "$NAMESPACE" --timeout=300s
}

build_backend() {
    echo "=== Building backend ($TAG) ==="
    docker build -q "${BUILD_FLAGS[@]}" -f backend/Dockerfile --build-arg TM_BACKEND_PORT="$TM_BACKEND_PORT" -t "terzeron/tm_backend:$TAG" . > /dev/null && \
    BACKEND_DIGEST="$(push_image tm_backend)"
}

build_frontend() {
    echo "=== Building frontend ($TAG) ==="
    docker build -q "${BUILD_FLAGS[@]}" -f frontend/Dockerfile -t "terzeron/tm_frontend:$TAG" . > /dev/null && \
    FRONTEND_DIGEST="$(push_image tm_frontend)"
}

rollout_backend() {
    echo "=== Rolling out backend ==="
    deploy_image tm-backend tm-backend "$BACKEND_DIGEST"
}

rollout_frontend() {
    echo "=== Rolling out frontend ==="
    deploy_image tm-frontend tm-frontend "$FRONTEND_DIGEST"
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
