//! HTTP 라우트와 앱 조립. 설계: docs/rust-test-seams.md 3.2
//!
//! # 구성
//! - `deps.rs`: `AppDeps`(주입 묶음), `AuthMode`. `traits.rs`: 주입하는 trait와 실제 구현.
//! - `routes.rs`: 라우트 등록부 `Routes`. `extract.rs`: 인증·JSON·쿼리 추출기. `errors.rs`: 오류 응답과 로깅.
//! - `auth_routes.rs`, `log_routes.rs`: 라우트 파일 예시. `cors.rs`, `cookies.rs`, `client_info.rs`: 공용 도우미.
//!
//! # 라우트를 붙이는 방법
//! 1. `src/main/routes_xxx.rs`를 만들고 `pub(super) fn routes() -> Routes`를 하나 둔다.
//!    핸들러는 `State<Arc<AppDeps>>`로 의존성을 받는다(`deps.book_manager()`, `deps.clock()` 등 접근자).
//!    인증은 핸들러 첫 인자에 `RequireAuth`/`RequireAdmin`/`OptionalAuth` 추출기를 두고, 등록 때 같은 이름을 `&["require_admin"]`처럼 적는다.
//! 2. 이 파일에 `mod routes_xxx;`를 더하고 `all_routes()`에 `.merge(routes_xxx::routes())` 한 줄을 더한다.
//! 3. 오류는 `Result<_, ApiError>`로 돌려준다(`ApiError::http(404, "..")`). 새 의존성이 필요하면 `AppDeps`에 필드, `with_xxx`, 접근자를 더한다.
//!
//! 앱 안의 로그는 스레드 로컬 수집(`LogCapture`)에 잡히도록 핸들러 안에서 `spawn`하지 않는다. 오래 걸리는 일은 `deps.task_runner()`로 넘긴다.

use axum::Router;
use axum::middleware::from_fn;
use std::sync::Arc;

mod auth_routes;
mod client_info;
mod cookies;
mod cors;
mod deps;
mod errors;
mod extract;
mod log_routes;
mod routes;
mod traits;

pub use crate::auth::{ACCESS_COOKIE_NAME, ACCESS_TOKEN_EXPIRATION_SECONDS, HTTPException, REFRESH_COOKIE_NAME, REFRESH_TOKEN_EXPIRATION_SECONDS, RoleResolver, observation_hash};
pub use crate::view_history_store::MAX_RECENT_VIEWS;
pub use client_info::{_client_ip, _request_ip_prefix, _request_user_agent, ClientInfo};
pub use cookies::{_clear_auth_cookies, _get_cookie_settings, _set_auth_cookies, CustomJSONResponse, ResponseHeaders, custom_jsonable_encoder, is_local_frontend_origin};
pub use deps::{AppDeps, AuthMode, AuthSlots, RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS, Slot};
pub use errors::{
    ApiError, ApiResult, GENERIC_SERVER_ERROR_DETAIL, RequestValidationError, SERVICE_UNAVAILABLE_DETAIL, VALIDATION_ERROR_DETAIL, general_exception_handler, http_exception_handler, json_response,
    summarize_request_body, validation_exception_handler,
};
pub use extract::{ApiJson, ApiQuery, OptionalAuth, RequireAdmin, RequireAuth};
pub use log_routes::{CLIENT_ERROR_FIELD_LIMITS, ClientErrorLogModel};
pub use routes::{RouteInfo, Routes};
pub use traits::{
    BookAccess, BookContent, BookManagerApi, BookResponse, BookStub, CategoryMappingApi, GoogleVerifier, InlineRunner, MappingError, TaskHandle, TaskRunner, ThreadRunner, UnavailableBookAccess,
    UnavailableBookManager, UnavailableGoogleVerifier, ViewHistoryApi,
};

/// Python 쪽 이름. `view_history_api` 테스트가 쓴다.
pub type HttpException = HTTPException;

/// 앱의 모든 라우트. 새 라우트 파일은 여기에 `.merge(..)` 한 줄로 붙인다.
fn all_routes() -> Routes {
    Routes::new().merge(auth_routes::routes()).merge(log_routes::routes())
}

/// 환경 변수로 조립한 운영용 앱.
pub fn app() -> Router {
    app_with(AppDeps::from_env())
}

/// 주입한 의존성으로 앱을 만든다. 테스트는 `AppDeps::new(..).with_xxx(..)`로 대역을 끼운다.
pub fn app_with(deps: AppDeps) -> Router {
    all_routes().into_router().fallback(errors::not_found).with_state(Arc::new(deps)).layer(from_fn(errors::error_middleware)).layer(from_fn(cors::cors_middleware))
}

/// 등록된 라우트 목록(경로, 메서드, 의존성). FastAPI `app.routes`의 `route.dependant.dependencies`에 대응한다.
pub fn route_table() -> Vec<RouteInfo> {
    all_routes().table().to_vec()
}

/// 경로에 붙은 의존성 이름. 같은 경로에 메서드가 여럿이면 합집합이다.
pub fn route_dependencies(path: &str) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    for info in route_table().into_iter().filter(|r| r.path == path) {
        for dep in info.dependencies {
            let known = ["require_auth", "require_admin", "optional_auth"].into_iter().find(|n| *n == dep);
            if let Some(name) = known.filter(|n| !found.contains(n)) {
                found.push(name);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn send(app: Router, req: Request<Body>) -> axum::response::Response {
        app.oneshot(req).await.unwrap()
    }

    fn plain_app() -> Router {
        app_with(AppDeps::new(AuthMode::Fixed(json!({"email": "a@example.com", "role": "admin"}))))
    }

    use serde_json::json;

    #[tokio::test]
    async fn unknown_path_is_json_404() {
        let resp = send(plain_app(), Request::get("/nope").body(Body::empty()).unwrap()).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), json!({"detail": "Not Found"}));
    }

    #[tokio::test]
    async fn wrong_method_is_json_405_with_allow_header() {
        let resp = send(plain_app(), Request::delete("/auth/me").body(Body::empty()).unwrap()).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(resp.headers().contains_key("allow"));
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), json!({"detail": "Method Not Allowed"}));
    }

    #[tokio::test]
    async fn handler_panic_becomes_generic_500() {
        let routes = Routes::new().get("/boom", &[], || async { panic!("kaboom") as &'static str });
        let app = routes.into_router().with_state(Arc::new(AppDeps::new(AuthMode::Cookies))).layer(from_fn(errors::error_middleware));
        let resp = send(app, Request::get("/boom").body(Body::empty()).unwrap()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), json!({"detail": GENERIC_SERVER_ERROR_DETAIL}));
    }

    #[test]
    fn route_table_lists_auth_dependencies() {
        assert_eq!(route_dependencies("/auth/me"), vec!["require_auth"]);
        assert_eq!(route_dependencies("/auth/sessions"), vec!["require_admin"]);
        assert_eq!(route_dependencies("/logs/client-error"), vec!["optional_auth"]);
        assert!(route_dependencies("/auth/google").is_empty());
    }

    #[tokio::test]
    async fn merged_route_files_share_one_router() {
        async fn hello() -> &'static str {
            "hi"
        }
        let extra = Routes::new().get("/extra", &[], hello);
        let routes = all_routes().merge(extra);
        assert!(routes.table().iter().any(|r| r.path == "/extra"));
        assert!(routes.table().iter().any(|r| r.path == "/auth/me"));
        let app = routes.into_router().with_state(Arc::new(AppDeps::new(AuthMode::Cookies)));
        let resp = send(app, Request::get("/extra").body(Body::empty()).unwrap()).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
