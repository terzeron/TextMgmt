#!/usr/bin/env python
"""PyJWT (jwt) dependency pinning.

backend 사용처:
- backend/auth.py:
  jwt.encode(payload, JWT_SECRET, algorithm="HS256")
  jwt.decode(token, JWT_SECRET, algorithms=["HS256"])
  jwt.ExpiredSignatureError
  jwt.InvalidTokenError

박제 API:
- jwt.encode(payload: dict, key: str, algorithm: str) -> str
- jwt.decode(token: str, key: str, algorithms: list[str]) -> dict
- jwt.ExpiredSignatureError, jwt.InvalidTokenError 예외 클래스
- 만료된 토큰 / 잘못된 서명 / 잘못된 형식에 대한 예외 흐름
"""

import time
import unittest


SECRET = "test_jwt_secret_for_testing_minimum_32bytes"
ALG = "HS256"


class TestJwtImport(unittest.TestCase):
    def test_module_import(self):
        import jwt

        self.assertTrue(callable(jwt.encode))
        self.assertTrue(callable(jwt.decode))

    def test_exception_classes_exist(self):
        import jwt

        self.assertTrue(issubclass(jwt.ExpiredSignatureError, Exception))
        self.assertTrue(issubclass(jwt.InvalidTokenError, Exception))


class TestJwtEncodeDecodeRoundTrip(unittest.TestCase):
    """auth.py: encode -> decode 왕복"""

    def test_encode_returns_str(self):
        import jwt

        token = jwt.encode({"sub": "user@x"}, SECRET, algorithm=ALG)
        # PyJWT 2.x: str 반환
        self.assertIsInstance(token, str)

    def test_decode_returns_dict_with_claims(self):
        import jwt

        now = int(time.time())
        payload = {"type": "access", "email": "user@x", "role": "viewer", "name": "U", "picture": "", "exp": now + 3600, "iat": now}
        token = jwt.encode(payload, SECRET, algorithm=ALG)
        decoded = jwt.decode(token, SECRET, algorithms=[ALG])
        self.assertEqual(decoded["email"], "user@x")
        self.assertEqual(decoded["role"], "viewer")
        self.assertEqual(decoded["type"], "access")

    def test_refresh_token_claim_roundtrip(self):
        """auth.py:create_refresh_token이 fid/jti claim을 인코딩"""
        import jwt

        now = int(time.time())
        token = jwt.encode({"type": "refresh", "fid": "fam1", "jti": "jti1", "exp": now + 3600, "iat": now}, SECRET, algorithm=ALG)
        decoded = jwt.decode(token, SECRET, algorithms=[ALG])
        self.assertEqual(decoded["fid"], "fam1")
        self.assertEqual(decoded["jti"], "jti1")


class TestJwtExpiredSignatureError(unittest.TestCase):
    """auth.py: except jwt.ExpiredSignatureError"""

    def test_expired_token_raises_expired_signature_error(self):
        import jwt

        now = int(time.time())
        token = jwt.encode({"sub": "x", "exp": now - 10, "iat": now - 20}, SECRET, algorithm=ALG)
        with self.assertRaises(jwt.ExpiredSignatureError):
            jwt.decode(token, SECRET, algorithms=[ALG])


class TestJwtInvalidTokenError(unittest.TestCase):
    """auth.py: except jwt.InvalidTokenError"""

    def test_invalid_signature_raises_invalid_token_error(self):
        import jwt

        token = jwt.encode({"sub": "x"}, SECRET, algorithm=ALG)
        with self.assertRaises(jwt.InvalidTokenError):
            jwt.decode(token, "wrong_secret_with_enough_length_xxxxx", algorithms=[ALG])

    def test_garbage_token_raises_invalid_token_error(self):
        import jwt

        with self.assertRaises(jwt.InvalidTokenError):
            jwt.decode("not-a-jwt", SECRET, algorithms=[ALG])

    def test_expired_signature_is_subclass_of_invalid_token(self):
        """auth.py가 except ExpiredSignatureError를 except InvalidTokenError 보다 먼저 잡는 이유 박제."""
        import jwt

        self.assertTrue(issubclass(jwt.ExpiredSignatureError, jwt.InvalidTokenError))


class TestJwtAlgorithmsListRequired(unittest.TestCase):
    """auth.py: jwt.decode(token, key, algorithms=[ALG]) — algorithms는 list여야 함"""

    def test_decode_with_algorithms_list(self):
        import jwt

        token = jwt.encode({"sub": "x"}, SECRET, algorithm=ALG)
        decoded = jwt.decode(token, SECRET, algorithms=[ALG])
        self.assertEqual(decoded["sub"], "x")


if __name__ == "__main__":
    unittest.main()
