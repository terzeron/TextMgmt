#!/usr/bin/env python

import asyncio
import sys
import os
import json
import time
import logging.config
import uuid
from pathlib import Path
from typing import Any, AsyncIterator, Literal, Callable, TypeVar
from contextlib import asynccontextmanager, suppress
from urllib.parse import urlparse
from fastapi import APIRouter, BackgroundTasks, Depends, FastAPI, HTTPException, Query, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.middleware.gzip import GZipMiddleware
from fastapi.responses import FileResponse, JSONResponse, Response
from fastapi.exceptions import RequestValidationError
from fastapi.encoders import jsonable_encoder
from google.auth.transport import requests as google_requests
from google.oauth2 import id_token as google_id_token

from pydantic import BaseModel
from backend.auth import require_auth, require_admin, optional_auth, determine_role, create_jwt_token, create_refresh_token, decode_refresh_token, observation_hash, ACCESS_TOKEN_EXPIRATION_SECONDS, REFRESH_TOKEN_EXPIRATION_SECONDS, ACCESS_COOKIE_NAME, REFRESH_COOKIE_NAME
from backend.book_manager import BookManager, MAX_LATEST_BOOK_COUNT
from backend.comics_manager import ComicsManager
from backend.bookstore import Yes24Bookstore, AladinBookstore, RidibooksBookstore, NaverShoppingBookstore, NaverSeriesBookstore, MunpiaBookstore
from backend.category_mapping import CategoryMapping
from backend.refresh_token_store import create_refresh_token_store
from backend.view_history_store import MAX_RECENT_VIEWS, create_view_history_store

# 에러 및 미디어 타입 상수 정의
ERR_MISSING_INPUT = "제목 또는 저자를 입력해주세요"
JSON_MEDIA_TYPE = "application/json"

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

if "TM_FRONTEND_URL" not in os.environ:
    LOGGER.error("The environment variable TM_FRONTEND_URL is not set.")
    sys.exit(-1)

app = FastAPI()
LOGGER.info("app ready")
origins = [url for url in [os.getenv("TM_FRONTEND_URL")] if url is not None]
# 최소 허용 CORS (CWE-942): 실제 사용하는 메서드/헤더만 명시. preflight(OPTIONS)는 Starlette가 자동 처리.
# 인증은 HttpOnly 쿠키(credentials include) 기반이라 Authorization 헤더는 미사용 → allow_headers에서 제외.
app.add_middleware(CORSMiddleware, allow_origins=origins, allow_credentials=True, allow_methods=["GET", "POST", "PUT", "DELETE"], allow_headers=["Content-Type"], expose_headers=["Accept-Ranges", "Content-Range", "Content-Length", "Content-Encoding", "X-Total-Pages", "X-Total-Chapters"])
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


class CustomJSONResponse(JSONResponse):
    def render(self, content) -> bytes:
        return json.dumps(content, ensure_ascii=False, separators=(",", ":"), indent=2).encode("utf-8")


# FastAPI의 기본 JSON 인코더 설정

# 원본 jsonable_encoder를 백업
_original_jsonable_encoder = jsonable_encoder


def stale_running_status(status: dict[str, Any], stale_seconds: float, now: float | None = None) -> dict[str, Any] | None:
    """갱신이 끊긴 running/applying 상태를 failed 로 바꾼 사본. 멀쩡하면 None 을 돌려준다.

    자동 분류는 pod 메모리 안의 백그라운드 태스크라 재배포·재시작이면 사라지는데,
    상태 파일은 볼륨에 남아 running 인 채로 굳는다. 그러면 POST 가 already_running
    으로 막아 버튼을 다시 눌러도 시작되지 않고 화면은 계속 회전한다(실제로 겪음).

    총 소요 시간이 아니라 "최근에 살아있다는 신호"로 판정해야, 정상적으로 오래 걸리는
    작업과 죽어서 안 풀리는 상태를 구분할 수 있다.
    """
    if status.get("status") not in ("running", "applying"):
        return None
    updated_at = status.get("updated_at")
    if isinstance(updated_at, (int, float)):
        age = (now if now is not None else time.time()) - float(updated_at)
        if age <= stale_seconds:
            return None
    # updated_at 이 없거나 꼴이 틀린 상태 파일은 살아있다고 볼 근거가 없다.
    return {**status, "status": "failed", "error": "백엔드가 다시 시작되어 자동 분류가 중단되었습니다. 다시 실행해 주세요."}


def custom_jsonable_encoder(obj, **kwargs):
    """한글이 유니코드 이스케이프로 인코딩되지 않도록 하는 커스텀 인코더"""
    match obj:
        case dict():
            return {k: custom_jsonable_encoder(v, **kwargs) for k, v in obj.items()}
        case list():
            return [custom_jsonable_encoder(item, **kwargs) for item in obj]
        case str():
            return obj
        case _:
            return _original_jsonable_encoder(obj, **kwargs)


# FastAPI 앱에 커스텀 JSON 인코더 설정
app.json_encoder = custom_jsonable_encoder  # type: ignore[attr-defined]

TM_GOOGLE_CLIENT_ID = os.getenv("TM_GOOGLE_CLIENT_ID")
TM_GOOGLE_CLIENT_SECRET = os.getenv("TM_GOOGLE_CLIENT_SECRET")
GOOGLE_ISSUERS = {"accounts.google.com", "https://accounts.google.com"}
GENERIC_SERVER_ERROR_DETAIL = "서버 내부 오류가 발생했습니다"
GENERIC_MAPPING_ERROR_DETAIL = "카테고리 매핑 처리 중 오류가 발생했습니다"
GENERIC_HIDDEN_CATEGORY_ERROR_DETAIL = "비노출 카테고리 처리 중 오류가 발생했습니다"
GENERIC_LATEST_EXCLUDED_CATEGORY_ERROR_DETAIL = "최신 자료 검색 제외 카테고리 처리 중 오류가 발생했습니다"
GENERIC_MISMATCH_ERROR = "카테고리 불일치 조회 중 오류가 발생했습니다"
# 재적재 진행 카운트를 DB에 기록하는 주기. 화면(폴링 10초)에서 잔여 건수가 이 주기로 움직인다.
RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS = 30


_SameSite = Literal["lax", "strict", "none"]


