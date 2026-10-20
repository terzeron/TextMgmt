"""관리자용 EPUB 표지 목록, 후보, 이미지 cache, 저장 API."""

import asyncio
import base64
import binascii
import fcntl
import hashlib
import io
import json
import logging
import math
import os
import re
import shutil
import tempfile
import time
import threading
import unicodedata
import zipfile
from pathlib import Path
from contextlib import contextmanager, nullcontext
from urllib.parse import urlparse
from typing import Any, Callable, Iterator

import requests
from PIL import Image

from fastapi import APIRouter, Depends, HTTPException, Query, Request
from fastapi.responses import Response
from pydantic import BaseModel, Field

from backend.auth import require_admin
from backend.book import Book
from backend.cover_sources import STORES, STORE_HOSTS, FetchCancelled, StoreBlocked, StoreGate, blocked_status, check_cancelled, fetch_bytes, search_candidates, store_lock, request_cancelled
from backend.epub_cover import CoverError, MAX_IMAGE_BYTES, add_cover, inspect_epub, normalize_image, remove_cover, revision
from backend.direct_cover_url import fetch_url_image
from utils.corpus_layout import IGNORED_DIR_NAMES

LOGGER = logging.getLogger(__name__)


def filename_title(path: Path) -> str:
    name = unicodedata.normalize("NFC", path.stem)
    name = "".join(" " if unicodedata.category(char)[0] in "PS" else char for char in name)
    return " ".join(name.split()) or "제목 없음"


