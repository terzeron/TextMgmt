# 카테고리 분류 제안·승인 구현 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 카테고리 관리 탭의 자동 분류를 "제안 → 검토 → 선택 승인"으로 바꾼다.

**Architecture:** 분류와 이동을 두 메서드로 나눈다. `propose_category_changes`가 파일별 제안과 확신도 등급을 만들어 공유 볼륨의 상태 파일에 쓰고, `apply_category_changes`가 승인된 항목만 옮긴다. 프론트는 제안 표를 그려 등급별 체크 상태를 주고, 사용자가 목적지를 직접 고칠 수 있게 한다.

**Tech Stack:** Python 3.13 / FastAPI / pytest, React / Vite / vitest, Elasticsearch

**Spec:** `docs/superpowers/specs/2026-09-16-category-classification-proposal-design.md`

## Global Constraints

- Python 패키지 추가·제거는 `uv add` / `uv remove`. `pip` 금지.
- 백엔드 변경 후 `source .env && uv run pytest tests/` 통과 확인.
- 프론트엔드 변경 후 `cd frontend && npx vitest run` 통과 확인.
- FastAPI 엔드포인트에는 `admin_dep`를 명시적으로 붙인다.
- 커밋 제목은 Conventional Commits, 본문은 `-` bullet 명령형 72자 이내.
- Edit/Write 도구는 포매터 훅이 파일 전체를 다시 포맷한다. 편집 후
  `git --no-pager diff --no-ext-diff --no-color -U0 -- <파일> | grep "^@@"`로
  hunk가 편집 영역 안인지 확인한다.
- 등급 문자열은 `certain` / `unsure` / `unknown` 세 개만 쓴다.
- 제안 항목의 고유 키는 `path_prefix` 기준 상대 경로 `file_path`다.

---

## File Structure

| 파일                                     | 책임                                           |
| ---------------------------------------- | ---------------------------------------------- |
| `backend/book_classifier.py`             | 모델 점수와 모델 카테고리를 `entry`로 내보낸다 |
| `backend/book_manager.py`                | 키워드 매칭, 등급 판정, 제안 생성, 승인 적용   |
| `backend/main.py`                        | 제안 상태 파일 입출력, 엔드포인트 4개          |
| `frontend/src/ClassifyProposalTable.jsx` | 제안 표 렌더와 선택 상태 (신규)                |
| `frontend/src/CategoryAdmin.jsx`         | 버튼 그룹 배치, 제안 흐름 연결                 |

`CategoryAdmin.jsx`는 이미 2,400줄이 넘는다. 표는 새 파일로 뺀다.

---

### Task 1: 분류기가 모델 점수를 내보내게 한다

**Files:**

- Modify: `backend/book_classifier.py:742-746`
- Test: `tests/test_book_classifier.py`

**Interfaces:**

- Produces: `classify_file()`이 돌려주는 `entry`에 `confidence: float | None`과
  `model_category: str | None`이 들어간다. 반환 튜플 모양은 그대로다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`tests/test_book_classifier.py`에 추가한다.

```python
def test_classify_file_exposes_model_confidence(tmp_path, monkeypatch):
    """등급을 매기려면 모델 점수가 밖으로 나와야 한다."""
    from backend.book_classifier import BookClassifierService

    service = BookClassifierService.__new__(BookClassifierService)
    service.cache = {}
    service.title_cache = {}
    service.library_root = tmp_path
    monkeypatch.setattr(service, "query_bookstores", lambda *a, **k: ({}, {}, {}))
    monkeypatch.setattr(service, "_decide_by_model", lambda *a, **k: ("3_SF", "모델 판정", 0.87))
    monkeypatch.setattr(service, "_decide_by_bookstore", lambda *a, **k: (None, "not_found", "서점에서 못 찾음"))

    class Policy:
        override_below = 0.5

        def prefers_bookstore(self, confidence):
            return confidence < self.override_below

    service.bookstore_policy = Policy()
    target = tmp_path / "[저자] 제목.epub"
    target.write_text("x")

    _cat, _method, _reason, entry = service.classify_file(target, tmp_path)

    assert entry["confidence"] == 0.87
    assert entry["model_category"] == "3_SF"
```

- [ ] **Step 2: 실패를 확인한다**

Run: `source .env && uv run pytest tests/test_book_classifier.py::test_classify_file_exposes_model_confidence -v`
Expected: FAIL — `KeyError: 'confidence'`

- [ ] **Step 3: 최소 구현**

`_decide`가 점수를 밖으로 넘기도록 반환값을 넓히고, `classify_file`이 `entry`에 담는다.
`backend/book_classifier.py`의 `_decide` 시그니처를 4-튜플로 바꾼다.

```python
    def _decide(self, fpath, effective_fname, entry, trust_single_match=True):
        """판정 결과에 모델 카테고리와 확신도를 함께 돌려준다.

        확신도는 등급 판정에 필요하다. 모델이 진 경우에도 점수를 알아야
        "점수가 낮아 서점을 썼다"를 화면에 설명할 수 있다.
        """
        model_cat, model_reason, confidence = self._decide_by_model(fpath, effective_fname)
        ...
        # 기존 return 4곳을 모두 (cat, method, reason, model_cat, confidence) 로 바꾼다
```

`classify_file`에서 받는다.

```python
        target_cat, method, reason, model_cat, confidence = self._decide(
            fpath if use_content_meta else None, effective_fname, entry, trust_single_match=trust_single_match
        )
        entry["confidence"] = confidence
        entry["model_category"] = model_cat
        entry["target_category"] = target_cat
        entry["status"] = method if target_cat else ("conflict" if "conflict" in method else "not_found")
        return target_cat, method, reason, entry
```

- [ ] **Step 4: 통과 확인**

Run: `source .env && uv run pytest tests/test_book_classifier.py -v`
Expected: PASS. 기존 테스트도 모두 통과한다.

- [ ] **Step 5: 커밋**

```bash
git add backend/book_classifier.py tests/test_book_classifier.py
git commit -m "feat: expose model confidence from classify_file

- 확신도 등급을 매기려면 모델 점수가 호출자에게 필요하다
- 반환 튜플은 그대로 두고 entry에 실어 기존 호출자를 건드리지 않는다"
```

---

### Task 2: 키워드 매칭을 떼어낸다

**Files:**

- Modify: `backend/book_manager.py:1283-1345` (`_classify_file_to_top_category`)
- Test: `tests/test_book_manager.py`

**Interfaces:**

