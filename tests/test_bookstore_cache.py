#!/usr/bin/env python3
"""서점 응답 캐시 테스트.

같은 책을 두 번 묻지 않게 하는 것이 목적이다. 그래서 확인할 것도 두 가지다.
어떤 키로 같은 책을 알아보는가, 그리고 어떤 응답을 남기면 안 되는가.
"""

import json
from pathlib import Path

from backend.bookstore_cache import BookstoreResponseCache

STORES = {"yes24": {"title": "달빛조각사", "author": "남희성", "cat": "소설 > 판타지", "mapped": "3_판타지", "isbn": "9791162203484"}, "aladin": {"title": "달빛조각사", "author": "남희성", "cat": "국내도서 > 판타지", "mapped": "3_판타지", "isbn": "9791162203484"}, "kyobo": {"title": "", "author": "", "cat": "", "mapped": None, "isbn": ""}}


def _cache(tmp_path: Path) -> BookstoreResponseCache:
    return BookstoreResponseCache(tmp_path / "bookstore_cache.jsonl")


def test_get_returns_nothing_before_anything_is_stored(tmp_path: Path):
    assert _cache(tmp_path).get(title="달빛조각사", author="남희성") is None


def test_put_then_get_by_title(tmp_path: Path):
    cache = _cache(tmp_path)
    cache.put(STORES, title="달빛조각사", author="남희성")

    assert cache.get(title="달빛조각사", author="남희성") == STORES


def test_put_indexes_by_isbn_taken_from_the_store_answers(tmp_path: Path):
    """ISBN 은 서점 응답에서 얻는다. 제목 표기가 달라도 같은 책을 찾아야 한다."""
    cache = _cache(tmp_path)
    cache.put(STORES, title="달빛조각사", author="남희성")

    assert cache.get(isbn="9791162203484") == STORES
    assert cache.get(isbn="9791162203484", title="전혀 다른 제목") == STORES


def test_put_infers_author_from_store_answers(tmp_path: Path):
    cache = _cache(tmp_path)
    cache.put(STORES, title="달빛조각사")

    assert cache.get(title="달빛조각사", author="남희성") == STORES


def test_authorless_response_is_only_indexed_by_isbn(tmp_path: Path):
    cache = _cache(tmp_path)
    answers = {"yes24": {"cat": "소설", "isbn": "9788900000000"}, "aladin": {}, "kyobo": {}}
    cache.put(answers, title="달빛조각사")

    assert cache.get(isbn="9788900000000") == answers
    assert cache.get(title="달빛조각사", author="남희성") is None


def test_get_prefers_isbn_over_title(tmp_path: Path):
    """제목은 같아도 다른 책일 수 있다. ISBN 을 알면 그쪽이 맞다."""
    cache = _cache(tmp_path)
    other = {"yes24": {"cat": "소설 > 무협", "mapped": "3_무협", "isbn": "9788900000000"}, "aladin": {}, "kyobo": {}}
    cache.put(STORES, title="달빛조각사", author="남희성")
    cache.put(other, title="달빛조각사", author="다른저자", isbn="9788900000000")

    assert cache.get(isbn="9788900000000", title="달빛조각사", author="다른저자") == other


def test_empty_answers_are_not_cached(tmp_path: Path):
    """서점이 다 빈손인 것은 진짜 없어서일 수도, 망이 끊겨서일 수도 있다.

    구분할 방법이 없으므로 남기지 않는다. 남기면 일시적인 실패가 영구 오답이 된다.
    """
    cache = _cache(tmp_path)
    cache.put({"yes24": {"cat": "", "mapped": None}, "aladin": {}, "kyobo": {}}, title="없는책", author="누구")

    assert cache.get(title="없는책", author="누구") is None


def test_entries_survive_a_new_instance(tmp_path: Path):
    """제안 생성은 매번 서비스를 새로 만든다. 파일에 남지 않으면 캐시가 아니다."""
    path = tmp_path / "bookstore_cache.jsonl"
    BookstoreResponseCache(path).put(STORES, title="달빛조각사", author="남희성")

    assert BookstoreResponseCache(path).get(title="달빛조각사", author="남희성") == STORES


def test_later_entry_wins_for_the_same_key(tmp_path: Path):
    path = tmp_path / "bookstore_cache.jsonl"
    cache = BookstoreResponseCache(path)
    cache.put(STORES, title="달빛조각사", author="남희성")
    newer = {"yes24": {"cat": "소설 > 판타지 > 현대판타지", "mapped": "3_판타지", "isbn": ""}, "aladin": {}, "kyobo": {}}
    cache.put(newer, title="달빛조각사", author="남희성")

    assert BookstoreResponseCache(path).get(title="달빛조각사", author="남희성") == newer


