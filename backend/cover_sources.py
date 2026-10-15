"""표지 전용 서점 검색. 이미지 bytes는 별도 요청에서만 내려받는다."""

import fcntl
import ipaddress
import json
import math
import socket
import time
import uuid
from contextlib import contextmanager
from contextvars import ContextVar
from pathlib import Path
from typing import Any, Callable, Iterator, cast
from urllib.parse import urlencode, urljoin, urlparse

import requests
from bs4 import BeautifulSoup

from backend.bookstore import (
    Yes24Bookstore, AladinBookstore, KyoboBookstore, NaverShoppingBookstore,
    RidibooksBookstore, MunpiaBookstore, NaverSeriesBookstore, JoaraBookstore,
    AbstractBookstore,
)

STORES: dict[str, Callable[..., AbstractBookstore]] = {
    "yes24": Yes24Bookstore, "aladin": AladinBookstore, "kyobo": KyoboBookstore,
    "naver": NaverShoppingBookstore, "ridi": RidibooksBookstore,
    "munpia": MunpiaBookstore, "naverseries": NaverSeriesBookstore, "joara": JoaraBookstore,
}
# 실제 서점 페이지/API에서 확인한 origin과 CDN만 허용한다.
STORE_HOSTS = {
    "yes24": ("yes24.com",), "aladin": ("aladin.co.kr",), "kyobo": ("kyobobook.co.kr",),
    "naver": ("shopping.naver.com", "pstatic.net"),
    "ridi": ("ridibooks.com", "ridicdn.net"),
    "munpia": ("munpia.com",), "naverseries": ("series.naver.com", "pstatic.net"),
    "joara": ("joara.com",),
}
request_cancelled: ContextVar[Callable[[], bool]] = ContextVar("cover_request_cancelled", default=lambda: False)


class FetchCancelled(Exception):
    """화면에서 더 이상 필요하지 않은 서점 요청."""


def check_cancelled() -> None:
    if request_cancelled.get()():
        raise FetchCancelled()


@contextmanager
def store_lock(cache_dir: Path, store: str) -> Iterator[None]:
    if store not in STORES:
        raise ValueError("지원하지 않는 서점입니다")
    cache_dir.mkdir(parents=True, exist_ok=True)
    # flock은 같은 호스트의 thread/process뿐 아니라 공유 볼륨 replica도 직렬화한다.
    with (cache_dir / f"{store}.lock").open("a+b") as lock:
        deadline = time.monotonic() + 120
        while True:
            check_cancelled()
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() > deadline:
                    raise TimeoutError("서점 요청이 밀려 있습니다. 다시 시도하세요")
                time.sleep(0.05)
        try:
            check_cancelled()
            yield
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


# 서점이 짧은 시간의 연속 요청을 418/429로 막는 경우가 있다(네이버는 4~5건 뒤).
# 요청 간격을 두고, 막히면 일정 시간 요청을 멈춘 뒤 한 건만 시험한다.
STORE_MIN_INTERVAL = {"naver": 2.0}
BLOCK_STATUSES = (418, 429)
BLOCK_BACKOFF_START = 60
BLOCK_BACKOFF_MAX = 300

_now = time.time
_sleep = time.sleep


class StoreBlocked(Exception):
    """서점이 요청을 제한 중이라 retry_after초 동안 요청하지 않는다."""

    def __init__(self, retry_after: int):
        super().__init__(f"store blocked for {retry_after}s")
        self.retry_after = retry_after


def blocked_status(exc: BaseException) -> bool:
    response = exc.response if isinstance(exc, requests.HTTPError) else None
    return response is not None and response.status_code in BLOCK_STATUSES