- Produces: `_match_category_by_keywords(file_path, source_category, mappings)
-> tuple[str | None, list[str], str | None]`.
  반환은 `(카테고리, 매칭 키워드, 동점 사유)`다. 동점이면 카테고리가 `None`이고
  사유가 채워진다. 매칭이 아예 없으면 셋 다 비어 있다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```python
def test_match_category_by_keywords_reports_tie(tmp_path: Path):
    """동점이면 목적지를 고르지 않고 사유를 돌려준다."""
    manager = make_manager(tmp_path, DummyES())
    target = tmp_path / "A" / "SF음악.epub"
    target.parent.mkdir()
    target.write_text("x")
    mappings = {"3_SF": ["SF"], "5_음악": ["음악"]}

    category, keywords, tie_reason = manager._match_category_by_keywords(target, "A", mappings)

    assert category is None
    assert tie_reason is not None
    assert keywords


def test_match_category_by_keywords_picks_single_best(tmp_path: Path):
    """단독 최고점이면 그 카테고리를 고른다."""
    manager = make_manager(tmp_path, DummyES())
    target = tmp_path / "A" / "과학소설 모음.epub"
    target.parent.mkdir()
    target.write_text("x")
    mappings = {"3_SF": ["과학소설"], "5_음악": ["음악"]}

    category, keywords, tie_reason = manager._match_category_by_keywords(target, "A", mappings)

    assert category == "3_SF"
    assert keywords == ["과학소설"]
    assert tie_reason is None
```

- [ ] **Step 2: 실패를 확인한다**

Run: `source .env && uv run pytest tests/test_book_manager.py -k match_category_by_keywords -v`
Expected: FAIL — `AttributeError: '_match_category_by_keywords'`

- [ ] **Step 3: 최소 구현**

`_classify_file_to_top_category`의 키워드 부분을 그대로 새 메서드로 옮긴다.
분류기 fallback 부분은 Task 3에서 `_propose_category_for_file`이 대신한다.

```python
    def _match_category_by_keywords(self, file_path: Path, source_category: str, mappings: dict[str, list[str]]) -> tuple[str | None, list[str], str | None]:
        """등록된 키워드로 목적지를 고른다. 동점이면 고르지 않고 사유를 돌려준다."""
        haystack = self._classification_haystack(file_path)
        scored: list[tuple[int, int, str, list[str]]] = []

        for raw_category, raw_keywords in mappings.items():
            if not isinstance(raw_category, str):
                continue
            target_category = raw_category.strip()
            if target_category == source_category:
                continue
            if not self._is_top_level_target_category(target_category):
                continue
            if not self._is_safe_category_name(target_category):
                continue

            matched_keywords: list[str] = []
            seen_keywords: set[str] = set()
            for raw_keyword in raw_keywords or []:
                if not isinstance(raw_keyword, str):
                    continue
                keyword = raw_keyword.strip()
                normalized_keyword = self._normalize_classification_text(keyword)
                if not normalized_keyword or normalized_keyword in seen_keywords:
                    continue
                seen_keywords.add(normalized_keyword)
                if normalized_keyword in haystack:
                    matched_keywords.append(keyword)

            if matched_keywords:
                score = sum(len(self._normalize_classification_text(keyword)) for keyword in matched_keywords)
                scored.append((score, len(matched_keywords), target_category, matched_keywords))

        if not scored:
            return None, [], None

        scored.sort(key=lambda item: (-item[0], -item[1], item[2]))
        best_score, best_match_count, best_category, best_keywords = scored[0]
        tied = [category for score, count, category, _kw in scored if score == best_score and count == best_match_count]
        if len(tied) > 1:
            return None, best_keywords, f"여러 카테고리가 동일 점수로 일치합니다: {', '.join(tied[:3])}"
        return best_category, best_keywords, None
```

- [ ] **Step 4: 통과 확인**

Run: `source .env && uv run pytest tests/test_book_manager.py -k match_category_by_keywords -v`
Expected: PASS

- [ ] **Step 5: 커밋**

```bash
git add backend/book_manager.py tests/test_book_manager.py
git commit -m "refactor: split keyword matching out of classification

- 제안 생성은 키워드 결과와 모델 결과를 모두 알아야 등급을 매긴다
- 기존 함수는 둘을 합쳐 하나만 돌려줘 등급 판정에 쓸 수 없다"
```

---

### Task 3: 등급 판정과 제안 생성

**Files:**

- Modify: `backend/book_manager.py`
- Test: `tests/test_book_manager.py`

**Interfaces:**

- Consumes: Task 1의 `entry["confidence"]`, `entry["model_category"]`.
  Task 2의 `_match_category_by_keywords`.
- Produces:
  - `_is_high_confidence(classifier_service, confidence) -> bool`
  - `_propose_category_for_file(file_path, source_category, mappings,
classifier_service, use_bookstore, use_content_meta) -> dict`
    반환 키: `target_category`, `grade`, `confidence`, `source`,
    `matched_keywords`, `model_category`, `reason`
  - `propose_category_changes(category, mappings, *, content_type,
use_bookstore, use_content_meta, delay, on_progress) -> tuple[dict, str | None]`
    결과 키: `source_category`, `total_count`, `processed_count`, `items`, `failures`

- [ ] **Step 1: 등급 8가지 실패 테스트를 쓴다**

```python
class FakeClassifier:
    """classify_file 결과를 고정해 등급 규칙만 검증한다."""

    def __init__(self, target, method, reason, model_category, confidence, override_below=0.5):
        self._result = (target, method, reason, {"confidence": confidence, "model_category": model_category})

        class Policy:
            def __init__(self, boundary):
                self.override_below = boundary

            def prefers_bookstore(self, value):
                return self.override_below > 0.0 and value < self.override_below

        self.bookstore_policy = Policy(override_below)

    def classify_file(self, *args, **kwargs):
        return self._result


@pytest.mark.parametrize(
    "mappings,fake,expected_target,expected_grade",
    [
        ({"3_SF": ["과학소설"]}, FakeClassifier("3_SF", "model", "r", "3_SF", 0.9), "3_SF", "certain"),
        ({"3_SF": ["과학소설"]}, FakeClassifier("3_SF", "model", "r", "3_SF", 0.1), "3_SF", "unsure"),
        ({"3_SF": ["과학소설"]}, FakeClassifier("5_음악", "model", "r", "5_음악", 0.9), "3_SF", "unsure"),
        ({"3_SF": ["과학소설"], "5_음악": ["과학소설"]}, FakeClassifier(None, "not_found", "r", None, 0.1), None, "unknown"),
        ({}, FakeClassifier("3_SF", "model", "r", "3_SF", 0.9), "3_SF", "certain"),
        ({}, FakeClassifier("3_SF", "bookstore_majority", "r", None, 0.1), "3_SF", "certain"),
        ({}, FakeClassifier("3_SF", "bookstore_single", "r", None, 0.1), "3_SF", "unsure"),
        ({}, FakeClassifier(None, "conflict", "r", "3_SF", 0.1), None, "unknown"),
        ({}, FakeClassifier(None, "not_found", "r", None, 0.1), None, "unknown"),
    ],
)
def test_propose_category_grades(tmp_path: Path, mappings, fake, expected_target, expected_grade):
    manager = make_manager(tmp_path, DummyES())
    source = tmp_path / "A"
    source.mkdir()
    target = source / "과학소설 모음.epub"
    target.write_text("x")

    proposal = manager._propose_category_for_file(target, "A", mappings, fake, True, True)

    assert proposal["target_category"] == expected_target
    assert proposal["grade"] == expected_grade


def test_high_confidence_needs_a_calibrated_boundary(tmp_path: Path):
    """정책 파일이 없으면 경계가 0이라 모든 점수가 높음이 된다. 그러면 전부 자동 체크된다."""
    manager = make_manager(tmp_path, DummyES())
    fake = FakeClassifier("3_SF", "model", "r", "3_SF", 0.9, override_below=0.0)

    assert manager._is_high_confidence(fake, 0.9) is False
```

