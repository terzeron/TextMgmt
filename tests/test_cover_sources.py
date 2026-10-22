import io
import json
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import Mock, MagicMock

import pytest


def test_html_candidates_use_real_page_images():
    from backend.cover_sources import extract_html_candidates

    for store in ("yes24", "aladin", "kyobo", "ridi", "munpia", "naverseries"):
        candidates = extract_html_candidates(store, 'https://example.invalid/book', '<html><meta property="og:title" content="책"><meta property="og:image" content="https://image.yes24.com/real.jpg"></html>')
        assert candidates[0]["image_url"] == "https://image.yes24.com/real.jpg"
        assert candidates[0]["title"]


def test_naver_and_joara_search_candidates():
    from backend.cover_sources import parse_naver_candidates, parse_joara_candidates

    html = '<a class="bookListItem_info_top__abc" href="https://cr3.shopping.naver.com/book/adcr?nvMid=123"><img alt="데미안" src="https://shopping-phinf.pstatic.net/book.jpg"><div class="bookListItem_define_item__abc"><span>저자</span><span>헤세</span></div></a>'
    assert parse_naver_candidates(html)[0]["author"] == "헤세"
    data = {"status": 1, "data": {"list": [{"book_code": "123", "subject": "책", "member_name": "저자", "cover": "https://cf-image.joara.com/book.png"}]}}
    assert parse_joara_candidates(data)[0]["image_url"] == "https://cf-image.joara.com/book.png"


@pytest.mark.parametrize("title,keyword", [("불멸과 소멸 1 이상문", "불멸과 소멸 1 이상문"), ("우리말의 상상력 1 어휘의 기원", "우리말의 상상력 1 어휘의 기원")])
def test_aladin_covers_use_clean_book_keyword_instead_of_all_products(monkeypatch, title, keyword):
    from urllib.parse import parse_qs, urlparse
    from backend import cover_sources as module
    seen = []
    def fetch(store, url, session):
        seen.append(url)
        return '<div id="Search3_Result">검색 결과 없음</div>'.encode()
    monkeypatch.setattr(module, "fetch_bytes", fetch)
    result = module.search_candidates("aladin", {"title": title})
    query = parse_qs(urlparse(seen[0]).query)
    assert query["SearchTarget"] == ["Book"]
    assert query["SearchWord"] == [keyword]
    assert result["status"] == "not_found"


def test_aladin_cover_search_keeps_isbn(monkeypatch):
    from urllib.parse import parse_qs,urlparse
    from backend import cover_sources as module
    seen = []
    monkeypatch.setattr(module, "fetch_bytes", lambda store,url,session: seen.append(url) or '검색 결과 없음'.encode())
    module.search_candidates("aladin", {"title": "제목 저자 1", "isbn": "9781234567890"})
    assert parse_qs(urlparse(seen[0]).query)["SearchWord"] == ["9781234567890"]


@pytest.mark.parametrize("store", ["yes24", "aladin", "kyobo"])
@pytest.mark.parametrize("isbn", ["9781234567890", ""])
def test_isbn_capable_stores_preserve_priority_and_filename_title_fallback(monkeypatch, store, isbn):
    from urllib.parse import parse_qs, urlparse
    from backend import cover_sources as module
    seen = []
    monkeypatch.setattr(module, "fetch_bytes", lambda _, url, session: seen.append(url) or "검색 결과 없음".encode())
    result = module.search_candidates(store, {"title": "랜들 개릿 다아시 경의 모험 1", "isbn": isbn})
    keyword = isbn or "랜들 개릿 다아시 경의 모험 1"
    assert [keyword] in parse_qs(urlparse(seen[0]).query).values()
    assert result["search_url"] == seen[0]


def test_aladin_common_no_image_placeholder_is_not_a_cover():
    from backend.cover_sources import extract_html_candidates
    html = '<meta property="og:title" content="제목"><meta property="og:image" content="https://image.aladin.co.kr/img/shop/2018/img_no.jpg">'
    assert extract_html_candidates("aladin", "https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=1", html) == []


def test_munpia_current_json_search():
    from backend.cover_sources import parse_munpia_candidates

    data = {"code": "M000_00000", "result": {"searchNovelTabDtos": [{"novelId": 104753, "title": "전지적 독자 시점", "author": "싱숑", "coverUrl": "https://cdn1.munpia.com/cover.jpg"}]}}
    result = parse_munpia_candidates(data)
    assert result[0]["image_url"] == "https://cdn1.munpia.com/cover.jpg"
    assert result[0]["detail_url"] == "https://www.munpia.com/novel/detail/104753"