class StoreGate:
    """서점별 요청 간격과 차단 backoff. store_lock 안에서만 쓴다.

    상태를 cache_dir의 파일에 두어 공유 볼륨의 replica끼리 같은 차단 상태를 본다.
    """

    def __init__(self, cache_dir: Path, store: str):
        self.store = store
        self.path = cache_dir / f"{store}.gate.json"

    def _load(self) -> dict[str, Any]:
        try:
            value = json.loads(self.path.read_text())
            return value if isinstance(value, dict) else {}
        except (OSError, ValueError):
            return {}

    def _save(self, state: dict[str, Any]) -> None:
        try:
            self.path.write_text(json.dumps(state))
        except OSError:
            pass  # 상태를 못 남겨도 검색 자체는 계속한다.

    def before_request(self) -> None:
        state = self._load()
        remaining = float(state.get("blocked_until", 0)) - _now()
        if remaining > 0:
            raise StoreBlocked(math.ceil(remaining))
        interval = STORE_MIN_INTERVAL.get(self.store, 0)
        while True:
            wait = float(state.get("last_request", 0)) + interval - _now()
            if wait <= 0:
                break
            check_cancelled()
            _sleep(min(wait, 0.2))
        state["last_request"] = _now()
        self._save(state)

    def record_success(self) -> None:
        state = self._load()
        state.pop("blocked_until", None)
        state.pop("backoff", None)
        self._save(state)

    def record_blocked(self) -> int:
        state = self._load()
        previous = int(state.get("backoff", 0))
        backoff = min(previous * 2, BLOCK_BACKOFF_MAX) if previous else BLOCK_BACKOFF_START
        state["backoff"] = backoff
        state["blocked_until"] = _now() + backoff
        self._save(state)
        return backoff


def validate_url(store: str, url: str) -> str:
    parsed = urlparse(url)
    host = (parsed.hostname or "").lower()
    if parsed.scheme != "https" or parsed.username or parsed.password or parsed.port not in (None, 443):
        raise ValueError("허용하지 않는 표지 URL입니다")
    if not any(host == domain or host.endswith("." + domain) for domain in STORE_HOSTS.get(store, ())):
        raise ValueError("허용하지 않는 서점 host입니다")
    addresses = socket.getaddrinfo(host, 443, type=socket.SOCK_STREAM)
    if not addresses or any(not ipaddress.ip_address(address[4][0]).is_global for address in addresses):
        raise ValueError("서점 host의 주소를 허용하지 않습니다")
    return url


def fetch_bytes(store: str, url: str, session: requests.Session, max_bytes: int = 4 * 1024 * 1024) -> bytes:
    for _ in range(5):
        check_cancelled()
        validate_url(store, url)
        with session.get(url, timeout=(5, 15), stream=True, allow_redirects=False) as response:
            if response.is_redirect:
                url = urljoin(url, response.headers.get("Location", ""))
                continue
            response.raise_for_status()
            output = bytearray()
            for chunk in response.iter_content(65536):
                check_cancelled()
                output.extend(chunk)
                if len(output) > max_bytes:
                    raise ValueError("서점 응답이 크기 제한을 초과했습니다")
            return bytes(output)
    raise ValueError("서점 redirect 횟수를 초과했습니다")


def extract_html_candidates(store: str, detail_url: str, html: str) -> list[dict[str, Any]]:
    soup = BeautifulSoup(html, "html.parser")
    client = STORES[store](verbose=False)
    try:
        info = client.extract_book_info(soup)
    finally:
        client.session.close()
    title_meta = soup.select_one('meta[property="og:title"]')
    title = info.get("title") or (title_meta.get("content", "") if title_meta else "")
    cover = soup.select_one('meta[property="og:image"]')
    image_url = str(cover.get("content", "")) if cover else ""
    if store == "aladin" and urlparse(image_url).path.endswith("/img_no.jpg"):
        return []
    if store == "naverseries":
        # og:image는 600x314 공유용 crop이다. 실제 책의 173x260 이미지를 사용한다.
        portrait = soup.select_one('a.pic_area img')
        if portrait:
            image_url = str(portrait.get("src", ""))
    if not title or not image_url:
        return []
    return [{"title": title, "author": info.get("author", ""), "detail_url": detail_url,
             "image_url": urljoin(detail_url, image_url)}]


def parse_naver_candidates(html: str) -> list[dict[str, Any]]:
    # 2026-10 페이지는 Next flight와 bookListItem DOM을 사용한다.
    soup = BeautifulSoup(html, "html.parser")
    candidates = []
    for link in soup.select('a[class*="bookListItem_info_top"]'):
        image = link.find("img")
        if image is None or not image.get("src"):
            continue
        author = ""
        for field in link.select('[class*="bookListItem_define_item"]'):
            fields = field.find_all("span")
            if len(fields) >= 2 and fields[0].get_text(strip=True) == "저자":
                author = fields[1].get_text(strip=True)
        candidates.append({"title": image.get("alt", ""), "author": author,
                           "detail_url": link.get("href", ""), "image_url": image["src"]})
        if len(candidates) == 4:
            break
    return candidates


