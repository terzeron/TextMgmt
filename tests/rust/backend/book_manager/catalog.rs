#[path = "../common/mod.rs"]
mod common;

use backend::main::{AppDeps, AuthMode, app_with};
use common::TestClient;

#[tokio::test]
async fn all_catalog_routes_require_auth() {
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies)));
    for prefix in ["", "/comics"] {
        for path in ["/books/1", "/categories", "/categories/public", "/latest", "/similar/1", "/search/hello"] {
            assert_eq!(client.get(&format!("{prefix}{path}")).await.status, 401, "{prefix}{path}");
        }
    }
}
