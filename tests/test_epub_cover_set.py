"""EPUB 뷰어의 표지 이미지 추가: BookManager.set_epub_cover 와 POST /epub-cover/{id}.

운영 파일 대신 임시 EPUB 를 쓰고, 이미지 다운로드만 가짜로 바꾼다.
"""

from datetime import datetime
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

from backend.book_manager import BookManager
from backend.epub_cover import CoverError, inspect_epub
from tests.test_cover_management import image_bytes, make_epub

URL = "https://img.example.com/cover.jpg"


def image_bytes_of(color):
    import io

    from PIL import Image

    output = io.BytesIO()
    Image.new("RGB", (120, 180), color).save(output, "JPEG")
    return output.getvalue()


@pytest.fixture
def env(tmp_path, monkeypatch):
    """(manager 대역, EPUB 경로, 책). get_book 은 이 EPUB 를 가리키는 책을 돌려준다."""
    monkeypatch.setattr("backend.cover_management.fetch_url_image", lambda url: image_bytes())
    path = make_epub(tmp_path / "분류" / "book.epub")
    book = SimpleNamespace(file_path=path, file_type="epub")
    es = MagicMock()
    es.update.return_value = True
    manager = SimpleNamespace(path_prefix=tmp_path, TRASH_DIRNAME="0_trash", es_manager=es, get_book=AsyncMock(return_value=(book, None)))
    return manager, path, book


async def call(manager, url=URL):
    return await BookManager.set_epub_cover(manager, 1, url)


@pytest.mark.asyncio
async def test_adds_a_cover_to_an_epub_without_one(env):
    manager, path, _ = env
    assert not inspect_epub(path)["has_cover"]

    result, error = await call(manager)

    assert error is None
    assert inspect_epub(path)["has_cover"]
    assert result["file_size"] == path.stat().st_size
    manager.es_manager.update.assert_called_once()
    args, kwargs = manager.es_manager.update.call_args
    assert args == (1,) and kwargs["file_size"] == path.stat().st_size
    # 최신 책 목록(updated_time 순)에 오르도록 방금 시각을 색인한다. 만화 인덱스의 strptime 형식과도 맞아야 한다.
    stamp = datetime.strptime(kwargs["updated_time"], "%Y-%m-%dT%H:%M:%S.%f")
    assert abs((datetime.now() - stamp).total_seconds()) < 60
    assert "warning" not in result


@pytest.mark.asyncio
async def test_replaces_an_existing_cover_and_keeps_the_original_in_0_trash(env, tmp_path):
    manager, path, _ = env
    make_epub(path, cover=True)
    original = path.read_bytes()

    result, error = await call(manager)

    assert error is None
    assert path.read_bytes() != original
    # PDF 편집과 같이 0_trash 에 원래 이름으로 둔다. 숨김 폴더 .cover_backups 는 쓰지 않는다.
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == original
    assert not (tmp_path / ".cover_backups").exists()


@pytest.mark.asyncio
async def test_second_edit_keeps_the_first_original(env, tmp_path, monkeypatch):
    manager, path, _ = env
    first_original = path.read_bytes()

    await call(manager)
    after_first = path.read_bytes()
    monkeypatch.setattr("backend.cover_management.fetch_url_image", lambda url: image_bytes_of("blue"))
    result, error = await call(manager)

    assert error is None
    assert path.read_bytes() != after_first
    # 처음 원본을 지킨다(덮어쓰지 않는다). 임시 롤백 파일도 남기지 않는다.
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == first_original
    assert [p.name for p in (tmp_path / "0_trash").iterdir()] == ["book.epub"]
    assert not list(path.parent.glob(".cover-*.tmp"))


def test_failed_write_restores_the_current_file_not_the_older_backup(tmp_path, monkeypatch):
    """백업이 이미 있으면 그것은 처음 원본이다. 쓰다가 실패하면 지금 원본으로 되돌려야 한다."""
    import backend.epub_cover as module

    path = make_epub(tmp_path / "book.epub")
    trash = tmp_path / "0_trash"
    module.add_cover(path, image_bytes(), module.inspect_epub(path)["revision"], trash, backup_name="book.epub")
    current = path.read_bytes()
    older = (trash / "book.epub").read_bytes()
    assert current != older

    real_fsync = module.os.fsync
    calls = 0

    def fail_on_write_back(fd):
        nonlocal calls
        calls += 1
        if calls == 3:  # 롤백 사본(1), 편집본(2), 본 파일에 쓴 뒤(3)
            raise OSError("write failed")
        return real_fsync(fd)

    monkeypatch.setattr(module.os, "fsync", fail_on_write_back)
    with pytest.raises(OSError):
        module.add_cover(path, image_bytes_of("green"), module.inspect_epub(path)["revision"], trash, replace_existing=True, backup_name="book.epub")

    assert path.read_bytes() == current
    assert (trash / "book.epub").read_bytes() == older
    assert not list(tmp_path.glob(".cover-*.tmp"))


