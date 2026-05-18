#!/usr/bin/env python

import asyncio
import sys
import os
import time
import logging.config
import uuid
from pathlib import Path
from typing import Any, Literal, Callable, TypeVar
from urllib.parse import urlparse
from fastapi import APIRouter, Depends, FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.middleware.gzip import GZipMiddleware
from fastapi.responses import FileResponse, Response
from fastapi.exceptions import RequestValidationError
from google.auth.transport import requests as google_requests
from google.oauth2 import id_token as google_id_token

# 에러 및 미디어 타입 상수 정의
ERR_MISSING_INPUT = "제목 또는 저자를 입력해주세요"
JSON_MEDIA_TYPE = "application/json"
from pydantic import BaseModel
from backend.auth import require_auth, require_admin, determine_role, create_jwt_token, create_refresh_token, decode_refresh_token, ACCESS_TOKEN_EXPIRATION_SECONDS, REFRESH_TOKEN_EXPIRATION_SECONDS, ACCESS_COOKIE_NAME, REFRESH_COOKIE_NAME
from backend.book_manager import BookManager
from backend.comics_manager import ComicsManager
from backend.bookstore import Yes24Bookstore, AladinBookstore, RidibooksBookstore, NaverShoppingBookstore, NaverSeriesBookstore, MunpiaBookstore
from backend.category_mapping import CategoryMapping
from backend.refresh_token_store import RefreshTokenStore

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_FRONTEND_URL" not in os.environ:
    LOGGER.error("The environment variable TM_FRONTEND_URL is not set.")
    sys.exit(-1)

app = FastAPI()
LOGGER.info("app ready")
origins = [url for url in [os.getenv("TM_FRONTEND_URL")] if url is not None]
app.add_middleware(CORSMiddleware, allow_origins=origins, allow_credentials=True, allow_methods=["*"], allow_headers=["*"], expose_headers=["Accept-Ranges", "Content-Range", "Content-Length", "Content-Encoding", "X-Total-Pages", "X-Total-Chapters"])
app.add_middleware(GZipMiddleware, minimum_size=1000)


@app.exception_handler(RequestValidationError)
async def validation_exception_handler(request: Request, exc: RequestValidationError):
    LOGGER.error("[422] %s %s", request.method, request.url.path)
    LOGGER.error("Validation error: %s", exc.errors())
    LOGGER.error("Request body summary: %s", _summarize_request_body(exc.body))
    from fastapi.responses import JSONResponse

    return JSONResponse(status_code=422, content={"detail": "입력값이 올바르지 않습니다"})


@app.exception_handler(HTTPException)
async def http_exception_handler(request: Request, exc: HTTPException):
    LOGGER.error("[%d] %s %s - %s", exc.status_code, request.method, request.url.path, exc.detail)
    from fastapi.responses import JSONResponse

    return JSONResponse(status_code=exc.status_code, content={"detail": exc.detail})


@app.exception_handler(Exception)
async def general_exception_handler(request: Request, exc: Exception):
    import traceback

    LOGGER.error("[500] %s %s", request.method, request.url.path)
    LOGGER.error("Exception: %s", str(exc))
    LOGGER.error("Traceback:\n%s", traceback.format_exc())
    from fastapi.responses import JSONResponse

    return JSONResponse(status_code=500, content={"detail": "서버 내부 오류가 발생했습니다"})


# JSON 응답에서 한글이 유니코드 이스케이프로 인코딩되지 않도록 설정
import json
from fastapi.responses import JSONResponse


class CustomJSONResponse(JSONResponse):
    def render(self, content) -> bytes:
        return json.dumps(content, ensure_ascii=False, separators=(",", ":"), indent=2).encode("utf-8")


# FastAPI의 기본 JSON 인코더 설정
from fastapi.encoders import jsonable_encoder

# 원본 jsonable_encoder를 백업
_original_jsonable_encoder = jsonable_encoder


def custom_jsonable_encoder(obj, **kwargs):
    """한글이 유니코드 이스케이프로 인코딩되지 않도록 하는 커스텀 인코더"""
    if isinstance(obj, dict):
        return {k: custom_jsonable_encoder(v, **kwargs) for k, v in obj.items()}
    elif isinstance(obj, list):
        return [custom_jsonable_encoder(item, **kwargs) for item in obj]
    elif isinstance(obj, str):
        return obj
    else:
        return _original_jsonable_encoder(obj, **kwargs)


# FastAPI 앱에 커스텀 JSON 인코더 설정
app.json_encoder = custom_jsonable_encoder  # type: ignore[attr-defined]

TM_GOOGLE_CLIENT_ID = os.getenv("TM_GOOGLE_CLIENT_ID")
TM_GOOGLE_CLIENT_SECRET = os.getenv("TM_GOOGLE_CLIENT_SECRET")
GOOGLE_ISSUERS = {"accounts.google.com", "https://accounts.google.com"}
GENERIC_SERVER_ERROR_DETAIL = "서버 내부 오류가 발생했습니다"
GENERIC_MAPPING_ERROR_DETAIL = "카테고리 매핑 처리 중 오류가 발생했습니다"
GENERIC_HIDDEN_CATEGORY_ERROR_DETAIL = "비노출 카테고리 처리 중 오류가 발생했습니다"
GENERIC_MISMATCH_ERROR = "카테고리 불일치 조회 중 오류가 발생했습니다"