def _get_cookie_settings() -> tuple[bool, _SameSite]:
    secure = _resolve_cookie_secure()
    samesite_raw = os.getenv("TM_COOKIE_SAMESITE", "lax").lower()
    samesite: _SameSite
    match samesite_raw:
        case "strict":
            samesite = "strict"
        case "none":
            samesite = "none"
        case _:
            samesite = "lax"
    # SameSite=None은 Secure=True가 필수 (브라우저 요구사항)
    if samesite == "none" and not secure:
        LOGGER.warning("SameSite=None requires Secure=True; falling back to SameSite=Lax")
        samesite = "lax"
    return secure, samesite


def _summarize_request_body(body: Any) -> dict[str, Any]:
    match body:
        case dict():
            return {"type": "dict", "keys": sorted(str(key) for key in body.keys())[:20], "key_count": len(body)}
        case list():
            return {"type": "list", "length": len(body)}
        case None:
            return {"type": "none"}
        case _:
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

    def __delattr__(self, item) -> None:
        delattr(self._get_instance(), item)

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
refresh_token_store = _LazyProxy(create_refresh_token_store, "refresh token store")
view_history_store = _LazyProxy(create_view_history_store, "view history store")


def _client_ip(request: Request) -> str:
    """실제 접속 클라이언트 IP. 경유 프록시 IP 가 잡히지 않도록 헤더 우선순위를 둔다.

    실제 경로는 클라이언트 → Cloudflare → Traefik → 백엔드 pod 다. 이 구성에서는
    - `request.client.host` 는 Traefik pod IP (Service 가 externalTrafficPolicy=Cluster 라 SNAT),
    - `X-Forwarded-For` 첫 항목은 Cloudflare edge IP (Cloudflare 가 XFF 를 보내지 않아
      Traefik 이 자기가 본 peer 로 헤더를 새로 만든다)
    라서 둘 다 프록시 IP 다. Cloudflare 가 원 클라이언트 IP 로 덮어써 주는
    `CF-Connecting-IP` 를 우선 쓰고, Cloudflare 를 거치지 않는 경로(사내망 직결·로컬
    개발)에서만 XFF·peer 로 내려간다. 스푸핑 가능한 값이지만 표시·감사 전용이며 인가
    판단에는 쓰지 않는다.
    """
    for header in ("CF-Connecting-IP", "True-Client-IP"):
        value = request.headers.get(header, "").strip()
        if value:
            return value
    forwarded = request.headers.get("X-Forwarded-For", "")
    if forwarded:
        first_hop = forwarded.split(",")[0].strip()
        if first_hop:
            return first_hop
    return request.client.host if request.client else ""


def _request_user_agent(request: Request) -> str:
    return request.headers.get("User-Agent", "")


def _request_ip_prefix(request: Request) -> str:
    """관측 로그용 IP 프리픽스. IPv4 는 /24, IPv6 는 /48 까지만 남긴다."""
    client_ip = _client_ip(request)
    if not client_ip:
        return ""
    if ":" in client_ip:
        return ":".join(client_ip.split(":")[:3])
    return ".".join(client_ip.split(".")[:3])


def _log_refresh_rotation_rejected(request: Request, *, status: str, email: str, family_id: str, token_id: str) -> None:
    """refresh 회전 거부 사건을 마스킹된 필드로만 기록한다.

    `reuse-detected` 가 같은 브라우저의 멀티탭 동시성에서 오는지, 다른 환경에서 복사된
    상태에서 오는지 구분하기 위한 관측 로그다. UA/IP 해시는 진단 신호일 뿐이며 인가
    판단에 사용하지 않는다. 원문 토큰·쿠키·user-agent·전체 IP·전체 이메일은 남기지 않는다.
    """
    try:
        observation = refresh_token_store.get_token_observation(token_id)
    except Exception as e:
        # 관측 실패가 refresh 응답을 막지 않아야 한다.
        LOGGER.debug("refresh rotation observation lookup failed: %s", e)
        observation = None
    LOGGER.warning(
        "Refresh token rotation rejected event=refresh-rotation-rejected status=%s email_hash=%s family_hash=%s jti_hash=%s replaced_by_present=%s request_user_agent_hash=%s request_ip_prefix_hash=%s",
        status,
        observation_hash(email),
        observation_hash(family_id),
        observation_hash(token_id),
        "true" if observation and observation["replaced_by_present"] else "false",
        observation_hash(request.headers.get("User-Agent", "")),
        observation_hash(_request_ip_prefix(request)),
    )


def _issue_auth_tokens(email: str, role: str, name: str = "", picture: str = "", family_id: str | None = None, client_ip: str = "", user_agent: str = "") -> tuple[str, str]:
    issued_at = int(time.time())
    refresh_token_id = uuid.uuid4().hex
    refresh_family_id = family_id or uuid.uuid4().hex
    access_token = create_jwt_token(email=email, role=role, name=name, picture=picture)
    refresh_token = create_refresh_token(email=email, role=role, name=name, picture=picture, family_id=refresh_family_id, token_id=refresh_token_id)
    refresh_token_store.store_issued(token_id=refresh_token_id, family_id=refresh_family_id, email=email, issued_at=issued_at, expires_at=issued_at + REFRESH_TOKEN_EXPIRATION_SECONDS, client_ip=client_ip, user_agent=user_agent)
    return access_token, refresh_token


def _category_matches_hidden(category: str, hidden_categories: list[str]) -> bool:
    if not category:
        return False
    return any(category == hidden_cat or category.startswith(hidden_cat + "/") for hidden_cat in hidden_categories)


async def _get_viewer_hidden_categories(payload: dict, content_type: str) -> list[str]:
    if payload.get("role") != "viewer":
        return []
    return await asyncio.to_thread(category_mapping.get_hidden_categories, content_type=content_type)


async def _get_latest_excluded_categories(content_type: str) -> list[str]:
    return await asyncio.to_thread(category_mapping.get_latest_excluded_categories, content_type=content_type)


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
    created_time: str = ""
    updated_time: str
    score: float = 0.0


class CategoryRenameModel(BaseModel):
    old_category: str
    new_category: str


class CategoryDeleteModel(BaseModel):
    category: str


class ReloadAllMismatchesModel(BaseModel):
    reload_source: Literal["bulk", "mismatch"] = "bulk"


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


