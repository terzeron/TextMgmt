#!/usr/bin/env python
"""fastapi dependency pinning.

backend 사용처:
- backend/main.py: FastAPI 앱 구성, 라우팅, 미들웨어, 예외 처리, 응답 모델
- backend/auth.py: HTTPException, Request 사용
- backend/book_manager.py: FileResponse, Response

박제 API:
- from fastapi import APIRouter, Depends, FastAPI, HTTPException, Request
- from fastapi.middleware.cors import CORSMiddleware
- from fastapi.middleware.gzip import GZipMiddleware
- from fastapi.responses import FileResponse, Response, JSONResponse
- from fastapi.exceptions import RequestValidationError
- from fastapi.encoders import jsonable_encoder
- @app.exception_handler(...) / @app.get/put/post/delete(...)
- app.add_middleware(...)
- app.include_router(router, prefix=...)
- HTTPException(status_code=..., detail=...)
- JSONResponse 서브클래싱 (main.py CustomJSONResponse 패턴)
- request.cookies, request.headers, request.url
"""

import os
import unittest


class TestFastAPIImports(unittest.TestCase):
    def test_core_symbols_importable(self):
        from fastapi import APIRouter, Depends, FastAPI, HTTPException, Request

        self.assertTrue(callable(FastAPI))
        self.assertTrue(callable(APIRouter))
        self.assertTrue(callable(HTTPException))
        self.assertTrue(callable(Depends))
        self.assertTrue(callable(Request))

    def test_middleware_imports(self):
        from fastapi.middleware.cors import CORSMiddleware
        from fastapi.middleware.gzip import GZipMiddleware

        self.assertTrue(callable(CORSMiddleware))
        self.assertTrue(callable(GZipMiddleware))

    def test_response_imports(self):
        from fastapi.responses import FileResponse, JSONResponse, Response

        self.assertTrue(callable(FileResponse))
        self.assertTrue(callable(Response))
        self.assertTrue(callable(JSONResponse))

    def test_exception_imports(self):
        from fastapi import HTTPException
        from fastapi.exceptions import RequestValidationError

        self.assertTrue(issubclass(HTTPException, Exception))
        self.assertTrue(issubclass(RequestValidationError, Exception))

    def test_encoders_import(self):
        from fastapi.encoders import jsonable_encoder

        self.assertTrue(callable(jsonable_encoder))


class TestHTTPExceptionUsage(unittest.TestCase):
    """main.py: raise HTTPException(status_code=..., detail=...)"""

    def test_construct_with_status_and_detail(self):
        from fastapi import HTTPException

        exc = HTTPException(status_code=403, detail="접근 권한이 없습니다")
        self.assertEqual(exc.status_code, 403)
        self.assertEqual(exc.detail, "접근 권한이 없습니다")

    def test_raise_and_catch(self):
        from fastapi import HTTPException

        with self.assertRaises(HTTPException) as ctx:
            raise HTTPException(status_code=400, detail="bad")
        self.assertEqual(ctx.exception.status_code, 400)


class TestAppRouting(unittest.TestCase):
    def test_route_decorators_register_routes(self):
        """main.py: @app.get / @app.put / @app.post / @app.delete / @app.exception_handler"""
        from fastapi import FastAPI

        app = FastAPI()

        @app.get("/g")
        async def g():
            return {}

        @app.put("/p/{x}")
        async def p(x: int):
            return {"x": x}

        @app.post("/c")
        async def c():
            return {}

        @app.delete("/d/{x}")
        async def d(x: int):
            return {"x": x}

        self.assertGreaterEqual(len(app.routes), 4)

    def test_router_decorators_with_dependencies(self):
        """main.py: APIRouter() + @router.get(..., dependencies=...)"""
        from fastapi import APIRouter, Depends

        async def dep():
            return {"u": 1}

        router = APIRouter()

        @router.get("/x", dependencies=[Depends(dep)])
        async def x():
            return {}

        self.assertGreater(len(router.routes), 0)

    def test_include_router_with_prefix(self):
        """main.py: app.include_router(router, prefix="/comics")"""
        from fastapi import APIRouter, FastAPI

        app = FastAPI()
        router = APIRouter()

        @router.get("/list")
        async def list_():
            return []

        app.include_router(router, prefix="/comics")
        paths = [getattr(r, "path", "") for r in app.routes]
        self.assertIn("/comics/list", paths)