_SameSite = Literal["lax", "strict", "none"]


def _get_cookie_settings() -> tuple[bool, _SameSite]:
    secure = _resolve_cookie_secure()
    samesite_raw = os.getenv("TM_COOKIE_SAMESITE", "lax").lower()
    if samesite_raw == "strict":
        samesite: _SameSite = "strict"
    elif samesite_raw == "none":
        samesite = "none"
    else:
        samesite = "lax"
    # SameSite=None은 Secure=True가 필수 (브라우저 요구사항)
    if samesite == "none" and not secure:
        LOGGER.warning("SameSite=None requires Secure=True; falling back to SameSite=Lax")
        samesite = "lax"
    return secure, samesite


def _summarize_request_body(body: Any) -> dict[str, Any]:
    if isinstance(body, dict):
        return {"type": "dict", "keys": sorted(str(key) for key in body.keys())[:20], "key_count": len(body)}
    if isinstance(body, list):
        return {"type": "list", "length": len(body)}
    if body is None:
        return {"type": "none"}
    body_text = str(body)
    return {"type": type(body).__name__.lower(), "length": len(body_text)}


def _is_local_frontend_origin(frontend_url: str | None) -> bool:
    if not frontend_url:
        return False
    parsed = urlparse(frontend_url)
    hostname = (parsed.hostname or "").lower()
    return hostname in {"localhost", "127.0.0.1", "::1"}


def _is_request_from_frontend_host(request: Request) -> bool:
    frontend_url = os.getenv("TM_FRONTEND_URL")
    if not frontend_url:
        return False
    frontend_host = (urlparse(frontend_url).hostname or "").lower()
    request_host = (request.url.hostname or "").lower()
    return bool(frontend_host) and request_host == frontend_host


def _resolve_cookie_secure() -> bool:
    frontend_url = os.getenv("TM_FRONTEND_URL")
    explicit_secure = os.getenv("TM_COOKIE_SECURE")
    is_local = _is_local_frontend_origin(frontend_url)
    if is_local:
        return (explicit_secure or "false").lower() == "true"
    if explicit_secure and explicit_secure.lower() != "true":
        LOGGER.warning("TM_COOKIE_SECURE=false is ignored for non-local frontend origins; forcing Secure cookies")
    return True


def _set_auth_cookies(response: JSONResponse, access_token: str, refresh_token: str | None = None) -> None:
    secure, samesite = _get_cookie_settings()
    response.set_cookie(ACCESS_COOKIE_NAME, access_token, httponly=True, secure=secure, samesite=samesite, max_age=ACCESS_TOKEN_EXPIRATION_SECONDS, path="/")
    if refresh_token is not None:
        response.set_cookie(REFRESH_COOKIE_NAME, refresh_token, httponly=True, secure=secure, samesite=samesite, max_age=REFRESH_TOKEN_EXPIRATION_SECONDS, path="/")


def _clear_auth_cookies(response: JSONResponse) -> None:
    secure, samesite = _get_cookie_settings()
    response.set_cookie(ACCESS_COOKIE_NAME, "", httponly=True, secure=secure, samesite=samesite, max_age=0, path="/")
    response.set_cookie(REFRESH_COOKIE_NAME, "", httponly=True, secure=secure, samesite=samesite, max_age=0, path="/")


T = TypeVar("T")


class _LazyProxy:
    """Initialize heavy dependencies lazily to avoid side effects at import time."""

    def __init__(self, factory: Callable[[], T], name: str) -> None:
        self._factory = factory
        self._instance: T | None = None
        self._name = name

    def _get_instance(self) -> T:
        if self._instance is None:
            self._instance = self._factory()
            LOGGER.info("%s ready", self._name)
        return self._instance

    def __getattr__(self, item):
        return getattr(self._get_instance(), item)

    def __setattr__(self, key, value) -> None:
        if key in {"_factory", "_instance", "_name"}:
            object.__setattr__(self, key, value)
            return
        setattr(self._get_instance(), key, value)

    def __repr__(self) -> str:
        return repr(self._get_instance())


def _create_book_manager() -> BookManager:
    return BookManager()


def _create_comics_manager() -> ComicsManager:
    return ComicsManager()


def _create_bookstore() -> Yes24Bookstore:
    return Yes24Bookstore(base_dir=".", verbose=True)


def _create_category_mapping() -> CategoryMapping:
    return CategoryMapping()


book_manager = _LazyProxy(_create_book_manager, "book manager")
comics_manager = _LazyProxy(_create_comics_manager, "comics manager")
bookstore = _LazyProxy(_create_bookstore, "bookstore")
category_mapping = _LazyProxy(_create_category_mapping, "category mapping")
refresh_token_store = RefreshTokenStore()


