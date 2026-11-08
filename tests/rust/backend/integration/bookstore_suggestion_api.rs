//! 편집 화면의 서점 카테고리 추천 API. 대응 로직은 `book_manager::store_category_matcher`와 같은 결과를 낸다.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use auth_manager::auth_service::Claims;
use backend::api_router::{AppDeps, AuthMode, CategoryMappingApi, MappingError, app_with};
use book_manager::store_category_matcher::{similarity_debug, top_similar_categories};
use common::TestClient;
use serde_json::{Value, json};

const PATH: &str = "/bookstore/category-suggestions";

struct Mapping {
    content_types: Mutex<Vec<String>>,
}

impl CategoryMappingApi for Mapping {
    fn get_all_mappings(&self, ct: &str) -> Result<BTreeMap<String, Vec<String>>, MappingError> {
        self.content_types.lock().unwrap().push(ct.to_string());
        Ok(BTreeMap::from([("3_SF".to_string(), vec!["과학소설".to_string()])]))
    }
}

fn client(auth: AuthMode) -> (TestClient, Arc<Mapping>) {
    let mapping = Arc::new(Mapping { content_types: Mutex::default() });
    let app = app_with(AppDeps::new(auth).with_refresh_token_store(common::session::store()).with_category_mapping(mapping.clone()));
    (TestClient::with_app(app), mapping)
}

fn body() -> Value {
    json!({"suggested": {"yes24_0_0": "SF 한국SF", "aladin_0_0": "예술/대중문화 클래식"}, "categories": ["3_SF", "5_음악", "2_시"], "top_n": 2, "debug": true})
}

#[tokio::test]
async fn admins_get_the_same_suggestions_as_the_matcher() {
    let (client, mapping) = client(AuthMode::Fixed(Claims::with_role("admin")));

    let response = client.post_json(PATH, &body()).await;

    assert_eq!(response.status, 200, "{}", response.text());
    let json: Value = serde_json::from_str(&response.text()).unwrap();
    let suggested = vec![("yes24_0_0".to_string(), "SF 한국SF".to_string()), ("aladin_0_0".to_string(), "예술/대중문화 클래식".to_string())];
    let categories: Vec<String> = ["3_SF", "5_음악", "2_시"].iter().map(|s| s.to_string()).collect();
    let mappings = BTreeMap::from([("3_SF".to_string(), vec!["과학소설".to_string()])]);
    assert_eq!(json["status"], "success");
    assert_eq!(json["result"]["top"], json!(top_similar_categories(&suggested, &categories, &mappings, 2)));
    assert_eq!(json["result"]["debug"], serde_json::to_value(similarity_debug(&suggested, &categories, &mappings, 2)).unwrap());
    assert_eq!(mapping.content_types.lock().unwrap().as_slice(), ["book"]);
}

#[tokio::test]
async fn debug_is_null_unless_asked() {
    let (client, _) = client(AuthMode::Fixed(Claims::with_role("admin")));
    let mut request = body();
    request.as_object_mut().unwrap().remove("debug");

    let response = client.post_json(PATH, &request).await;

    assert_eq!(response.status, 200, "{}", response.text());
    let json: Value = serde_json::from_str(&response.text()).unwrap();
    assert!(json["result"]["top"].is_array());
    assert_eq!(json["result"]["debug"], Value::Null);
}

#[tokio::test]
async fn unauthenticated_and_viewer_requests_are_rejected() {
    common::setup_env();
    let (anonymous, mapping) = client(AuthMode::Cookies);
    assert_eq!(anonymous.post_json(PATH, &body()).await.status, 401);
    let token = common::session::token("viewer@test.com", "viewer", None, None);
    let viewer = anonymous.bearer(&token);
    assert_eq!(viewer.post_json(PATH, &body()).await.status, 403);
    assert!(mapping.content_types.lock().unwrap().is_empty());
}
