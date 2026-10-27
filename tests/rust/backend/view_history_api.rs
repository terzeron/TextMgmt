//! 원본: tests/test_view_history_api.py
//!
//! 조회 이력 엔드포인트 테스트. 스토어는 대역으로 두고 라우팅, 인증, 스냅샷 규칙을 본다.
//! 실제 SQL 동작은 `view_history_store.rs`가 MySQL 컨테이너로 검증한다.
//!
//! 가정한 API (아직 구현 전이다. 설계: docs/rust-test-seams.md 3.2, 3.4, 3.13):
//! - `backend::main::MAX_RECENT_VIEWS: usize`
//! - `backend::main::app_with(deps: AppDeps) -> axum::Router`: FastAPI `dependency_overrides`와 `monkeypatch.setattr(main_mod, ...)`의 대응 seam
//! - `backend::main::AppDeps::new(auth: AuthMode) -> AppDeps` (나머지는 실제 구현), 소비형 빌더:
//!   `with_view_history_store(Arc<dyn ViewHistoryApi>)`, `with_book_access(Arc<dyn BookAccess>)`, `with_task_runner(Arc<dyn TaskRunner>)`
//!   (구 `AppDeps { view_history_store, book_access, auth_override, admin_override, ..Default::default() }` 리터럴)
//! - `backend::main::AuthMode::{Split { auth: Value, admin: Value }, AuthOnly(Value)}`
//!   - `Split`은 `require_auth`와 `require_admin`을 각각 고정 payload로 바꾼다(구 `auth_override`, `admin_override`)
//!   - `AuthOnly`는 `require_auth`만 고정하고 `require_admin`은 실제 의존성이다
//! - `backend::main::{TaskRunner, InlineRunner}`: `InlineRunner`는 백그라운드 작업을 응답 전에 같은 스레드에서 끝낸다. 스레드 로컬 `LogCapture`가 로그를 잡으려면 필요하다
//! - `backend::main::ViewHistoryApi` (뼈대가 `main/traits.rs`에 둔다):
//!   `record_view(&self, email: &str, content_type: &str, book_id: i64, title: &str, category: &str) -> Result<(), ViewHistoryError>`,
//!   `list_recent_views(&self, limit: usize) -> Result<Value, ViewHistoryError>` (`ViewHistoryStore`도 구현한다)
//! - `ViewHistoryStore::get_connection(&self) -> Result<Box<dyn DbConnection>, DbError>` (구 `_get_connection() -> mysql::Conn`), `DbConnection::execute(&mut self, sql, &[DbValue]) -> Result<u64, DbError>`
//! - `backend::main::BookAccess` (Python `_get_book_and_ensure_viewer_access`):
//!   `get_book_and_ensure_viewer_access(&self, book_id: i64, payload: &Value, content_type: &str) -> Result<(Option<BookStub>, Option<String>), HttpException>`
//! - `backend::main::BookStub { title: String, category: Option<String> }`, `backend::main::HttpException { status_code: u16, detail: String }`
//! - `backend::main::route_dependencies(path: &str) -> Vec<&'static str>`: 라우트에 붙은 의존성 이름 (`route.dependant.dependencies` 대응)
//! - 인증 없는 요청은 401을 돌려준다.
//! - 로그는 `log::warn!`로 남기고 `common::log_capture::LogCapture`로 수집한다.

use crate::{common, mysql_fixture};

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::http::Method;
use backend::main::{AppDeps, AuthMode, BookAccess, BookStub, HttpException, InlineRunner, MAX_RECENT_VIEWS, ViewHistoryApi, app_with, route_dependencies};
use backend::view_history_store::{ViewHistoryError, ViewHistoryStore};
use common::TestClient;
use common::log_capture::LogCapture;
use log::Level;
use serde_json::{Value, json};

fn payload(email: &str, role: &str, name: &str) -> Value {
    let exp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600;
    json!({"email": email, "role": role, "name": name, "picture": "", "exp": exp})
}

