import json
import time
from pathlib import Path
from types import SimpleNamespace

import pytest


@pytest.fixture

def normalizer():
    from utils import normalize_comic_directories
    return normalize_comic_directories


def test_name_helpers_cover_author_title_and_volume_variants(normalizer):
    n = normalizer
    assert n.parse_filename(Path("[001] [홍길동] 원피스 01권.pdf")) == ("원피스", "홍길동")
    assert n.parse_filename(Path("제12화-나루토 ch.3.epub")) == ("나루토", None)
    assert n.parse_filename(Path("[작가] 제목 1-3권 完.pdf")) == ("제목", "작가")
    assert n.parse_filename(Path("[123] !!!.pdf")) is None
    assert n.parse_filename(Path("만화 17.txt")) == ("만화", None)
    assert n.title_key("  ＡBC 가 나  ") == "abc가나"
    assert n.directory_author("[작가] 작품") == "작가"
    assert n.directory_author("[comic] 작품") is None
    assert n.directory_author("[123] 작품") is None
    assert n.korean_author(None) is None
    assert n.korean_author("") is None
    assert n.korean_author("글 김용") == "김용"
    assert n.korean_author("Kim") is None
    assert n.korean_author("김용 Kim") is None
    assert n.directory_title("[작가] 작품 1-3권 완결") == "작품"
    assert n.directory_title("제목 [태그]") == "제목"
    assert n.directory_title("제목 (완결 1-3권)") == "제목"
    assert n.directory_title("제목 (foo 1-3권") == "제목"
    assert n.existing_metadata("작품 (1-3권 완 외)") == "(1-3권 완 외)"
    assert n.existing_metadata("작품") == ""
    assert n.matched_numbers(n.VOLUME_NUMBER, "1권 vol.2 volume3") == [1, 2, 3]
    assert n.matched_numbers(n.CHAPTER_NUMBER, "ch.0 3화 chapter4") == [3, 4]


def test_local_author_references_and_ambiguous_matching(tmp_path, normalizer):
    n = normalizer
    root = tmp_path / "root"
    (root / "3_판타지" / "[홍길동] 원피스 1-3권").mkdir(parents=True)
    (root / "3_판타지" / "[김철수] 원피스 외전").mkdir()
    (root / "3_판타지" / "[comic] 제외").mkdir()
    (root / "4_대기" / "[이작가] 나루토").mkdir(parents=True)
    (root / "ignored.txt").write_text("x")
    refs = n.local_author_references(root)
    assert ("원피스", "홍길동") in refs
    assert ("나루토", "이작가") in refs
    assert n.matching_local_author("나루토", refs) == "이작가"
    assert n.matching_local_author("원피스", refs) == "홍길동"
    assert n.matching_local_author("나루", refs) == "이작가"


def test_directory_metadata_extracts_chapters_volumes_fallback_and_flags(tmp_path, normalizer):
    n = normalizer
    chapters = tmp_path / "작품 미완"
    chapters.mkdir()
    for name in ("제01화.pdf", "제12화.epub", "외전.pdf", "readme.txt"):
        (chapters / name).write_bytes(b"x")
    assert n.normalized_directory_metadata(chapters) == "(1-12화 미완외)"

    volumes = tmp_path / "책 1-3권 완결"
    volumes.mkdir()
    for name in ("책 1권.pdf", "책 2권.epub", "책 완결.pdf"):
        (volumes / name).write_bytes(b"x")
    assert n.normalized_directory_metadata(volumes) == "(1-2권 완)"

    trailing = tmp_path / "연속 번호"
    trailing.mkdir()
    for name in ("책 01.pdf", "책 03.epub"):
        (trailing / name).write_bytes(b"x")
    assert n.normalized_directory_metadata(trailing) == "(1-3권)"

    no_range = tmp_path / "단권"
    no_range.mkdir()
    (no_range / "제목.pdf").write_bytes(b"x")
    assert n.normalized_directory_metadata(no_range) is None

    complete_from_file = tmp_path / "작품"
    complete_from_file.mkdir()
    (complete_from_file / "작품 1권.pdf").write_bytes(b"x")
    (complete_from_file / "작품 2권 완결.pdf").write_bytes(b"x")
    assert n.normalized_directory_metadata(complete_from_file) == "(1-2권 완)"


