//! 도서·만화 수정/삭제 라우트: 임시 파일, 실제 관리자와 fake ES로 검증한다.
mod common;
use backend::{
    book_manager::{BookManager, BookManagerDeps},
    main::{AppDeps, AuthMode, BookManagerApi, CategoryMappingApi, MappingError, app_with},
};
use common::{TestClient, es_fake::FakeEsManager};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Policy(Mutex<Vec<Value>>);
impl CategoryMappingApi for Policy {
    fn rename_category(&self, old: &str, new: &str, ct: &str) -> Result<bool, MappingError> {
        self.0.lock().unwrap().push(json!(["rename", old, new, ct]));
        Ok(false)
    }
    fn delete_category(&self, category: &str, ct: &str, prefix: bool) -> Result<bool, MappingError> {
        self.0.lock().unwrap().push(json!(["delete", category, ct, prefix]));
        Ok(true)
    }
    fn get_hidden_categories(&self, _: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec!["A/child".into(), "AB".into()])
    }
    fn get_latest_excluded_categories(&self, _: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec!["A/old".into(), "AB".into()])
    }
    fn set_hidden(&self, category: &str, value: bool, ct: &str) -> Result<bool, MappingError> {
        self.0.lock().unwrap().push(json!(["hidden", category, value, ct]));
        Ok(true)
    }
    fn set_latest_excluded(&self, category: &str, value: bool, ct: &str) -> Result<bool, MappingError> {
        self.0.lock().unwrap().push(json!(["excluded", category, value, ct]));
        Ok(true)
    }
}
fn doc() -> Value {
    json!({"category":"A","title":"book","author":"author","file_path":"A/book.txt","file_type":"txt","file_size":4,"updated_time":"2026-10-25"})
}
fn client(root: &Path, es: Arc<FakeEsManager>, mapping: Arc<Policy>, comics: bool) -> TestClient {
    common::setup_env();
    let managers = BookManagerDeps::new(root.into(), es);
    let deps = AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_category_mapping(mapping);
    let deps =
        if comics { deps.with_comics_manager(Arc::new(backend::comics_manager::ComicsManager::with_deps(managers))) } else { deps.with_book_manager(Arc::new(BookManager::with_deps(managers))) };
    TestClient::with_app(app_with(deps))
}

#[tokio::test]
async fn delete_book_removes_file_and_reports_already_missing_file() {
    for (prefix, comics) in [("", false), ("/comics", true)] {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("A")).unwrap();
        let file = tmp.path().join("A/book.txt");
        std::fs::write(&file, b"book").unwrap();
        let es = FakeEsManager::with_doc(doc());
        let client = client(tmp.path(), es.clone(), Arc::new(Policy::default()), comics);
        assert_eq!(client.delete(&format!("{prefix}/books/7")).await.json(), json!({"status":"success","result":"Ok"}));
        assert!(!file.exists());
        assert_eq!(es.calls_of("delete"), vec![json!([7])]);
        let response = client.delete(&format!("{prefix}/books/7")).await;
        assert_eq!(response.json()["status"], "success");
        assert_eq!(response.json()["result"], "Warning");
        assert!(response.json()["warning"].as_str().unwrap().contains("이미 삭제"));
        es.fail("delete", "ES failed");
        assert_eq!(client.delete(&format!("{prefix}/books/7")).await.json()["status"], "failure");
    }
}