def test_a_broken_line_does_not_kill_the_whole_cache(tmp_path: Path):
    """줄 단위로 덧붙이므로 쓰다 죽으면 마지막 줄이 깨질 수 있다. 나머지는 살아야 한다."""
    path = tmp_path / "bookstore_cache.jsonl"
    BookstoreResponseCache(path).put(STORES, title="달빛조각사", author="남희성")
    with path.open("a", encoding="utf-8") as f:
        f.write("\n")
        f.write(json.dumps({"keys": ["title:잘못된레코드"], "stores": []}, ensure_ascii=False) + "\n")
        f.write('{"keys": ["title:잘린책"], "stor\n')

    assert BookstoreResponseCache(path).get(title="달빛조각사", author="남희성") == STORES


def test_missing_file_is_not_an_error(tmp_path: Path):
    assert BookstoreResponseCache(tmp_path / "없는파일.jsonl").get(title="아무책", author="저자") is None


def test_unreadable_cache_file_is_ignored(tmp_path: Path, caplog):
    path = tmp_path / "cache.jsonl"
    path.mkdir()

    assert BookstoreResponseCache(path).get(title="아무책", author="저자") is None
    assert "서점 캐시를 읽지 못했다" in caplog.text


def test_answers_without_any_cache_key_are_not_persisted(tmp_path: Path):
    cache = _cache(tmp_path)
    cache.put({"yes24": {"cat": "소설"}, "aladin": {}, "kyobo": {}})

    assert not cache.path.exists()


def test_cache_write_failure_keeps_in_memory_answer(tmp_path: Path, caplog):
    parent = tmp_path / "not-a-directory"
    parent.write_text("file blocks directory creation", encoding="utf-8")
    cache = BookstoreResponseCache(parent / "cache.jsonl")
    cache.put(STORES, title="달빛조각사", author="남희성")

    assert cache.get(title="달빛조각사", author="남희성") == STORES
    assert "서점 캐시에 쓰지 못했다" in caplog.text


def test_title_key_ignores_surrounding_space(tmp_path: Path):
    cache = _cache(tmp_path)
    cache.put(STORES, title="  달빛조각사 ", author=" 남희성 ")

    assert cache.get(title="달빛조각사", author="남희성") == STORES


def test_title_lookup_requires_matching_author(tmp_path: Path):
    cache = _cache(tmp_path)
    cache.put(STORES, title="달빛조각사", author="남희성")

    assert cache.get(title="달빛조각사", author="다른 저자") is None


def test_isbn_lookup_rejects_mismatched_saved_isbn_or_author(tmp_path: Path):
    cache = _cache(tmp_path)
    cache.put(STORES, title="달빛조각사", author="남희성", isbn="9791162203484")

    assert cache.get(isbn="9780000000000", title="달빛조각사", author="남희성") is None
    assert cache.get(isbn="9791162203484", title="달빛조각사", author="다른 저자") is None


def test_query_bookstores_uses_the_cache_instead_of_calling_stores_again(tmp_path: Path):
    """같은 제목을 두 번 물으면 두 번째는 서점을 안 부른다. 한 건에 3.6초가 걸린다."""
    from backend.book_classifier import BookClassifierService

    service = BookClassifierService(library_root=tmp_path, cache_file=tmp_path / "cache.json", delay=0.0, bookstore_cache_file=tmp_path / "bs.jsonl")
    calls: list[str] = []

    class Store:
        def __init__(self, name):
            self.name = name

        def search(self, isbn="", title="", author=""):
            calls.append(self.name)
            return [("달빛조각사", "남희성", "소설 > 판타지", "http://x", "http://s", "9791162203484")], title, "title"

    service.yes24, service.aladin, service.kyobo = Store("y"), Store("a"), Store("k")

    first = service.query_bookstores("달빛조각사", "남희성", "달빛조각사")
    assert len(calls) == 3

    second = service.query_bookstores("달빛조각사", "남희성", "달빛조각사")
    assert len(calls) == 3
    assert second == first


def test_query_bookstores_finds_the_cache_by_isbn_when_the_title_differs(tmp_path: Path):
    """제목 표기가 달라도 ISBN 이 같으면 같은 책이다."""
    from backend.book_classifier import BookClassifierService

    service = BookClassifierService(library_root=tmp_path, cache_file=tmp_path / "cache.json", delay=0.0, bookstore_cache_file=tmp_path / "bs.jsonl")
    calls: list[str] = []

    class Store:
        def search(self, isbn="", title="", author=""):
            calls.append(title)
            return [("달빛조각사", "남희성", "소설 > 판타지", "http://x", "http://s", "9791162203484")], title, "title"

    service.yes24, service.aladin, service.kyobo = Store(), Store(), Store()

    service.query_bookstores("달빛조각사", "남희성", "달빛조각사")
    assert len(calls) == 3

    service.query_bookstores("달빛 조각사 1부", "남희성", "달빛 조각사 1부", isbn="9791162203484")
    assert len(calls) == 3