def test_infer_identity_covers_fallback_dominance_author_and_mismatch(tmp_path, normalizer):
    n = normalizer
    one = tmp_path / "[작가] 작품 1-3권"
    one.mkdir()
    (one / "!!! 1권.pdf").write_bytes(b"x")
    (one / "??? 2권.epub").write_bytes(b"x")
    fallback = n.infer_identity(one)
    assert fallback.title == "작품" and fallback.author == "작가"

    repeated = tmp_path / "나루토"
    repeated.mkdir()
    for filename in ("나루토 1권.pdf", "[홍길동] 나루토 2권.epub", "[홍길동] 나루토 3권.pdf"):
        (repeated / filename).write_bytes(b"x")
    inferred = n.infer_identity(repeated)
    assert inferred.title == "나루토" and inferred.author == "홍길동"

    insufficient = tmp_path / "few"
    insufficient.mkdir()
    (insufficient / "a.pdf").write_bytes(b"x")
    assert n.infer_identity(insufficient).failure_status == "SKIP_INSUFFICIENT_FILES"

    mismatch = tmp_path / "다른 제목"
    mismatch.mkdir()
    for filename in ("나루토 1.pdf", "나루토 2.pdf"):
        (mismatch / filename).write_bytes(b"x")
    assert n.infer_identity(mismatch).failure_status == "SKIP_TITLE_MISMATCH"

    ambiguous = tmp_path / "나루토 1권"
    ambiguous.mkdir()
    for filename in ("나루토 1.pdf", "블리치 2.pdf", "원피스 3.pdf"):
        (ambiguous / filename).write_bytes(b"x")
    assert n.infer_identity(ambiguous).failure_status == "SKIP_AMBIGUOUS_FILE_TITLES"


def test_title_matching_and_filesystem_name_sanitization(normalizer):
    n = normalizer
    assert n.bookstore_title_base("원피스 (1-3권)") == "원피스"
    assert n.bookstore_title_base("원피스 3권") == "원피스"
    assert n.title_variants("나루토: Naruto Edition")
    assert n.titles_equivalent("나루토 (완전판)", "나루토")
    assert n.bookstore_title_matches("원피스 3권", "원피스")
    assert n.filesystem_safe_name('A/B:C*?"<>|') == "A／B：C＊？＂＜＞｜"
    assert n.filesystem_safe_name("  title.  ") == "title"
    assert n.filesystem_safe_name("") is None
    assert n.filesystem_safe_name("..") is None
    assert n.filesystem_safe_name("x" * 256) is None


def test_cache_load_put_ttl_and_request_wait(tmp_path, monkeypatch, normalizer, capsys):
    n = normalizer
    path = tmp_path / "cache" / "items.jsonl"
    path.parent.mkdir()
    path.write_text("\n".join([
        "broken json",
        json.dumps({"query": "q", "store": "yes24", "cached_at": 100, "results": []}),
        json.dumps({"query": "bad", "store": "yes24", "results": [{"title": "missing fields"}]}),
        json.dumps({"query": "iso", "store": "aladin", "cached_at": "not-a-date", "results": []}),
    ]) + "\n", encoding="utf-8")
    cache = n.BookstoreSearchCache(path, negative_ttl_seconds=10)
    monkeypatch.setattr(n.time, "time", lambda: 200)
    assert cache.get("q", "yes24") is None
    assert cache.get("bad", "yes24") is None
    assert cache.get("iso", "aladin") is None
    cache.put("new", "yes24", [{"title": "t", "author": "a", "category": "c", "url": "u", "isbn": "i"}])
    assert cache.get("new", "yes24")[0]["title"] == "t"
    assert "new" in path.read_text(encoding="utf-8")

    refreshed = n.BookstoreSearchCache(path, refresh=True)
    assert path.read_text() == "" and not refreshed.entries
    assert n.BookstoreSearchCache(tmp_path / "missing" / "cache").entries == {}

    delays = []
    monkeypatch.setattr(n.time, "monotonic", lambda: 100)
    monkeypatch.setattr(n.time, "sleep", delays.append)
    refreshed.last_request_completed_at = 99.5
    refreshed.wait_for_request_slot(1.0)
    refreshed.wait_for_request_slot(0)
    assert delays == [0.5]