@pytest.mark.asyncio
async def test_rejects_a_book_that_is_not_an_epub(env):
    manager, path, book = env
    book.file_type = "pdf"
    before = path.read_bytes()

    result, error = await call(manager)

    assert result is None and "EPUB" in error
    assert path.read_bytes() == before


@pytest.mark.asyncio
async def test_reports_a_missing_book(env):
    manager, *_ = env
    manager.get_book.return_value = (None, "No book found by '1'")

    result, error = await call(manager)

    assert result is None and "No book found" in error


@pytest.mark.asyncio
async def test_blank_url_is_rejected_before_touching_the_file(env):
    manager, path, _ = env
    before = path.read_bytes()

    result, error = await call(manager, "   ")

    assert result is None and "URL" in error
    assert path.read_bytes() == before


@pytest.mark.asyncio
async def test_download_failure_returns_the_reason_and_keeps_the_file(env, monkeypatch):
    manager, path, _ = env

    def refuse(url):
        raise CoverError("내부 주소의 이미지는 사용할 수 없습니다")

    monkeypatch.setattr("backend.cover_management.fetch_url_image", refuse)
    before = path.read_bytes()

    result, error = await call(manager)

    assert result is None and "내부 주소" in error
    assert path.read_bytes() == before
    manager.es_manager.update.assert_not_called()


@pytest.mark.asyncio
async def test_unexpected_io_error_hides_internal_details(env, monkeypatch):
    manager, *_ = env

    def broken(url):
        raise OSError("/secret/disk/path failed")

    monkeypatch.setattr("backend.cover_management.fetch_url_image", broken)

    result, error = await call(manager)

    assert result is None
    assert "secret" not in error


@pytest.mark.asyncio
async def test_index_update_failure_keeps_the_saved_cover_and_warns(env):
    manager, path, _ = env
    manager.es_manager.update.return_value = False

    result, error = await call(manager)

    assert error is None
    assert inspect_epub(path)["has_cover"]
    assert "색인" in result["warning"]


class TestEpubCoverEndpoint:
    @pytest.fixture
    def client(self):
        from fastapi.testclient import TestClient

        import backend.main as main
        from backend.auth import require_admin

        fake = MagicMock()
        fake.set_epub_cover = AsyncMock(return_value=({"file_size": 10}, None))
        proxy = main.book_manager
        previous = proxy._instance
        proxy._instance = fake
        main.app.dependency_overrides[require_admin] = lambda: {"role": "admin"}
        try:
            yield TestClient(main.app), fake
        finally:
            main.app.dependency_overrides.pop(require_admin, None)
            proxy._instance = previous

    def test_success(self, client):
        http, fake = client
        response = http.post("/epub-cover/1", json={"url": URL})
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": {"file_size": 10}}
        fake.set_epub_cover.assert_awaited_once_with(1, URL)

    def test_failure_is_reported_in_the_body(self, client):
        http, fake = client
        fake.set_epub_cover.return_value = (None, "EPUB이 변경되었습니다. 목록을 새로 불러오세요")
        body = http.post("/epub-cover/1", json={"url": URL}).json()
        assert body == {"status": "failure", "error": "EPUB이 변경되었습니다. 목록을 새로 불러오세요"}

    def test_empty_or_oversized_url_is_a_validation_error(self, client):
        http, _ = client
        assert http.post("/epub-cover/1", json={"url": ""}).status_code == 422
        assert http.post("/epub-cover/1", json={"url": "x" * 5000}).status_code == 422

    def test_requires_admin(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        response = TestClient(main.app).post("/epub-cover/1", json={"url": URL})
        assert response.status_code in (401, 403)

    def test_comics_router_exposes_the_same_endpoint(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        response = TestClient(main.app).post("/comics/epub-cover/1", json={"url": URL})
        assert response.status_code in (401, 403)
