//! 카테고리 설정 API의 권한, 전달 인자, 실패 응답 및 blocking DB 경계를 검증한다.
mod common;
use backend::main::{AppDeps, AuthMode, CategoryMappingApi, MappingError, app_with};
use common::TestClient;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};

struct Mapping {
    calls: Mutex<Vec<(String, Value)>>,
    thread: ThreadId,
    success: bool,
    error: bool,
}
impl Mapping {
    fn new(success: bool, error: bool) -> Arc<Self> {
        Arc::new(Self { calls: Mutex::new(vec![]), thread: thread::current().id(), success, error })
    }
    fn call(&self, name: &str, args: Value) -> Result<(), MappingError> {
        assert_ne!(thread::current().id(), self.thread, "DB call blocked HTTP runtime: {name}");
        self.calls.lock().unwrap().push((name.into(), args));
        if self.error { Err(MappingError::Other("private SQL credentials".into())) } else { Ok(()) }
    }
}
impl CategoryMappingApi for Mapping {
    fn get_all_mappings(&self, ct: &str) -> Result<Value, MappingError> {
        self.call("get_all", json!(ct))?;
        Ok(json!({"fiction":["one"]}))
    }
    fn get_keywords(&self, category: &str, ct: &str) -> Result<Vec<String>, MappingError> {
        self.call("get_keywords", json!([category, ct]))?;
        Ok(vec!["one".into()])
    }
    fn set_keywords(&self, category: &str, keywords: &[String], ct: &str) -> Result<bool, MappingError> {
        self.call("set_keywords", json!([category, keywords, ct]))?;
        Ok(self.success)
    }
    fn add_keyword(&self, category: &str, keyword: &str, ct: &str) -> Result<bool, MappingError> {
        self.call("add_keyword", json!([category, keyword, ct]))?;
        Ok(self.success)
    }
    fn remove_keyword(&self, category: &str, keyword: &str, ct: &str) -> Result<bool, MappingError> {
        self.call("remove_keyword", json!([category, keyword, ct]))?;
        Ok(self.success)
    }
    fn delete_category(&self, category: &str, ct: &str, prefix: bool) -> Result<bool, MappingError> {
        self.call("delete_category", json!([category, ct, prefix]))?;
        Ok(self.success)
    }
    fn update_all_mappings(&self, mappings: &Value, ct: &str) -> Result<bool, MappingError> {
        self.call("update_all", json!([mappings, ct]))?;
        Ok(self.success)
    }
    fn get_hidden_categories(&self, ct: &str) -> Result<Vec<String>, MappingError> {
        self.call("get_hidden", json!(ct))?;
        Ok(vec!["secret".into()])
    }
    fn get_latest_excluded_categories(&self, ct: &str) -> Result<Vec<String>, MappingError> {
        self.call("get_excluded", json!(ct))?;
        Ok(vec!["old".into()])
    }
    fn set_hidden(&self, category: &str, value: bool, ct: &str) -> Result<bool, MappingError> {
        self.call("set_hidden", json!([category, value, ct]))?;
        Ok(self.success)
    }
    fn set_latest_excluded(&self, category: &str, value: bool, ct: &str) -> Result<bool, MappingError> {
        self.call("set_excluded", json!([category, value, ct]))?;
        Ok(self.success)
    }
}
fn client(mapping: Arc<Mapping>, auth: AuthMode) -> TestClient {
    TestClient::with_app(app_with(AppDeps::new(auth).with_category_mapping(mapping)))
}
fn admin() -> AuthMode {
    AuthMode::Fixed(json!({"role":"admin"}))
}

#[tokio::test]
async fn keyword_routes_forward_content_type_and_return_python_contracts() {
    let mapping = Mapping::new(true, false);
    let client = client(mapping.clone(), admin());
    assert_eq!(client.get("/category-mappings?content_type=comic").await.json(), json!({"status":"success","result":{"fiction":["one"]}}));
    assert_eq!(client.get("/category-mappings/fiction").await.json(), json!({"status":"success","result":["one"]}));
    assert_eq!(client.put_json("/category-mappings/fiction?content_type=comic", &json!({"keywords":["one","two"]})).await.json(), json!({"status":"success","result":["one"]}));
    assert_eq!(client.post_json("/category-mappings/fiction/keywords", &json!({"keyword":"two"})).await.json()["status"], "success");
    assert_eq!(client.delete("/category-mappings/fiction/keywords/two").await.json()["result"], json!(["one"]));
    assert_eq!(client.put_json("/category-mappings?content_type=comic", &json!({"mappings":{"fiction":["two"]}})).await.json()["result"], json!({"fiction":["one"]}));
    assert_eq!(client.delete("/category-mappings/fiction?content_type=comic").await.json(), json!({"status":"success"}));
    let calls = mapping.calls.lock().unwrap();
    for expected in [
        ("get_all", json!("comic")),
        ("set_keywords", json!(["fiction", ["one", "two"], "comic"])),
        ("add_keyword", json!(["fiction", "two", "book"])),
        ("remove_keyword", json!(["fiction", "two", "book"])),
        ("delete_category", json!(["fiction", "comic", false])),
        ("update_all", json!([{"fiction":["two"]},"comic"])),
    ] {
        assert!(calls.contains(&(expected.0.into(), expected.1)), "{calls:?}");
    }
}