struct Update(Mutex<Vec<Value>>);
#[async_trait::async_trait]
impl BookManagerApi for Update {
    fn path_prefix(&self) -> PathBuf {
        "/tmp/update-library".into()
    }
    async fn update_book(&self, id: i64, category: &str, title: &str, author: &str, path: &Path, kind: &str, force: bool) -> Result<(), String> {
        self.0.lock().unwrap().push(json!([id, category, title, author, path, kind, force]));
        if id == 9 { Err("update failed".into()) } else { Ok(()) }
    }
}
#[tokio::test]
async fn update_book_uses_path_id_root_and_force_with_body_validation() {
    let manager = Arc::new(Update(Mutex::new(vec![])));
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager(manager.clone())));
    let mut body = doc();
    body["book_id"] = json!(999);
    assert_eq!(client.put_json("/books/7?force=true", &body).await.json(), json!({"status":"success","result":"Ok"}));
    assert_eq!(manager.0.lock().unwrap()[0], json!([7, "A", "book", "author", "/tmp/update-library/A/book.txt", "txt", true]));
    assert_eq!(client.put_json("/books/9", &body).await.json(), json!({"status":"failure","error":"update failed"}));
    assert_eq!(client.put_json("/books/7?force=invalid", &body).await.status, 422);
    for key in ["book_id", "file_size", "updated_time", "title"] {
        let mut invalid = body.clone();
        invalid.as_object_mut().unwrap().remove(key);
        assert_eq!(client.put_json("/books/7", &invalid).await.status, 422, "{key}");
    }
    assert_eq!(manager.0.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn rename_category_moves_directory_and_reports_mapping_result() {
    for (prefix, kind, comics) in [("", "book", false), ("/comics", "comic", true)] {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("A")).unwrap();
        std::fs::write(tmp.path().join("A/book.txt"), b"book").unwrap();
        let policy = Arc::new(Policy::default());
        let client = client(tmp.path(), FakeEsManager::new(), policy.clone(), comics);
        let response = client.put_json(&format!("{prefix}/categories/rename"), &json!({"old_category":"A","new_category":"B"})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["status"], "success");
        assert_eq!(response.json()["result"]["mapping_updated"], false);
        assert!(tmp.path().join("B/book.txt").exists());
        assert!(!tmp.path().join("A").exists());
        assert_eq!(policy.0.lock().unwrap().as_slice(), &[json!(["rename", "A", "B", kind])]);
        let response = client.put_json(&format!("{prefix}/categories/rename"), &json!({"old_category":"_root","new_category":"C"})).await;
        assert_eq!(response.json()["status"], "failure");
        assert_eq!(policy.0.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn category_delete_preserves_files_by_default_and_cleans_only_descendant_policy() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("A")).unwrap();
    let file = tmp.path().join("A/book.txt");
    std::fs::write(&file, b"book").unwrap();
    let policy = Arc::new(Policy::default());
    let client = client(tmp.path(), FakeEsManager::new(), policy.clone(), false);
    let response = client.post_json("/categories/delete", &json!({"category":"A"})).await;
    assert_eq!(response.json()["status"], "success");
    assert_eq!(response.json()["result"]["mapping_deleted"], true);
    assert!(file.exists());
    assert_eq!(
        policy.0.lock().unwrap().as_slice(),
        &[
            json!(["delete", "A", "book", true]),
            json!(["hidden", "A", false, "book"]),
            json!(["hidden", "A/child", false, "book"]),
            json!(["excluded", "A", false, "book"]),
            json!(["excluded", "A/old", false, "book"])
        ]
    );
    let response = client.post_json("/categories/delete", &json!({"category":"A","delete_files":true})).await;
    assert_eq!(response.json()["result"]["fs_deleted"], true);
    assert!(!file.exists());
}

#[tokio::test]
async fn category_preview_and_pdf_stats_use_real_manager_and_mask_stats_errors() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("A/child")).unwrap();
    std::fs::write(tmp.path().join("A/book.txt"), b"book").unwrap();
    let es = FakeEsManager::new();
    es.with(|s| s.category_pdf_stats = json!({"file_count":2,"page_count":100}));
    let client = client(tmp.path(), es.clone(), Arc::new(Policy::default()), false);
    let response = client.get("/category-delete-preview?category=A").await;
    assert_eq!(response.json(), json!({"status":"success","result":{"category":"A","directory_count":1,"file_count":1,"es_count":1}}));
    assert_eq!(client.get("/category-pdf-stats?category=A").await.json(), json!({"status":"success","result":{"file_count":2,"page_count":100}}));
    assert_eq!(client.get("/category-pdf-stats?category=_root").await.status, 400);
    assert_eq!(client.get("/category-pdf-stats").await.status, 422);
    es.fail("search_category_pdf_stats", "private details");
    assert_eq!(client.get("/category-pdf-stats?category=A").await.json(), json!({"status":"failure","error":"PDF 통계를 불러오지 못했습니다."}));
}

#[tokio::test]
async fn category_named_rename_remains_readable_after_admin_route_registration() {
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    es.with(|s| s.category_docs = vec![(1, doc(), 0.0)]);
    let client = client(tmp.path(), es, Arc::new(Policy::default()), false);
    for category in ["rename", "delete"] {
        assert_eq!(client.get(&format!("/categories/{category}")).await.json()["status"], "success");
    }
}

#[tokio::test]
async fn real_update_rejects_a_destination_outside_the_library() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("protected.txt");
    std::fs::write(&target, b"keep").unwrap();
    let es = FakeEsManager::with_doc(doc());
    let client = client(tmp.path(), es.clone(), Arc::new(Policy::default()), false);
    let mut body = doc();
    body["book_id"] = json!(7);
    body["file_path"] = json!(target);
    let response = client.put_json("/books/7?force=true", &body).await;
    assert_eq!(response.json(), json!({"status":"failure","error":"잘못된 경로입니다"}));
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    assert!(es.calls_of("delete").is_empty());
}

#[tokio::test]
async fn admin_operations_reject_real_viewer_token_before_touching_managers() {
    common::setup_env();
    let token = backend::auth::create_jwt_token("viewer@test.com", "viewer", None, None);
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies))).bearer(&token);
    for prefix in ["", "/comics"] {
        for (method, path, body) in [
            (axum::http::Method::PUT, "/books/1", doc()),
            (axum::http::Method::DELETE, "/books/1", Value::Null),
            (axum::http::Method::PUT, "/categories/rename", json!({"old_category":"A","new_category":"B"})),
            (axum::http::Method::POST, "/categories/delete", json!({"category":"A"})),
            (axum::http::Method::GET, "/category-delete-preview?category=A", Value::Null),
            (axum::http::Method::GET, "/category-pdf-stats?category=A", Value::Null),
        ] {
            assert_eq!(client.request(method, &format!("{prefix}{path}"), Some("application/json"), serde_json::to_vec(&body).unwrap()).await.status, 403, "{prefix}{path}");
        }
    }
}