fn admin_payload() -> Value {
    payload("admin@test.com", "admin", "Admin")
}

fn viewer_payload() -> Value {
    payload("viewer@test.com", "viewer", "Viewer")
}

/// `_FakeStore`: `record_view` 호출을 그대로 모아두는 스토어 대역.
struct FakeStore {
    recorded: Mutex<Vec<Value>>,
    list_result: Mutex<Value>,
    list_calls: Mutex<Vec<usize>>,
    record_error: Mutex<Option<String>>,
    list_error: Mutex<Option<String>>,
}

impl FakeStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            recorded: Mutex::new(Vec::new()),
            list_result: Mutex::new(json!({"limit": 50, "users": []})),
            list_calls: Mutex::new(Vec::new()),
            record_error: Mutex::new(None),
            list_error: Mutex::new(None),
        })
    }

    fn recorded(&self) -> Vec<Value> {
        self.recorded.lock().unwrap().clone()
    }
}

impl ViewHistoryApi for FakeStore {
    fn record_view(&self, email: &str, content_type: &str, book_id: i64, title: &str, category: &str) -> Result<(), ViewHistoryError> {
        if let Some(message) = self.record_error.lock().unwrap().clone() {
            return Err(ViewHistoryError::RuntimeError(message));
        }
        self.recorded.lock().unwrap().push(json!({"email": email, "content_type": content_type, "book_id": book_id, "title": title, "category": category}));
        Ok(())
    }

    fn list_recent_views(&self, limit: usize) -> Result<Value, ViewHistoryError> {
        self.list_calls.lock().unwrap().push(limit);
        if let Some(message) = self.list_error.lock().unwrap().clone() {
            return Err(ViewHistoryError::RuntimeError(message));
        }
        Ok(self.list_result.lock().unwrap().clone())
    }
}

type AccessFn = dyn Fn(i64, &Value, &str) -> Result<(Option<BookStub>, Option<String>), HttpException> + Send + Sync;

/// `_get_book_and_ensure_viewer_access` 대역. 클로저가 결과를 정한다.
struct FnAccess(Box<AccessFn>);

#[async_trait]
impl BookAccess for FnAccess {
    async fn get_book_and_ensure_viewer_access(&self, book_id: i64, payload: &Value, content_type: &str) -> Result<(Option<BookStub>, Option<String>), HttpException> {
        (self.0)(book_id, payload, content_type)
    }
}

/// `_make_book`
fn make_book(title: &str, category: &str) -> BookStub {
    BookStub { title: title.to_string(), category: Some(category.to_string()) }
}

/// `allow_book` fixture: 책 조회와 viewer 접근 검사를 통과시키고 호출 인자를 기록한다.
fn allow_book() -> (Arc<dyn BookAccess>, Arc<Mutex<Vec<Value>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sink = calls.clone();
    let access = FnAccess(Box::new(move |book_id, payload, content_type| {
        sink.lock().unwrap().push(json!({"book_id": book_id, "payload": payload, "content_type": content_type}));
        Ok((Some(make_book("원래 제목", "소설")), None))
    }));
    (Arc::new(access), calls)
}

/// `admin_client` fixture: `require_auth`와 `require_admin`을 admin payload로 고정한다.
fn admin_client(store: Arc<dyn ViewHistoryApi>, access: Option<Arc<dyn BookAccess>>) -> TestClient {
    let mut deps = AppDeps::new(AuthMode::Split { auth: admin_payload(), admin: admin_payload() }).with_view_history_store(store);
    if let Some(access) = access {
        deps = deps.with_book_access(access);
    }
    TestClient::with_app(app_with(deps))
}

mod test_record_book_view {
    use super::*;