def create_item_router(manager, content_type: str = "book") -> APIRouter:
    """공통 CRUD 엔드포인트를 생성하는 라우터 팩토리"""
    admin_dep = [Depends(require_admin)]
    router = APIRouter()
    # 상태 파일 하나를 제안(propose)과 승인 적용(apply) 두 단계가 함께 쓴다.
    classify_proposal_state: dict[str, Any] = {"status": "idle", "remaining_count": 0}

    # 갱신이 이만큼 끊기면 죽은 작업으로 본다. category_mapping 의
    # RELOAD_LOCK_HEARTBEAT_STALE_SECONDS 와 같은 방식이다.
    # 파일 1건 처리는 서점 조회 때문에 3초 남짓이라 5분이면 넉넉하다.
    CLASSIFY_PROPOSAL_STALE_SECONDS = 5 * 60

    def _classify_proposal_path() -> Path | None:
        try:
            return Path(manager.path_prefix) / f".classify_proposal_{content_type}.json"
        except (TypeError, ValueError):
            return None

    def _read_classify_proposal() -> dict[str, Any]:
        status_path = _classify_proposal_path()
        if status_path is None or not status_path.exists():
            return dict(classify_proposal_state)
        try:
            with status_path.open("r", encoding="utf-8") as status_file:
                status = json.load(status_file)
        except Exception as e:
            LOGGER.warning("classify_proposal 파일 읽기 실패: %s", e)
            return dict(classify_proposal_state)
        if not isinstance(status, dict):
            return dict(classify_proposal_state)
        status = _fail_if_stale(status)
        classify_proposal_state.clear()
        classify_proposal_state.update(status)
        return dict(classify_proposal_state)

    def _fail_if_stale(status: dict[str, Any]) -> dict[str, Any]:
        """죽은 작업이 화면과 재실행을 막지 않도록, 갱신이 끊긴 running 을 failed 로 굳힌다."""
        stale = stale_running_status(status, CLASSIFY_PROPOSAL_STALE_SECONDS)
        if stale is None:
            return status
        LOGGER.warning("분류 작업 상태가 %.0f초 넘게 갱신되지 않아 중단된 작업으로 본다", CLASSIFY_PROPOSAL_STALE_SECONDS)
        _replace_classify_proposal(stale)
        return stale

    def _replace_classify_proposal(next_status: dict[str, Any]) -> None:
        next_status = {**next_status, "updated_at": time.time()}
        classify_proposal_state.clear()
        classify_proposal_state.update(next_status)
        status_path = _classify_proposal_path()
        if status_path is None or not status_path.parent.exists():
            return
        tmp_path = status_path.with_name(f"{status_path.name}.{os.getpid()}.{uuid.uuid4().hex}.tmp")
        try:
            with tmp_path.open("w", encoding="utf-8") as status_file:
                json.dump(next_status, status_file, ensure_ascii=False, separators=(",", ":"))
            tmp_path.replace(status_path)
        except Exception as e:
            LOGGER.warning("classify_proposal 파일 쓰기 실패: %s", e)
            try:
                tmp_path.unlink(missing_ok=True)
            except OSError:
                pass

    def _start_classify_proposal(category: str) -> None:
        """제안 시작 시 이전 상태를 지우고 진행 중 표시로 새로 시작한다.

        항목 목록(items)은 DB(classify_proposal_items)에 있으므로 상태 파일에는
        카운트와 상태만 둔다 — 실제 클리어는 _run_classify_proposal_job이 한다.
        """
        _replace_classify_proposal({"status": "running", "content_type": content_type, "source_category": category, "total_count": 0, "processed_count": 0, "failures": []})

    def _progress_classify_proposal(progress: dict[str, int]) -> None:
        """제안(running)과 적용(applying)이 status 필드를 공유해도, 진행률 갱신은
        status를 건드리지 않는다. 그러지 않으면 적용 중 진행률 콜백이 status를
        running으로 되돌려 화면에 "적용 중"이 아니라 "제안 생성 중"으로 보인다."""
        _replace_classify_proposal({**_read_classify_proposal(), **progress})

    async def _run_classify_proposal_job(category: str, use_bookstore: bool, use_content_meta: bool, delay: float) -> None:
        # 새로 분류된 항목을 100건 단위로 모아 DB에 적재한다. 책 한 권마다 INSERT +
        # COMMIT을 하면 왕복 비용이 책 수만큼 생기므로, 여기서 모았다가
        # add_classify_proposal_items(executemany 한 번 + commit 한 번)로 넘긴다.
        pending: list[dict[str, Any]] = []
        # 100건 기준이면 100권 미만 카테고리(이 저장소 카테고리 중앙값 5권)는 분류가 끝날
        # 때까지 화면이 빈다 — "중간에 들어와서 진행 상황 보기"가 정작 흔한 경우에서 안
        # 된다. 10건으로 낮춰도 가장 큰 카테고리(79,589권) 기준 왕복 7,959번, 20초 남짓이라
        # 비용은 문제가 안 된다.
        CLASSIFY_PROPOSAL_ITEMS_BATCH_SIZE = 10

        def _flush_pending() -> None:
            if not pending:
                return
            category_mapping.add_classify_proposal_items(list(pending), category, content_type=content_type)
            pending.clear()

        def _on_progress(progress: dict[str, Any]) -> None:
            # book_manager가 이번 틱에서 새로 만든 항목만 new_items로 보낸다. 상태
            # 파일에는 카운트만 남기고(items를 넣으면 다시 파일 하나에 전체 목록을
            # 담는 옛 방식으로 되돌아간다), 항목은 버퍼에 모았다가 100건마다 DB로 넘긴다.
            new_items = progress.pop("new_items", None)
            if new_items:
                pending.extend(new_items)
                if len(pending) >= CLASSIFY_PROPOSAL_ITEMS_BATCH_SIZE:
                    _flush_pending()
            _progress_classify_proposal(progress)

        try:
            await asyncio.to_thread(category_mapping.clear_classify_proposal_items, content_type=content_type)
            mappings = await asyncio.to_thread(category_mapping.get_all_mappings, content_type=content_type)
            result, error = await manager.propose_category_changes(category, mappings, content_type=content_type, use_bookstore=use_bookstore, use_content_meta=use_content_meta, delay=delay, on_progress=_on_progress)
        except Exception as e:
            LOGGER.error("classify proposal error: %s", e)
            # 예외로 중단되더라도, 그때까지 모아둔 항목은 버리지 않고 DB에 남겨
            # 화면에서 어디까지 진행됐는지 볼 수 있게 한다.
            _flush_pending()
            _replace_classify_proposal({**_read_classify_proposal(), "status": "failed", "error": "분류 제안에 실패했습니다."})
            return
        # 100의 배수가 아닌 꼬리를 반드시 비운다. 누락되면 관리자가 목록 끝의 책들을
        # 못 보고 승인하게 된다.
        _flush_pending()
        if error is None:
            result_counts = {k: v for k, v in result.items() if k != "items"}
            _replace_classify_proposal({**_read_classify_proposal(), **result_counts, "status": "ready"})
        else:
            _replace_classify_proposal({**_read_classify_proposal(), "status": "failed", "error": error})

    async def _run_classify_apply_job(items: list[dict[str, Any]], allowed: set[str], clean_existing: bool, apply_token: str) -> None:
        # applying 선점은 핸들러가 응답 전에 동기로 이미 해뒀다(TOCTOU 창을 없애려고). 여기서
        # 다시 _read_classify_proposal() 로 읽어 "applying"을 또 쓰면, 그 사이 다른 요청이
        # 상태를 바꿨어도 이 시점에 덮어써서 선점의 의미가 없어진다.
        def _token_still_valid() -> bool:
            # 하트비트 만료로 상태가 failed로 굳은 뒤 두 번째 apply가 새 토큰으로 다시
            # 선점하면, 이 작업(먼저 돈 쪽)은 더 이상 이 승인의 주인이 아니다. 계속
            # 진행하면 두 작업이 같은 행에 경쟁적으로 써서 moved를 failed로 덮어쓸 수 있다.
            return _read_classify_proposal().get("apply_token") == apply_token

        async def _on_item_done(entry: dict[str, Any]) -> None:
            if entry["file_path"] not in allowed:
                # book_manager는 "제안 목록에 없는 파일입니다" 같은 거절 항목도 on_item_done으로
                # 알려준다. 이 작업이 애초에 허용하지 않은 파일(예: 재승인 때 클라이언트가
                # 여전히 들고 있는 이미 moved인 항목)의 행을 건드리면 moved가 failed로
                # 덮어써진다 — 그 행은 이 작업의 소관이 아니므로 손대지 않는다.
                return
            # 파일 이동/실패 직후 그 행 하나만 바로 기록한다. 끝나고 한꺼번에 병합하면
            # 도중에 pod가 죽었을 때 무엇이 옮겨졌는지 알 수 없다 — book_manager는 DB를
            # 모르므로 이 콜백에서 여기(main.py)가 직접 쓴다. pymysql은 블로킹 호출이라
            # 단일 워커의 이벤트 루프를 막지 않도록 다른 DB 호출들처럼 스레드로 넘긴다.
            await asyncio.to_thread(category_mapping.update_classify_proposal_item_status, entry["file_path"], entry["apply_status"], entry.get("apply_error"), content_type=content_type)

        try:
            result, error = await manager.apply_category_changes(items, allowed, content_type=content_type, clean_existing=clean_existing, on_progress=_progress_classify_proposal, on_item_done=_on_item_done, should_continue=_token_still_valid)
            if not _token_still_valid():
                # 다른 승인 작업이 이미 이 작업을 대체했다 — 상태 파일을 더 건드리면 그
                # 새 작업의 진행 상황을 덮어쓰게 되므로 아무것도 쓰지 않고 물러난다.
                LOGGER.warning("classify apply job stopped: apply_token mismatch (다른 승인 작업이 선점함)")
                return
            if error is not None:
                _replace_classify_proposal({**_read_classify_proposal(), "status": "failed", "error": error})
                return
            current = _read_classify_proposal()
            _replace_classify_proposal({**current, "status": "done", "applied_count": result.get("applied_count"), "failed_count": result.get("failed_count")})
        except Exception as e:
            LOGGER.error("classify apply error: %s", e)
            if _token_still_valid():
                _replace_classify_proposal({**_read_classify_proposal(), "status": "failed", "error": "분류 적용에 실패했습니다."})

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
        match result:
            case "Ok":
                response_object["status"] = "success"
                response_object["result"] = result
            case "Warning":
                response_object["status"] = "success"
                response_object["result"] = result
                response_object["warning"] = message
            case _:
                response_object["error"] = message
        return response_object

    @router.get("/download/{book_id}", response_model=None)
    async def get_book_content(book_id: int, payload: dict = Depends(require_auth)) -> str | FileResponse:
        LOGGER.debug("# get_book(book_id=%d)", book_id)
        await _ensure_viewer_book_allowed(manager, book_id, payload, content_type)
        return await manager.get_book_content(book_id=book_id)

    @router.get("/preview/{book_id}", response_model=None)
    async def get_book_preview(book_id: int, pages: int = 5, chapters: int = 10, payload: dict = Depends(require_auth)):
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

        book, _ = await manager.get_book(book_id)
        if not book:
            response_object["error"] = f"Book not found: {book_id}"
            return response_object
        await _ensure_viewer_category_allowed(payload, book.category, content_type)

        match book.file_type:
            case "epub":
                result, error = await manager.validate_epub(book_id)
            case "pdf":
                result, error = await manager.validate_pdf(book_id)
            case _:
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

    @router.get("/latest")
    async def get_latest_books(limit: int = Query(MAX_LATEST_BOOK_COUNT, ge=1, le=MAX_LATEST_BOOK_COUNT), payload: dict = Depends(require_auth)) -> dict[str, Any]:
        LOGGER.debug("# get_latest_books(limit=%d)", limit)
        response_object: dict[str, Any] = {"status": "failure"}
        hidden_categories = await _get_viewer_hidden_categories(payload, content_type)
        latest_excluded_categories = await _get_latest_excluded_categories(content_type)
        excluded_categories = list(dict.fromkeys(hidden_categories + latest_excluded_categories))
        result, total, error = await manager.get_latest_books(size=limit, exclude_categories=excluded_categories)
        if error is None:
            response_object["status"] = "success"
            response_object["result"] = [BookModel(**book.dict()) for book in result]
            response_object["total"] = total
        else:
            response_object["error"] = error
        return response_object

    # 책 라우터는 루트, 만화 라우터는 /comics prefix 에 붙으므로 경로 템플릿을 유형별로
    # 맞춘다: POST /books/view-history/{id} 와 POST /comics/view-history/{id}.
    view_history_path = "/books/view-history/{book_id}" if content_type == "book" else "/view-history/{book_id}"

    @router.post(view_history_path)
    async def record_book_view(book_id: int, payload: dict = Depends(require_auth)) -> dict[str, Any]:
        """열람 뷰어 진입을 1건 기록한다.

        제목·카테고리는 클라이언트를 믿지 않고 서버가 자기 레코드에서 스냅샷을 뜬다.
        접근 검사를 먼저 하므로 viewer 가 볼 수 없는 책을 기록하거나 이 엔드포인트를
        존재 확인 수단으로 쓰는 것도 막힌다.
        """
        LOGGER.debug("# record_book_view(book_id=%d)", book_id)
        book, error = await _get_book_and_ensure_viewer_access(manager, book_id, payload, content_type)
        if book is None or error is not None:
            raise HTTPException(status_code=404, detail=error or "Book not found")
        try:
            view_history_store.record_view(email=payload.get("email", ""), content_type=content_type, book_id=book_id, title=book.title, category=book.category or "")
        except Exception as e:
            # 이력 기록 실패가 열람을 막아서는 안 된다. 실패는 로그로 드러내고 응답에도 표시한다.
            LOGGER.warning("조회 이력 기록 실패 (content_type=%s, book_id=%d): %s", content_type, book_id, e)
            return {"status": "success", "result": {"recorded": False}}
        return {"status": "success", "result": {"recorded": True}}

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
            # latest_excluded_categories에서 해당 카테고리 및 하위 카테고리 정리
            await asyncio.to_thread(category_mapping.set_latest_excluded, body.category, False, content_type=content_type)
            latest_excluded_list = await asyncio.to_thread(category_mapping.get_latest_excluded_categories, content_type=content_type)
            for excluded_cat in latest_excluded_list:
                if excluded_cat.startswith(cat_prefix):
                    await asyncio.to_thread(category_mapping.set_latest_excluded, excluded_cat, False, content_type=content_type)
            if not mapping_deleted:
                LOGGER.warning("delete_category: MySQL 키워드 매핑 삭제 대상 없음 (category='%s')", body.category)
            result["mapping_deleted"] = mapping_deleted
            response_object["status"] = "success"
            response_object["result"] = result
        else:
            response_object["error"] = error
        return response_object

    @router.post("/categories/classify-proposal", dependencies=admin_dep)
    async def start_classify_proposal(body: ClassifyProposalModel, background_tasks: BackgroundTasks) -> dict[str, Any]:
        """선택 카테고리의 분류 제안을 백그라운드로 만든다. 파일은 옮기지 않는다."""
        current = _read_classify_proposal()
        if current.get("status") in ("running", "applying"):
            return {"status": "success", "result": {"already_running": True, **current}}
        _start_classify_proposal(body.category)
        background_tasks.add_task(_run_classify_proposal_job, body.category, body.use_bookstore, body.use_content_meta, body.delay)
        return {"status": "success", "result": {"started": True, **_read_classify_proposal()}}

    @router.get("/categories/classify-proposal", dependencies=admin_dep)
    async def get_classify_proposal() -> dict[str, Any]:
        """진행 중이거나 마지막으로 만든 분류 제안 상태 조회 (폴링용).

        항목 목록(items)은 상태 파일이 아니라 DB(classify_proposal_items)에서 읽어
        합친다. 화면은 지금처럼 items 키를 그대로 읽으므로 응답 모양은 바뀌지 않는다.
        """
        status = _read_classify_proposal()
        items = await asyncio.to_thread(category_mapping.get_classify_proposal_items, content_type=content_type)
        return {"status": "success", "result": {**status, "items": items}}

    @router.post("/categories/classify-proposal/apply", dependencies=admin_dep)
    async def apply_classify_proposal(body: ClassifyApplyModel, background_tasks: BackgroundTasks) -> dict[str, Any]:
        """승인된 항목만 적용한다.

        허용 경로는 요청 본문(body.items)이 아니라 DB에 저장된 제안 항목(서버가 만든
        classify_proposal_items)에서만 가져온다. 브라우저가 보낸 file_path를 그대로
        허용 집합으로 쓰면 apply_category_changes가 가진 "제안 목록에 없는 파일은
        거부한다"는 검증이 통째로 무의미해져, 임의 경로를 옮기는 요청도 통과하게 된다.
        """
        current = _read_classify_proposal()
        # running/applying은 이미 다른 작업이 도는 중이라 거절한다. idle은 애초에 승인할
        # 제안이 없다. ready/done/failed는 모두 받는다 — 승인 도중 중단되면 하트비트
        # 만료로 failed까지 굳는데, 그때도 남은 pending 행을 다시 승인해 이어갈 수
        # 있어야 한다. 그러지 않으면 관리자는 파일이 반쯤 옮겨진 채로 아무것도 못 한다.
        if current.get("status") not in ("ready", "done", "failed"):
            return {"status": "failure", "error": "적용할 제안이 없습니다."}
        proposal_items = await asyncio.to_thread(category_mapping.get_classify_proposal_items, content_type=content_type)
        # 이미 옮겨진(moved) 행은 allowed에 들어가도 apply_category_changes가 파일 없음으로
        # 걸러 이중 이동은 안 일어나지만, 불필요하므로 애초에 pending 행만 담는다.
        allowed = {item.get("file_path") for item in proposal_items if item.get("file_path") and item.get("apply_status") == "pending"}
        items = [item.model_dump() for item in body.items]
        # 백그라운드 작업이 시작되기 전, 응답을 돌려주기 전에 동기적으로 applying을 선점한다.
        # await 지점 없이 여기까지 오므로 동시에 들어온 두 번째 apply 요청은 이 쓰기 뒤에야
        # 상태를 읽어 ready가 아님을 보고 거부된다. 선점을 뒤로 미루면(예: 백그라운드
        # 작업 안에서) 응답이 나간 뒤 실제로 상태가 바뀌기 전까지 창이 열려 있어, 그 사이
        # 두 번째 apply나 새 propose가 끼어들 수 있다.
        # 각 승인 작업에 고유 토큰을 발급한다. 하트비트 만료로 상태가 failed로 굳은 뒤
        # 원래 작업이 아직 살아있는 채로 두 번째 apply가 들어오면, 새 토큰을 쓴 이 작업이
        # 유일한 주인이 되고 먼저 돈 작업은 should_continue에서 토큰 불일치를 보고 멈춘다.
        apply_token = uuid.uuid4().hex
        _replace_classify_proposal({**current, "status": "applying", "apply_token": apply_token})
        background_tasks.add_task(_run_classify_apply_job, items, allowed, body.clean_existing, apply_token)
        return {"status": "success", "result": {"started": True, "total_count": len(items)}}

    @router.delete("/categories/classify-proposal", dependencies=admin_dep)
    async def delete_classify_proposal() -> dict[str, Any]:
        """제안 상태를 지워 화면을 초기 상태로 되돌린다. DB에 남은 제안 항목도 함께 지운다.

        지우지 않으면 폐기한 제안의 항목이 DB에 남아, 다음 GET이 idle 상태인데도
        옛 항목을 보여주는 모순이 생긴다.
        """
        await asyncio.to_thread(category_mapping.clear_classify_proposal_items, content_type=content_type)
        _replace_classify_proposal({"status": "idle", "content_type": content_type})
        return {"status": "success", "result": {"cleared": True}}

    @router.delete("/categories/classify-proposal/applied", dependencies=admin_dep)
    async def delete_applied_classify_proposal_items_route() -> dict[str, Any]:
        """이동이 끝난(apply_status = 'moved') 행만 지운다. 대기·실패 행은 남겨 재시도할 수 있게 한다.

        작업이 도는 중에 지우면, 건별 기록이 방금 찍은 행을 그 작업 밑에서 지울 수
        있으므로 running/applying 동안은 거절한다.
        """
        current = _read_classify_proposal()
        if current.get("status") in ("running", "applying"):
            return {"status": "failure", "error": "작업이 진행 중입니다."}
        deleted_count = await asyncio.to_thread(category_mapping.delete_applied_classify_proposal_items, content_type=content_type)
        return {"status": "success", "result": {"deleted_count": deleted_count}}

    @router.get("/categories/{category:path}")
    async def get_books_in_category(category: str, limit: int = 0, cursor: str = "", payload: dict = Depends(require_auth)) -> dict[str, Any]:
        LOGGER.debug("# get_books_in_category(category='%s', limit=%d, cursor='%s')", category, limit, cursor)
        response_object: dict[str, Any] = {"status": "failure"}
        await _ensure_viewer_category_allowed(payload, category, content_type)
        if limit > 0:
            # 커서 기반 페이지 조회: 10000건 상한 없이 카테고리 전체에 도달한다.
            paged, total, next_cursor, error = await manager.get_books_in_category_paged(category, size=limit, cursor=cursor)
            if error is None:
                response_object["status"] = "success"
                response_object["result"] = [BookModel(**book.dict()) for book in paged]
                response_object["total"] = total
                response_object["next_cursor"] = next_cursor
            else:
                response_object["error"] = error
            return response_object
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

    def _make_reload_progress_cb(latest_counts: dict[str, int]) -> Callable[[dict[str, int]], None]:
        """book_manager의 진행 보고를 메모리에만 누적한다.

        보고는 파일 1건마다 들어오므로 여기서 DB를 때리면 UPDATE가 파일 수만큼 나간다. 실제 flush는
        _flush_reload_progress_periodically가 고정 주기로 담당한다.
        """

        def _on_reload_progress(counts: dict[str, int]) -> None:
            latest_counts.update(counts)

        return _on_reload_progress

    async def _flush_reload_progress_periodically(category: str | None, latest_counts: dict[str, int]) -> None:
        """RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS마다 최신 진행 카운트를 재적재 락에 기록한다.

        진행 콜백이 전혀 없는 구간(시작 시 전체 불일치 스캔, 카테고리별 상세 조회, 마지막 ES
        refresh와 재스캔)에서도 heartbeat와 화면의 잔여 건수가 멈추지 않게 하는 것이 목적이다.
        카운트가 아직 없으면 빈 UPDATE가 나가 heartbeat(updated_at)만 갱신된다.
        """
        while True:
            await asyncio.sleep(RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS)
            try:
                await asyncio.to_thread(category_mapping.heartbeat_reload_lock, content_type, category, **dict(latest_counts))
            except Exception as e:
                # flush 실패로 재적재 자체를 중단시키지는 않는다 — 다음 주기에 다시 시도한다.
                LOGGER.warning("재적재 진행률 flush 실패 (category=%s): %s", category, e)

    @asynccontextmanager
    async def _reload_progress_reporter(category: str | None) -> AsyncIterator[Callable[[dict[str, int]], None]]:
        """진행 콜백과 주기적 flush 태스크를 함께 관리한다."""
        latest_counts: dict[str, int] = {}
        flusher = asyncio.create_task(_flush_reload_progress_periodically(category, latest_counts))
        try:
            yield _make_reload_progress_cb(latest_counts)
        finally:
            flusher.cancel()
            with suppress(asyncio.CancelledError):
                await flusher

    async def _run_reload_mismatch_files_job(category: str) -> None:
        try:
            async with _reload_progress_reporter(category) as on_progress:
                result, error = await manager.reload_category_mismatch_files(category, content_type=content_type, on_progress=on_progress)
        except Exception as e:
            LOGGER.error("reload_category_mismatch_files error: %s", e)
            await asyncio.to_thread(category_mapping.complete_reload_lock, content_type, "failed", GENERIC_MISMATCH_ERROR, category)
            return
        if error is None:
            await asyncio.to_thread(category_mapping.complete_reload_lock, content_type, "done", None, category, indexed_count=result["indexed_count"], deleted_count=result["deleted_count"], failed_count=result["failed_count"], before_count=result["before_count"], after_count=result["after_count"])
            LOGGER.info("reload_category_mismatch_files 응답: success — %s", result)
        else:
            await asyncio.to_thread(category_mapping.complete_reload_lock, content_type, "failed", error, category)
            LOGGER.error("reload_category_mismatch_files 응답: failure — %s", error)

    async def _run_reload_all_mismatches_job() -> None:
        try:
            async with _reload_progress_reporter(None) as on_progress:
                result, error = await manager.reload_category_mismatches(content_type=content_type, on_progress=on_progress)
        except Exception as e:
            LOGGER.error("reload_all_category_mismatches error: %s", e)
            await asyncio.to_thread(category_mapping.complete_reload_lock, content_type, "failed", GENERIC_MISMATCH_ERROR)
            return
        if error is None:
            await asyncio.to_thread(category_mapping.complete_reload_lock, content_type, "done", None, None, indexed_count=result["indexed_count"], deleted_count=result["deleted_count"], failed_count=result["failed_count"], before_count=result["before_count"], after_count=result["after_count"])
            LOGGER.info("reload_all_category_mismatches 응답: success — %s", result)
        else:
            await asyncio.to_thread(category_mapping.complete_reload_lock, content_type, "failed", error)
            LOGGER.error("reload_all_category_mismatches 응답: failure — %s", error)

    @router.post("/category-mismatches/reload-mismatches", dependencies=admin_dep)
    async def reload_category_mismatch_files(body: CategoryDeleteModel, background_tasks: BackgroundTasks) -> dict[str, Any]:
        """특정 카테고리의 현재 불일치 항목만 ES에 재적재/정리 (백그라운드 실행, 즉시 응답).

        카테고리별 락을 쓰므로, 다른 카테고리의 재적재와는 독립적으로 동시 진행된다. 일괄
        재적재가 이미 진행 중이면(모든 카테고리에 영향을 주므로) 대신 그 작업 상태에 연결된다.
        """
        LOGGER.info("reload_category_mismatch_files 요청: category='%s', content_type='%s'", body.category, content_type)
        response_object: dict[str, Any] = {"status": "failure"}
        acquired, lock_error, blocking_status = await asyncio.to_thread(category_mapping.acquire_reload_lock, content_type, body.category, "mismatch")
        if not acquired:
            response_object["status"] = "success"
            response_object["result"] = {"already_running": True, **(blocking_status or {})}
            LOGGER.info("reload_category_mismatch_files: 진행 중인 작업에 연결 — %s", lock_error)
            return response_object
        background_tasks.add_task(_run_reload_mismatch_files_job, body.category)
        response_object["status"] = "success"
        response_object["result"] = {"started": True, "content_type": content_type, "category": body.category, "reload_source": "mismatch"}
        return response_object

    @router.post("/category-mismatches/reload-all", dependencies=admin_dep)
    async def reload_all_category_mismatches(background_tasks: BackgroundTasks, body: ReloadAllMismatchesModel | None = None) -> dict[str, Any]:
        """현재 카테고리 불일치 항목을 일괄 ES 재적재/정리 (백그라운드 실행, 즉시 응답).

        모든 카테고리에 영향을 주므로, 카테고리별 재적재든 다른 일괄 재적재든 이 content_type에
        진행 중인 작업이 하나라도 있으면 획득할 수 없고 그 작업 상태에 연결된다.
        """
        reload_source = body.reload_source if body else "bulk"
        LOGGER.info("reload_all_category_mismatches 요청: content_type='%s', reload_source='%s'", content_type, reload_source)
        response_object: dict[str, Any] = {"status": "failure"}
        acquired, lock_error, blocking_status = await asyncio.to_thread(category_mapping.acquire_reload_lock, content_type, None, reload_source)
        if not acquired:
            response_object["status"] = "success"
            response_object["result"] = {"already_running": True, **(blocking_status or {})}
            LOGGER.info("reload_all_category_mismatches: 진행 중인 작업에 연결 — %s", lock_error)
            return response_object
        background_tasks.add_task(_run_reload_all_mismatches_job)
        response_object["status"] = "success"
        response_object["result"] = {"started": True, "content_type": content_type, "reload_source": reload_source}
        return response_object

    @router.get("/category-mismatches/reload-status", dependencies=admin_dep)
    async def get_reload_status(category: str | None = None) -> dict[str, Any]:
        """진행 중이거나 마지막으로 끝난 재적재 작업 상태 조회 (폴링용).

        category 생략 시 일괄/전체 재적재 락 상태를, 지정 시 그 카테고리 전용 락 상태를 본다.
        """
        status = await asyncio.to_thread(category_mapping.get_reload_status, content_type, category)
        return {"status": "success", "result": status or {"status": "idle"}}

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
async def verify_google_token(request: Request, request_body: dict):
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

    access_token, refresh_token = _issue_auth_tokens(email=email, role=role, name=name, picture=picture, client_ip=_client_ip(request), user_agent=_request_user_agent(request))

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
        rotation_status = refresh_token_store.rotate(current_token_id=current_token_id, new_token_id=new_token_id, family_id=family_id, email=email, issued_at=issued_at, expires_at=issued_at + REFRESH_TOKEN_EXPIRATION_SECONDS, client_ip=_client_ip(request), user_agent=_request_user_agent(request))
    except Exception as e:
        LOGGER.error("refresh_token_store.rotate() failed for %s: %s", email, e)
        raise HTTPException(status_code=503, detail="서비스를 일시적으로 사용할 수 없습니다. 잠시 후 다시 시도해 주세요.")
    if rotation_status != "ok":
        _log_refresh_rotation_rejected(request, status=rotation_status, email=email, family_id=family_id, token_id=current_token_id)
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


