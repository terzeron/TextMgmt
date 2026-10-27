//! backend 테스트 공용 도구. Python `tests/conftest.py`의 환경 설정과 FastAPI `TestClient`에 대응한다.
//!
//! 가정한 backend API (아직 구현 전이다):
//! - `backend::main::app() -> axum::Router`
//!
//! 각 테스트 파일은 `mod common;`으로 가져온다. 새 모듈은 `use common::clock::FakeClock;`처럼 경로로 쓴다.
//! (`use common::image;`로 모듈 자체를 들이면 `image` crate 이름과 겹치므로 항목을 직접 가져온다.)
//!
//! 모듈 설계: docs/rust-test-seams.md 3장(통일안)과 5장(중복 통합). `book_fixtures`, `cover_fixtures`, `mysql_fixture`는
//! 아직 `#[path = "common/xxx.rs"]`로 쓰는 기존 방식이며, 새 모듈로 옮기는 중이라 여기서 선언하지 않는다.
//! `es_dummy`, `log_capture`는 이제 이 모듈 아래에 있으므로 더는 `#[path]`로 가져오지 않는다(두 번 컴파일되면 로거와 스레드 로컬이 갈라진다).

#![allow(dead_code)]

pub mod clock;
pub mod edits;
pub mod env;
pub mod epub;
pub mod es_dummy;
pub mod es_fake;
pub mod faulty_fs;
pub mod fsx;
pub mod image;
pub mod log_capture;
pub mod pdf;
pub mod raster;
pub mod runner;
pub mod zipx;

use std::path::PathBuf;
use std::sync::Once;

use axum::body::Body;
use axum::http::{Method, Request};
use serde_json::Value;
use tower::ServiceExt;

static INIT: Once = Once::new();

/// 저장소 루트. `tests/rust/backend/common`에서 세 단계 위가 아니라 `CARGO_MANIFEST_DIR` 기준이다.
pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().find(|p| p.join("Cargo.toml").exists() && p.join("backend").is_dir()).expect("project root").to_path_buf()
}

/// conftest.py와 같은 테스트용 환경 변수를 한 번만 설정한다.
/// `Book.path_prefix`처럼 import 시점에 읽는 값이 있어 backend를 쓰기 전에 불러야 한다.
/// 컨테이너 fixture 가 `TM_MYSQL_*` / `TM_ES_*` 를 컨테이너 값으로 설정했다는 표식. `setup_env()` 는 이 값을 지우지 않는다.
/// fixture 모듈(`mysql_fixture.rs`, `elasticsearch_fixture.rs`)도 같은 이름을 쓴다.
pub const MYSQL_FROM_FIXTURE: &str = "TM_TEST_MYSQL_FROM_FIXTURE";
pub const ES_FROM_FIXTURE: &str = "TM_TEST_ES_FROM_FIXTURE";

pub fn setup_env() {
    INIT.call_once(|| {
        let root = project_root();
        // SAFETY: Once 안에서 테스트 시작 전에 한 번만 설정한다.
        unsafe {
            std::env::set_var("TM_BOOK_DIR", root.join("tests/books"));
            std::env::set_var("TM_COMICS_DIR", root.join("tests/comics"));
            std::env::set_var("TM_FRONTEND_URL", "http://localhost:3000");
            std::env::set_var("TM_JWT_SECRET", "test_jwt_secret_for_testing_minimum_32bytes");
            std::env::set_var("TM_ADMIN_EMAIL", "admin@test.com");
            std::env::set_var("TM_ALLOWED_EMAILS", "viewer@test.com");
            if std::env::var_os("TM_ES_COMICS_INDEX").is_none() {
                std::env::set_var("TM_ES_COMICS_INDEX", "test_comics_index");
            }
            // 운영 모델 파일(165MB)을 읽지 않도록 없는 경로를 가리킨다. Python 은 TM_CLASSIFIER_MODEL(joblib),
            // Rust 는 TM_CLASSIFIER_MODEL_DIR(내보내기 디렉토리)을 읽는다.
            std::env::set_var("TM_CLASSIFIER_MODEL", root.join("tests/model-that-does-not-exist.joblib"));
            std::env::set_var("TM_CLASSIFIER_MODEL_DIR", root.join("tests/model-dir-that-does-not-exist"));
            // 개발 셸(direnv 의 `.env`)에 든 실서비스 접속 정보가 테스트에 새지 않게 지운다. 그대로 두면 테스트가
            // 운영 MySQL·Elasticsearch 에 붙어 데이터를 바꾼다. 컨테이너 fixture 가 먼저 설정한 값(표식이 있음)은 둔다.
            if std::env::var_os(MYSQL_FROM_FIXTURE).is_none() {
                for k in ["TM_MYSQL_HOST", "TM_MYSQL_PORT", "TM_MYSQL_USER", "TM_MYSQL_PASSWORD", "TM_MYSQL_DATABASE", "TM_MYSQL_ROOT_PASSWORD"] {
                    std::env::remove_var(k);
                }
            }
            if std::env::var_os(ES_FROM_FIXTURE).is_none() {
                for k in ["TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"] {
                    std::env::remove_var(k);
                }
            }
        }
    });
}

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

/// FastAPI `TestClient(app)`에 대응한다. 요청마다 새 `Router`를 만들지 않고 복제해서 쓴다.
pub struct TestClient {
    app: axum::Router,
    headers: Vec<(String, String)>,
}

impl TestClient {
    pub fn new() -> Self {
        setup_env();
        Self { app: backend::main::app(), headers: Vec::new() }
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
        let mut builder = Request::builder().method(method).uri(path);
        for (k, v) in &self.headers {
            builder = builder.header(k, v);
        }
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
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
