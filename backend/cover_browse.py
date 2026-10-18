"""책·만화 탭 커버 목록의 묶음 규칙.

한 디렉토리에 권별 파일이 모여 있는 경우(만화 등)에 파일마다 커버를 그리면 같은 작품이 줄줄이
이어진다. 그래서 기준 디렉토리(scope)마다 같은 규칙을 쓴다: scope에 직접 속한 파일은 한 권씩,
scope 바로 아래 디렉토리는 대표 한 권으로 묶는다. 전체 보기의 scope는 라이브러리 루트다.

정렬은 최신순이다(최신 탭과 같다). 파일은 수정 시각, 생성 시각이 큰 것부터, 같으면 경로순이다.
묶음은 안에 든 문서 중 가장 최근 시각을 자기 시각으로 쓰고, 같으면 디렉토리 경로순이다.
시각 값은 ES 정렬값과 같은 epoch 밀리초이고, 값이 없으면 가장 뒤로 보낸다.

ES와 무관한 순수 로직만 둔다.
"""

import re
from dataclasses import dataclass
from typing import Any

from utils.corpus_layout import IGNORED_DIR_NAMES

# 최신 만화 탭의 FIRST_INSTALLMENT_PATTERN 과 같다. "1권", "01화"는 1권으로 본다.
FIRST_INSTALLMENT_PATTERN = re.compile(r"(?:^|[^\d])0*1\s*(?:권|화)(?=$|[^\d])")
ROOT_CATEGORY = "_root"
# ES가 desc 정렬에서 값 없는 문서에 주는 정렬값(missing: _last)과 같다.
MISSING_TIME = -(2**63)

# 카테고리별 (문서 수, 가장 최근 수정 시각, 가장 최근 생성 시각). 시각이 없으면 None.
CategoryStat = tuple[int, float | None, float | None]


def latest_rank(updated: float | None, created: float | None, path: str) -> tuple[int, int, str]:
    """최신순으로 오름차순 비교하면 가장 최근 것이 먼저 오도록 만든 비교 키."""
    return (-int(MISSING_TIME if updated is None else updated), -int(MISSING_TIME if created is None else created), path)


def file_rank(sort_values: list[Any]) -> tuple[int, int, str]:
    """ES 최신순 정렬값 [수정 시각, 생성 시각, 파일 경로]를 비교 키로 바꾼다."""
    updated, created, path = sort_values
    return latest_rank(updated, created, str(path))


@dataclass(frozen=True)
class CoverGroup:
    """작품 디렉토리 하나. key는 디렉토리 전체 경로, name은 화면에 보일 마지막 조각이다."""

    name: str
    key: str
    updated: float | None = None
    created: float | None = None

    @property
    def rank(self) -> tuple[int, int, str]:
        return latest_rank(self.updated, self.created, self.key)

    @property
    def cursor_values(self) -> list[Any]:
        """다음 페이지 커서에 담을 값. rank와 같은 순서를 되살릴 수 있어야 한다."""
        return [self.updated, self.created, self.key]


def category_matches(category: str, hidden_categories: list[str]) -> bool:
    return any(category == hidden or category.startswith(hidden + "/") for hidden in hidden_categories)


def is_ignored_category(category: str) -> bool:
    """캐시·추출 산출물 디렉토리는 라이브러리 책이 아니므로 목록에서 뺀다. 색인 순회와 같은 기준이다."""
    return any(segment.startswith(".") or segment in IGNORED_DIR_NAMES for segment in category.split("/"))


def _later(current: float | None, candidate: float | None) -> float | None:
    if candidate is None:
        return current
    return candidate if current is None else max(current, candidate)


def plan_cover_scope(category_stats: dict[str, CategoryStat], scope: str = "", hidden_categories: list[str] | None = None) -> tuple[list[str], list[CoverGroup]]:
    """보여줄 범위를 개별 파일 카테고리와 작품 묶음으로 나눈다.

    scope가 비어 있으면 라이브러리 루트다. 루트 직속 파일(_root)은 한 권씩, 최상위 디렉토리는
    대표 한 권으로 묶는다. scope가 있으면 그 디렉토리에 직접 있는 파일은 한 권씩, 바로 아래
    디렉토리는 대표 한 권으로 묶는다. 묶음은 최신순으로 돌려준다.
    """
    hidden = hidden_categories or []
    file_categories: list[str] = []
    groups: dict[str, CoverGroup] = {}
    for category, (_count, updated, created) in category_stats.items():
        if is_ignored_category(category) or category_matches(category, hidden):
            continue
        if scope:
            if category == scope:
                file_categories.append(category)
                continue
            if not category.startswith(scope + "/"):
                continue
            segments = category[len(scope) + 1 :].split("/")
            key = f"{scope}/{segments[0]}"
            name = segments[0]
        else:
            if category == ROOT_CATEGORY:
                file_categories.append(category)
                continue
            key = name = category.split("/")[0]
        if category_matches(key, hidden):
            continue
        known = groups.get(key)
        groups[key] = CoverGroup(name, key, _later(known.updated if known else None, updated), _later(known.created if known else None, created))
    return file_categories, sorted(groups.values(), key=lambda group: group.rank)


def pick_representative(hits: list[tuple[int, dict[str, Any]]]) -> tuple[int, dict[str, Any]] | None:
    """파일 경로순으로 받은 후보 중 1권을 우선하고, 없으면 맨 앞을 대표로 한다."""
    if not hits:
        return None
    for hit in hits:
        source = hit[1]
        file_name = str(source.get("file_path", "")).rsplit("/", 1)[-1]
        if FIRST_INSTALLMENT_PATTERN.search(f"{source.get('title', '')} {file_name}"):
            return hit
    return hits[0]


def merge_cover_page(groups: list[CoverGroup], files: list[tuple[int, dict[str, Any], list[Any]]], size: int) -> tuple[list[tuple[str, Any]], int, int]:
    """최신순으로 정렬된 묶음과 파일을 합쳐 size개만 고른다.

    순서가 같으면 묶음이 먼저다. 반환: (항목 목록, 쓴 묶음 수, 쓴 파일 수).
    항목은 ("group", CoverGroup) 또는 ("file", (id, source, sort)).
    """
    entries: list[tuple[str, Any]] = []
    group_index = 0
    file_index = 0
    while len(entries) < size and (group_index < len(groups) or file_index < len(files)):
        take_group = file_index >= len(files)
        if not take_group and group_index < len(groups):
            take_group = groups[group_index].rank <= file_rank(files[file_index][2])
        if take_group:
            entries.append(("group", groups[group_index]))
            group_index += 1
        else:
            entries.append(("file", files[file_index]))
            file_index += 1
    return entries, group_index, file_index
