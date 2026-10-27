//! 라우트 등록부. 라우트를 등록하면서 `route_table()`에 쓰는 정보(경로, 메서드, 의존성)를 함께 기록한다.
//!
//! 라우트 파일 하나는 `pub(super) fn routes() -> Routes`를 하나 두고 `main.rs`의 `all_routes()`에서 `.merge(..)`로 붙인다.
//! 의존성 이름(`"require_auth"`, `"require_admin"`, `"optional_auth"`)은 핸들러가 실제로 쓰는 추출기(`extract.rs`)와 맞춰 적는다.

use std::sync::Arc;

use axum::Router;
use axum::handler::Handler;
use axum::routing::{delete, get, patch, post, put};

use super::deps::AppDeps;

/// FastAPI `route.path`, `route.methods`, `route.dependant.dependencies`에 대응한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInfo {
    pub path: String,
    pub methods: Vec<String>,
    pub dependencies: Vec<String>,
}

pub struct Routes {
    router: Router<Arc<AppDeps>>,
    table: Vec<RouteInfo>,
}

impl Default for Routes {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! method_fn {
    ($name:ident, $verb:ident, $label:literal) => {
        pub fn $name<H, T>(mut self, path: &str, dependencies: &[&str], handler: H) -> Self
        where
            H: Handler<T, Arc<AppDeps>>,
            T: 'static,
        {
            self.router = self.router.route(path, $verb(handler));
            self.table.push(RouteInfo { path: path.to_string(), methods: vec![$label.to_string()], dependencies: dependencies.iter().map(|d| d.to_string()).collect() });
            self
        }
    };
}

impl Routes {
    pub fn new() -> Self {
        Self { router: Router::new(), table: Vec::new() }
    }

    method_fn!(get, get, "GET");
    method_fn!(post, post, "POST");
    method_fn!(put, put, "PUT");
    method_fn!(delete, delete, "DELETE");
    method_fn!(patch, patch, "PATCH");

    /// 다른 라우트 묶음을 합친다. 같은 경로·메서드가 겹치면 panic한다(axum과 같다).
    pub fn merge(mut self, other: Routes) -> Self {
        self.router = self.router.merge(other.router);
        self.table.extend(other.table);
        self
    }

    pub fn table(&self) -> &[RouteInfo] {
        &self.table
    }

    pub(super) fn into_router(self) -> Router<Arc<AppDeps>> {
        self.router
    }
}