- [ ] **Step 2: 실패를 확인한다**

Run: `source .env && uv run pytest tests/test_book_manager.py -k "propose_category_grades or high_confidence" -v`
Expected: FAIL — `AttributeError: '_propose_category_for_file'`

- [ ] **Step 3: 최소 구현**

```python
    GRADE_CERTAIN = "certain"
    GRADE_UNSURE = "unsure"
    GRADE_UNKNOWN = "unknown"

    @staticmethod
    def _is_high_confidence(classifier_service: Any, confidence: float | None) -> bool:
        """모델 점수가 '높음'인가.

        경계는 서점 정책이 구간별 정답률로 보정한 값이다. 정책 파일이 없으면
        override_below 가 0 이고 prefers_bookstore 가 늘 False 를 돌려준다. 그 값을
        그대로 믿으면 점수와 무관하게 전부 '확실'이 되어 자동 체크된다. 근거가
        없으면 낮음으로 본다.
        """
        if confidence is None:
            return False
        policy = getattr(classifier_service, "bookstore_policy", None)
        if policy is None or getattr(policy, "override_below", 0.0) <= 0.0:
            return False
        return not policy.prefers_bookstore(confidence)

    def _propose_category_for_file(self, file_path, source_category, mappings, classifier_service, use_bookstore, use_content_meta) -> dict[str, Any]:
        """한 파일의 제안 목적지와 등급을 만든다. 파일을 옮기지 않는다."""
        keyword_category, matched_keywords, tie_reason = self._match_category_by_keywords(file_path, source_category, mappings)

        model_category = None
        confidence = None
        classified_category = None
        method = "not_found"
        reason = tie_reason or ""
        if classifier_service is not None:
            classified_category, method, classifier_reason, entry = classifier_service.classify_file(
                file_path, self._category_dir(source_category), use_bookstore=use_bookstore, use_content_meta=use_content_meta
            )
            model_category = (entry or {}).get("model_category")
            confidence = (entry or {}).get("confidence")
            reason = reason or classifier_reason or ""

        high = self._is_high_confidence(classifier_service, confidence)

        if keyword_category:
            grade = self.GRADE_CERTAIN if (high and model_category == keyword_category) else self.GRADE_UNSURE
            return {
                "target_category": keyword_category,
                "grade": grade,
                "confidence": confidence,
                "source": "keyword",
                "matched_keywords": matched_keywords,
                "model_category": model_category if model_category != keyword_category else None,
                "reason": reason,
            }

        if tie_reason:
            return {"target_category": None, "grade": self.GRADE_UNKNOWN, "confidence": confidence, "source": "keyword", "matched_keywords": matched_keywords, "model_category": model_category, "reason": tie_reason}

        if method == "model" and high and classified_category:
            grade, target = self.GRADE_CERTAIN, classified_category
        elif method == "bookstore_majority" and classified_category:
            grade, target = self.GRADE_CERTAIN, classified_category
        elif method == "bookstore_single" and classified_category:
            grade, target = self.GRADE_UNSURE, classified_category
        else:
            # 점수가 낮은 모델 답은 목적지로 쓰지 않는다. 근거에만 남긴다.
            grade, target = self.GRADE_UNKNOWN, None

        return {"target_category": target, "grade": grade, "confidence": confidence, "source": method, "matched_keywords": matched_keywords, "model_category": model_category, "reason": reason}
```

- [ ] **Step 4: 통과 확인**

Run: `source .env && uv run pytest tests/test_book_manager.py -k "propose_category_grades or high_confidence" -v`
Expected: PASS (10건)

- [ ] **Step 5: `propose_category_changes` 실패 테스트를 쓴다**

```python
def test_propose_category_changes_lists_items_without_moving(tmp_path: Path):
    """제안만 만들고 파일은 그대로 둔다."""
    manager = make_manager(tmp_path, DummyES())
    source = tmp_path / "A"
    source.mkdir()
    book = source / "과학소설 모음.epub"
    book.write_text("x")

    result, error = manager_propose(manager, "A", {"3_SF": ["과학소설"]})

    assert error is None
    assert result["total_count"] == 1
    assert result["items"][0]["file_path"] == "A/과학소설 모음.epub"
    assert book.is_file()
```

`manager_propose`는 테스트 헬퍼다. 파일 상단에 둔다.

```python
def manager_propose(manager, category, mappings):
    return asyncio_runner(manager.propose_category_changes(category, mappings, use_bookstore=False, use_content_meta=False))
```

- [ ] **Step 6: 실패 확인 후 구현**

Run: `source .env && uv run pytest tests/test_book_manager.py -k propose_category_changes -v`
Expected: FAIL → 구현 후 PASS