def _issue_auth_tokens(email: str, role: str, name: str = "", picture: str = "", family_id: str | None = None) -> tuple[str, str]:
    issued_at = int(time.time())
    refresh_token_id = uuid.uuid4().hex
    refresh_family_id = family_id or uuid.uuid4().hex
    access_token = create_jwt_token(email=email, role=role, name=name, picture=picture)
    refresh_token = create_refresh_token(email=email, role=role, name=name, picture=picture, family_id=refresh_family_id, token_id=refresh_token_id)
    refresh_token_store.store_issued(token_id=refresh_token_id, family_id=refresh_family_id, email=email, issued_at=issued_at, expires_at=issued_at + REFRESH_TOKEN_EXPIRATION_SECONDS)
    return access_token, refresh_token


def _category_matches_hidden(category: str, hidden_categories: list[str]) -> bool:
    if not category:
        return False
    return any(category == hidden_cat or category.startswith(hidden_cat + "/") for hidden_cat in hidden_categories)


async def _get_viewer_hidden_categories(payload: dict, content_type: str) -> list[str]:
    if payload.get("role") != "viewer":
        return []
    return await asyncio.to_thread(category_mapping.get_hidden_categories, content_type=content_type)


async def _ensure_viewer_category_allowed(payload: dict, category: str, content_type: str) -> None:
    hidden_categories = await _get_viewer_hidden_categories(payload, content_type)
    if _category_matches_hidden(category, hidden_categories):
        raise HTTPException(status_code=403, detail="접근 권한이 없는 카테고리입니다.")


async def _get_book_and_ensure_viewer_access(manager, book_id: int, payload: dict, content_type: str):
    book, error = await manager.get_book(book_id)
    if book:
        await _ensure_viewer_category_allowed(payload, book.category, content_type)
    return book, error


async def _ensure_viewer_book_allowed(manager, book_id: int, payload: dict, content_type: str) -> None:
    if payload.get("role") != "viewer":
        return
    await _get_book_and_ensure_viewer_access(manager, book_id, payload, content_type)


class BookModel(BaseModel):
    book_id: int
    category: str
    title: str
    author: str
    file_path: str
    file_type: str
    file_size: int
    line_count: int = 0
    page_count: int = 0
    isbn: str = ""
    updated_time: str
    score: float = 0.0


class CategoryRenameModel(BaseModel):
    old_category: str
    new_category: str


class CategoryDeleteModel(BaseModel):
    category: str