def test_cache_io_errors_are_reported(tmp_path, monkeypatch, normalizer, capsys):
    n = normalizer
    path = tmp_path / "cache.jsonl"
    path.write_text("{}\n")
    monkeypatch.setattr(Path, "open", lambda *_args, **_kwargs: (_ for _ in ()).throw(OSError("read failed")))
    n.BookstoreSearchCache(path)
    assert "Cache read failed" in capsys.readouterr().err

    cache = n.BookstoreSearchCache(path)
    assert cache.get("absent", "store") is None
    cache.path = tmp_path / "blocked" / "cache.jsonl"
    monkeypatch.setattr(Path, "open", lambda *_args, **_kwargs: (_ for _ in ()).throw(OSError("write failed")))
    cache.put("query", "store", [])
    assert "Cache write failed" in capsys.readouterr().err


class FakeStore:
    def __init__(self, rows=None, error=None, **_kwargs):
        self.rows, self.error = rows or [], error

    def search(self, **_kwargs):
        if self.error:
            raise self.error
        return self.rows, "", ""


def store_row(title="원피스 1권", author="오다 에이치로"):
    return (title, author, "만화", "https://example/book", "https://example/search", "123")


def test_canonical_title_uses_caches_votes_and_rejected_candidates(tmp_path, normalizer, monkeypatch, capsys):
    n = normalizer
    cache = n.BookstoreSearchCache(tmp_path / "cache.jsonl")
    stores = {"yes24": FakeStore([store_row()]), "aladin": FakeStore([store_row("다른 책")]), "kyobo": FakeStore(error=RuntimeError("offline"))}
    result = n.canonical_book_title("원피스", None, stores, 0, "query", cache)
    assert result[0:2] == ("원피스", "오다 에이치로")
    assert result[2] == "yes24: 원피스" and result[3].startswith("-")
    assert "다른 책" not in result[2]
    assert "kyobo 검색 실패" not in capsys.readouterr().err

    # The cached first store has no Korean author, so the second matching result is also queried.
    cache2 = n.BookstoreSearchCache(tmp_path / "empty.jsonl")
    stores2 = {"yes24": FakeStore([store_row(author="" )]), "aladin": FakeStore([store_row(author="홍길동")])}
    got = n.canonical_book_title("원피스", "홍길동", stores2, 0, "other", cache2)
    assert got[0:2] == ("원피스", "홍길동")


def create_series(root: Path, directory: str, titles=("원피스", "원피스")) -> Path:
    path = root / directory
    path.mkdir(parents=True, exist_ok=True)
    for i, title in enumerate(titles, 1):
        (path / f"{title} {i}권.pdf").write_bytes(b"x")
    return path


def test_build_proposals_handles_skip_rename_search_limit_and_conflicts(tmp_path, normalizer, monkeypatch):
    n = normalizer
    root = tmp_path / "comics"
    create_series(root / "3_미정리", "원피스 모음")
    create_series(root / "3_미정리", "나루토", ("나루토", "나루토"))
    create_series(root / "4_대기", "한 권", ("단권",))
    create_series(root / "9_temp", "파일명불일치", ("나루토", "블리치"))

    class Store:
        def __init__(self, verbose=False):
            pass
        def search(self, **kwargs):
            return [store_row("나루토", "기시모토 마사시")], "", ""

    monkeypatch.setattr(n, "BOOKSTORES", (("yes24", Store),))
    proposals = n.build_proposals(root, 0, 1, n.BookstoreSearchCache(tmp_path / "cache.jsonl"))
    statuses = {item.source.name: item.status for item in proposals}
    assert statuses["원피스 모음"] == "SKIP_TITLE_MISMATCH"
    assert statuses["한 권"] == "SKIP_INSUFFICIENT_FILES"
    assert statuses["나루토"] in {"RENAME", "SKIP_SEARCH_LIMIT"}
    assert statuses["파일명불일치"] in {"SKIP_AMBIGUOUS_FILE_TITLES", "SKIP_SEARCH_LIMIT"}

    rename = next((item for item in proposals if item.status == "RENAME"), None)
    if rename:
        rename.target.mkdir()
        again = n.build_proposals(root, 0, 0, n.BookstoreSearchCache(tmp_path / "cache2.jsonl"))
        assert any(item.status == "SKIP_CONFLICT" for item in again)