```python
    async def propose_category_changes(self, category: str, mappings: dict[str, list[str]] | None = None, *, content_type: str = "book", use_bookstore: bool = True, use_content_meta: bool = True, delay: float = 1.2, on_progress: Callable[[dict[str, int]], None] | None = None) -> tuple[dict[str, Any], str | None]:
        """선택 카테고리 직하위 파일의 분류 제안을 만든다. 파일을 옮기지 않는다."""
        if not category:
            return {}, "카테고리 이름이 비어있습니다"
        if not self._is_safe_category_name(category):
            return {}, "잘못된 카테고리 경로입니다"
        source_dir = self._category_dir(category)
        if not source_dir.is_dir():
            return {}, f"디렉토리를 찾을 수 없습니다: {category}"

        mappings = mappings or {}
        classifier_service: BookClassifierService | None = None
        if use_bookstore or use_content_meta:
            classifier_service = BookClassifierService(library_root=self.path_prefix, delay=delay, es_manager=self.es_manager)

        file_paths = self._iter_category_indexable_files(category, recursive=False)
        result: dict[str, Any] = {"content_type": content_type, "source_category": category, "total_count": len(file_paths), "processed_count": 0, "items": [], "failures": []}

        for file_path in file_paths:
            result["processed_count"] += 1
            try:
                rel_path = str(file_path.relative_to(self.path_prefix))
            except ValueError:
                result["failures"].append({"file_path": str(file_path), "error": "잘못된 파일 경로입니다"})
                continue
            try:
                proposal = self._propose_category_for_file(file_path, category, mappings, classifier_service, use_bookstore, use_content_meta)
            except Exception as e:
                LOGGER.error("분류 제안 실패: %s — %s", rel_path, e)
                result["failures"].append({"file_path": rel_path, "error": "분류 제안에 실패했습니다"})
                continue
            result["items"].append({"file_path": rel_path, "title": file_path.stem, "current_category": category, "apply_status": "pending", "apply_error": None, **proposal})
            if on_progress is not None:
                on_progress({"total_count": result["total_count"], "processed_count": result["processed_count"]})

        return result, None
```

- [ ] **Step 7: 커밋**

```bash
git add backend/book_manager.py tests/test_book_manager.py
git commit -m "feat: propose category changes without moving files

- 관리자가 결과를 미리 보고 고를 수 있어야 한다
- 확신도 등급을 8가지 경우로 나눠 체크박스 초기 상태를 정한다
- 정책 경계가 0이면 낮음으로 본다. 전부 자동 체크되면 검토가 무의미하다"
```

---

### Task 4: 승인 적용

**Files:**

- Modify: `backend/book_manager.py`
- Test: `tests/test_book_manager.py`

**Interfaces:**

- Consumes: `_move_classified_file(file_path, target_category, matched_keywords,
content_type=..., dry_run=False, clean_existing=..., source_category=...)`
- Produces: `apply_category_changes(items, allowed_file_paths, *, content_type,
clean_existing, on_progress) -> tuple[dict, str | None]`.
  `items`는 `{"file_path": str, "target_category": str}` 목록이다.
  `allowed_file_paths`는 제안 목록의 경로 집합이다.
  결과 키: `total_count`, `applied_count`, `failed_count`, `results`

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```python
def test_apply_category_changes_rejects_unknown_file(tmp_path: Path):
    """제안에 없던 파일은 옮기지 않는다. 화면이 보낸 경로를 그대로 믿으면 안 된다."""
    manager = make_manager(tmp_path, DummyES())
    (tmp_path / "A").mkdir()
    (tmp_path / "A" / "book.epub").write_text("x")

    result, error = asyncio_runner(
        manager.apply_category_changes([{"file_path": "A/book.epub", "target_category": "3_SF"}], allowed_file_paths=set())
    )

    assert error is None
    assert result["applied_count"] == 0
    assert result["results"][0]["apply_error"] == "제안 목록에 없는 파일입니다"


def test_apply_category_changes_rejects_unsafe_target(tmp_path: Path):
    """사용자가 고른 목적지도 검증한다."""
    manager = make_manager(tmp_path, DummyES())
    (tmp_path / "A").mkdir()
    (tmp_path / "A" / "book.epub").write_text("x")
    allowed = {"A/book.epub"}

    result, _error = asyncio_runner(
        manager.apply_category_changes([{"file_path": "A/book.epub", "target_category": "../밖"}], allowed_file_paths=allowed)
    )

    assert result["applied_count"] == 0
    assert "카테고리" in result["results"][0]["apply_error"]


def test_apply_category_changes_uses_user_chosen_target(tmp_path: Path):
    """사용자가 제안과 다른 목적지를 고르면 그쪽으로 옮긴다."""
    manager = make_manager(tmp_path, DummyES())
    (tmp_path / "A").mkdir()
    (tmp_path / "5_음악").mkdir()
    (tmp_path / "A" / "book.epub").write_text("x")
    allowed = {"A/book.epub"}

    result, _error = asyncio_runner(
        manager.apply_category_changes([{"file_path": "A/book.epub", "target_category": "5_음악"}], allowed_file_paths=allowed)
    )

    assert result["applied_count"] == 1
    assert (tmp_path / "5_음악" / "book.epub").is_file()


def test_apply_category_changes_records_missing_file(tmp_path: Path):
    """제안을 만든 뒤 파일이 사라졌으면 실패로 남기고 나머지를 계속 처리한다."""
    manager = make_manager(tmp_path, DummyES())
    (tmp_path / "A").mkdir()
    allowed = {"A/사라진책.epub"}

    result, _error = asyncio_runner(
        manager.apply_category_changes([{"file_path": "A/사라진책.epub", "target_category": "3_SF"}], allowed_file_paths=allowed)
    )

    assert result["failed_count"] == 1
    assert "파일" in result["results"][0]["apply_error"]
```

- [ ] **Step 2: 실패를 확인한다**

Run: `source .env && uv run pytest tests/test_book_manager.py -k apply_category_changes -v`
Expected: FAIL — `AttributeError: 'apply_category_changes'`

- [ ] **Step 3: 최소 구현**

```python
    async def apply_category_changes(self, items: list[dict[str, Any]], allowed_file_paths: set[str], *, content_type: str = "book", clean_existing: bool = False, on_progress: Callable[[dict[str, int]], None] | None = None) -> tuple[dict[str, Any], str | None]:
        """승인된 항목만 실제로 옮긴다.

        목적지는 사용자가 화면에서 고칠 수 있으므로 값을 그대로 믿지 않는다.
        제안 목록에 있던 파일인지, 목적지가 안전한 최상위 카테고리인지 다시 본다.
        """
        result: dict[str, Any] = {"content_type": content_type, "total_count": len(items), "applied_count": 0, "failed_count": 0, "results": []}

        for item in items:
            file_path_value = item.get("file_path")
            target_category = item.get("target_category")
            entry: dict[str, Any] = {"file_path": file_path_value, "target_category": target_category, "apply_status": "failed", "apply_error": None}

            if not isinstance(file_path_value, str) or file_path_value not in allowed_file_paths:
                entry["apply_error"] = "제안 목록에 없는 파일입니다"
            elif not isinstance(target_category, str) or not self._is_top_level_target_category(target_category) or not self._is_safe_category_name(target_category):
                entry["apply_error"] = "옮길 수 없는 카테고리입니다"
            else:
                absolute_path = self.path_prefix / file_path_value
                if not absolute_path.is_file():
                    entry["apply_error"] = "파일을 찾을 수 없습니다"
                else:
                    source_category = file_path_value.rsplit("/", 1)[0] if "/" in file_path_value else "_root"
                    file_result, error = await self._move_classified_file(
                        absolute_path, target_category, item.get("matched_keywords") or [], content_type=content_type, dry_run=False, clean_existing=clean_existing, source_category=source_category
                    )
                    if error is not None or file_result is None:
                        entry["apply_error"] = error or "분류 적용에 실패했습니다"
                    else:
                        entry["apply_status"] = "moved"

            if entry["apply_status"] == "moved":
                result["applied_count"] += 1
            else:
                result["failed_count"] += 1
            result["results"].append(entry)
            if on_progress is not None:
                on_progress({"total_count": result["total_count"], "applied_count": result["applied_count"], "failed_count": result["failed_count"]})

        return result, None
```