def create_item_router(manager, content_type: str = "book") -> APIRouter:
    """공통 CRUD 엔드포인트를 생성하는 라우터 팩토리"""
    auth_dep = [Depends(require_auth)]
    admin_dep = [Depends(require_admin)]
    router = APIRouter()

    @router.put("/books/{book_id}", dependencies=admin_dep)
    async def update_book(book_id: int, book_item: BookModel, force: bool = False) -> dict[str, Any]:
        LOGGER.debug("# update_book(book_id=%d, book=%r, force=%s)", book_id, book_item, force)
        response_object: dict[str, Any] = {"status": "failure"}
        result, error = await manager.update_book(book_id, new_category=book_item.category, new_title=book_item.title, new_author=book_item.author, new_path=manager.path_prefix / book_item.file_path, new_type=book_item.file_type, force=force)
        if error is None:
            response_object["status"] = "success"
            response_object["result"] = result
        else:
            response_object["error"] = error
        return response_object

    @router.delete("/books/{book_id}", dependencies=admin_dep)
    async def delete_book(book_id: int) -> dict[str, Any]:
        LOGGER.debug("# delete_book(book_id=%d)", book_id)
        response_object: dict[str, Any] = {"status": "failure"}
        result, message = await manager.delete_book(book_id)
        if result == "Ok":
            response_object["status"] = "success"
            response_object["result"] = result
        elif result == "Warning":
            response_object["status"] = "success"
            response_object["result"] = result
            response_object["warning"] = message
        else:
            response_object["error"] = message
        return response_object

    @router.get("/download/{book_id}", response_model=None)
    async def get_book_content(book_id: int, payload: dict = Depends(require_auth)) -> str | FileResponse:
        LOGGER.debug("# get_book(book_id=%d)", book_id)
        await _ensure_viewer_book_allowed(manager, book_id, payload, content_type)
        return await manager.get_book_content(book_id=book_id)

    @router.get("/preview/{book_id}", response_model=None)
    async def get_book_preview(book_id: int, pages: int = 5, chapters: int = 3, payload: dict = Depends(require_auth)):
        LOGGER.debug("# get_book_preview(book_id=%d, pages=%d, chapters=%d)", book_id, pages, chapters)
        api_prefix = "/comics" if content_type == "comic" else ""
        await _ensure_viewer_book_allowed(manager, book_id, payload, content_type)
        return await manager.get_book_preview(book_id=book_id, pages=pages, chapters=chapters, resource_base_url=f"{api_prefix}/html-resource/{book_id}")

    @router.get("/html-resource/{book_id}", response_model=None)
    async def get_html_resource(book_id: int, path: str, payload: dict = Depends(require_auth)) -> Response | FileResponse:
        LOGGER.debug("# get_html_resource(book_id=%d, path='%s')", book_id, path)
        await _ensure_viewer_book_allowed(manager, book_id, payload, content_type)
        return await manager.get_html_resource(book_id=book_id, resource_path=path)

    @router.get("/pdf-pages/{book_id}", response_model=None)
    async def get_pdf_pages(book_id: int, start: int = 1, end: int = 1, payload: dict = Depends(require_auth)):
        LOGGER.debug("# get_pdf_pages(book_id=%d, start=%d, end=%d)", book_id, start, end)
        await _ensure_viewer_book_allowed(manager, book_id, payload, content_type)
        return await manager.get_pdf_pages(book_id=book_id, start=start, end=end)

    @router.get("/validate/{book_id}")
    async def validate_book(book_id: int, payload: dict = Depends(require_auth)) -> dict[str, Any]:
        LOGGER.debug("# validate_book(book_id=%d)", book_id)
        response_object: dict[str, Any] = {"status": "failure"}

        book, err = await manager.get_book(book_id)
        if not book:
            response_object["error"] = f"Book not found: {book_id}"
            return response_object
        await _ensure_viewer_category_allowed(payload, book.category, content_type)

        if book.file_type == "epub":
            result, error = await manager.validate_epub(book_id)
        elif book.file_type == "pdf":
            result, error = await manager.validate_pdf(book_id)
        else:
            response_object["error"] = f"Validation not supported for type: {book.file_type}"
            return response_object

        if result is not None and error is None:
            response_object["status"] = "success"
            response_object["result"] = result
        else:
            response_object["error"] = error
        return response_object

    @router.get("/books/{book_id}")
    async def get_book(book_id: int, payload: dict = Depends(require_auth)) -> dict[str, Any]:
        LOGGER.debug("# get_book(book_id=%d)", book_id)
        response_object: dict[str, Any] = {"status": "failure"}
        book, error = await _get_book_and_ensure_viewer_access(manager, book_id, payload, content_type)
        if book and error is None:
            response_object["status"] = "success"
            response_object["result"] = BookModel(**book.dict())
        else:
            response_object["error"] = error
        return response_object

    @router.put("/categories/rename", dependencies=admin_dep)
    async def rename_category(body: CategoryRenameModel) -> dict[str, Any]:
        LOGGER.debug("# rename_category(old='%s', new='%s')", body.old_category, body.new_category)
        response_object: dict[str, Any] = {"status": "failure"}
        result, error = await manager.rename_category(body.old_category, body.new_category)
        if error is None:
            # MySQL 카테고리 매핑 갱신
            mapping_updated = await asyncio.to_thread(category_mapping.rename_category, body.old_category, body.new_category, content_type=content_type)
            if not mapping_updated:
                LOGGER.warning("rename_category: MySQL 매핑 갱신 실패 (old='%s', new='%s')", body.old_category, body.new_category)
            result["mapping_updated"] = mapping_updated
            response_object["status"] = "success"
            response_object["result"] = result
        else:
            response_object["error"] = error
        return response_object

    @router.post("/categories/delete", dependencies=admin_dep)
    async def delete_category(body: CategoryDeleteModel) -> dict[str, Any]:
        LOGGER.debug("# delete_category(category='%s')", body.category)
        response_object: dict[str, Any] = {"status": "failure"}
        result, error = await manager.delete_category(body.category)
        if error is None:
            # MySQL 카테고리 매핑 삭제 (하위 카테고리 포함, 이벤트 루프 블로킹 방지)
            mapping_deleted = await asyncio.to_thread(category_mapping.delete_category, body.category, content_type=content_type, prefix=True)
            # hidden_categories에서 해당 카테고리 및 하위 카테고리 정리
            await asyncio.to_thread(category_mapping.set_hidden, body.category, False, content_type=content_type)
            hidden_list = await asyncio.to_thread(category_mapping.get_hidden_categories, content_type=content_type)
            cat_prefix = body.category + "/"
            for hidden_cat in hidden_list:
                if hidden_cat.startswith(cat_prefix):
                    await asyncio.to_thread(category_mapping.set_hidden, hidden_cat, False, content_type=content_type)
            if not mapping_deleted:
                LOGGER.warning("delete_category: MySQL 키워드 매핑 삭제 대상 없음 (category='%s')", body.category)
            result["mapping_deleted"] = mapping_deleted
            response_object["status"] = "success"
            response_object["result"] = result
        else:
            response_object["error"] = error
        return response_object

    @router.get("/categories/{category:path}")
    async def get_books_in_category(category: str, payload: dict = Depends(require_auth)) -> dict[str, Any]:
        LOGGER.debug("# get_books_in_category(category='%s')", category)
        response_object: dict[str, Any] = {"status": "failure"}
        await _ensure_viewer_category_allowed(payload, category, content_type)
        result, error = await manager.get_books_in_category(category)
        if error is None:
            response_object["status"] = "success"
            response_object["result"] = [BookModel(**book.dict()) for book in result]
        else:
            response_object["error"] = error
        return response_object

    @router.get("/categories")
    async def get_categories(payload: dict = Depends(require_auth)) -> dict[str, Any]:
        LOGGER.debug("# get_categories()")
        response_object: dict[str, Any] = {"status": "failure"}
        result, error = await manager.get_categories()
        if error is None:
            hidden_categories = await _get_viewer_hidden_categories(payload, content_type)
            if hidden_categories:
                result = {category: count for category, count in result.items() if not _category_matches_hidden(category, hidden_categories)}
            response_object["status"] = "success"
            response_object["result"] = result
        else:
            response_object["error"] = error
        return response_object

    @router.get("/similar/{book_id}")
    async def search_similar_books(book_id: int, offset: int = 0, limit: int = 10, payload: dict = Depends(require_auth)) -> dict[str, Any]:
        LOGGER.debug("# search_similar_books(book_id=%d, offset=%d, limit=%d)", book_id, offset, limit)
        response_object: dict[str, Any] = {"status": "failure"}
        await _ensure_viewer_book_allowed(manager, book_id, payload, content_type)
        similar_list, total, error = await manager.search_similar_books_paged(book_id, size=limit, offset=offset)
        hidden_categories = await _get_viewer_hidden_categories(payload, content_type)
        if hidden_categories:
            similar_list = [book for book in similar_list if not _category_matches_hidden(book.category, hidden_categories)]
            total = len(similar_list)
        if similar_list and error is None:
            response_object["status"] = "success"
            response_object["result"] = [BookModel(**book.dict()) for book in similar_list]
            response_object["total"] = total
            return response_object
        book, err2 = await manager.get_book(book_id)
        if book and err2 is None:
            response_object["status"] = "success"
            response_object["result"] = [BookModel(**book.dict())]
            response_object["total"] = 1
        else:
            response_object["error"] = error or err2
        return response_object

    @router.get("/search/{keyword}")
    async def search_by_keyword(keyword: str, offset: int = 0, limit: int = 10, exclude_categories: str = "", payload: dict = Depends(require_auth)) -> dict[str, Any]:
        LOGGER.debug("# search(keyword=%s, offset=%d, limit=%d, exclude_categories=%s)", keyword, offset, limit, exclude_categories)
        response_object: dict[str, Any] = {"status": "failure"}
        excluded = [c.strip() for c in exclude_categories.split(",") if c.strip()] if exclude_categories else None
        hidden_categories = await _get_viewer_hidden_categories(payload, content_type)
        if hidden_categories:
            excluded = list(dict.fromkeys((excluded or []) + hidden_categories))
        result, total, error = await manager.search_by_keyword_paged(keyword, size=limit, offset=offset, exclude_categories=excluded)
        if error is None:
            response_object["status"] = "success"
            response_object["result"] = [BookModel(**book.dict()) for book in result]
            response_object["total"] = total
        else:
            response_object["error"] = error
        return response_object

    @router.get("/category-mismatches", dependencies=admin_dep)
    async def get_category_mismatches() -> dict[str, Any]:
        """ES 카테고리별 문서 수와 파일시스템 파일 수의 불일치 검출"""
        LOGGER.debug("# get_category_mismatches()")
        response_object: dict[str, Any] = {"status": "failure"}
        try:
            result = await asyncio.to_thread(manager.get_category_mismatches)
            response_object["status"] = "success"
            response_object["result"] = result
        except Exception as e:
            LOGGER.error("get_category_mismatches error: %s", e)
            response_object["error"] = GENERIC_MISMATCH_ERROR
        return response_object

    @router.post("/category-mismatches/index-file", dependencies=admin_dep)
    async def index_single_file(body: dict[str, str]) -> dict[str, Any]:
        """파일시스템의 파일을 ES에 적재"""
        LOGGER.debug("# index_single_file(body=%r)", body)
        file_path = body.get("file_path", "")
        if not file_path:
            raise HTTPException(status_code=400, detail="file_path is required")
        response_object: dict[str, Any] = {"status": "failure"}
        book_id, error = await manager.index_single_file(file_path)
        if book_id is not None and error is None:
            response_object["status"] = "success"
            response_object["result"] = {"book_id": book_id}
        else:
            response_object["error"] = error
        return response_object

    @router.post("/category-mismatches/delete-file", dependencies=admin_dep)
    async def delete_file(body: dict[str, str]) -> dict[str, Any]:
        """파일시스템에서 파일 삭제"""
        LOGGER.debug("# delete_file(body=%r)", body)
        file_path = body.get("file_path", "")
        if not file_path:
            raise HTTPException(status_code=400, detail="file_path is required")
        response_object: dict[str, Any] = {"status": "failure"}
        result, error = await manager.delete_file(file_path)
        if result == "Ok":
            response_object["status"] = "success"
            response_object["result"] = result
        else:
            response_object["error"] = error
        return response_object

    @router.delete("/category-mismatches/es-doc/{book_id}", dependencies=admin_dep)
    async def delete_es_doc_only(book_id: int) -> dict[str, Any]:
        """ES 문서만 삭제 (파일은 유지) — 중복 문서 정리용"""
        LOGGER.debug("# delete_es_doc_only(book_id=%d)", book_id)
        response_object: dict[str, Any] = {"status": "failure"}
        if manager.es_manager.delete(book_id):
            response_object["status"] = "success"
        else:
            response_object["error"] = f"ES 문서 삭제 실패: {book_id}"
        return response_object

    @router.post("/category-mismatches/reload", dependencies=admin_dep)
    async def reload_category(body: CategoryDeleteModel) -> dict[str, Any]:
        """카테고리 전체를 ES에 재적재"""
        LOGGER.info("reload_category 요청: category='%s', content_type='%s'", body.category, content_type)
        response_object: dict[str, Any] = {"status": "failure"}
        result, error = await manager.reload_category(body.category, content_type=content_type)
        if error is None:
            response_object["status"] = "success"
            response_object["result"] = result
            LOGGER.info("reload_category 응답: success — %s", result)
        else:
            response_object["error"] = error
            LOGGER.error("reload_category 응답: failure — %s", error)
        return response_object

    @router.get("/category-mismatches/{category:path}", dependencies=admin_dep)
    async def get_category_mismatch_details(category: str) -> dict[str, Any]:
        """특정 카테고리의 책 수준 불일치 상세 조회"""
        LOGGER.debug("# get_category_mismatch_details(category='%s')", category)
        response_object: dict[str, Any] = {"status": "failure"}
        try:
            result = await asyncio.to_thread(manager.get_category_mismatch_details, category)
            response_object["status"] = "success"
            response_object["result"] = result
        except Exception as e:
            LOGGER.error("get_category_mismatch_details error: %s", e)
            response_object["error"] = GENERIC_MISMATCH_ERROR
        return response_object

    return router