    #[tokio::test]
    async fn test_records_book_view_with_server_side_snapshot() {
        let store = FakeStore::new();
        let (access, _calls) = allow_book();
        let client = admin_client(store.clone(), Some(access));

        let resp = client.request(Method::POST, "/books/view-history/42", None, Vec::new()).await;

        assert_eq!(resp.status, 200);
        assert_eq!(resp.json()["result"]["recorded"], json!(true));
        assert_eq!(store.recorded(), vec![json!({"email": "admin@test.com", "content_type": "book", "book_id": 42, "title": "원래 제목", "category": "소설"})]);
    }

    #[tokio::test]
    async fn test_comics_route_records_comic_type() {
        let store = FakeStore::new();
        let (access, _calls) = allow_book();
        let client = admin_client(store.clone(), Some(access));

        let resp = client.request(Method::POST, "/comics/view-history/7", None, Vec::new()).await;

        assert_eq!(resp.status, 200);
        assert_eq!(store.recorded()[0]["content_type"], json!("comic"));
        assert_eq!(store.recorded()[0]["book_id"], json!(7));
    }

    #[tokio::test]
    async fn test_ignores_client_supplied_title() {
        // 제목은 서버 레코드에서만 온다. 본문으로 위조한 값은 무시된다.
        let store = FakeStore::new();
        let (access, _calls) = allow_book();
        let client = admin_client(store.clone(), Some(access));

        let resp = client.post_json("/books/view-history/42", &json!({"title": "위조된 제목", "category": "위조 카테고리"})).await;

        assert_eq!(resp.status, 200);
        assert_eq!(store.recorded()[0]["title"], json!("원래 제목"));
        assert_eq!(store.recorded()[0]["category"], json!("소설"));
    }

    #[tokio::test]
    async fn test_enforces_viewer_access_check() {
        let store = FakeStore::new();
        let (access, calls) = allow_book();
        let client = admin_client(store.clone(), Some(access));

        client.request(Method::POST, "/books/view-history/42", None, Vec::new()).await;

        // 기록 전에 접근 검사를 반드시 거친다
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0]["book_id"], json!(42));
        assert_eq!(calls[0]["content_type"], json!("book"));
    }

    #[tokio::test]
    async fn test_viewer_without_access_is_rejected_and_nothing_recorded() {
        let store = FakeStore::new();
        let forbidden = FnAccess(Box::new(|_, _, _| Err(HttpException { status_code: 403, detail: "Access denied".to_string() })));
        let deps = AppDeps::new(AuthMode::AuthOnly(viewer_payload())).with_view_history_store(store.clone()).with_book_access(Arc::new(forbidden));
        let client = TestClient::with_app(app_with(deps));

        let resp = client.request(Method::POST, "/books/view-history/42", None, Vec::new()).await;

        assert_eq!(resp.status, 403);
        assert!(store.recorded().is_empty());
    }

    #[tokio::test]
    async fn test_unknown_book_returns_404_and_records_nothing() {
        let store = FakeStore::new();
        let missing = FnAccess(Box::new(|_, _, _| Ok((None, Some("Book not found".to_string())))));
        let client = admin_client(store.clone(), Some(Arc::new(missing)));

        let resp = client.request(Method::POST, "/books/view-history/999", None, Vec::new()).await;

        assert_eq!(resp.status, 404);
        assert!(store.recorded().is_empty());
    }

    #[tokio::test]
    async fn test_requires_authentication() {
        // dependency_overrides를 비운 실제 앱이다.
        let client = TestClient::new();
        assert_eq!(client.request(Method::POST, "/books/view-history/1", None, Vec::new()).await.status, 401);
        assert_eq!(client.request(Method::POST, "/comics/view-history/1", None, Vec::new()).await.status, 401);
    }

    #[tokio::test]
    async fn test_store_failure_does_not_break_viewing() {
        // 이력 기록 실패는 열람을 막지 않지만 조용히 넘기지도 않는다.
        let store = FakeStore::new();
        *store.record_error.lock().unwrap() = Some("db down".to_string());
        let (access, _calls) = allow_book();
        // 스레드 로컬 LogCapture가 경고를 잡도록 InlineRunner로 같은 스레드에서 돌린다.
        let deps =
            AppDeps::new(AuthMode::Split { auth: admin_payload(), admin: admin_payload() }).with_view_history_store(store.clone()).with_book_access(access).with_task_runner(Arc::new(InlineRunner));
        let client = TestClient::with_app(app_with(deps));

        let logs = LogCapture::start(Level::Warn);
        let resp = client.request(Method::POST, "/books/view-history/42", None, Vec::new()).await;

        assert_eq!(resp.status, 200);
        assert_eq!(resp.json()["result"]["recorded"], json!(false));
        let text = logs.text();
        assert!(text.contains("조회 이력 기록 실패"), "{text}");
    }

    #[tokio::test]
    async fn test_rejects_non_integer_book_id() {
        let store = FakeStore::new();
        let (access, _calls) = allow_book();
        let client = admin_client(store.clone(), Some(access));

        assert_eq!(client.request(Method::POST, "/books/view-history/abc", None, Vec::new()).await.status, 422);
        assert!(store.recorded().is_empty());
    }
}