- [ ] **Step 4: 통과 확인**

Run: `source .env && uv run pytest tests/test_book_manager.py -k apply_category_changes -v`
Expected: PASS (4건)

- [ ] **Step 5: 커밋**

```bash
git add backend/book_manager.py tests/test_book_manager.py
git commit -m "feat: apply only approved category changes

- 목적지를 화면에서 고칠 수 있으므로 서버가 값을 다시 검증한다
- 제안 목록에 없는 경로와 최상위가 아닌 카테고리를 거부한다
- 제안은 시점 스냅샷이라 사라진 파일은 실패로 남기고 계속 진행한다"
```

---

### Task 5: 상태 파일과 엔드포인트

**Files:**

- Modify: `backend/main.py:414-550` (상태 파일 기계와 모델), `backend/main.py:731-783` (엔드포인트)
- Test: `tests/test_main_mock.py`

**Interfaces:**

- Consumes: Task 3의 `propose_category_changes`, Task 4의 `apply_category_changes`
- Produces: 엔드포인트 4개.
  - `POST /categories/classify-proposal` body `{"category": str}`
  - `GET /categories/classify-proposal`
  - `POST /categories/classify-proposal/apply` body
    `{"items": [{"file_path": str, "target_category": str}]}`
  - `DELETE /categories/classify-proposal`

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```python
def test_classify_proposal_apply_passes_allowed_paths(monkeypatch, admin_client):
    """승인 요청의 목적지는 받되, 허용 경로는 서버가 제안 상태에서 가져온다."""
    captured = {}

    async def fake_apply(items, allowed_file_paths, **kwargs):
        captured["items"] = items
        captured["allowed"] = allowed_file_paths
        return {"total_count": len(items), "applied_count": len(items), "failed_count": 0, "results": []}, None

    monkeypatch.setattr(book_manager, "apply_category_changes", fake_apply)
    monkeypatch.setattr(main, "_read_classify_proposal", lambda: {"status": "ready", "items": [{"file_path": "A/book.epub"}]})

    response = admin_client.post("/categories/classify-proposal/apply", json={"items": [{"file_path": "A/book.epub", "target_category": "5_음악"}]})

    assert response.status_code == 200
    assert captured["allowed"] == {"A/book.epub"}
    assert captured["items"][0]["target_category"] == "5_음악"
```

- [ ] **Step 2: 실패를 확인한다**

Run: `source .env && uv run pytest tests/test_main_mock.py -k classify_proposal -v`
Expected: FAIL — 404

- [ ] **Step 3: 상태 파일 기계 이름을 바꾼다**

`backend/main.py`에서 이름만 바꾸고 동작은 그대로 둔다. 하트비트 만료 처리를 그대로 쓰기 위해서다.

| 기존                                    | 새 이름                           |
| --------------------------------------- | --------------------------------- |
| `auto_classify_status`                  | `classify_proposal_state`         |
| `_auto_classify_status_path`            | `_classify_proposal_path`         |
| `_read_auto_classify_status`            | `_read_classify_proposal`         |
| `_replace_auto_classify_status`         | `_replace_classify_proposal`      |
| `_start_auto_classify_status`           | `_start_classify_proposal`        |
| `_progress_auto_classify_status`        | `_progress_classify_proposal`     |
| `AUTO_CLASSIFY_HEARTBEAT_STALE_SECONDS` | `CLASSIFY_PROPOSAL_STALE_SECONDS` |

파일명도 `.auto_classify_status_{content_type}.json`에서
`.classify_proposal_{content_type}.json`으로 바꾼다.

`stale_auto_classify_status`는 모듈 수준 순수 함수이므로 이름을
`stale_running_status`로 바꾸고 그대로 쓴다.

- [ ] **Step 4: 엔드포인트를 쓴다**

```python
class ClassifyProposalModel(BaseModel):
    category: str
    use_bookstore: bool = True
    use_content_meta: bool = True
    delay: float = 1.2


class ClassifyApplyItemModel(BaseModel):
    file_path: str
    target_category: str


class ClassifyApplyModel(BaseModel):
    items: list[ClassifyApplyItemModel]
    clean_existing: bool = False
```

```python
    @router.post("/categories/classify-proposal", dependencies=admin_dep)
    async def start_classify_proposal(body: ClassifyProposalModel, background_tasks: BackgroundTasks) -> dict[str, Any]:
        """선택 카테고리의 분류 제안을 백그라운드로 만든다."""
        current = _read_classify_proposal()
        if current.get("status") in ("running", "applying"):
            return {"status": "success", "result": {"already_running": True, **current}}
        _start_classify_proposal(body.category)
        background_tasks.add_task(_run_classify_proposal_job, body.category, body.use_bookstore, body.use_content_meta, body.delay)
        return {"status": "success", "result": {"started": True, **_read_classify_proposal()}}

    @router.get("/categories/classify-proposal", dependencies=admin_dep)
    async def get_classify_proposal() -> dict[str, Any]:
        return {"status": "success", "result": _read_classify_proposal()}

    @router.post("/categories/classify-proposal/apply", dependencies=admin_dep)
    async def apply_classify_proposal(body: ClassifyApplyModel, background_tasks: BackgroundTasks) -> dict[str, Any]:
        """승인된 항목만 적용한다. 허용 경로는 서버 제안 상태에서 가져온다."""
        current = _read_classify_proposal()
        if current.get("status") != "ready":
            return {"status": "failure", "error": "적용할 제안이 없습니다."}
        allowed = {item.get("file_path") for item in current.get("items") or [] if item.get("file_path")}
        items = [item.model_dump() for item in body.items]
        background_tasks.add_task(_run_classify_apply_job, items, allowed, body.clean_existing)
        return {"status": "success", "result": {"started": True, "total_count": len(items)}}

    @router.delete("/categories/classify-proposal", dependencies=admin_dep)
    async def delete_classify_proposal() -> dict[str, Any]:
        _replace_classify_proposal({"status": "idle", "content_type": content_type, "items": []})
        return {"status": "success", "result": {"cleared": True}}
```