app.include_router(create_item_router(book_manager, content_type="book"))
app.include_router(create_item_router(comics_manager, content_type="comic"), prefix="/comics")


@app.get("/wake")
async def wake_storage(request: Request):
    """USB HDD를 깨우기 위해 책 볼륨 최상위 디렉토리에 접근"""
    if not _is_request_from_frontend_host(request):
        raise HTTPException(status_code=404, detail="Not found")
    book_dir = os.environ.get("TM_BOOK_DIR", "/books")
    try:
        await asyncio.to_thread(os.listdir, book_dir)
        return {"status": "success"}
    except Exception:
        LOGGER.warning("wake_storage failed")
        return JSONResponse(status_code=503, content={"status": "failure"})


@app.get("/search/bookstore/{store_name}", dependencies=[Depends(require_auth)])
async def search_bookstore_api(store_name: str, title: str = "", author: str = "", isbn: str = ""):
    """
    지정된 온라인 서점에서 책을 검색하여 상위 결과의 메타데이터를 반환합니다.
    검색 우선순위: ISBN > 제목+저자 > 제목 > 저자
    """
    store_class = None
    if store_name.lower() == "yes24":
        store_class = Yes24Bookstore
    elif store_name.lower() == "aladin":
        store_class = AladinBookstore
    elif store_name.lower() == "ridi":
        store_class = RidibooksBookstore
    elif store_name.lower() == "naver":
        store_class = NaverShoppingBookstore
    elif store_name.lower() == "naverseries":
        store_class = NaverSeriesBookstore
    elif store_name.lower() == "munpia":
        store_class = MunpiaBookstore
    else:
        raise HTTPException(status_code=404, detail="Bookstore not found")

    if not title and not author and not isbn:
        raise HTTPException(status_code=400, detail=ERR_MISSING_INPUT)

    bookstore = store_class()

    # 입력값 정리
    title = title.strip() if title else ""
    author = author.strip() if author else ""
    isbn = isbn.strip() if isbn else ""

    # 통합 검색 메서드 사용 - 이벤트 루프 차단 방지를 위해 스레드에서 실행
    results, search_keyword, search_method = await asyncio.to_thread(bookstore.search, isbn=isbn, title=title, author=author)

    # 결과가 튜플 리스트이므로 딕셔너리로 변환 (isbn은 튜플 6번째 원소로 이미 포함)
    books_data = []
    for r in results[:5]:
        book_title, book_author, category, book_url, _, book_isbn = r
        item = {"title": book_title, "author": book_author, "category": category, "book_url": book_url}
        if book_isbn:
            item["isbn"] = book_isbn
        books_data.append(item)

    if not books_data:
        return {"status": "not_found", "store": store_name, "search_keyword": search_keyword, "search_method": search_method, "search_url": bookstore.build_search_url(search_keyword) if search_keyword else "", "result": []}

    return {"status": "success", "store": store_name, "search_keyword": search_keyword, "search_method": search_method, "search_url": bookstore.build_search_url(search_keyword) if search_keyword else "", "result": books_data}