mod test_list_view_history {
    use super::*;

    #[tokio::test]
    async fn test_returns_store_result() {
        let store = FakeStore::new();
        *store.list_result.lock().unwrap() =
            json!({"limit": 50, "users": [{"email": "u@example.com", "last_viewed_at": 100, "book": [{"book_id": 1, "title": "책", "category": "소설", "viewed_at": 100}], "comic": []}]});
        let client = admin_client(store.clone(), None);

        let resp = client.get("/view-history").await;

        assert_eq!(resp.status, 200);
        let body = resp.json();
        assert_eq!(body["status"], json!("success"));
        assert_eq!(body["result"]["users"][0]["email"], json!("u@example.com"));
        assert_eq!(body["result"]["users"][0]["book"][0]["title"], json!("책"));
    }

    #[tokio::test]
    async fn test_default_limit_is_max_recent_views() {
        let store = FakeStore::new();
        let client = admin_client(store.clone(), None);

        client.get("/view-history").await;

        assert_eq!(store.list_calls.lock().unwrap()[0], MAX_RECENT_VIEWS);
    }

    #[tokio::test]
    async fn test_limit_param_is_passed_through() {
        let store = FakeStore::new();
        let client = admin_client(store.clone(), None);

        client.get("/view-history?limit=5").await;

        assert_eq!(store.list_calls.lock().unwrap()[0], 5);
    }

    #[tokio::test]
    async fn test_rejects_out_of_range_limit() {
        let store = FakeStore::new();
        let client = admin_client(store.clone(), None);

        assert_eq!(client.get("/view-history?limit=0").await.status, 422);
        assert_eq!(client.get(&format!("/view-history?limit={}", MAX_RECENT_VIEWS + 1)).await.status, 422);
        assert_eq!(client.get(&format!("/view-history?limit={MAX_RECENT_VIEWS}")).await.status, 200);
    }

    #[tokio::test]
    async fn test_store_failure_returns_503() {
        let store = FakeStore::new();
        *store.list_error.lock().unwrap() = Some("db down".to_string());
        let client = admin_client(store.clone(), None);

        assert_eq!(client.get("/view-history").await.status, 503);
    }

    #[tokio::test]
    async fn test_requires_admin() {
        let store = FakeStore::new();
        // require_admin 은 override 하지 않아 실제 의존성이 동작한다
        let deps = AppDeps::new(AuthMode::AuthOnly(viewer_payload())).with_view_history_store(store.clone());
        let client = TestClient::with_app(app_with(deps));

        let status = client.get("/view-history").await.status;
        assert!(status == 401 || status == 403, "상태 코드: {status}");
    }

    #[tokio::test]
    async fn test_unauthenticated_is_rejected() {
        // dependency_overrides를 비운 실제 앱이다. 인증 검사가 스토어보다 먼저라 스토어 대역은 필요 없다.
        let client = TestClient::new();
        assert_eq!(client.get("/view-history").await.status, 401);
    }