백그라운드 작업 두 개를 추가한다.

```python
    async def _run_classify_proposal_job(category: str, use_bookstore: bool, use_content_meta: bool, delay: float) -> None:
        try:
            mappings = await asyncio.to_thread(category_mapping.get_all_mappings, content_type=content_type)
            result, error = await manager.propose_category_changes(category, mappings, content_type=content_type, use_bookstore=use_bookstore, use_content_meta=use_content_meta, delay=delay, on_progress=_progress_classify_proposal)
        except Exception as e:
            LOGGER.error("classify proposal error: %s", e)
            _replace_classify_proposal({**_read_classify_proposal(), "status": "failed", "error": "분류 제안에 실패했습니다."})
            return
        if error is None:
            _replace_classify_proposal({**_read_classify_proposal(), **result, "status": "ready"})
        else:
            _replace_classify_proposal({**_read_classify_proposal(), "status": "failed", "error": error})

    async def _run_classify_apply_job(items: list[dict[str, Any]], allowed: set[str], clean_existing: bool) -> None:
        _replace_classify_proposal({**_read_classify_proposal(), "status": "applying"})
        try:
            result, error = await manager.apply_category_changes(items, allowed, content_type=content_type, clean_existing=clean_existing, on_progress=_progress_classify_proposal)
        except Exception as e:
            LOGGER.error("classify apply error: %s", e)
            _replace_classify_proposal({**_read_classify_proposal(), "status": "failed", "error": "분류 적용에 실패했습니다."})
            return
        if error is not None:
            _replace_classify_proposal({**_read_classify_proposal(), "status": "failed", "error": error})
            return
        applied = {entry["file_path"]: entry for entry in result["results"]}
        current = _read_classify_proposal()
        merged_items = []
        for item in current.get("items") or []:
            outcome = applied.get(item.get("file_path"))
            merged_items.append({**item, **({"apply_status": outcome["apply_status"], "apply_error": outcome["apply_error"]} if outcome else {})})
        _replace_classify_proposal({**current, "items": merged_items, "status": "done", "applied_count": result["applied_count"], "failed_count": result["failed_count"]})
```

- [ ] **Step 5: 구 엔드포인트를 걷어낸다**

`POST /categories/auto-classify`, `GET /categories/auto-classify-status`,
`CategoryAutoClassifyModel`, `BookManager.auto_classify_category`,
`_classify_file_to_top_category`를 지운다. 관련 테스트도 새 흐름 테스트로 바꾼다.

Run: `source .env && uv run grep -rn "auto_classify" backend/ utils/ | wc -l`
Expected: `0`

- [ ] **Step 6: 통과 확인**

Run: `source .env && uv run pytest tests/ -q`
Expected: 전부 통과

- [ ] **Step 7: 커밋**

```bash
git add backend/main.py tests/test_main_mock.py tests/test_main.py tests/test_book_manager.py
git commit -m "feat: add classify proposal and approval endpoints

- 제안 생성과 승인 적용을 백그라운드 작업 두 개로 나눈다
- 허용 경로는 서버 제안 상태에서 가져와 임의 경로 이동을 막는다
- 즉시 이동하던 auto-classify 경로를 걷어낸다"
```

---

### Task 6: 버튼 그룹을 키워드 영역 아래로

**Files:**

- Modify: `frontend/src/CategoryAdmin.jsx:1743-1900`
- Test: `frontend/tests/CategoryAdmin.test.jsx`

**Interfaces:**

- Produces: 없음. 배치만 바꾼다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```jsx
it("버튼 그룹이 키워드 영역보다 아래에 온다", async () => {
  setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
  render(<CategoryAdmin />);
  await waitFor(() => expect(screen.getByText("1_fiction")).toBeTruthy());
  fireEvent.click(screen.getByText("1_fiction"));

  const keywordArea = await screen.findByText(/키워드/);
  const renameButton = screen.getByTitle("이름 변경");
  const position = keywordArea.compareDocumentPosition(renameButton);

  expect(position & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cd frontend && npx vitest run tests/CategoryAdmin.test.jsx -t "버튼 그룹이 키워드"`
Expected: FAIL

- [ ] **Step 3: JSX 블록 순서를 바꾼다**

`selectedCategory && !selectedMismatch` 패널 안에서 버튼 그룹 `<div>`를 키워드
입력 영역 뒤로 옮긴다. 마크업은 그대로 두고 위치만 바꾼다.

- [ ] **Step 4: 통과 확인**

Run: `cd frontend && npx vitest run tests/CategoryAdmin.test.jsx`
Expected: PASS

- [ ] **Step 5: 커밋**

```bash
git add frontend/src/CategoryAdmin.jsx frontend/tests/CategoryAdmin.test.jsx
git commit -m "refactor: move category action buttons below keyword area"
```

---

### Task 7: 제안 표 컴포넌트

**Files:**

- Create: `frontend/src/ClassifyProposalTable.jsx`
- Create: `frontend/tests/ClassifyProposalTable.test.jsx`

**Interfaces:**