@app.post("/auth/google")
async def verify_google_token(request_body: dict):
    credential = request_body.get("credential")
    if not credential:
        raise HTTPException(status_code=400, detail="Credential is required")
    if not TM_GOOGLE_CLIENT_ID:
        raise HTTPException(status_code=500, detail="Google client ID is not configured")

    try:
        result = await asyncio.to_thread(google_id_token.verify_oauth2_token, credential, google_requests.Request(), TM_GOOGLE_CLIENT_ID)
    except ValueError as err:
        LOGGER.error("Google token verification failed: %s", err)
        raise HTTPException(status_code=401, detail="Invalid Google token") from err

    if result.get("iss") not in GOOGLE_ISSUERS:
        LOGGER.error("Google token issuer mismatch: %s", result.get("iss"))
        raise HTTPException(status_code=401, detail="Invalid token issuer")
    if not result.get("email_verified", False):
        LOGGER.warning("Google login rejected because email is not verified: %s", result.get("email", ""))
        raise HTTPException(status_code=401, detail="Email is not verified")

    email = result.get("email", "")
    name = result.get("name", "")
    picture = result.get("picture", "")

    # 서버 측 role 결정
    role = determine_role(email)
    if role is None:
        LOGGER.warning("Unauthorized email login attempt: %s", email)
        raise HTTPException(status_code=403, detail="Access denied")

    access_token, refresh_token = _issue_auth_tokens(email=email, role=role, name=name, picture=picture)

    response = JSONResponse({"status": "success", "email": email, "name": name, "picture": picture, "role": role, "expires_in": ACCESS_TOKEN_EXPIRATION_SECONDS})
    _set_auth_cookies(response, access_token, refresh_token)
    return response