def test_store_locks_serialize_same_store_and_allow_different_stores(tmp_path):
    from backend.cover_sources import store_lock

    active = {}
    maximum = {}
    total = 0
    max_total = 0
    state_lock = threading.Lock()

    def fetch(store):
        nonlocal total, max_total
        with store_lock(tmp_path, store):
            with state_lock:
                active[store] = active.get(store, 0) + 1
                maximum[store] = max(maximum.get(store, 0), active[store])
                total += 1
                max_total = max(max_total, total)
            time.sleep(0.03)
            with state_lock:
                active[store] -= 1
                total -= 1

    with ThreadPoolExecutor(max_workers=6) as pool:
        list(pool.map(fetch, ["yes24", "yes24", "aladin", "aladin", "kyobo", "kyobo"]))
    assert all(count == 1 for count in maximum.values())
    assert max_total > 1


@pytest.mark.parametrize("url", ["http://image.yes24.com/a", "https://localhost/a", "https://image.yes24.com.evil.invalid/a", "https://image.yes24.com@127.0.0.1/a", "https://image.yes24.com:8443/a"])
def test_download_rejects_unapproved_urls(url, monkeypatch):
    from backend.cover_sources import validate_url

    with pytest.raises(ValueError):
        validate_url("yes24", url)


def test_download_rejects_private_dns(monkeypatch):
    from backend import cover_sources

    monkeypatch.setattr(cover_sources.socket, "getaddrinfo", lambda *args, **kwargs: [(2, 1, 6, "", ("127.0.0.1", 443))])
    with pytest.raises(ValueError):
        cover_sources.validate_url("yes24", "https://image.yes24.com/a")


@pytest.mark.parametrize("empty", [False, True])
def test_public_dns_and_empty_dns(monkeypatch, empty):
    from backend import cover_sources as module
    monkeypatch.setattr(module.socket, "getaddrinfo", lambda *args, **kwargs: [] if empty else [(2, 1, 6, "", ("8.8.8.8", 443))])
    url = "https://image.yes24.com/a"
    if empty:
        with pytest.raises(ValueError):
            module.validate_url("yes24", url)
    else:
        assert module.validate_url("yes24", url) == url


def test_cancelled_and_timed_out_store_locks(tmp_path, monkeypatch):
    from backend import cover_sources as module
    with pytest.raises(ValueError):
        with module.store_lock(tmp_path, "unknown"):
            pass
    token = module.request_cancelled.set(lambda: True)
    try:
        with pytest.raises(module.FetchCancelled):
            with module.store_lock(tmp_path, "yes24"):
                pass
    finally:
        module.request_cancelled.reset(token)
    monkeypatch.setattr(module.fcntl, "flock", Mock(side_effect=BlockingIOError))
    times = iter([0, 121])
    monkeypatch.setattr(module.time, "monotonic", lambda: next(times))
    with pytest.raises(TimeoutError):
        with module.store_lock(tmp_path, "yes24"):
            pass


@pytest.mark.parametrize("mode", ["success", "too_large", "redirect_limit"])
def test_fetch_bytes_redirects_and_size_limit(monkeypatch, mode):
    from backend import cover_sources as module
    monkeypatch.setattr(module, "validate_url", lambda store, url: url)
    first = MagicMock()
    first.__enter__.return_value = first
    first.is_redirect = True
    first.headers = {"Location": "/image.jpg"}
    final = MagicMock()
    final.__enter__.return_value = final
    final.is_redirect = False
    final.iter_content.return_value = [b"im", b"age"]
    session = Mock()
    session.get.side_effect = [first] * 5 if mode == "redirect_limit" else [first, final]
    if mode == "success":
        assert module.fetch_bytes("yes24", "https://image.yes24.com/start", session) == b"image"
        assert session.get.call_args.args[0] == "https://image.yes24.com/image.jpg"
        assert final.__exit__.called
    else:
        with pytest.raises(ValueError):
            module.fetch_bytes("yes24", "https://image.yes24.com/start", session, max_bytes=3)


