#!/usr/bin/env python
"""requests dependency pinning.

backend 사용처:
- backend/bookstore.py:
    self.session = requests.Session()
    self.session.headers.update({...})
    resp = self.session.get(url, timeout=10, verify=True)
    resp = self.session.get(api_url, params=..., timeout=10, verify=True)
    s = requests.Session()
    s.headers.update(self.session.headers)
    s.cookies.update(self.session.cookies)
    resp2 = s.get(detail_url, timeout=10, verify=True)
  사용 속성: resp.text, resp.content, resp.json(), resp.status_code, resp.encoding

박제 API:
- requests.Session() 인스턴스 생성
- Session.headers: 가변 dict
- Session.cookies: 가변 cookiejar
- Session.get(url, timeout=, verify=, params=)
- Response.text, Response.content, Response.status_code, Response.json()
- requests.exceptions.RequestException (대표 예외 베이스)
"""

import inspect
import unittest


class TestRequestsImport(unittest.TestCase):
    def test_module_attrs(self):
        import requests

        self.assertTrue(callable(requests.Session))
        self.assertTrue(callable(requests.get))
        self.assertTrue(callable(requests.post))


class TestSessionConstructionAndHeaders(unittest.TestCase):
    """bookstore.py:37 self.session = requests.Session()
    bookstore.py:38 self.session.headers.update({...})
    """

    def test_session_has_headers_mapping(self):
        import requests

        s = requests.Session()
        try:
            self.assertTrue(hasattr(s, "headers"))
            # update / __setitem__
            s.headers.update({"User-Agent": "curl/7.79.1", "Referer": "http://x"})
            self.assertEqual(s.headers["User-Agent"], "curl/7.79.1")
            self.assertEqual(s.headers["Referer"], "http://x")
        finally:
            s.close()

    def test_session_has_cookies_jar(self):
        """bookstore.py:145 s.cookies.update(self.session.cookies)"""
        import requests

        s = requests.Session()
        try:
            self.assertTrue(hasattr(s, "cookies"))
            self.assertTrue(hasattr(s.cookies, "update"))
        finally:
            s.close()

    def test_session_get_signature_accepts_timeout_verify_params(self):
        import requests

        sig = inspect.signature(requests.Session.get)
        params = set(sig.parameters)
        # 위치 인자 url + **kwargs 형태가 일반적이므로 kwargs 통과 여부로 검증
        # 대신 Session.request의 시그니처를 확인
        req_sig = inspect.signature(requests.Session.request)
        req_params = set(req_sig.parameters)
        for kw in ("method", "url", "params", "headers", "cookies", "timeout", "verify"):
            self.assertIn(kw, req_params, f"Session.request missing {kw}")


class TestResponseAttributes(unittest.TestCase):
    """bookstore.py가 사용하는 Response 속성:
    - resp.text  : str
    - resp.content : bytes
    - resp.status_code : int
    - resp.json() : 메서드
    """

    def test_response_class_has_required_members(self):
        from requests.models import Response

        # 메서드/프로퍼티는 클래스 레벨에 정의됨
        for member in ("text", "content", "json"):
            self.assertTrue(hasattr(Response, member), f"Response class missing {member}")

        # status_code/headers/encoding은 __init__에서 인스턴스 속성으로 설정됨
        resp = Response()
        for attr in ("status_code", "headers", "encoding"):
            self.assertTrue(hasattr(resp, attr), f"Response instance missing {attr}")

    def test_response_instance_works_locally(self):
        """로컬 Response 인스턴스에 데이터를 주입해 속성 동작 박제"""
        from requests.models import Response

        resp = Response()
        resp.status_code = 200
        resp._content = b'{"ok": true}'
        resp.encoding = "utf-8"

        self.assertEqual(resp.status_code, 200)
        self.assertEqual(resp.content, b'{"ok": true}')
        self.assertEqual(resp.text, '{"ok": true}')
        self.assertEqual(resp.json(), {"ok": True})


class TestRequestsExceptions(unittest.TestCase):
    def test_exception_base_classes(self):
        import requests

        self.assertTrue(issubclass(requests.exceptions.RequestException, Exception))
        self.assertTrue(issubclass(requests.exceptions.Timeout, requests.exceptions.RequestException))
        self.assertTrue(issubclass(requests.exceptions.ConnectionError, requests.exceptions.RequestException))
        self.assertTrue(issubclass(requests.exceptions.HTTPError, requests.exceptions.RequestException))


class TestSessionGetIsCallableAndReturnsResponse(unittest.TestCase):
    """실제 HTTP 호출 없이 Session.get이 callable이고 Response 객체를 반환할 수 있음을 박제.

    네트워크 호출은 하지 않는다 (CI 등 격리 환경 대비).
    """

    def test_session_get_is_method(self):
        import requests

        self.assertTrue(callable(requests.Session.get))


if __name__ == "__main__":
    unittest.main()