@app.post("/auth/refresh")
async def refresh_access_token(request: Request):
    refresh_token = request.cookies.get(REFRESH_COOKIE_NAME, "")
    if not refresh_token:
        raise HTTPException(status_code=400, detail="Refresh token is required")

    payload = decode_refresh_token(refresh_token)

    email = payload.get("email", "")
    role = determine_role(email)
    if role is None:
        raise HTTPException(status_code=403, detail="Access denied")

    current_token_id = payload["jti"]
    family_id = payload["fid"]
    name = payload.get("name", "")
    picture = payload.get("picture", "")
    issued_at = int(time.time())
    new_token_id = uuid.uuid4().hex
    try:
        rotation_status = refresh_token_store.rotate(current_token_id=current_token_id, new_token_id=new_token_id, family_id=family_id, email=email, issued_at=issued_at, expires_at=issued_at + REFRESH_TOKEN_EXPIRATION_SECONDS)
    except Exception as e:
        LOGGER.error("refresh_token_store.rotate() failed for %s: %s", email, e)
        raise HTTPException(status_code=503, detail="서비스를 일시적으로 사용할 수 없습니다. 잠시 후 다시 시도해 주세요.")
    if rotation_status != "ok":
        LOGGER.warning("Refresh token rotation rejected for %s: %s", email, rotation_status)
        response = JSONResponse(status_code=401, content={"detail": "Invalid refresh token state"})
        _clear_auth_cookies(response)
        return response

    token = create_jwt_token(email=email, role=role, name=name, picture=picture)
    new_refresh_token = create_refresh_token(email=email, role=role, name=name, picture=picture, family_id=family_id, token_id=new_token_id)

    response = JSONResponse({"status": "success", "expires_in": ACCESS_TOKEN_EXPIRATION_SECONDS})
    _set_auth_cookies(response, token, new_refresh_token)
    return response


@app.get("/auth/me")
async def auth_me(payload: dict = Depends(require_auth)):
    remaining = max(0, payload.get("exp", 0) - int(time.time()))
    return {"status": "success", "result": {"email": payload.get("email", ""), "role": payload.get("role", ""), "name": payload.get("name", ""), "picture": payload.get("picture", ""), "expires_in": remaining}}


@app.post("/auth/logout")
async def logout(request: Request):
    refresh_token = request.cookies.get(REFRESH_COOKIE_NAME, "")
    if refresh_token:
        try:
            payload = decode_refresh_token(refresh_token)
            refresh_token_store.revoke_family(payload["fid"], reason="logout")
        except HTTPException:
            LOGGER.debug("Ignoring invalid refresh token during logout")
    response = JSONResponse({"status": "success"})
    _clear_auth_cookies(response)
    return response


# === 카테고리 매핑 API ===


class CategoryKeywordsModel(BaseModel):
    keywords: list[str]


class CategoryMappingsModel(BaseModel):
    mappings: dict[str, list[str]]


