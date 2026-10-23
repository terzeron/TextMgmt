#!/usr/bin/env bash
# 기본 실행은 대상 확인만 한다. --apply는 선택한 파일을 loader로 재적재한다.
set -euo pipefail

mode="${1:---list}"
if [[ "$#" -gt 1 || ( "$mode" != "--list" && "$mode" != "--apply" ) ]]; then
    echo "Usage: bash utils/reload_samgukji_epubs.sh [--list|--apply]" >&2
    exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
set -a
source "$repo_root/.env"
set +a
export PYTHONPATH="$repo_root"

target_dir="${TM_BOOK_DIR:?}/2_소설역사"
if [[ ! -d "$target_dir" ]]; then
    echo "대상 디렉토리를 찾을 수 없습니다: $target_dir" >&2
    exit 1
fi

# NUL 구분자로 공백·괄호·대괄호가 있는 경로를 그대로 전달한다.
mapfile -d '' files < <(find "$target_dir" -type f -name '*삼국지*' -iname '*.epub' -print0 | sort -z)
if [[ "${#files[@]}" -eq 0 ]]; then
    echo "삼국지 EPUB이 없습니다: $target_dir" >&2
    exit 1
fi
printf '대상: %s개 EPUB\n' "${#files[@]}"
printf '%s\n' "${files[@]}"

if [[ "$mode" == "--apply" ]]; then
    # 개별 파일 인자는 기존 문서도 강제 재적재한다. 디렉토리 전체를 넘기지 않는다.
    exec uv run --no-sync python -m utils.loader book "${files[@]}"
fi
echo "재적재: bash utils/reload_samgukji_epubs.sh --apply"
