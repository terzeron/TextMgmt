//! HTTP 라우트와 앱 조립. 설계: docs/rust-test-seams.md 3.2

use axum::Router;

/// 환경 변수로 조립한 운영용 앱.
pub fn app() -> Router {
    Router::new()
}
