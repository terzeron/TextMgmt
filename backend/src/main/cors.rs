//! CORS. Python `CORSMiddleware(allow_origins=[TM_FRONTEND_URL], allow_credentials=True, allow_methods=[GET,POST,PUT,DELETE], allow_headers=[Content-Type])`.
//!
//! 인증은 HttpOnly 쿠키(credentials include) 기반이라 `Authorization` 헤더는 허용하지 않는다(CWE-942: 실제 쓰는 메서드·헤더만 연다).
//! Origin이 `TM_FRONTEND_URL`과 같을 때만 허용 헤더를 붙인다. 허용 Origin은 요청 시점에 환경 변수에서 읽는다.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

const ALLOW_METHODS: &str = "GET, POST, PUT, DELETE";
const ALLOW_HEADERS: &str = "Content-Type";
const EXPOSE_HEADERS: &str = "Accept-Ranges, Content-Range, Content-Length, Content-Encoding, X-Total-Pages, X-Total-Chapters";
const PREFLIGHT_MAX_AGE: &str = "600";

fn allowed_origin(headers: &HeaderMap) -> Option<HeaderValue> {
    let origin = headers.get(header::ORIGIN)?;
    let frontend = std::env::var("TM_FRONTEND_URL").ok().filter(|u| !u.is_empty())?;
    (origin.to_str().ok()? == frontend).then(|| origin.clone())
}

fn plain(status: StatusCode, text: &str) -> Response {
    let mut resp = Response::new(Body::from(text.to_string()));
    *resp.status_mut() = status;
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    resp
}

fn vary_origin(resp: &mut Response) {
    resp.headers_mut().append(header::VARY, HeaderValue::from_static("Origin"));
}

fn preflight(req: &Request, origin: Option<HeaderValue>) -> Response {
    let method_ok = req.headers().get("access-control-request-method").and_then(|v| v.to_str().ok()).is_some_and(|m| ALLOW_METHODS.split(", ").any(|a| a == m));
    // 요청 헤더는 허용 목록(Content-Type)의 부분집합이어야 한다.
    let headers_ok = req
        .headers()
        .get("access-control-request-headers")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|h| h.split(',').map(str::trim).filter(|s| !s.is_empty()).all(|name| name.eq_ignore_ascii_case(ALLOW_HEADERS)));
    let Some(origin) = origin else {
        let mut resp = plain(StatusCode::BAD_REQUEST, "Disallowed CORS origin");
        vary_origin(&mut resp);
        return resp;
    };
    if !method_ok || !headers_ok {
        let what = if !method_ok { "Disallowed CORS method" } else { "Disallowed CORS headers" };
        let mut resp = plain(StatusCode::BAD_REQUEST, what);
        resp.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        resp.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, HeaderValue::from_static("true"));
        vary_origin(&mut resp);
        return resp;
    }
    let mut resp = plain(StatusCode::OK, "OK");
    let h = resp.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    h.insert(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, HeaderValue::from_static("true"));
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static(ALLOW_METHODS));
    h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static(ALLOW_HEADERS));
    h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static(PREFLIGHT_MAX_AGE));
    vary_origin(&mut resp);
    resp
}

pub async fn cors_middleware(req: Request, next: Next) -> Response {
    let has_origin = req.headers().contains_key(header::ORIGIN);
    let origin = allowed_origin(req.headers());
    let is_preflight = req.method() == Method::OPTIONS && req.headers().contains_key("access-control-request-method");
    if has_origin && is_preflight {
        return preflight(&req, origin);
    }
    let mut resp = next.run(req).await;
    if let Some(origin) = origin {
        let h = resp.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        h.insert(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, HeaderValue::from_static("true"));
        h.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static(EXPOSE_HEADERS));
        vary_origin(&mut resp);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::middleware::from_fn;
    use axum::routing::get;
    use tower::ServiceExt;

    const ORIGIN: &str = "http://localhost:3000";

    fn app() -> Router {
        Router::new().route("/x", get(|| async { "ok" })).layer(from_fn(cors_middleware))
    }

    // 환경 변수는 프로세스 전역이라 이 모듈의 테스트는 한 테스트 안에서 순서대로 확인한다.
    #[tokio::test]
    async fn cors_follows_frontend_url() {
        // SAFETY: 이 lib 단위 테스트 바이너리에서 TM_FRONTEND_URL을 바꾸는 테스트는 이것 하나다.
        unsafe { std::env::set_var("TM_FRONTEND_URL", ORIGIN) };

        let ok = app().oneshot(Request::get("/x").header("origin", ORIGIN).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(ok.headers()["access-control-allow-origin"], ORIGIN);
        assert_eq!(ok.headers()["access-control-allow-credentials"], "true");
        assert!(ok.headers()["access-control-expose-headers"].to_str().unwrap().contains("X-Total-Pages"));

        let other = app().oneshot(Request::get("/x").header("origin", "https://evil.example.com").body(Body::empty()).unwrap()).await.unwrap();
        assert!(!other.headers().contains_key("access-control-allow-origin"));

        let preflight = |method: &str, headers: &str| {
            Request::builder()
                .method("OPTIONS")
                .uri("/x")
                .header("origin", ORIGIN)
                .header("access-control-request-method", method)
                .header("access-control-request-headers", headers)
                .body(Body::empty())
                .unwrap()
        };
        let resp = app().oneshot(preflight("POST", "content-type")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()["access-control-allow-methods"], ALLOW_METHODS);
        let resp = app().oneshot(preflight("PATCH", "content-type")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        // Authorization 헤더는 허용 목록에 없다.
        let resp = app().oneshot(preflight("GET", "authorization")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        unsafe { std::env::remove_var("TM_FRONTEND_URL") };
        let none = app().oneshot(Request::get("/x").header("origin", ORIGIN).body(Body::empty()).unwrap()).await.unwrap();
        assert!(!none.headers().contains_key("access-control-allow-origin"));
    }
}