class TestMiddleware(unittest.TestCase):
    def test_add_cors_middleware_with_options(self):
        """main.py:41 add_middleware(CORSMiddleware, allow_origins, allow_credentials, allow_methods, allow_headers, expose_headers)"""
        from fastapi import FastAPI
        from fastapi.middleware.cors import CORSMiddleware

        app = FastAPI()
        app.add_middleware(CORSMiddleware, allow_origins=["http://x"], allow_credentials=True, allow_methods=["*"], allow_headers=["*"], expose_headers=["X-Total-Pages"])

    def test_add_gzip_middleware_with_minimum_size(self):
        """main.py:42 add_middleware(GZipMiddleware, minimum_size=1000)"""
        from fastapi import FastAPI
        from fastapi.middleware.gzip import GZipMiddleware

        app = FastAPI()
        app.add_middleware(GZipMiddleware, minimum_size=1000)


class TestExceptionHandlers(unittest.TestCase):
    def test_exception_handler_decorators(self):
        """main.py: @app.exception_handler(RequestValidationError|HTTPException|Exception)"""
        from fastapi import FastAPI, HTTPException
        from fastapi.exceptions import RequestValidationError

        app = FastAPI()

        @app.exception_handler(RequestValidationError)
        async def h1(req, exc):
            return None

        @app.exception_handler(HTTPException)
        async def h2(req, exc):
            return None

        @app.exception_handler(Exception)
        async def h3(req, exc):
            return None

    def test_request_validation_error_has_body_attribute(self):
        """main.py:49 LOGGER.error('Request body summary: %s', _summarize_request_body(exc.body))"""
        from fastapi.exceptions import RequestValidationError

        exc = RequestValidationError(errors=[], body={"x": 1})
        self.assertEqual(exc.body, {"x": 1})


class TestJSONResponse(unittest.TestCase):
    """main.py CustomJSONResponse 패턴"""

    def test_json_response_construct(self):
        from fastapi.responses import JSONResponse

        resp = JSONResponse(status_code=422, content={"detail": "x"})
        self.assertEqual(resp.status_code, 422)

    def test_json_response_subclass_render_override(self):
        """main.py:80
        class CustomJSONResponse(JSONResponse):
            def render(self, content) -> bytes: ...
        """
        import json

        from fastapi.responses import JSONResponse

        class CustomJSONResponse(JSONResponse):
            def render(self, content) -> bytes:
                return json.dumps(content, ensure_ascii=False, separators=(",", ":")).encode("utf-8")

        resp = CustomJSONResponse(content={"한글": "테스트"})
        self.assertIn("한글".encode("utf-8"), resp.body)


class TestJsonableEncoder(unittest.TestCase):
    def test_encoder_basic(self):
        from fastapi.encoders import jsonable_encoder

        result = jsonable_encoder({"k": "v", "n": 1})
        self.assertEqual(result, {"k": "v", "n": 1})


class TestRequestObject(unittest.TestCase):
    """main.py가 사용하는 Request 속성/메서드:
    - request.cookies
    - request.headers
    - request.url, request.url.hostname
    """

    def test_request_class_has_expected_attributes(self):
        from fastapi import Request

        # 핵심 속성들이 정의되어 있는지 확인
        self.assertTrue(hasattr(Request, "cookies"))
        self.assertTrue(hasattr(Request, "headers"))
        self.assertTrue(hasattr(Request, "url"))