# === 로그인 세션 관리 API (admin 전용) ===

# family_id 는 uuid4().hex 이므로 32자 소문자 hex 다.
FAMILY_ID_LENGTH = 32
FAMILY_ID_CHARS = frozenset("0123456789abcdef")
SESSION_STATUS_FILTERS = ("active", "all")
MAX_SESSION_PAGE_SIZE = 100


def _is_family_id(value: str) -> bool:
    return len(value) == FAMILY_ID_LENGTH and all(c in FAMILY_ID_CHARS for c in value)


def _current_refresh_family_id(request: Request) -> str | None:
    """요청자의 refresh 쿠키에서 family_id 를 얻는다. 없거나 유효하지 않으면 None."""
    refresh_token = request.cookies.get(REFRESH_COOKIE_NAME, "")
    if not refresh_token:
        return None
    try:
        return decode_refresh_token(refresh_token).get("fid")
    except HTTPException:
        return None


@app.get("/auth/sessions", dependencies=[Depends(require_admin)])
async def list_login_sessions(request: Request, page: int = Query(1, ge=1), pageSize: int = Query(50, ge=1, le=MAX_SESSION_PAGE_SIZE), status: str = "active", email: str | None = None):
    """서버 측 refresh 세션(family) 목록을 조회한다. jti·토큰 값은 노출하지 않는다."""
    if status not in SESSION_STATUS_FILTERS:
        raise HTTPException(status_code=400, detail=f"status must be one of: {', '.join(SESSION_STATUS_FILTERS)}")
    try:
        result = refresh_token_store.list_sessions(status=status, page=page, page_size=pageSize, email=email, current_family_id=_current_refresh_family_id(request))
    except Exception as e:
        LOGGER.error("refresh_token_store.list_sessions() failed: %s", e)
        raise HTTPException(status_code=503, detail="서비스를 일시적으로 사용할 수 없습니다. 잠시 후 다시 시도해 주세요.")
    return {"status": "success", "result": result}