def test_report_and_atomic_rename_success_and_rollback(tmp_path, normalizer, monkeypatch):
    n = normalizer
    source = tmp_path / "old"
    source.mkdir()
    target = tmp_path / "new"
    proposal = n.Proposal(source, target, 0.9, 2, "RENAME", "evidence\tline")
    report = tmp_path / "nested" / "report.tsv"
    n.write_report([proposal], report)
    assert report.read_text(encoding="utf-8-sig").splitlines()[1].startswith("RENAME\t0.900\t2")
    assert n.apply_renames([proposal]) == 1 and target.is_dir() and not source.exists()

    source2 = tmp_path / "old2"
    source2.mkdir()
    target2 = tmp_path / "new2"
    p2 = n.Proposal(source2, target2, 1.0, 2, "RENAME")
    original_rename = Path.rename
    calls = {"count": 0}
    def fail_second_rename(self, destination):
        calls["count"] += 1
        if calls["count"] == 2:
            raise OSError("rename failed")
        return original_rename(self, destination)
    monkeypatch.setattr(Path, "rename", fail_second_rename)
    with pytest.raises(OSError, match="rename failed"):
        n.apply_renames([p2])
    assert source2.is_dir() and not target2.exists()


def test_main_validates_root_writes_dry_run_report_and_apply(tmp_path, normalizer, monkeypatch, capsys):
    n = normalizer
    root = tmp_path / "comics"
    root.mkdir()
    monkeypatch.setattr(n, "build_proposals", lambda *args: [])
    monkeypatch.setattr(n, "apply_renames", lambda proposals: 3)
    monkeypatch.setattr(n.sys, "argv", ["normalize", "--root", str(root), "--report", str(tmp_path / "out.tsv"), "--cache", str(tmp_path / "cache"), "--apply"])
    assert n.main() == 0
    output = capsys.readouterr().out
    assert "Renamed: 3" in output
    assert "Directories scanned: 0" in output

    monkeypatch.setattr(n.sys, "argv", ["normalize", "--root", str(tmp_path / "missing")])
    with pytest.raises(SystemExit) as exited:
        n.main()
    assert exited.value.code == 2


def test_metadata_skips_symlinks_and_other_entries_and_detects_completed_names(tmp_path, normalizer):
    import socket

    n = normalizer
    directory = tmp_path / "series"
    directory.mkdir()
    (directory / "book 1.pdf").write_bytes(b"x")
    (directory / "book 2.pdf").write_bytes(b"x")
    (directory / "readme.txt").write_text("ignore")
    (directory / "12권").mkdir()
    (directory / "link 3권.pdf").symlink_to(directory / "book 1.pdf")
    socket_path = directory / "socket"
    sock = socket.socket(socket.AF_UNIX)
    try:
        sock.bind(str(socket_path))
        assert n.normalized_directory_metadata(directory) == "(12-12권)"
    finally:
        sock.close()
        socket_path.unlink(missing_ok=True)

    completed = tmp_path / "series without label"
    completed.mkdir()
    (completed / "book 1권.pdf").write_text("x")
    (completed / "book 2권 완결.pdf").write_text("x")
    assert n.normalized_directory_metadata(completed) == "(1-2권 완)"
    assert n.directory_title("제목 (완결 1-3권)") == "제목"


def test_author_references_ignore_symlinks_and_partial_titles_can_be_ambiguous(tmp_path, normalizer):
    n = normalizer
    root = tmp_path / "root"
    root.mkdir()
    (root / "3_미정리" / "[홍길동] 원피스").mkdir(parents=True)
    (root / "3_미정리" / "[김철수] 원피스 외전").mkdir()
    (root / "4_대기").mkdir()
    (root / "4_대기" / "[가짜] 링크").symlink_to(root / "3_미정리" / "[홍길동] 원피스", target_is_directory=True)
    refs = n.local_author_references(root)
    assert n.matching_local_author("원피스", refs) == "홍길동"
    assert n.matching_local_author("원", refs) is None


