//! backend 테스트 공용 도구.
//!
//! 가정한 backend API (아직 구현 전이다):
//! - `backend::api_router::app() -> axum::Router`
//!
//! 각 테스트 파일은 `mod common;`으로 가져온다. 새 모듈은 `use common::clock::FakeClock;`처럼 경로로 쓴다.
//! (`use common::image;`로 모듈 자체를 들이면 `image` crate 이름과 겹치므로 항목을 직접 가져온다.)
//!
//! 모듈 설계: docs/rust-test-seams.md 3장(통일안)과 5장(중복 통합). `book_fixtures`, `cover_fixtures`, `mysql_fixture`는
//! 아직 `#[path = "common/xxx.rs"]`로 쓰는 기존 방식이며, 새 모듈로 옮기는 중이라 여기서 선언하지 않는다.
//! `es_dummy`, `log_capture`는 이제 이 모듈 아래에 있으므로 더는 `#[path]`로 가져오지 않는다(두 번 컴파일되면 로거와 스레드 로컬이 갈라진다).

#![allow(dead_code)]

pub mod auth_fakes;
pub mod category_apply;
pub mod clock;
pub mod cover_docs;
pub mod cover_env;
pub mod dyn_es;
pub mod edits;
pub mod env;
pub mod epub;
pub mod epub_book;
pub mod es_dummy;
pub mod es_fake;
pub mod faulty_fs;
pub mod file_reader_fakes;
pub mod fsx;
pub mod image;
pub mod log_capture;
pub mod pdf;
pub mod pdf_edit_env;
pub mod pdf_env;
pub mod preview_epubs;
pub mod raster;
pub mod runner;
pub mod session;
pub mod shared_fixtures;
pub mod zipx;

use axum::body::Body;
use axum::http::{Method, Request};
use serde_json::Value;
use tower::ServiceExt;

mod setup;

// 테스트 파일마다 쓰는 이름이 달라 일부가 쓰이지 않는다(`dead_code` 허용과 같은 이유).
#[allow(unused_imports)]
pub use setup::{ES_FROM_FIXTURE, MYSQL_FROM_FIXTURE, project_root, setup_env};

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("JSON 본문")
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// 앱에 요청을 보내는 테스트 클라이언트. 요청마다 새 `Router`를 만들지 않고 복제해서 쓴다.
pub struct TestClient {
    app: axum::Router,
    headers: Vec<(String, String)>,
}

impl TestClient {
    pub fn new() -> Self {
        setup_env();
        Self { app: backend::api_router::app(), headers: Vec::new() }
    }

    pub fn with_app(app: axum::Router) -> Self {
        setup_env();
        Self { app, headers: Vec::new() }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn bearer(self, token: &str) -> Self {
        self.header("authorization", &format!("Bearer {token}"))
    }

    pub fn cookie(self, name: &str, value: &str) -> Self {
        self.header("cookie", &format!("{name}={value}"))
    }

    pub async fn request(&self, method: Method, path: &str, content_type: Option<&str>, body: Vec<u8>) -> Response {
        let browser_mutation = !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);
        let mut builder = Request::builder().method(method).uri(path);
        for (k, v) in &self.headers {
            builder = builder.header(k, v);
        }
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        // 브라우저의 상태 변경 요청은 Origin을 보낸다. 출처 누락 검사는 raw Request로 한다.
        if browser_mutation
            && !self.headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("origin"))
            && let Ok(origin) = std::env::var("TM_FRONTEND_URL")
        {
            builder = builder.header("origin", origin);
        }
        let resp = self.app.clone().oneshot(builder.body(Body::from(body)).unwrap()).await.unwrap();
        let status = resp.status().as_u16();
        let headers = resp.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
        Response { status, headers, body }
    }

    pub async fn get(&self, path: &str) -> Response {
        self.request(Method::GET, path, None, Vec::new()).await
    }

    pub async fn delete(&self, path: &str) -> Response {
        self.request(Method::DELETE, path, None, Vec::new()).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Response {
        self.request(Method::POST, path, Some("application/json"), serde_json::to_vec(body).unwrap()).await
    }

    pub async fn put_json(&self, path: &str, body: &Value) -> Response {
        self.request(Method::PUT, path, Some("application/json"), serde_json::to_vec(body).unwrap()).await
    }

    pub async fn patch_json(&self, path: &str, body: &Value) -> Response {
        self.request(Method::PATCH, path, Some("application/json"), serde_json::to_vec(body).unwrap()).await
    }

    /// multipart/form-data 본문을 직접 받는다. `boundary`는 `content_type`과 본문이 같아야 한다.
    pub async fn post_multipart(&self, path: &str, boundary: &str, body: Vec<u8>) -> Response {
        let ct = format!("multipart/form-data; boundary={boundary}");
        self.request(Method::POST, path, Some(&ct), body).await
    }
}