def test_parsers_handle_missing_fields_invalid_responses_and_limits():
    from backend import cover_sources as module
    assert module.extract_html_candidates("yes24", "https://www.yes24.com/", "<html></html>") == []
    portrait = module.extract_html_candidates("naverseries", "https://series.naver.com/a", '<meta property="og:title" content="책"><meta property="og:image" content="wide.jpg"><a class="pic_area"><img src="portrait.jpg"></a>')
    assert portrait[0]["image_url"].endswith("portrait.jpg")
    html = '<a class="bookListItem_info_top"><img></a><a class="bookListItem_info_top"></a>'
    html += '<a class="bookListItem_info_top"><img alt="책" src="a.jpg"><div class="bookListItem_define_item"><span>출판사</span><span>출판</span></div><div class="bookListItem_define_item"><span>저자</span></div></a>' * 5
    assert len(module.parse_naver_candidates(html)) == 4
    with pytest.raises(ValueError):
        module.parse_joara_candidates({})
    with pytest.raises(ValueError):
        module.parse_munpia_candidates({})
    assert module.parse_joara_candidates({"status": 1}) == []
    assert module.parse_munpia_candidates({"code": "M000_00000"}) == []
    assert module.parse_joara_candidates({"status": 1, "data": {"list": [{"cover": "a"}, {"book_code": "1"}]}}) == []
    assert module.parse_munpia_candidates({"code": "M000_00000", "result": {"searchNovelTabDtos": [{"coverUrl": "a"}, {"novelId": 1}]}}) == []


@pytest.mark.parametrize("store", ["yes24", "aladin", "kyobo", "naver", "ridi", "munpia", "naverseries", "joara"])
def test_all_store_search_flows_with_local_fixtures(monkeypatch, store):
    from backend import cover_sources as module
    client = Mock()
    client.session.headers = {}
    client.SUPPORTS_ISBN_SEARCH = store == "yes24"
    client.API_KEY, client.API_URL, client.BASE_URL = "test-key", "https://api.joara.com/search", "https://ridibooks.com"
    client.build_search_url.return_value = "https://example.org/search"
    client.extract_search_links.return_value = [] if store == "ridi" else ["https://example.org/detail"]
    client.extract_book_info.return_value = {"title": "책", "author": "저자"}
    monkeypatch.setitem(module.STORES, store, lambda **kwargs: client)
    requests = []
    def fetch(_, url, session):
        requests.append(url)
        if store == "joara":
            return json.dumps({"status": 1, "data": {"list": [{"book_code": "1", "cover": "https://image.example.org/a", "subject": "책"}]}}).encode()
        if store == "munpia":
            return json.dumps({"code": "M000_00000", "result": {"searchNovelTabDtos": [{"novelId": 1, "coverUrl": "https://image.example.org/a", "title": "책"}]}}).encode()
        if store == "naver":
            return '<a class="bookListItem_info_top"><img alt="책" src="https://image.example.org/a"></a>'.encode()
        return '<html><a href="/books/1">책</a><meta property="og:image" content="https://image.example.org/a">검색</html>'.encode()
    monkeypatch.setattr(module, "fetch_bytes", fetch)
    result = module.search_candidates(store, {"title": "검색 제목", "isbn": "1234567890"})
    assert result["status"] == "success"
    assert result["candidates"][0]["title"] == "책"
    assert client.session.close.called
    if store != "aladin":
        client.build_search_url.assert_called_with("1234567890" if store == "yes24" else "검색 제목")
    if store in ("joara", "munpia"):
        from urllib.parse import parse_qs, urlparse
        assert parse_qs(urlparse(requests[0]).query)["query"] == ["검색 제목"]
    if store == "ridi":
        assert client.session.headers["Referer"] == "https://ridibooks.com/"


@pytest.mark.parametrize("html,status", [("검색 결과 없음", "not_found"), ("Just a moment", "error"), ("blocked", "error")])
def test_empty_and_blocked_search_closes_session(monkeypatch, html, status):
    from backend import cover_sources as module
    client = Mock()
    client.SUPPORTS_ISBN_SEARCH = False
    client.extract_search_links.return_value = []
    client.build_search_url.return_value = "https://www.yes24.com/search"
    monkeypatch.setitem(module.STORES, "yes24", lambda **kwargs: client)
    monkeypatch.setattr(module, "fetch_bytes", lambda *args: html.encode())
    if status == "error":
        with pytest.raises(ValueError):
            module.search_candidates("yes24", {"title": "책"})
    else:
        assert module.search_candidates("yes24", {"title": "책"})["status"] == "not_found"
    assert client.session.close.called
    with pytest.raises(ValueError):
        module.search_candidates("unknown", {"title": "책"})