@app.get("/category-mappings", dependencies=[Depends(require_auth)])
async def get_all_category_mappings(content_type: str = "book") -> dict[str, Any]:
    """모든 카테고리-키워드 매핑 조회"""
    LOGGER.debug("# get_all_category_mappings(content_type=%s)", content_type)
    try:
        mappings = await asyncio.to_thread(category_mapping.get_all_mappings, content_type=content_type)
        return {"status": "success", "result": mappings}
    except Exception as e:
        LOGGER.error("get_all_category_mappings error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_MAPPING_ERROR_DETAIL)


@app.get("/category-mappings/{category}", dependencies=[Depends(require_auth)])
async def get_category_keywords(category: str, content_type: str = "book") -> dict[str, Any]:
    """특정 카테고리의 키워드 목록 조회"""
    LOGGER.debug("# get_category_keywords(category='%s', content_type=%s)", category, content_type)
    try:
        keywords = await asyncio.to_thread(category_mapping.get_keywords, category, content_type=content_type)
        return {"status": "success", "result": keywords}
    except Exception as e:
        LOGGER.error("get_category_keywords error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_MAPPING_ERROR_DETAIL)


@app.put("/category-mappings/{category}", dependencies=[Depends(require_admin)])
async def set_category_keywords(category: str, body: CategoryKeywordsModel, content_type: str = "book") -> dict[str, Any]:
    """카테고리의 키워드 목록 설정 (기존 대체)"""
    LOGGER.debug("# set_category_keywords(category='%s', keywords=%s, content_type=%s)", category, body.keywords, content_type)
    try:
        success = await asyncio.to_thread(category_mapping.set_keywords, category, body.keywords, content_type=content_type)
        if success:
            return {"status": "success", "result": await asyncio.to_thread(category_mapping.get_keywords, category, content_type=content_type)}
        else:
            raise HTTPException(status_code=500, detail="Failed to set keywords")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("set_category_keywords error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_MAPPING_ERROR_DETAIL)


@app.post("/category-mappings/{category}/keywords", dependencies=[Depends(require_admin)])
async def add_category_keyword(category: str, body: dict[str, str], content_type: str = "book") -> dict[str, Any]:
    """카테고리에 키워드 추가"""
    keyword = body.get("keyword", "")
    LOGGER.debug("# add_category_keyword(category='%s', keyword='%s', content_type=%s)", category, keyword, content_type)
    if not keyword:
        raise HTTPException(status_code=400, detail="Keyword is required")
    try:
        success = await asyncio.to_thread(category_mapping.add_keyword, category, keyword, content_type=content_type)
        if success:
            return {"status": "success", "result": await asyncio.to_thread(category_mapping.get_keywords, category, content_type=content_type)}
        else:
            return {"status": "duplicate", "message": "Keyword already exists", "result": await asyncio.to_thread(category_mapping.get_keywords, category, content_type=content_type)}
    except Exception as e:
        LOGGER.error("add_category_keyword error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_MAPPING_ERROR_DETAIL)


@app.delete("/category-mappings/{category}/keywords/{keyword}", dependencies=[Depends(require_admin)])
async def remove_category_keyword(category: str, keyword: str, content_type: str = "book") -> dict[str, Any]:
    """카테고리에서 키워드 삭제"""
    LOGGER.debug("# remove_category_keyword(category='%s', keyword='%s', content_type=%s)", category, keyword, content_type)
    try:
        success = await asyncio.to_thread(category_mapping.remove_keyword, category, keyword, content_type=content_type)
        if success:
            return {"status": "success", "result": await asyncio.to_thread(category_mapping.get_keywords, category, content_type=content_type)}
        else:
            raise HTTPException(status_code=404, detail="Keyword not found")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("remove_category_keyword error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_MAPPING_ERROR_DETAIL)


@app.delete("/category-mappings/{category}", dependencies=[Depends(require_admin)])
async def delete_category_mapping(category: str, content_type: str = "book") -> dict[str, Any]:
    """카테고리의 모든 키워드 삭제"""
    LOGGER.debug("# delete_category_mapping(category='%s', content_type=%s)", category, content_type)
    try:
        success = await asyncio.to_thread(category_mapping.delete_category, category, content_type=content_type)
        if success:
            return {"status": "success"}
        else:
            raise HTTPException(status_code=404, detail="Category not found")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("delete_category_mapping error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_MAPPING_ERROR_DETAIL)


@app.put("/category-mappings", dependencies=[Depends(require_admin)])
async def update_all_category_mappings(body: CategoryMappingsModel, content_type: str = "book") -> dict[str, Any]:
    """전체 매핑 일괄 업데이트"""
    LOGGER.debug("# update_all_category_mappings(content_type=%s)", content_type)
    try:
        success = await asyncio.to_thread(category_mapping.update_all_mappings, body.mappings, content_type=content_type)
        if success:
            return {"status": "success", "result": await asyncio.to_thread(category_mapping.get_all_mappings, content_type=content_type)}
        else:
            raise HTTPException(status_code=500, detail="Failed to update mappings")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("update_all_category_mappings error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_MAPPING_ERROR_DETAIL)


# === 비노출 카테고리 API ===


class HiddenCategoryModel(BaseModel):
    hidden: bool


@app.get("/hidden-categories")
async def get_hidden_categories(payload: dict = Depends(require_auth), content_type: str = "book") -> dict[str, Any]:
    """비노출 카테고리 목록 조회"""
    LOGGER.debug("# get_hidden_categories(content_type=%s)", content_type)
    try:
        if payload.get("role") == "viewer":
            return {"status": "success", "result": []}
        categories = await asyncio.to_thread(category_mapping.get_hidden_categories, content_type=content_type)
        return {"status": "success", "result": categories}
    except Exception as e:
        LOGGER.error("get_hidden_categories error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_HIDDEN_CATEGORY_ERROR_DETAIL)


@app.post("/hidden-categories/{category:path}", dependencies=[Depends(require_admin)])
async def set_hidden_category(category: str, body: HiddenCategoryModel, content_type: str = "book") -> dict[str, Any]:
    """카테고리 비노출 설정/해제"""
    LOGGER.debug("# set_hidden_category(category='%s', hidden=%s, content_type=%s)", category, body.hidden, content_type)
    try:
        success = await asyncio.to_thread(category_mapping.set_hidden, category, body.hidden, content_type=content_type)
        if success:
            return {"status": "success", "result": await asyncio.to_thread(category_mapping.get_hidden_categories, content_type=content_type)}
        else:
            raise HTTPException(status_code=500, detail="Failed to update hidden category")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("set_hidden_category error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_HIDDEN_CATEGORY_ERROR_DETAIL)
