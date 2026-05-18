#!/usr/bin/env python
"""google-auth dependency pinning.

backend 사용처:
- backend/main.py:17-18
  from google.auth.transport import requests as google_requests
  from google.oauth2 import id_token as google_id_token
- backend/main.py:678
  google_id_token.verify_oauth2_token(credential, google_requests.Request(), TM_GOOGLE_CLIENT_ID)

박제 API:
- google.auth.transport.requests.Request 클래스 (호출 가능, no-arg 생성)
- google.oauth2.id_token.verify_oauth2_token(token, request, audience) 시그니처
"""

import inspect
import unittest


class TestGoogleAuthTransportRequests(unittest.TestCase):
    def test_module_importable(self):
        from google.auth.transport import requests as google_requests

        self.assertTrue(hasattr(google_requests, "Request"))

    def test_request_class_no_arg_constructible(self):
        """main.py가 google_requests.Request()로 인자 없이 생성"""
        from google.auth.transport import requests as google_requests

        req = google_requests.Request()
        self.assertIsNotNone(req)


class TestGoogleOAuth2IdToken(unittest.TestCase):
    def test_module_importable(self):
        from google.oauth2 import id_token as google_id_token

        self.assertTrue(hasattr(google_id_token, "verify_oauth2_token"))

    def test_verify_oauth2_token_callable(self):
        from google.oauth2 import id_token as google_id_token

        self.assertTrue(callable(google_id_token.verify_oauth2_token))

    def test_verify_oauth2_token_signature(self):
        """main.py: verify_oauth2_token(credential, request, audience) — 위치 인자 3개"""
        from google.oauth2 import id_token as google_id_token

        sig = inspect.signature(google_id_token.verify_oauth2_token)
        # 최소 3개의 파라미터를 받아야 함 (id_token, request, audience)
        positional_params = [p for p in sig.parameters.values() if p.kind in (inspect.Parameter.POSITIONAL_OR_KEYWORD, inspect.Parameter.POSITIONAL_ONLY)]
        self.assertGreaterEqual(len(positional_params), 3)

    def test_verify_oauth2_token_returns_dict_with_iss(self):
        """main.py: result.get('iss') in GOOGLE_ISSUERS

        실제 호출은 외부 네트워크 필요. 여기서는 잘못된 토큰을 줘서
        ValueError(혹은 GoogleAuthError) 계열의 예외가 나는지만 확인.
        """
        from google.auth.transport import requests as google_requests
        from google.oauth2 import id_token as google_id_token

        with self.assertRaises(Exception):
            google_id_token.verify_oauth2_token("not-a-real-token", google_requests.Request(), "fake-client-id")


if __name__ == "__main__":
    unittest.main()