- Produces: `<ClassifyProposalTable items categories selection onSelectionChange
targets onTargetChange />`
  - `items`: 서버 제안 항목 배열
  - `categories`: 목적지 후보 문자열 배열 (최상위 카테고리)
  - `selection`: `Set<string>` 선택된 `file_path`
  - `targets`: `{[file_path]: string}` 사용자가 고른 목적지
  - `onSelectionChange(nextSet)`, `onTargetChange(filePath, category)`

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```jsx
// @vitest-environment jsdom
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import ClassifyProposalTable from "../src/ClassifyProposalTable";

const ITEMS = [
  {
    file_path: "A/a.epub",
    title: "확실한 책",
    current_category: "A",
    target_category: "3_SF",
    grade: "certain",
    confidence: 0.91,
    source: "model",
    reason: "모델 판정",
  },
  {
    file_path: "A/b.epub",
    title: "애매한 책",
    current_category: "A",
    target_category: "5_음악",
    grade: "unsure",
    confidence: 0.2,
    source: "bookstore_single",
    reason: "서점 한 곳",
  },
  {
    file_path: "A/c.epub",
    title: "불확실한 책",
    current_category: "A",
    target_category: null,
    grade: "unknown",
    confidence: null,
    source: "conflict",
    reason: "서점 판정이 갈림",
  },
];
const CATEGORIES = ["3_SF", "5_음악", "1_서양고전"];

function renderTable(overrides = {}) {
  const props = {
    items: ITEMS,
    categories: CATEGORIES,
    selection: new Set(["A/a.epub"]),
    targets: {},
    onSelectionChange: vi.fn(),
    onTargetChange: vi.fn(),
    ...overrides,
  };
  render(<ClassifyProposalTable {...props} />);
  return props;
}

describe("ClassifyProposalTable", () => {
  it("등급에 따라 체크박스 초기 상태가 다르다", () => {
    renderTable();
    expect(screen.getByLabelText("A/a.epub 선택").checked).toBe(true);
    expect(screen.getByLabelText("A/b.epub 선택").checked).toBe(false);
    expect(screen.getByLabelText("A/c.epub 선택").disabled).toBe(true);
  });

  it("불확실 행도 목적지를 고르면 체크할 수 있다", () => {
    const props = renderTable({ targets: { "A/c.epub": "1_서양고전" } });
    expect(screen.getByLabelText("A/c.epub 선택").disabled).toBe(false);
    fireEvent.click(screen.getByLabelText("A/c.epub 선택"));
    expect(props.onSelectionChange).toHaveBeenCalled();
  });

  it("전체 선택은 비활성 행을 건너뛴다", () => {
    const props = renderTable();
    fireEvent.click(screen.getByLabelText("전체 선택"));
    const next = props.onSelectionChange.mock.calls[0][0];
    expect(next.has("A/c.epub")).toBe(false);
    expect(next.has("A/b.epub")).toBe(true);
  });

  it("목적지를 바꾸면 알린다", () => {
    const props = renderTable();
    fireEvent.change(screen.getByLabelText("A/b.epub 목적지"), {
      target: { value: "1_서양고전" },
    });
    expect(props.onTargetChange).toHaveBeenCalledWith("A/b.epub", "1_서양고전");
  });
});
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cd frontend && npx vitest run tests/ClassifyProposalTable.test.jsx`
Expected: FAIL — 모듈 없음

- [ ] **Step 3: 컴포넌트를 쓴다**

```jsx
import PropTypes from "prop-types";
import { Table, Form } from "react-bootstrap";

// 목적지가 있어야 승인할 수 있다. 불확실 행도 사용자가 고르면 켜진다.
export function resolveTarget(item, targets) {
  return targets[item.file_path] ?? item.target_category ?? "";
}

export function isSelectable(item, targets) {
  return Boolean(resolveTarget(item, targets));
}

export default function ClassifyProposalTable({
  items,
  categories,
  selection,
  targets,
  onSelectionChange,
  onTargetChange,
}) {
  const selectablePaths = items
    .filter((item) => isSelectable(item, targets))
    .map((item) => item.file_path);
  const allSelected =
    selectablePaths.length > 0 &&
    selectablePaths.every((path) => selection.has(path));

  const toggleAll = () => {
    onSelectionChange(allSelected ? new Set() : new Set(selectablePaths));
  };

  const toggleOne = (filePath) => {
    const next = new Set(selection);
    if (next.has(filePath)) next.delete(filePath);
    else next.add(filePath);
    onSelectionChange(next);
  };

  return (
    <Table size="sm" bordered hover responsive className="mt-2">
      <thead>
        <tr>
          <th>
            <Form.Check
              type="checkbox"
              aria-label="전체 선택"
              checked={allSelected}
              onChange={toggleAll}
            />
          </th>
          <th>책</th>
          <th>현재</th>
          <th>제안</th>
          <th>점수</th>
          <th>근거</th>
        </tr>
      </thead>
      <tbody>
        {items.map((item) => {
          const target = resolveTarget(item, targets);
          const selectable = isSelectable(item, targets);
          const changed =
            target && item.target_category && target !== item.target_category;
          return (
            <tr
              key={item.file_path}
              className={
                item.apply_status === "failed" ? "table-danger" : undefined
              }
            >
              <td>
                <Form.Check
                  type="checkbox"
                  aria-label={`${item.file_path} 선택`}
                  checked={selection.has(item.file_path)}
                  disabled={!selectable}
                  onChange={() => toggleOne(item.file_path)}
                />
              </td>
              <td>{item.title || item.file_path}</td>
              <td>{item.current_category}</td>
              <td>
                <Form.Select
                  size="sm"
                  aria-label={`${item.file_path} 목적지`}
                  value={target}
                  onChange={(event) =>
                    onTargetChange(item.file_path, event.target.value)
                  }
                >
                  <option value="">(선택 안 함)</option>
                  {categories.map((category) => (
                    <option key={category} value={category}>
                      {category}
                    </option>
                  ))}
                </Form.Select>
                {changed && <small className="text-primary">직접 지정</small>}
              </td>
              <td>
                {item.confidence == null ? "-" : item.confidence.toFixed(2)}
              </td>
              <td>
                <small>{item.apply_error || item.reason}</small>
              </td>
            </tr>
          );
        })}
      </tbody>
    </Table>
  );
}

ClassifyProposalTable.propTypes = {
  items: PropTypes.array.isRequired,
  categories: PropTypes.array.isRequired,
  selection: PropTypes.object.isRequired,
  targets: PropTypes.object.isRequired,
  onSelectionChange: PropTypes.func.isRequired,
  onTargetChange: PropTypes.func.isRequired,
};
```

- [ ] **Step 4: 통과 확인**

Run: `cd frontend && npx vitest run tests/ClassifyProposalTable.test.jsx`
Expected: PASS (4건)

- [ ] **Step 5: 커밋**

```bash
git add frontend/src/ClassifyProposalTable.jsx frontend/tests/ClassifyProposalTable.test.jsx
git commit -m "feat: add classify proposal table component

- 등급에 따라 체크박스를 켜고 끄고 잠근다
- 목적지를 직접 고르면 불확실 행도 승인할 수 있다"
```

---

### Task 8: 제안 흐름 연결과 복원

**Files:**

- Modify: `frontend/src/CategoryAdmin.jsx`
- Test: `frontend/tests/CategoryAdmin.test.jsx`

**Interfaces:**