def test_identity_clears_inconsistent_author_votes(tmp_path, normalizer):
    directory = tmp_path / "원피스"
    directory.mkdir()
    (directory / "[홍길동] 원피스 1권.pdf").write_text("x")
    (directory / "원피스 2권.pdf").write_text("x")
    result = normalizer.infer_identity(directory)
    assert result.title == "원피스" and result.author is None


def test_canonical_title_keeps_rejections_and_continues_after_store_error(tmp_path, normalizer, capsys):
    n = normalizer
    cache = n.BookstoreSearchCache(tmp_path / "cache.jsonl")
    stores = {
        "yes24": FakeStore([store_row("다른 작품", "홍길동")]),
        "aladin": FakeStore(error=RuntimeError("network down")),
        "kyobo": FakeStore([]),
    }
    title, author, evidence, status = n.canonical_book_title("원피스", None, stores, 0, "key", cache)
    assert title is None and author is None
    assert evidence == "yes24 candidates: 다른 작품"
    assert status == "---" and cache.searches == 3
    assert "aladin 검색 실패" in capsys.readouterr().err


def test_build_proposals_covers_success_unchanged_no_author_invalid_name_and_conflicts(tmp_path, normalizer, monkeypatch):
    n = normalizer
    root = tmp_path / "comics"
    parent = root / "3_미정리"
    parent.mkdir(parents=True)
    names = ["source-a", "source-b", "unchanged", "no-author", "invalid", "occupied"]
    for name in names:
        (parent / name).mkdir()

    monkeypatch.setattr(n, "BOOKSTORES", (("yes24", FakeStore),))
    monkeypatch.setattr(n, "local_author_references", lambda _root: [])
    monkeypatch.setattr(n, "infer_identity", lambda directory: n.IdentityResult(n.directory_title(directory.name), None, 0.9, 2))
    monkeypatch.setattr(n, "normalized_directory_metadata", lambda _directory: "(1-2권)")
    def canonical(title, author, stores, delay, query, cache):
        if title == "no-author":
            return "NoAuthor", None, "found", "+"
        if title == "invalid":
            return "x" * 300, "홍길동", "found", "+"
        return title, "홍길동", "found", "+"

    monkeypatch.setattr(n, "canonical_book_title", canonical)
    # The existing destination is occupied by an unplanned directory.
    (parent / "[홍길동] occupied (1-2권)").mkdir()
    # Make one proposal already canonical so it is classified as unchanged.
    unchanged_target = parent / "[홍길동] unchanged (1-2권)"
    (parent / "unchanged").rename(unchanged_target)
    # Give the occupied source a different title so its destination is separate.
    proposals = n.build_proposals(root, 0, 0, n.BookstoreSearchCache(tmp_path / "cache.jsonl"))
    statuses = {item.source.name: item.status for item in proposals}
    assert statuses["source-a"] == "RENAME"
    assert statuses["source-b"] == "RENAME"
    assert statuses["[홍길동] unchanged (1-2권)"] == "UNCHANGED"
    assert statuses["no-author"] == "SKIP_NO_AUTHOR"
    assert statuses["invalid"] == "SKIP_INVALID_NAME"
    assert statuses["occupied"] == "SKIP_CONFLICT"


def test_build_proposals_covers_no_range_limit_and_no_match(tmp_path, normalizer, monkeypatch):
    n = normalizer
    root = tmp_path / "comics"
    parent = root / "3_미정리"
    parent.mkdir(parents=True)
    for name in ("first", "second", "third"):
        (parent / name).mkdir()
    monkeypatch.setattr(n, "BOOKSTORES", (("yes24", FakeStore),))
    monkeypatch.setattr(n, "local_author_references", lambda _root: [])
    monkeypatch.setattr(n, "infer_identity", lambda directory: n.IdentityResult(directory.name, None, 0.9, 2))
    monkeypatch.setattr(n, "normalized_directory_metadata", lambda directory: None if directory.name == "first" else "(1-2권)")
    monkeypatch.setattr(n, "canonical_book_title", lambda *args: (None, None, "no match", "-"))
    proposals = n.build_proposals(root, 0, 1, n.BookstoreSearchCache(tmp_path / "cache.jsonl"))
    assert [proposal.status for proposal in proposals] == ["SKIP_NO_RANGE", "SKIP_NO_BOOKSTORE_MATCH", "SKIP_SEARCH_LIMIT"]