def parse_joara_candidates(data: dict[str, Any]) -> list[dict[str, Any]]:
    if data.get("status") != 1:
        raise ValueError("조아라 검색 요청을 처리하지 못했습니다")
    results = []
    for book in ((data.get("data") or {}).get("list") or [])[:4]:
        if book.get("cover") and book.get("book_code"):
            results.append({"title": book.get("subject", ""), "author": book.get("member_name", ""),
                            "detail_url": f"https://www.joara.com/book/{book['book_code']}", "image_url": book["cover"]})
    return results


def parse_munpia_candidates(data: dict[str, Any]) -> list[dict[str, Any]]:
    if data.get("code") != "M000_00000":
        raise ValueError("문피아 검색 요청을 처리하지 못했습니다")
    results = []
    for book in ((data.get("result") or {}).get("searchNovelTabDtos") or [])[:4]:
        if book.get("coverUrl") and book.get("novelId"):
            results.append({"title": book.get("title", ""), "author": book.get("author", ""),
                            "detail_url": f"https://www.munpia.com/novel/detail/{book['novelId']}", "image_url": book["coverUrl"]})
    return results


def search_candidates(store: str, metadata: dict[str, Any]) -> dict[str, Any]:
    if store not in STORES:
        raise ValueError("지원하지 않는 서점입니다")
    client = STORES[store](verbose=False)
    if store == "ridi":
        # curl UA의 HTML challenge 대신 공개 웹 client와 호환되는 요청 header를 사용한다.
        client.session.headers.update({
            "User-Agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36",
            "Referer": "https://ridibooks.com/", "Accept": "application/json,text/html",
            "Accept-Language": "ko-KR,ko;q=0.9",
        })
    keyword = metadata.get("isbn") if client.SUPPORTS_ISBN_SEARCH else ""
    keyword = keyword or metadata["title"]
    if store == "aladin":
        search_url = AladinBookstore.BASE_URL + "/search/wsearchresult.aspx?" + urlencode({"SearchTarget": "Book", "SearchWord": keyword})
    else:
        search_url = client.build_search_url(keyword)
    try:
        if store == "joara":
            joara = cast(JoaraBookstore, client)

            params = {"api_key": joara.API_KEY, "device": "mw", "devicetoken": "mw",
                      "deviceuid": uuid.uuid4().hex, "query": metadata["title"], "target": "subject",
                      "page": "1", "offset": "4", "store": "all", "orderby": "score"}
            raw = fetch_bytes(store, joara.API_URL + "?" + urlencode(params), client.session)
            results = parse_joara_candidates(json.loads(raw))
        elif store == "munpia":
            # 공식 PC client(2026-09)의 MAIN_SEARCH API와 coverUrl 필드를 사용한다.
            search_url = "https://www.munpia.com/search?" + urlencode({"query": metadata["title"]})
            params = {"query": metadata["title"], "page": "0", "size": "4"}
            raw = fetch_bytes(store, "https://www.munpia.com/api/v1/main/search?" + urlencode(params), client.session)
            results = parse_munpia_candidates(json.loads(raw))
        else:
            html = fetch_bytes(store, search_url, client.session).decode("utf-8", errors="replace")
            soup = BeautifulSoup(html, "html.parser")
            if store == "naver":
                results = parse_naver_candidates(html)
            else:
                links = client.extract_search_links(soup)
                if store == "ridi" and not links:
                    links = list(dict.fromkeys(urljoin(client.BASE_URL, str(a["href"])) for a in soup.select('a[href^="/books/"]')))
                results = []
                # 기존 AbstractBookstore의 ThreadPoolExecutor를 사용하지 않는다.
                for detail_url in links[:4]:
                    detail = fetch_bytes(store, detail_url, client.session).decode("utf-8", errors="replace")
                    results.extend(extract_html_candidates(store, detail_url, detail))
            if not results and ("Just a moment" in html or "검색" not in soup.get_text()):
                raise ValueError("서점 검색 페이지에 접근하지 못했습니다")
        return {"status": "success" if results else "not_found", "search_url": search_url,
                "candidates": results[:4]}
    finally:
        client.session.close()
