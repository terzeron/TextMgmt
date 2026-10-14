"""공개 이미지 URL을 검증한 IP에 직접 연결하여 내려받는다."""

import ipaddress
import socket
from urllib.parse import urljoin, urlparse

import requests
import urllib3

from backend.epub_cover import CoverError, MAX_IMAGE_BYTES


def fetch_url_image(url: str) -> bytes:
    for _ in range(5):
        parsed = urlparse(url)
        try:
            port = parsed.port or (443 if parsed.scheme == "https" else 80)
            host = (parsed.hostname or "").encode("idna").decode("ascii")
        except (ValueError, UnicodeError) as exc:
            raise CoverError("올바른 이미지 URL을 입력하세요") from exc
        if (parsed.scheme not in ("http", "https") or not host or parsed.username is not None
                or parsed.password is not None or port != (443 if parsed.scheme == "https" else 80)):
            raise CoverError("공개 HTTP/HTTPS 이미지 URL만 사용할 수 있습니다")
        addresses = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
        if not addresses or any(not ipaddress.ip_address(entry[4][0]).is_global for entry in addresses):
            raise CoverError("내부 주소의 이미지는 사용할 수 없습니다")
        ip = addresses[0][4][0]
        # 연결 대상 IP는 DNS 검증 결과로 고정한다. TLS SNI와 인증서는 원래 host로 검증한다.
        pool = (urllib3.HTTPSConnectionPool(ip, port=port, server_hostname=host, assert_hostname=host,
                                          cert_reqs="CERT_REQUIRED", ca_certs=requests.certs.where())
                if parsed.scheme == "https" else urllib3.HTTPConnectionPool(ip, port=port))
        target = parsed.path or "/"
        if parsed.query:
            target += "?" + parsed.query
        headers = {"Host": f"[{host}]" if ":" in host else host,
                   "User-Agent": "TextMgmt/cover", "Accept": "image/*", "Accept-Encoding": "identity"}
        try:
            with pool as connection:
                response = connection.urlopen("GET", target, headers=headers, redirect=False, retries=False,
                                              preload_content=False, timeout=urllib3.Timeout(connect=5, read=15))
                try:
                    if response.status in (301, 302, 303, 307, 308):
                        location = response.headers.get("Location")
                        if not location:
                            raise CoverError("이미지 redirect 주소가 없습니다")
                        url = urljoin(url, location)
                        continue
                    if response.status != 200:
                        raise CoverError("이미지 서버에서 파일을 받지 못했습니다")
                    output = bytearray()
                    for chunk in response.stream(65536, decode_content=False):
                        output.extend(chunk)
                        if len(output) > MAX_IMAGE_BYTES:
                            raise CoverError("표지 이미지 크기가 제한을 초과했습니다")
                    return bytes(output)
                finally:
                    response.close()
        except urllib3.exceptions.HTTPError as exc:
            raise CoverError("이미지 서버에 연결하지 못했습니다. URL을 확인하세요") from exc
    raise CoverError("이미지 redirect 횟수를 초과했습니다")