    #[test]
    fn test_declares_admin_dependency() {
        let dependencies = route_dependencies("/view-history");
        assert!(dependencies.contains(&"require_admin"), "{dependencies:?}");
    }
}

/// 엔드포인트가 실제 MySQL 스토어에 쓰고, admin 조회로 되읽히는지 확인한다.
///
/// 책 조회만 대역으로 두고 스토어는 진짜를 쓴다(스토어 대역 테스트와 실제 SQL 테스트
/// 사이의 배선 공백을 닫는다).
// DB 잠금은 테스트 전체 동안 잡아야 한다. 테스트마다 런타임과 스레드가 따로라서 await 중에 잡고 있어도 교착하지 않는다.
#[allow(clippy::await_holding_lock)]
mod test_end_to_end_with_real_store {
    use super::*;

    /// `real_store` fixture. 컨테이너를 못 띄우면 panic한다.
    async fn real_store() -> (Arc<ViewHistoryStore>, std::sync::MutexGuard<'static, ()>) {
        // testcontainers의 blocking 러너는 tokio 런타임 안에서 직접 부를 수 없다.
        tokio::task::spawn_blocking(mysql_fixture::mysql_container).await.expect("컨테이너 기동");
        let guard = mysql_fixture::db_lock();
        let store = ViewHistoryStore::new().expect("스토어 연결");
        let mut conn = store.get_connection().expect("연결");
        conn.execute("TRUNCATE TABLE view_history", &[]).expect("TRUNCATE");
        (Arc::new(store), guard)
    }

    #[tokio::test]
    async fn test_recorded_view_is_visible_to_admin() {
        let (store, _guard) = real_store().await;
        let access = FnAccess(Box::new(|book_id, _, content_type| {
            let title = if content_type == "comic" { "만화 제목" } else { "책 제목" };
            Ok((Some(make_book(&format!("{title} {book_id}"), "분류")), None))
        }));
        let client = admin_client(store.clone(), Some(Arc::new(access)));

        assert_eq!(client.request(Method::POST, "/books/view-history/1", None, Vec::new()).await.json()["result"]["recorded"], json!(true));
        assert_eq!(client.request(Method::POST, "/comics/view-history/2", None, Vec::new()).await.json()["result"]["recorded"], json!(true));
        // 같은 책 재조회는 중복 row 를 만들지 않는다
        assert_eq!(client.request(Method::POST, "/books/view-history/1", None, Vec::new()).await.json()["result"]["recorded"], json!(true));

        let result = client.get("/view-history").await.json()["result"].clone();

        assert_eq!(result["limit"], json!(MAX_RECENT_VIEWS));
        assert_eq!(result["users"].as_array().unwrap().len(), 1);
        let user = &result["users"][0];
        assert_eq!(user["email"], json!("admin@test.com"));
        let book_titles: Vec<&str> = user["book"].as_array().unwrap().iter().map(|item| item["title"].as_str().unwrap()).collect();
        let comic_titles: Vec<&str> = user["comic"].as_array().unwrap().iter().map(|item| item["title"].as_str().unwrap()).collect();
        assert_eq!(book_titles, vec!["책 제목 1"]);
        assert_eq!(comic_titles, vec!["만화 제목 2"]);
    }

    #[tokio::test]
    async fn test_limit_narrows_returned_items() {
        let (store, _guard) = real_store().await;
        let access = FnAccess(Box::new(|book_id, _, _| Ok((Some(make_book(&format!("책 {book_id}"), "분류")), None))));
        let client = admin_client(store.clone(), Some(Arc::new(access)));
        for book_id in 0..4 {
            client.request(Method::POST, &format!("/books/view-history/{book_id}"), None, Vec::new()).await;
        }

        let result = client.get("/view-history?limit=2").await.json()["result"].clone();

        assert_eq!(result["users"][0]["book"].as_array().unwrap().len(), 2);
    }
}