@app.delete("/auth/sessions/{session_id}")
async def revoke_login_session(session_id: str, request: Request, admin: dict = Depends(require_admin)):
    """세션(family) 하나를 폐기한다. 되돌릴 수 없으므로 단일 family 로만 범위를 제한한다."""
    if not _is_family_id(session_id):
        raise HTTPException(status_code=400, detail="Invalid session id")
    try:
        outcome = refresh_token_store.revoke_session(family_id=session_id)
    except Exception as e:
        LOGGER.error("refresh_token_store.revoke_session() failed: %s", e)
        raise HTTPException(status_code=503, detail="서비스를 일시적으로 사용할 수 없습니다. 잠시 후 다시 시도해 주세요.")
    if not outcome["found"]:
        raise HTTPException(status_code=404, detail="Session not found")

    revoked_current = _current_refresh_family_id(request) == session_id
    # 관리자 행위 감사 로그. 대상 세션은 해시로만 남긴다.
    LOGGER.warning("Admin %s revoked login session session_hash=%s revoked_current=%s", admin.get("email", ""), observation_hash(session_id), revoked_current)

    session = outcome["session"] or {}
    response = JSONResponse({"status": "success", "result": {"session_id": session_id, "revoked": outcome["revoked"], "revoked_current": revoked_current, "status": session.get("status", "revoked"), "revoke_reason": session.get("revoke_reason")}})
    if revoked_current:
        # 본인 세션을 폐기했으면 쿠키도 정리해 프런트가 미인증 흐름으로 넘어가게 한다.
        _clear_auth_cookies(response)
    return response