#[tokio::test]
async fn false_writes_have_specific_errors_and_duplicates_are_successful() {
    let client = client(Mapping::new(false, false), admin());
    let duplicate = client.post_json("/category-mappings/fiction/keywords", &json!({"keyword":"one"})).await;
    assert_eq!(duplicate.json(), json!({"status":"success","warning":"이미 등록된 키워드입니다.","result":["one"]}));
    let response = client.put_json("/category-mappings/fiction", &json!({"keywords":[]})).await;
    assert_eq!((response.status, response.json()["detail"].as_str()), (500, Some("Failed to set keywords")));
    let response = client.delete("/category-mappings/fiction/keywords/missing").await;
    assert_eq!((response.status, response.json()["detail"].as_str()), (404, Some("Keyword not found")));
    let response = client.delete("/category-mappings/missing").await;
    assert_eq!((response.status, response.json()["detail"].as_str()), (404, Some("Category not found")));
    let response = client.put_json("/category-mappings", &json!({"mappings":{}})).await;
    assert_eq!((response.status, response.json()["detail"].as_str()), (500, Some("Failed to update mappings")));
}

#[tokio::test]
async fn policy_routes_support_nested_paths_and_hide_settings_from_viewers() {
    for (path, key, getter, value) in [("hidden-categories", "hidden", "get_hidden", "secret"), ("latest-excluded-categories", "excluded", "get_excluded", "old")] {
        let mapping = Mapping::new(true, false);
        let client = client(mapping.clone(), admin());
        assert_eq!(client.get(&format!("/{path}?content_type=comic")).await.json(), json!({"status":"success","result":[value]}));
        for enabled in [true, false] {
            assert_eq!(client.post_json(&format!("/{path}/parent/child?content_type=comic"), &json!({key:enabled})).await.json()["result"], json!([value]));
        }
        assert!(mapping.calls.lock().unwrap().contains(&(getter.into(), json!("comic"))));
        assert!(mapping.calls.lock().unwrap().contains(&((if key == "hidden" { "set_hidden" } else { "set_excluded" }).into(), json!(["parent/child", false, "comic"]))));
        let mapping = Mapping::new(true, true);
        let viewer = self::client(mapping.clone(), AuthMode::AuthOnly(json!({"role":"viewer"})));
        assert_eq!(viewer.get(&format!("/{path}")).await.json(), json!({"status":"success","result":[]}));
        assert!(mapping.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn db_errors_are_generic_and_never_expose_private_details() {
    let client = client(Mapping::new(true, true), admin());
    for (path, detail) in [
        ("category-mappings", "카테고리 매핑 처리 중 오류가 발생했습니다"),
        ("hidden-categories", "비노출 카테고리 처리 중 오류가 발생했습니다"),
        ("latest-excluded-categories", "최신 자료 검색 제외 카테고리 처리 중 오류가 발생했습니다"),
    ] {
        let response = client.get(&format!("/{path}")).await;
        assert_eq!(response.status, 500);
        assert_eq!(response.json(), json!({"detail":detail}));
        assert!(!response.text().contains("credentials"));
    }
}

#[tokio::test]
async fn mapping_auth_and_body_validation_run_before_db() {
    let mapping = Mapping::new(true, false);
    let client = client(mapping.clone(), AuthMode::Cookies);
    assert_eq!(client.get("/category-mappings").await.status, 401);
    assert_eq!(client.put_json("/category-mappings/x", &json!({"keywords":[]})).await.status, 401);
    assert_eq!(client.delete("/category-mappings/x").await.status, 401);
    let client = self::client(mapping.clone(), admin());
    for body in [json!({}), json!({"keywords":"invalid"}), json!({"keywords":[1]})] {
        assert_eq!(client.put_json("/category-mappings/x", &body).await.status, 422);
    }
    assert_eq!(client.post_json("/category-mappings/x/keywords", &json!({})).await.status, 400);
    assert_eq!(client.post_json("/category-mappings/x/keywords", &json!({"keyword":1})).await.status, 422);
    assert_eq!(client.put_json("/category-mappings", &json!({"mappings":{"x":"invalid"}})).await.status, 422);
    assert_eq!(client.post_json("/hidden-categories/x", &json!({})).await.status, 422);
    assert!(mapping.calls.lock().unwrap().is_empty());
}

#[test]
fn real_mapping_facade_preserves_existing_database_failure_contract() {
    use backend::{category_mapping::CategoryMapping, db::DbError};
    let mapping = CategoryMapping::with_connection_provider(Arc::new(|| Err(DbError::Other("offline".into()))));
    assert_eq!(CategoryMappingApi::get_all_mappings(&mapping, "book").unwrap(), json!({}));
    assert_eq!(CategoryMappingApi::get_keywords(&mapping, "fiction", "book").unwrap(), Vec::<String>::new());
    assert!(!CategoryMappingApi::set_keywords(&mapping, "fiction", &["one".into()], "book").unwrap());
    assert!(!CategoryMappingApi::add_keyword(&mapping, "fiction", "one", "book").unwrap());
    assert!(!CategoryMappingApi::remove_keyword(&mapping, "fiction", "one", "book").unwrap());
    assert!(!CategoryMappingApi::delete_category(&mapping, "fiction", "book", false).unwrap());
    assert!(!CategoryMappingApi::update_all_mappings(&mapping, &json!({"fiction":["one"]}), "book").unwrap());
    assert!(!CategoryMappingApi::set_hidden(&mapping, "fiction", true, "book").unwrap());
    assert!(!CategoryMappingApi::set_latest_excluded(&mapping, "fiction", true, "book").unwrap());
}

#[tokio::test]
async fn policy_boolean_inputs_match_pydantic() {
    let mapping = Mapping::new(true, false);
    let client = client(mapping.clone(), admin());
    for (value, expected) in [(json!(true), true), (json!(1.0), true), (json!(-0.0), false), (json!("Y"), true), (json!("f"), false)] {
        let response = client.post_json("/hidden-categories/x", &json!({"hidden":value})).await;
        assert_eq!(response.status, 200, "{value}");
        assert_eq!(mapping.calls.lock().unwrap().iter().rev().find(|(name, _)| name == "set_hidden").unwrap().1, json!(["x", expected, "book"]));
    }
    for value in [json!("1.0"), json!("0.0"), json!(" true"), json!(2), Value::Null] {
        assert_eq!(client.post_json("/hidden-categories/x", &json!({"hidden":value})).await.status, 422, "{value}");
    }
}

#[tokio::test]
async fn failed_policy_changes_return_specific_errors() {
    let client = client(Mapping::new(false, false), admin());
    for (path, key, detail) in [("hidden-categories", "hidden", "Failed to update hidden category"), ("latest-excluded-categories", "excluded", "Failed to update latest excluded category")] {
        let response = client.post_json(&format!("/{path}/parent/child"), &json!({key:true})).await;
        assert_eq!(response.status, 500);
        assert_eq!(response.json(), json!({"detail":detail}));
    }
}

#[tokio::test]
async fn viewer_token_cannot_change_any_category_setting() {
    common::setup_env();
    let mapping = Mapping::new(true, false);
    let token = backend::auth::create_jwt_token("viewer@test.com", "viewer", None, None);
    let client = client(mapping.clone(), AuthMode::Cookies).bearer(&token);
    assert_eq!(client.get("/category-mappings").await.status, 200);
    for (method, path, body) in [
        (axum::http::Method::PUT, "/category-mappings", json!({"mappings":{}})),
        (axum::http::Method::PUT, "/category-mappings/x", json!({"keywords":[]})),
        (axum::http::Method::POST, "/category-mappings/x/keywords", json!({"keyword":"one"})),
        (axum::http::Method::DELETE, "/category-mappings/x/keywords/one", Value::Null),
        (axum::http::Method::DELETE, "/category-mappings/x", Value::Null),
        (axum::http::Method::POST, "/hidden-categories/x", json!({"hidden":true})),
        (axum::http::Method::POST, "/latest-excluded-categories/x", json!({"excluded":true})),
    ] {
        let response = client.request(method, path, Some("application/json"), serde_json::to_vec(&body).unwrap()).await;
        assert_eq!(response.status, 403, "{path}: {}", response.text());
    }
    assert_eq!(mapping.calls.lock().unwrap().len(), 1);
}
