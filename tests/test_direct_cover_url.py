"""직접 표지 URL의 SSRF 차단과 제한된 다운로드를 검증한다."""

from unittest.mock import MagicMock
import pytest

from backend import direct_cover_url as module
from backend.epub_cover import CoverError


def public_dns(monkeypatch):
    monkeypatch.setattr(module.socket, "getaddrinfo", lambda *args, **kwargs: [(2, 1, 6, "", ("8.8.8.8", args[1]))])


def test_mixed_public_and_private_dns_is_rejected_before_connecting(monkeypatch):
    monkeypatch.setattr(module.socket, "getaddrinfo", lambda *args, **kwargs: [
        (2, 1, 6, "", ("8.8.8.8", 443)), (2, 1, 6, "", ("192.168.1.10", 443)),
    ])
    pool = MagicMock()
    monkeypatch.setattr(module.urllib3, "HTTPSConnectionPool", pool)
    with pytest.raises(CoverError, match="내부 주소"):
        module.fetch_url_image("https://images.example.org/cover.jpg")
    pool.assert_not_called()


def test_ipv6_image_uses_bracketed_host_and_validated_address(monkeypatch):
    address = "2001:4860:4860::8888"
    monkeypatch.setattr(module.socket, "getaddrinfo", lambda *args, **kwargs: [(10, 1, 6, "", (address, 443, 0, 0))])
    pool = MagicMock()
    response = pool.__enter__.return_value.urlopen.return_value
    response.status = 200
    response.stream.return_value = [b"image"]
    factory = MagicMock(return_value=pool)
    monkeypatch.setattr(module.urllib3, "HTTPSConnectionPool", factory)
    assert module.fetch_url_image(f"https://[{address}]/cover.jpg") == b"image"
    assert factory.call_args.args == (address,)
    assert pool.__enter__.return_value.urlopen.call_args.kwargs["headers"]["Host"] == f"[{address}]"
    assert response.close.called


def test_stream_limit_counts_chunks_instead_of_trusting_content_length(monkeypatch):
    public_dns(monkeypatch)
    monkeypatch.setattr(module, "MAX_IMAGE_BYTES", 5)
    pool = MagicMock()
    response = pool.__enter__.return_value.urlopen.return_value
    response.status = 200
    response.headers = {"Content-Length": "1"}
    def chunks():
        yield b"abc"
        yield b"def"
        raise AssertionError("읽기를 제한 뒤에 계속하면 안 된다")
    response.stream.return_value = chunks()
    monkeypatch.setattr(module.urllib3, "HTTPSConnectionPool", lambda *args, **kwargs: pool)
    with pytest.raises(CoverError, match="크기"):
        module.fetch_url_image("https://images.example.org/cover.jpg")
    assert response.close.called


@pytest.mark.parametrize("status", [301, 302, 303, 307, 308])
def test_relative_redirect_downloads_image_and_closes_every_response(monkeypatch, status):
    public_dns(monkeypatch)
    first, second = MagicMock(), MagicMock()
    redirect = first.__enter__.return_value.urlopen.return_value
    redirect.status = status
    redirect.headers = {"Location": "/real.jpg?version=2"}
    image = second.__enter__.return_value.urlopen.return_value
    image.status = 200
    image.stream.return_value = [b"image"]
    monkeypatch.setattr(module.urllib3, "HTTPSConnectionPool", MagicMock(side_effect=[first, second]))
    assert module.fetch_url_image("https://example.org/cover.jpg") == b"image"
    assert second.__enter__.return_value.urlopen.call_args.args[:2] == ("GET", "/real.jpg?version=2")
    assert redirect.close.called and image.close.called


def test_https_connects_to_validated_ip_with_original_tls_hostname(monkeypatch):
    public_dns(monkeypatch)
    pool = MagicMock()
    response = pool.__enter__.return_value.urlopen.return_value
    response.status = 200
    response.headers = {}
    response.stream.return_value = [b"im", b"age"]
    factory = MagicMock(return_value=pool)
    monkeypatch.setattr(module.urllib3, "HTTPSConnectionPool", factory)
    assert module.fetch_url_image("https://images.example.org/a.jpg?q=1#ignored") == b"image"
    args, kwargs = factory.call_args
    assert args == ("8.8.8.8",)
    assert kwargs["server_hostname"] == "images.example.org"
    assert kwargs["assert_hostname"] == "images.example.org"
    assert kwargs["cert_reqs"] == "CERT_REQUIRED"
    assert response.close.called
    assert pool.__enter__.return_value.urlopen.call_args.args[:2] == ("GET", "/a.jpg?q=1")


@pytest.mark.parametrize("url", ["file:///etc/passwd", "ftp://example.org/x", "http://u:p@example.org/x", "https://example.org:444/x", "https:///no-host", "http://127.0.0.1/x"])
def test_rejects_invalid_urls_and_private_addresses(monkeypatch, url):
    monkeypatch.setattr(module.socket, "getaddrinfo", lambda *args, **kwargs: [(2, 1, 6, "", ("127.0.0.1", 80))])
    with pytest.raises(CoverError):
        module.fetch_url_image(url)


def test_redirect_is_revalidated_before_connecting(monkeypatch):
    monkeypatch.setattr(module.socket, "getaddrinfo", lambda host, *args, **kwargs: [(2, 1, 6, "", ("8.8.8.8" if host == "example.org" else "10.0.0.1", 443))])
    pool = MagicMock()
    response = pool.__enter__.return_value.urlopen.return_value
    response.status = 302
    response.headers = {"Location": "https://internal.example.org/a.jpg"}
    factory = MagicMock(return_value=pool)
    monkeypatch.setattr(module.urllib3, "HTTPSConnectionPool", factory)
    with pytest.raises(CoverError):
        module.fetch_url_image("https://example.org/a.jpg")
    assert factory.call_count == 1
    assert response.close.called


@pytest.mark.parametrize("mode", ["http", "no_location", "bad_status", "size", "redirect_limit", "read_error", "bad_port", "bad_host", "empty_dns"])
def test_download_limits_cleanup_and_url_validation(monkeypatch, mode):
    public_dns(monkeypatch)
    pool = MagicMock()
    response = pool.__enter__.return_value.urlopen.return_value
    response.status = 302 if mode in ("no_location", "redirect_limit") else 500 if mode == "bad_status" else 200
    response.headers = {"Location": "/next"} if mode == "redirect_limit" else {}
    response.stream.return_value = [b"data"]
    monkeypatch.setattr(module.urllib3, "HTTPConnectionPool", MagicMock(return_value=pool))
    monkeypatch.setattr(module.urllib3, "HTTPSConnectionPool", MagicMock(return_value=pool))
    if mode == "size":
        monkeypatch.setattr(module, "MAX_IMAGE_BYTES", 3)
    if mode == "read_error":
        response.stream.side_effect = module.urllib3.exceptions.ReadTimeoutError(None, "/", "timeout")
    if mode == "empty_dns":
        monkeypatch.setattr(module.socket, "getaddrinfo", lambda *args, **kwargs: [])
    url = "http://example.org" if mode == "http" else "https://example.org:image/a" if mode == "bad_port" else "https://\ud800.org/a" if mode == "bad_host" else "https://example.org/a"
    if mode == "http":
        assert module.fetch_url_image(url) == b"data"
    else:
        with pytest.raises(CoverError):
            module.fetch_url_image(url)
    if mode not in ("bad_port", "bad_host", "empty_dns"):
        assert response.close.called