def test_build_proposals_reports_search_limit_and_progress(tmp_path, normalizer, monkeypatch, capsys):
    n = normalizer
    root = tmp_path / "comics"
    parent = root / "3_미정리"
    parent.mkdir(parents=True)
    titles = [f"title-{index:02}" for index in range(25)]
    for title in titles:
        (parent / title).mkdir()
    monkeypatch.setattr(n, "BOOKSTORES", (("yes24", FakeStore),))
    monkeypatch.setattr(n, "local_author_references", lambda _root: [])
    monkeypatch.setattr(n, "infer_identity", lambda directory: n.IdentityResult(directory.name, None, 0.9, 2))
    monkeypatch.setattr(n, "normalized_directory_metadata", lambda _directory: "(1-2권)")
    monkeypatch.setattr(n, "canonical_book_title", lambda title, *_args: (title, "홍길동", "found", "+"))
    proposals = n.build_proposals(root, 0, 0, n.BookstoreSearchCache(tmp_path / "cache.jsonl"))
    assert len(proposals) == 25
    assert "Title queries: 25" in capsys.readouterr().err


def test_main_dry_run_and_apply_report(tmp_path, normalizer, monkeypatch, capsys):
    n = normalizer
    root = tmp_path / "comics"
    root.mkdir()
    report = tmp_path / "report.tsv"
    monkeypatch.setattr(n, "build_proposals", lambda *_args: [n.Proposal(root, None, 0.0, 0, "SKIP_NO_RANGE")])
    monkeypatch.setattr(n, "apply_renames", lambda _proposals: 1)
    monkeypatch.setattr("sys.argv", ["normalize", "--root", str(root), "--report", str(report)])
    assert n.main() == 0
    assert "Dry run only" in capsys.readouterr().out
    monkeypatch.setattr("sys.argv", ["normalize", "--root", str(root), "--report", str(report), "--apply"])
    assert n.main() == 0
    output = capsys.readouterr().out
    assert "Renamed: 1" in output
    assert "Rebuild or refresh" in output


def test_module_entrypoint_adds_repo_root_and_exits_on_help(monkeypatch):
    import runpy
    import sys

    from utils import normalize_comic_directories as module

    repo_root = str(module.REPO_ROOT)
    original_path = list(sys.path)
    monkeypatch.setattr(sys, "path", [path for path in original_path if path != repo_root])
    monkeypatch.setattr(sys, "argv", [str(Path(module.__file__)), "--help"])
    with pytest.raises(SystemExit) as exc:
        runpy.run_module("utils.normalize_comic_directories", run_name="__main__")
    assert exc.value.code == 0


def test_apply_renames_preflight_and_race_rollback_preserve_unexpected_destination(tmp_path, normalizer, monkeypatch):
    n = normalizer
    source = tmp_path / "source"
    source.mkdir()
    target = tmp_path / "target"
    target.mkdir()
    with pytest.raises(RuntimeError, match="preflight"):
        n.apply_renames([n.Proposal(source, target, 1.0, 2, "RENAME")])

    target.rmdir()
    original_rename = Path.rename
    raced = {"done": False}

    def race_target(self, destination):
        result = original_rename(self, destination)
        if self == source and not raced["done"]:
            target.mkdir()
            (target / "external").write_text("keep")
            raced["done"] = True
        return result

    monkeypatch.setattr(Path, "rename", race_target)
    with pytest.raises(FileExistsError):
        n.apply_renames([n.Proposal(source, target, 1.0, 2, "RENAME")])
    assert (target / "external").read_text() == "keep"
    assert source.is_dir()