# === 사용자별 조회 이력 API (admin 전용) ===


@app.get("/view-history", dependencies=[Depends(require_admin)])
async def list_view_history(limit: int = Query(MAX_RECENT_VIEWS, ge=1, le=MAX_RECENT_VIEWS)):
    """사용자별 최근 조회 목록(책/만화 각각)을 돌려준다."""
    try:
        result = view_history_store.list_recent_views(limit=limit)
    except Exception as e:
        LOGGER.error("view_history_store.list_recent_views() failed: %s", e)
        raise HTTPException(status_code=503, detail="서비스를 일시적으로 사용할 수 없습니다. 잠시 후 다시 시도해 주세요.")
    return {"status": "success", "result": result}


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


class LatestExcludedCategoryModel(BaseModel):
    excluded: bool


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


@app.get("/latest-excluded-categories")
async def get_latest_excluded_categories(payload: dict = Depends(require_auth), content_type: str = "book") -> dict[str, Any]:
    """최신 자료 검색 제외 카테고리 목록 조회"""
    LOGGER.debug("# get_latest_excluded_categories(content_type=%s)", content_type)
    try:
        if payload.get("role") == "viewer":
            return {"status": "success", "result": []}
        categories = await _get_latest_excluded_categories(content_type)
        return {"status": "success", "result": categories}
    except Exception as e:
        LOGGER.error("get_latest_excluded_categories error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_LATEST_EXCLUDED_CATEGORY_ERROR_DETAIL)


@app.post("/latest-excluded-categories/{category:path}", dependencies=[Depends(require_admin)])
async def set_latest_excluded_category(category: str, body: LatestExcludedCategoryModel, content_type: str = "book") -> dict[str, Any]:
    """카테고리 최신 자료 검색 제외 설정/해제"""
    LOGGER.debug("# set_latest_excluded_category(category='%s', excluded=%s, content_type=%s)", category, body.excluded, content_type)
    try:
        success = await asyncio.to_thread(category_mapping.set_latest_excluded, category, body.excluded, content_type=content_type)
        if success:
            return {"status": "success", "result": await _get_latest_excluded_categories(content_type)}
        else:
            raise HTTPException(status_code=500, detail="Failed to update latest excluded category")
    except HTTPException:
        raise
    except Exception as e:
        LOGGER.error("set_latest_excluded_category error: %s", e)
        raise HTTPException(status_code=500, detail=GENERIC_LATEST_EXCLUDED_CATEGORY_ERROR_DETAIL)


class ClientErrorLogModel(BaseModel):
    error_type: Literal["REACT_RENDER_ERROR", "WINDOW_ERROR", "UNHANDLED_PROMISE", "CUSTOM_ERROR"]
    message: str
    stack: str | None = None
    component_stack: str | None = None
    url: str
    user_agent: str | None = None
    timestamp: str | None = None


@app.post("/logs/client-error")
async def log_client_error(body: ClientErrorLogModel, auth_user: dict | None = Depends(optional_auth)) -> dict[str, str]:
    """프론트엔드 런타임/렌더링 에러 로그 수집 및 기록"""
    email = auth_user.get("email") if auth_user else "anonymous"
    role = auth_user.get("role") if auth_user else "anonymous"

    LOGGER.error("[CLIENT_ERROR] type=%s, user=%s(%s), url=%s, message=%s", body.error_type, email, role, body.url, body.message)
    if body.component_stack:
        LOGGER.error("[CLIENT_ERROR] Component Stack:\n%s", body.component_stack.strip())
    if body.stack:
        LOGGER.error("[CLIENT_ERROR] Stack Trace:\n%s", body.stack.strip())

    return {"status": "ok"}
