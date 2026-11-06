//! axum HTTP 계층: 라우트와 핸들러를 모아 `Router`를 조립한다. 서버 실행은 `bin/backend.rs`가 한다.
//! 설계: docs/rust-test-seams.md 3.2
//!
//! # 구성
//! - `app_deps.rs`: `AppDeps`(주입 묶음), `AuthMode`. `app_capabilities.rs`: 주입하는 trait와 실제 구현.
//! - `route_registry.rs`: 라우트 등록부 `Routes`. `request_extractors.rs`: 인증·JSON·쿼리 추출기. `error_responses.rs`: 오류 응답과 로깅.
//! - `routes_*.rs`, `auth_routes.rs`, `log_routes.rs`: 자원별 라우트. `cors_layer.rs`, `auth_cookies.rs`: 공용 도우미. Google ID 토큰 검증은 `auth_manager::google_token_verifier`에 있다.
//!
//! # 라우트를 붙이는 방법
//! 1. 라우트가 부르는 도메인 이름으로 `src/api_router/<도메인>_routes.rs`를 만들고 `pub(super) fn routes() -> Routes`를 하나 둔다.
//!    책 도메인(`book_manager`) 라우트는 `book_routes/<모듈>_routes.rs`에 둔다.
//!    핸들러는 `State<Arc<AppDeps>>`로 의존성을 받는다(`deps.book_manager()`, `deps.clock()` 등 접근자).
//!    인증은 핸들러 첫 인자에 `RequireAuth`/`RequireAdmin`/`OptionalAuth` 추출기를 두고, 등록 때 같은 이름을 `&["require_admin"]`처럼 적는다.
//! 2. 이 파일에 `mod <도메인>_routes;`를 더하고 `all_routes()`에 `.merge(<도메인>_routes::routes())` 한 줄을 더한다.
//! 3. 오류는 `Result<_, ApiError>`로 돌려준다(`ApiError::http(404, "..")`). 새 의존성이 필요하면 `AppDeps`에 필드, `with_xxx`, 접근자를 더한다.
//!
//! 앱 안의 로그는 스레드 로컬 수집(`LogCapture`)에 잡히도록 핸들러 안에서 `spawn`하지 않는다. 오래 걸리는 일은 `deps.task_runner()`로 넘긴다.

use axum::Router;
use axum::middleware::from_fn;
use std::sync::Arc;

mod app_capabilities;
mod app_deps;
mod auth_cookies;
mod auth_routes;
mod book_routes;
mod bookstore_routes;
mod category_mapping_routes;
mod cors_layer;
mod cover_routes;
mod error_responses;
mod log_routes;
mod request_extractors;
mod route_registry;
mod view_history_routes;
mod wake_routes;

pub use app_capabilities::{BookAccess, BookStub, CategoryMappingApi, MappingError, UnavailableBookAccess, ViewHistoryApi};
pub use app_deps::{AppDeps, AuthMode, AuthSlots, BookstoreFactory, RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS, Slot};
pub use auth_cookies::{CustomJSONResponse, ResponseHeaders, clear_auth_cookies, custom_jsonable_encoder, get_cookie_settings, is_local_frontend_origin, set_auth_cookies};
pub use auth_manager::auth_service::{ACCESS_COOKIE_NAME, ACCESS_TOKEN_EXPIRATION_SECONDS, HttpStatusError, REFRESH_COOKIE_NAME, REFRESH_TOKEN_EXPIRATION_SECONDS, RoleResolver, observation_hash};
pub use auth_manager::google_token_verifier::{GoogleIdTokenVerifier, GoogleJwksFetcher, GoogleVerifier, UnavailableGoogleVerifier};
pub use book_manager::{ApplyCategoryChanges, BookContent, BookManagerApi, BookResponse, ProposeCategoryChanges, ReloadProgress, UnavailableBookManager};
pub use bookstore::store_clients::BookstoreSearch;
pub use db::view_history_store::MAX_RECENT_VIEWS;
pub use error_responses::{
    ApiError, ApiResult, GENERIC_SERVER_ERROR_DETAIL, RequestValidationError, SERVICE_UNAVAILABLE_DETAIL, VALIDATION_ERROR_DETAIL, general_exception_handler, http_exception_handler, json_response,
    summarize_request_body, validation_exception_handler,
};
pub use log_routes::{CLIENT_ERROR_FIELD_LIMITS, ClientErrorLogModel};
pub(crate) use request_extractors::parse_py_bool;
pub use request_extractors::{ApiJson, ApiQuery, OptionalAuth, RequireAdmin, RequireAuth};
pub use request_extractors::{ClientInfo, client_ip, request_ip_prefix, request_user_agent};
pub use route_registry::{RouteInfo, Routes};
pub use sysdeps::task_runner::{InlineRunner, TaskHandle, TaskRunner, ThreadRunner};

/// 앱의 모든 라우트. 새 라우트 파일은 여기에 `.merge(..)` 한 줄로 붙인다.
fn all_routes() -> Routes {
    Routes::new()
        .merge(auth_routes::routes())
        .merge(log_routes::routes())
        .merge(view_history_routes::routes())
        .merge(book_routes::routes())
        .merge(bookstore_routes::routes())
        .merge(wake_routes::routes())
        .merge(category_mapping_routes::routes())
        .merge(cover_routes::routes())
}

/// 환경 변수로 조립한 운영용 앱.
pub fn app() -> Router {
    app_with(AppDeps::from_env())
}

/// 주입한 의존성으로 앱을 만든다. 테스트는 `AppDeps::new(..).with_xxx(..)`로 대역을 끼운다.
pub fn app_with(deps: AppDeps) -> Router {
    all_routes().into_router().fallback(error_responses::not_found).with_state(Arc::new(deps)).layer(from_fn(error_responses::error_middleware)).layer(from_fn(cors_layer::cors_middleware))
}

/// 등록된 라우트 목록(경로, 메서드, 인증 의존성).
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

/// 분류 적용 백그라운드 작업의 실패 상태를 저장한다.
pub async fn run_classify_apply_job(
    book_manager: &dyn BookManagerApi,
    category_mapping: &dyn CategoryMappingApi,
    content_type: &str,
    items: Vec<serde_json::Value>,
    allowed: std::collections::HashSet<String>,
    clean_existing: bool,
    _apply_token: &str,
) {
    if let Err(error) = book_manager.apply_category_changes(&items, &allowed, clean_existing).await {
        let mut state = match category_mapping.get_classify_proposal_status(content_type, 300) {
            Ok(state) => state,
            Err(error) => {
                log::error!("분류 적용 실패 상태 조회 실패: {error}");
                return;
            }
        };
        state["status"] = serde_json::json!("failed");
        state["error"] = serde_json::json!(error);
        if let Err(save_error) = category_mapping.set_classify_proposal_status(state, content_type) {
            log::error!("분류 적용 실패 상태 저장 실패: {save_error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auth_manager::auth_service::Claims;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn send(app: Router, req: Request<Body>) -> axum::response::Response {
        app.oneshot(req).await.unwrap()
    }

    fn plain_app() -> Router {
        app_with(AppDeps::new(AuthMode::Fixed(Claims { email: "a@example.com".into(), role: "admin".into(), ..Claims::default() })))
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
        let app = routes.into_router().with_state(Arc::new(AppDeps::new(AuthMode::Cookies))).layer(from_fn(error_responses::error_middleware));
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