- Consumes: Task 7의 `ClassifyProposalTable`, Task 5의 엔드포인트 4개

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```jsx
it("제안이 준비되면 표를 보여주고 선택한 것만 승인한다", async () => {
  const proposal = {
    status: "ready",
    source_category: "1_fiction",
    items: [
      {
        file_path: "1_fiction/a.epub",
        title: "a",
        current_category: "1_fiction",
        target_category: "3_SF",
        grade: "certain",
        confidence: 0.9,
        source: "model",
        reason: "r",
      },
      {
        file_path: "1_fiction/b.epub",
        title: "b",
        current_category: "1_fiction",
        target_category: "5_음악",
        grade: "unsure",
        confidence: 0.2,
        source: "bookstore_single",
        reason: "r",
      },
    ],
  };
  mockJsonGetReq.mockImplementation((url, _p, resolve) => {
    if (url === "/categories") resolve(CATEGORIES_RESPONSE);
    else if (url === "/category-mismatches")
      resolve({ mismatches: [], es_only: [], fs_only: [] });
    else if (url === "/categories/classify-proposal") resolve(proposal);
    else if (url.startsWith("/category-mismatches/reload-status"))
      resolve({ status: "idle" });
    else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
    else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    else if (url.startsWith("/latest-excluded-categories"))
      resolve(LATEST_EXCLUDED_RESPONSE);
  });

  render(<CategoryAdmin />);
  await waitFor(() =>
    expect(screen.getByLabelText("1_fiction/a.epub 선택")).toBeTruthy(),
  );
  expect(screen.getByLabelText("1_fiction/a.epub 선택").checked).toBe(true);
  expect(screen.getByLabelText("1_fiction/b.epub 선택").checked).toBe(false);

  mockJsonPostReq.mockImplementation((url, payload, resolve) =>
    resolve({ started: true, total_count: payload.items.length }),
  );
  fireEvent.click(screen.getByRole("button", { name: /분류 승인/ }));
  fireEvent.click(
    within(await screen.findByRole("dialog")).getByRole("button", {
      name: /분류 승인/,
    }),
  );

  await waitFor(() => {
    const call = mockJsonPostReq.mock.calls.find(
      ([url]) => url === "/categories/classify-proposal/apply",
    );
    expect(call[1].items).toEqual([
      { file_path: "1_fiction/a.epub", target_category: "3_SF" },
    ]);
  });
});
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cd frontend && npx vitest run tests/CategoryAdmin.test.jsx -t "제안이 준비되면"`
Expected: FAIL

- [ ] **Step 3: 구현한다**

`CategoryAdmin.jsx`에 상태를 넣는다.

```jsx
const [proposal, setProposal] = useState(null); // 서버 제안 상태
const [proposalSelection, setProposalSelection] = useState(new Set());
const [proposalTargets, setProposalTargets] = useState({});
```

마운트 시 한 번 조회하고, `ready`면 `source_category`를 선택 상태로 되돌린다.
`certain` 항목을 기본 선택으로 채운다.

```jsx
const applyProposalState = useCallback((result) => {
  setProposal(result);
  if (!result || !Array.isArray(result.items)) return;
  setProposalSelection(
    new Set(
      result.items
        .filter((item) => item.grade === "certain" && item.target_category)
        .map((item) => item.file_path),
    ),
  );
  setProposalTargets({});
  if (result.status === "ready" && result.source_category) {
    // 표는 선택 패널 안에 있다. 돌아왔을 때 하던 작업으로 복귀시킨다.
    setSelectedCategory(result.source_category);
  }
}, []);
```

승인 요청은 선택된 행만, 사용자가 고친 목적지로 보낸다.

```jsx
const handleApplyProposal = useCallback(() => {
  const items = (proposal?.items || [])
    .filter((item) => proposalSelection.has(item.file_path))
    .map((item) => ({
      file_path: item.file_path,
      target_category: proposalTargets[item.file_path] ?? item.target_category,
    }));
  jsonPostReq(
    apiPrefix + "/categories/classify-proposal/apply",
    { items },
    () => setProposalPolling(true),
    (error) =>
      setMessage(formatErrorMessage(error, "분류 승인에 실패했습니다.")),
  );
}, [proposal, proposalSelection, proposalTargets, apiPrefix]);
```

표는 버튼 그룹 아래에 렌더한다.

```jsx
{
  proposal?.status === "ready" && (
    <>
      <ClassifyProposalTable
        items={proposal.items}
        categories={topLevelCategories}
        selection={proposalSelection}
        targets={proposalTargets}
        onSelectionChange={setProposalSelection}
        onTargetChange={(filePath, category) =>
          setProposalTargets((prev) => ({ ...prev, [filePath]: category }))
        }
      />
      <Button
        variant="primary"
        size="sm"
        disabled={proposalSelection.size === 0}
        onClick={() => setShowApplyModal(true)}
      >
        분류 승인 ({proposalSelection.size}건)
      </Button>
    </>
  );
}
```

`topLevelCategories`는 `esDocCounts`의 키에서 `/`가 없고 `_root`가 아닌 것만 추린다.

- [ ] **Step 4: 통과 확인**

Run: `cd frontend && npx vitest run && npm run lint`
Expected: 전부 통과

- [ ] **Step 5: 전체 검증**

Run: `source .env && uv run pytest tests/ -q`
Run: `cd frontend && npx vitest run`
Expected: 전부 통과

- [ ] **Step 6: 커밋**

```bash
git add frontend/src/CategoryAdmin.jsx frontend/tests/CategoryAdmin.test.jsx
git commit -m "feat: wire classify proposal review and approval flow

- 확실 항목만 기본 선택하고 사용자가 고친 목적지로 승인 요청을 보낸다
- 돌아왔을 때 제안이 남아 있으면 그 카테고리를 다시 선택해 표를 되살린다"
```

---

## Self-Review

**Spec coverage**

| 명세 절              | 담당 Task                             |
| -------------------- | ------------------------------------- |
| 2 확신도 등급        | Task 3 (8가지 parametrize)            |
| 3 상태 파일          | Task 5 Step 3                         |
| 4.1 메서드 2개       | Task 3, Task 4                        |
| 4.2 분류기 확장      | Task 1                                |
| 4.3 API 4개          | Task 5                                |
| 4.4 제거 대상        | Task 5 Step 5                         |
| 5.1 버튼 재배치      | Task 6                                |
| 5.2 제안 표·드롭다운 | Task 7                                |
| 5.3 복원             | Task 8                                |
| 6 오류 처리          | Task 4 (건별 실패), Task 5 (job 예외) |
| 7 테스트             | 각 Task의 테스트 단계                 |

**Type consistency**

- `grade` 값은 `certain` / `unsure` / `unknown`으로 Task 3·7·8에서 동일하다.
- `apply_category_changes(items, allowed_file_paths, ...)`의 인자 이름이 Task 4
  구현과 Task 5 테스트에서 같다.
- 승인 payload는 `{items: [{file_path, target_category}]}`로 Task 5·7·8에서 같다.

**남은 위험**

- Task 5는 참조가 약 170곳이라 가장 크다. 테스트 교체가 작업의 절반이다.
- 제안 생성이 모든 파일에 모델을 돌려 느려진다. 진행률로만 완화한다.