class TestFileResponseAndResponse(unittest.TestCase):
    """book_manager.py: FileResponse, Response"""

    def test_response_constructor(self):
        from fastapi.responses import Response

        resp = Response(content="ok", media_type="text/plain")
        self.assertEqual(resp.media_type, "text/plain")

    def test_file_response_is_callable(self):
        from fastapi.responses import FileResponse

        self.assertTrue(callable(FileResponse))


class TestDependsCallable(unittest.TestCase):
    """main.py: Depends(require_auth), Depends(require_admin)"""

    def test_depends_wraps_callable(self):
        from fastapi import Depends

        def dep():
            return "x"

        d = Depends(dep)
        self.assertEqual(d.dependency, dep)


class TestRouteFunctionalEndToEnd(unittest.TestCase):
    """실제 호출 흐름 박제 - TestClient로 cookie/cors/exception_handler 동작 확인"""

    def test_cookie_set_via_response(self):
        """main.py: _set_auth_cookies(response, access_token, refresh_token)
        Response.set_cookie(key, value, max_age, httponly, secure, samesite, path)
        """
        from fastapi.responses import JSONResponse

        resp = JSONResponse(content={"ok": True})
        resp.set_cookie(key="access_token", value="v", max_age=900, httponly=True, secure=True, samesite="lax", path="/")
        # set-cookie 헤더가 추가됨
        cookie_headers = [h for h in resp.raw_headers if h[0].lower() == b"set-cookie"]
        self.assertGreaterEqual(len(cookie_headers), 1)

    def test_response_delete_cookie(self):
        """main.py: _clear_auth_cookies(response) -> response.delete_cookie(...)"""
        from fastapi.responses import JSONResponse

        resp = JSONResponse(content={})
        resp.delete_cookie(key="access_token", path="/")
        cookie_headers = [h for h in resp.raw_headers if h[0].lower() == b"set-cookie"]
        self.assertGreaterEqual(len(cookie_headers), 1)


class TestCorsPolicy(unittest.TestCase):
    """main.py CORS 최소권한(CWE-942) 회귀 테스트.

    allow_methods/allow_headers 가 wildcard("*")가 아니라 실제 사용분만 허용하는지 검증한다.
    wildcard 설정이면 disallowed 메서드/헤더 preflight도 통과하므로 아래 단언이 깨진다.
    """

    @classmethod
    def setUpClass(cls):
        from fastapi.testclient import TestClient
        import backend.main as main_mod

        cls.client = TestClient(main_mod.app)
        cls.origin = os.environ["TM_FRONTEND_URL"]

    def _preflight(self, method, request_headers=None):
        headers = {"Origin": self.origin, "Access-Control-Request-Method": method}
        if request_headers:
            headers["Access-Control-Request-Headers"] = request_headers
        return self.client.options("/", headers=headers)

    def test_allowed_method_preflight_succeeds(self):
        resp = self._preflight("PUT", "content-type")
        self.assertEqual(resp.status_code, 200)
        self.assertEqual(resp.headers.get("access-control-allow-origin"), self.origin)
        self.assertEqual(resp.headers.get("access-control-allow-credentials"), "true")
        allow_methods = resp.headers.get("access-control-allow-methods", "")
        self.assertIn("PUT", allow_methods)

    def test_allow_methods_not_wildcard(self):
        # wildcard였다면 PATCH 등 모든 메서드가 노출/허용된다.
        allow_methods = self._preflight("PUT", "content-type").headers.get("access-control-allow-methods", "")
        self.assertNotIn("*", allow_methods)
        self.assertNotIn("PATCH", allow_methods)
        self.assertEqual(self._preflight("PATCH").status_code, 400)

    def test_allow_headers_not_wildcard(self):
        # Content-Type(safelisted)은 허용되지만 임의 커스텀 헤더는 거부되어야 한다.
        self.assertEqual(self._preflight("POST", "content-type").status_code, 200)
        self.assertEqual(self._preflight("POST", "x-evil-header").status_code, 400)


if __name__ == "__main__":
    unittest.main()