class CoverManagement:
    def __init__(self, root: Path):
        self.root = root.resolve()
        self.cache = self.root / ".cover_management"

    def resolve(self, relative: str, *, directory=False) -> Path:
        path = Path(relative or ".")
        if path.is_absolute() or ".." in path.parts or "\\" in relative or "\x00" in relative:
            raise CoverError("올바른 책 상대 경로가 아닙니다")
        current = self.root
        for part in path.parts:
            if part.startswith(".") or part in IGNORED_DIR_NAMES:
                raise CoverError("내부 관리 디렉토리는 사용할 수 없습니다")
            current = current / part
            if current.is_symlink():
                raise CoverError("symlink는 표지 관리에서 제외합니다")
        resolved = current.resolve()
        if not resolved.is_relative_to(self.root):
            raise CoverError("책 디렉토리 밖의 경로는 사용할 수 없습니다")
        if directory:
            if not resolved.is_dir():
                raise CoverError("책 디렉토리를 찾을 수 없습니다")
        elif resolved.suffix.lower() != ".epub" or not resolved.is_file():
            raise CoverError("EPUB 파일을 찾을 수 없습니다")
        return resolved

    def directories(self, directory: str) -> list[dict[str, Any]]:
        parent = self.resolve(directory, directory=True)
        children = []
        with os.scandir(parent) as entries:
            for entry in entries:
                if entry.name.startswith(".") or entry.name in IGNORED_DIR_NAMES or not entry.is_dir(follow_symlinks=False):
                    continue
                children.append({"id": str(Path(entry.path).relative_to(self.root)), "label": entry.name,
                                 "fileType": "folder", "children": []})
        return sorted(children, key=lambda item: item["label"].casefold())

    def missing(self, directory: str, cursor: str, limit: int, *, covered: bool = False) -> dict[str, Any]:
        parent = self.resolve(directory, directory=True)
        if cursor and (not cursor.isdigit() or len(cursor) > 10):
            raise CoverError("목록 cursor가 올바르지 않습니다")
        offset = int(cursor or "0")
        files = sorted((path for path in parent.iterdir() if not path.is_symlink() and path.is_file()
                        and not path.name.startswith(".") and path.suffix.lower() == ".epub"), key=lambda path: path.name.casefold())
        items = []
        for path in files[offset:offset + limit]:
            relative = str(path.relative_to(self.root))
            identity = {"file_path": relative, "book_id": path.stat().st_ino,
                        "category": str(path.parent.relative_to(self.root)) if path.parent != self.root else "_root",
                        "title": filename_title(path)}
            try:
                metadata = inspect_epub(path)
                if metadata["has_cover"] != covered:
                    continue
                items.append({**metadata, **identity, "status": "covered" if covered else "missing"})
            except CoverError as exc:
                items.append({**identity, "revision": revision(path), "status": "error", "error": str(exc)})
        next_offset = min(offset + limit, len(files))
        return {"items": items, "next_cursor": str(next_offset) if next_offset < len(files) else None,
                "scanned": next_offset, "total": len(files)}

    def _write(self, path: Path, data: bytes) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        descriptor, temporary = tempfile.mkstemp(prefix=".tmp-", dir=path.parent)
        try:
            with os.fdopen(descriptor, "wb") as output:
                output.write(data)
            os.replace(temporary, path)
        finally:
            Path(temporary).unlink(missing_ok=True)

    def _read_json(self, path: Path, ttl: int | None = None) -> dict[str, Any] | None:
        try:
            if ttl is None or time.time() - path.stat().st_mtime < ttl:
                value = json.loads(path.read_text())
                return value if isinstance(value, dict) else None
        except (OSError, ValueError):
            pass
        return None

    def _book_key(self, relative: str) -> str:
        return hashlib.sha256(str(Path(relative)).encode()).hexdigest()[:32]

    def _book_cache(self, relative: str) -> Path:
        return self.cache / f"book-{self._book_key(relative)}"

    @contextmanager
    def _book_lock(self, relative: str, *, exclusive: bool = False) -> Iterator[None]:
        # lock inode는 삭제하지 않는다. 대기 요청과 새 요청이 같은 lock을 사용해야 한다.
        lock_dir = self.cache / ".book_locks"
        lock_dir.mkdir(parents=True, exist_ok=True)
        with (lock_dir / f"{self._book_key(relative)}.lock").open("a+b") as lock:
            mode = fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH
            while True:
                check_cancelled()
                try:
                    fcntl.flock(lock, mode | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    time.sleep(0.05)
            try:
                check_cancelled()
                yield
            finally:
                fcntl.flock(lock, fcntl.LOCK_UN)

    def _cached_search(self, path: Path) -> dict[str, Any] | None:
        cached = self._read_json(path)
        if cached and cached["status"] == "success":
            for candidate in cached["candidates"]:
                record = self._read_json(self._candidate_path(candidate["candidate_id"]))
                if not record:
                    return None
                image_path = self._candidate_path(candidate["candidate_id"]).parent / f"image-{candidate['candidate_id']}.jpg"
                if image_path.is_file() and not all(key in record for key in ("width", "height", "byte_size")):
                    self._image_info(candidate["candidate_id"], record, image_path.read_bytes())
                for key in ("width", "height", "byte_size"):
                    if key in record:
                        candidate[key] = record[key]
            return cached
        return self._read_json(path, 120 if cached and cached["status"] == "error" else 3600)

    def candidates(self, store: str, relative: str, *, include_covered: bool = False, refresh: bool = False) -> dict[str, Any]:
        if store not in STORES:
            raise CoverError("지원하지 않는 서점입니다")
        path = self.resolve(relative)
        relative = str(path.relative_to(self.root))
        with self._book_lock(relative):
            metadata = inspect_epub(path)
            metadata["title"] = filename_title(path)
            if metadata["has_cover"] and not include_covered:
                raise CoverError("EPUB에 이미 표지가 있습니다")
            key = f"{store}:{relative}:{metadata['revision']}:filename-v1"
            cache_key = hashlib.sha256(key.encode()).hexdigest()
            book_cache = self._book_cache(relative)
            result_path = book_cache / f"search-{cache_key}.json"
            if refresh:
                result_path.unlink(missing_ok=True)
                (self.cache / f"search-{cache_key}.json").unlink(missing_ok=True)
            cached = self._cached_search(result_path)
            if cached:
                return cached
            # 이전 배포의 flat cache도 재사용한다.
            cached = self._cached_search(self.cache / f"search-{cache_key}.json")
            if cached:
                return cached
            with store_lock(self.cache, store):
                cached = self._cached_search(result_path)
                if cached:
                    return cached
                gate = StoreGate(self.cache, store)
                try:
                    gate.before_request()
                    result = search_candidates(store, metadata)
                    gate.record_success()
                    for candidate in result["candidates"]:
                        suffix = hashlib.sha256(f"{cache_key}:{candidate['image_url']}".encode()).hexdigest()[:32]
                        candidate_id = self._book_key(relative) + suffix
                        record = {**candidate, "store": store, "file_path": relative, "revision": metadata["revision"]}
                        self._write(book_cache / f"candidate-{candidate_id}.json", json.dumps(record, ensure_ascii=False).encode())
                        candidate["candidate_id"] = candidate_id
                        candidate.pop("image_url")
                except FetchCancelled:
                    raise
                except StoreBlocked as blocked:
                    # 차단 안내는 책별 캐시에 남기지 않는다. 서점 단위 상태(gate)가 판단한다.
                    return self._blocked_result(store, metadata, blocked.retry_after)
                except Exception as exc:
                    if blocked_status(exc):
                        retry_after = gate.record_blocked()
                        LOGGER.warning("Cover store rate-limited (%s): HTTP %s, pausing %ss", store, exc.response.status_code, retry_after)  # type: ignore[attr-defined]
                        return self._blocked_result(store, metadata, retry_after)
                    status = getattr(getattr(exc, "response", None), "status_code", None)
                    LOGGER.warning("Cover candidate search failed (%s): %s%s", store, type(exc).__name__, f" HTTP {status}" if status else "")
                    result = {"status": "error", "error": "서점에 접근하지 못했습니다. 잠시 후 다시 시도하세요",
                              "search_url": STORES[store](verbose=False).build_search_url(metadata["title"]), "candidates": []}
                self._write(result_path, json.dumps(result, ensure_ascii=False).encode())
                return result

    def _blocked_result(self, store: str, metadata: dict[str, Any], retry_after: int) -> dict[str, Any]:
        minutes = max(1, math.ceil(retry_after / 60))
        return {"status": "error", "blocked": True, "retry_after": retry_after,
                "error": f"서점이 일시적으로 요청을 제한하고 있습니다. 약 {minutes}분 뒤 다시 시도하세요",
                "search_url": STORES[store](verbose=False).build_search_url(metadata["title"]), "candidates": []}

    def _candidate_path(self, candidate_id: str) -> Path:
        if not re.fullmatch(r"[a-f0-9]{64}", candidate_id):
            raise CoverError("표지 후보 ID가 올바르지 않습니다")
        grouped = self.cache / f"book-{candidate_id[:32]}" / f"candidate-{candidate_id}.json"
        return grouped if grouped.is_file() else self.cache / f"candidate-{candidate_id}.json"

    def _candidate(self, candidate_id: str) -> dict[str, Any]:
        record = self._read_json(self._candidate_path(candidate_id))
        if not record:
            raise CoverError("표지 후보를 찾을 수 없습니다. 후보를 다시 검색하세요")
        return record

    def _image_info(self, candidate_id: str, candidate: dict[str, Any], data: bytes) -> bytes:
        if not all(key in candidate for key in ("width", "height", "byte_size")):
            with Image.open(io.BytesIO(data)) as image:
                info = {"width": image.width, "height": image.height, "byte_size": len(data)}
            candidate.update(info)
            self._write(self._candidate_path(candidate_id), json.dumps(candidate, ensure_ascii=False).encode())
        return data

    def image(self, candidate_id: str) -> bytes:
        candidate = self._candidate(candidate_id)
        with self._book_lock(candidate["file_path"]):
            candidate = self._candidate(candidate_id)
            if revision(self.resolve(candidate["file_path"])) != candidate["revision"]:
                raise CoverError("EPUB이 변경되었습니다. 목록을 새로 불러오세요")
            image_path = self._candidate_path(candidate_id).parent / f"image-{candidate_id}.jpg"
            if image_path.exists():
                return self._image_info(candidate_id, candidate, image_path.read_bytes())
            store = candidate["store"]
            with store_lock(self.cache, store):
                if image_path.exists():
                    return self._image_info(candidate_id, candidate, image_path.read_bytes())
                with STORES[store](verbose=False).session as session:
                    data = normalize_image(fetch_bytes(store, candidate["image_url"], session, MAX_IMAGE_BYTES))
                self._write(image_path, data)
                return self._image_info(candidate_id, candidate, data)

    def _delete_book_cache(self, relative: str, expected_revision: str) -> None:
        book_cache = self._book_cache(relative)
        if book_cache.exists():
            shutil.rmtree(book_cache)
        # 이전 flat cache의 모든 서점·revision 후보도 해당 책만 정리한다.
        revisions = {expected_revision}
        aliases = {relative}
        for path in self.cache.glob("candidate-*.json"):
            candidate = self._read_json(path)
            if not candidate or Path(candidate.get("file_path", "")) != Path(relative):
                continue
            revisions.add(candidate["revision"])
            aliases.add(candidate["file_path"])
            candidate_id = path.stem.removeprefix("candidate-")
            (self.cache / f"image-{candidate_id}.jpg").unlink(missing_ok=True)
            path.unlink(missing_ok=True)
        for store in STORES:
            for alias in aliases:
                for old_revision in revisions:
                    key = hashlib.sha256(f"{store}:{alias}:{old_revision}".encode()).hexdigest()
                    (self.cache / f"search-{key}.json").unlink(missing_ok=True)

    def apply(self, relative: str, candidate_id: str, expected_revision: str) -> dict[str, Any]:
        path = self.resolve(relative)
        relative = str(path.relative_to(self.root))
        with self._book_lock(relative, exclusive=True):
            candidate = self._candidate(candidate_id)
            if Path(candidate["file_path"]) != Path(relative) or candidate["revision"] != expected_revision:
                raise CoverError("표지 후보와 EPUB이 일치하지 않습니다. 목록을 새로 불러오세요")
            image_path = self._candidate_path(candidate_id).parent / f"image-{candidate_id}.jpg"
            if not image_path.is_file():
                raise CoverError("표지 이미지를 먼저 확인하세요")
            return self._save_cover(path, relative, image_path.read_bytes(), expected_revision)

    def apply_url(self, relative: str, url: str, expected_revision: str, *, replace_existing: bool = False, backup_dir: Path | None = None, backup_name: str | None = None) -> dict[str, Any]:
        path = self.resolve(relative)
        relative = str(path.relative_to(self.root))
        with self._book_lock(relative, exclusive=True):
            metadata = inspect_epub(path)
            if metadata["revision"] != expected_revision:
                raise CoverError("EPUB이 변경되었습니다. 목록을 새로 불러오세요")
            if metadata["has_cover"] and not replace_existing:
                raise CoverError("EPUB에 이미 표지가 있습니다")
            host = (urlparse(url).hostname or "").lower()
            store = next((name for name, domains in STORE_HOSTS.items() if any(host == domain or host.endswith("." + domain) for domain in domains)), None)
            with store_lock(self.cache, store) if store else nullcontext():
                data = fetch_url_image(url)
            return self._save_cover(path, relative, data, expected_revision, replace_existing=replace_existing, backup_dir=backup_dir, backup_name=backup_name)

    def apply_image(self, relative: str, image_data: str, expected_revision: str, *, backup_dir: Path | None = None, backup_name: str | None = None) -> dict[str, Any]:
        path = self.resolve(relative)
        relative = str(path.relative_to(self.root))
        try:
            data = base64.b64decode(image_data, validate=True)
        except (binascii.Error, ValueError) as exc:
            raise CoverError("표지 이미지 데이터를 읽을 수 없습니다") from exc
        with self._book_lock(relative, exclusive=True):
            return self._save_cover(path, relative, data, expected_revision, replace_existing=True, backup_dir=backup_dir, backup_name=backup_name)

    def clear_cache(self, relative: str, expected_revision: str) -> dict[str, Any]:
        path = self.resolve(relative)
        relative = str(path.relative_to(self.root))
        with self._book_lock(relative, exclusive=True):
            if revision(path) != expected_revision:
                raise CoverError("EPUB이 변경되었습니다. 목록을 새로 불러오세요")
            self._delete_book_cache(relative, expected_revision)
        return {"cleared": True, "file_path": relative}

    def _save_cover(self, path: Path, relative: str, image: bytes, expected_revision: str, *, replace_existing: bool = False, backup_dir: Path | None = None, backup_name: str | None = None) -> dict[str, Any]:
        result = add_cover(path, image, expected_revision, backup_dir or self.root / ".cover_backups", replace_existing=replace_existing, backup_name=backup_name)
        return self._finish_save(relative, result, expected_revision)

    def remove(self, relative: str, expected_revision: str, *, backup_dir: Path | None = None, backup_name: str | None = None) -> dict[str, Any]:
        """이 도구가 넣은 표지를 EPUB 에서 빼고, 저장 때와 같이 캐시를 비운다."""
        path = self.resolve(relative)
        relative = str(path.relative_to(self.root))
        with self._book_lock(relative, exclusive=True):
            result = remove_cover(path, expected_revision, backup_dir or self.root / ".cover_backups", backup_name=backup_name)
            return self._finish_save(relative, result, expected_revision)

    def invalidate_view_cache(self, book_id: int) -> None:
        """mtime을 보존하는 표지 편집 뒤 해당 책 ID의 썸네일과 미리보기를 지운다."""
        for directory in (".cover_cache", ".preview_cache"):
            cache_dir = self.root / directory
            if cache_dir.is_dir():
                for cached in cache_dir.glob(f"{book_id}*"):
                    if cached.is_file() and cached.name.startswith((f"{book_id}.", f"{book_id}_")):
                        cached.unlink(missing_ok=True)

    def _finish_save(self, relative: str, result: dict[str, Any], expected_revision: str) -> dict[str, Any]:
        try:
            self._delete_book_cache(relative, expected_revision)
        except OSError as exc:
            LOGGER.warning("Cover saved; candidate cache cleanup failed: %s", type(exc).__name__)
            result["warning"] = "EPUB 표지를 저장했습니다. 후보 캐시를 삭제하지 못했습니다"
        self.invalidate_view_cache(result["book_id"])
        response = {"file_path": relative, "revision": result["revision"], "book_id": result["book_id"],
                    "file_size": result["file_size"], "cover_image": result["cover_image"],
                    "backup_path": str(Path(result["backup_path"]).relative_to(self.root))}
        if result.get("warning"):
            response["warning"] = result["warning"]
        return response

    def preview(self, relative: str, *, original: bool = False) -> bytes:
        path = self.resolve(relative)
        # 저장 중인 파일을 읽지 않도록 표지 검사에서 shared lock을 확인한다.
        if not inspect_epub(path)["has_cover"]:
            raise CoverError("EPUB에 표지가 없습니다")
        from backend.book_manager import BookManager

        if original:
            try:
                with path.open("rb") as source:
                    fcntl.flock(source, fcntl.LOCK_SH | fcntl.LOCK_NB)
                    with zipfile.ZipFile(source) as archive:
                        member = BookManager._find_epub_cover_member(archive)
                        if not member or archive.getinfo(member).file_size > MAX_IMAGE_BYTES:
                            raise CoverError("저장한 표지를 읽을 수 없습니다")
                        return normalize_image(archive.read(member), max_width=600)
            except BlockingIOError as exc:
                raise CoverError("이 EPUB은 현재 저장 중입니다") from exc
            except (KeyError, zipfile.BadZipFile) as exc:
                raise CoverError("저장한 표지를 읽을 수 없습니다") from exc
        data = BookManager._extract_cover_thumbnail(path, "epub")
        if data is None:
            raise CoverError("저장한 표지를 읽을 수 없습니다")
        return data


class BookCoverModel(BaseModel):
    file_path: str = Field(min_length=1, max_length=4096)
    revision: str = Field(min_length=1, max_length=200)


class ApplyCoverModel(BookCoverModel):
    candidate_id: str = Field(pattern=r"^[a-f0-9]{64}$")


class ApplyUrlModel(BookCoverModel):
    url: str = Field(min_length=1, max_length=4096)
    replace_existing: bool = False


class ApplyImageModel(BookCoverModel):
    image_data: str = Field(min_length=1, max_length=((MAX_IMAGE_BYTES + 2) // 3) * 4)


def create_cover_router(get_book_manager: Callable[[], Any]) -> APIRouter:
    router = APIRouter(prefix="/admin/book-covers")

    async def run(operation: Callable[..., Any], *args: Any, request: Request | None = None) -> Any:
        cancelled = threading.Event()
        def execute() -> Any:
            token = request_cancelled.set(cancelled.is_set)
            try:
                return operation(*args)
            finally:
                request_cancelled.reset(token)
        task = asyncio.create_task(asyncio.to_thread(execute))
        try:
            if request is not None:
                while not task.done():
                    await asyncio.wait({task}, timeout=0.1)
                    if not task.done() and await request.is_disconnected():
                        raise HTTPException(status_code=499, detail="표지 요청을 취소했습니다")
            return await task
        except CoverError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        except (OSError, ValueError, requests.RequestException) as exc:
            LOGGER.warning("Cover management failed: %s", type(exc).__name__)
            raise HTTPException(status_code=503, detail="표지 작업을 완료하지 못했습니다. 다시 시도하세요") from exc
        finally:
            cancelled.set()
            if not task.done():
                task.add_done_callback(lambda finished: finished.exception() if not finished.cancelled() else None)

    @router.get("/directories", dependencies=[Depends(require_admin)])
    async def directories(directory: str = Query("", max_length=4096)):
        return {"status": "success", "result": await run(CoverManagement(Book.path_prefix).directories, directory)}

    @router.get("/missing", dependencies=[Depends(require_admin)])
    async def missing(directory: str = Query("", max_length=4096), cursor: str = Query("", max_length=10), limit: int = Query(60, ge=1, le=100), covered: bool = False):
        return {"status": "success", "result": await run(lambda: CoverManagement(Book.path_prefix).missing(directory, cursor, limit, covered=covered))}

    @router.get("/candidates/{store_name}", dependencies=[Depends(require_admin)])
    async def candidates(request: Request, store_name: str, file_path: str = Query(..., min_length=1, max_length=4096), include_covered: bool = False, refresh: bool = False):
        service = CoverManagement(Book.path_prefix)
        return {"status": "success", "result": await run(lambda: service.candidates(store_name, file_path, include_covered=include_covered, refresh=refresh), request=request)}

    @router.get("/candidate-image/{candidate_id}", dependencies=[Depends(require_admin)])
    async def image(request: Request, candidate_id: str):
        data = await run(CoverManagement(Book.path_prefix).image, candidate_id, request=request)
        return Response(data, media_type="image/jpeg", headers={"Cache-Control": "private, max-age=3600"})

    async def complete_apply(result: dict[str, Any]) -> dict[str, Any]:
        try:
            manager = get_book_manager()
            def update_index() -> None:
                doc = manager.es_manager.search_by_id(result["book_id"])
                if doc:
                    if not manager.es_manager.update(result["book_id"], file_size=result["file_size"]):
                        raise ValueError("ES update failed")
            await asyncio.to_thread(update_index)
        except Exception as exc:
            LOGGER.warning("Cover saved; ES metadata update failed: %s", type(exc).__name__)
            result["warning"] = "EPUB 표지를 저장했습니다. 검색 색인의 파일 크기는 갱신하지 못했습니다"
        return {"status": "success", "result": result}

    @router.post("/apply", dependencies=[Depends(require_admin)])
    async def apply(body: ApplyCoverModel):
        return await complete_apply(await run(CoverManagement(Book.path_prefix).apply, body.file_path, body.candidate_id, body.revision))

    @router.post("/apply-url", dependencies=[Depends(require_admin)])
    async def apply_url(body: ApplyUrlModel):
        service = CoverManagement(Book.path_prefix)
        return await complete_apply(await run(lambda: service.apply_url(body.file_path, body.url, body.revision, replace_existing=body.replace_existing)))

    @router.post("/apply-image", dependencies=[Depends(require_admin)])
    async def apply_image(body: ApplyImageModel):
        return await complete_apply(await run(CoverManagement(Book.path_prefix).apply_image, body.file_path, body.image_data, body.revision))

    @router.post("/cache/clear", dependencies=[Depends(require_admin)])
    async def clear_cache(body: BookCoverModel):
        return {"status": "success", "result": await run(CoverManagement(Book.path_prefix).clear_cache, body.file_path, body.revision)}

    @router.get("/preview", dependencies=[Depends(require_admin)])
    async def preview(file_path: str = Query(..., min_length=1, max_length=4096), v: str = Query("", max_length=200), original: bool = False):
        service = CoverManagement(Book.path_prefix)
        data = await run(lambda: service.preview(file_path, original=original))
        return Response(data, media_type="image/jpeg", headers={"Cache-Control": "private, no-store"})

    return router