def http_error(status):
    import requests

    response = requests.Response()
    response.status_code = status
    return requests.HTTPError(response=response)


def fake_clock(monkeypatch, start=1000.0):
    from backend import cover_sources as module

    clock = {"now": start, "slept": []}
    monkeypatch.setattr(module, "_now", lambda: clock["now"])

    def sleep(seconds):
        clock["slept"].append(seconds)
        clock["now"] += seconds

    monkeypatch.setattr(module, "_sleep", sleep)
    return clock


@pytest.mark.parametrize("status,blocked", [(418, True), (429, True), (500, False), (404, False)])
def test_only_rate_limit_statuses_count_as_store_block(status, blocked):
    from backend.cover_sources import blocked_status

    assert blocked_status(http_error(status)) is blocked
    assert blocked_status(ValueError("418")) is False


def test_store_gate_backoff_doubles_up_to_cap_and_resets_after_success(tmp_path, monkeypatch):
    from backend.cover_sources import StoreBlocked, StoreGate

    clock = fake_clock(monkeypatch)
    gate = StoreGate(tmp_path, "naver")
    gate.before_request()
    assert gate.record_blocked() == 60
    with pytest.raises(StoreBlocked) as blocked:
        gate.before_request()
    assert blocked.value.retry_after == 60
    clock["now"] += 30
    with pytest.raises(StoreBlocked) as blocked:
        gate.before_request()
    assert blocked.value.retry_after == 30
    expected = [120, 240, 300, 300]
    for wait in expected:
        clock["now"] += 301
        gate.before_request()  # 시험 요청
        assert gate.record_blocked() == wait
    clock["now"] += 301
    gate.before_request()
    gate.record_success()
    gate.before_request()
    assert gate.record_blocked() == 60


def test_store_gate_state_is_shared_through_cache_dir(tmp_path, monkeypatch):
    from backend.cover_sources import StoreBlocked, StoreGate

    fake_clock(monkeypatch)
    StoreGate(tmp_path, "naver").record_blocked()
    with pytest.raises(StoreBlocked):
        StoreGate(tmp_path, "naver").before_request()
    StoreGate(tmp_path, "yes24").before_request()


def test_store_gate_spaces_naver_requests_but_not_other_stores(tmp_path, monkeypatch):
    from backend.cover_sources import StoreGate

    clock = fake_clock(monkeypatch)
    naver = StoreGate(tmp_path, "naver")
    naver.before_request()
    assert clock["slept"] == []
    naver.before_request()
    assert sum(clock["slept"]) == pytest.approx(2.0)
    clock["slept"].clear()
    yes24 = StoreGate(tmp_path, "yes24")
    yes24.before_request()
    yes24.before_request()
    assert clock["slept"] == []


def test_store_gate_wait_can_be_cancelled(tmp_path, monkeypatch):
    from backend.cover_sources import FetchCancelled, StoreGate, request_cancelled

    fake_clock(monkeypatch)
    gate = StoreGate(tmp_path, "naver")
    gate.before_request()
    token = request_cancelled.set(lambda: True)
    try:
        with pytest.raises(FetchCancelled):
            gate.before_request()
    finally:
        request_cancelled.reset(token)


def test_store_gate_ignores_corrupt_state_file(tmp_path, monkeypatch):
    from backend.cover_sources import StoreGate

    fake_clock(monkeypatch)
    (tmp_path / "naver.gate.json").write_text("{not json")
    gate = StoreGate(tmp_path, "naver")
    gate.before_request()
    assert gate.record_blocked() == 60


def test_store_gate_keeps_working_when_state_cannot_be_saved(tmp_path, monkeypatch):
    from backend.cover_sources import StoreGate

    gate = StoreGate(tmp_path, "naver")

    def fail(*args, **kwargs):
        raise OSError("read-only")

    monkeypatch.setattr(type(gate.path), "write_text", fail)
    assert gate.record_blocked() == 60
    assert not gate.path.exists()
    gate.before_request()  # 상태가 없으니 막히지 않는다.
