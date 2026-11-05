//! 인증 테스트 대역(`FixedRole`)과 토큰·앱 준비물. 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

#[allow(unused_imports)]
pub use super::auth_fixtures::{FixedRole, access_token};
use super::router_fixtures::Fx;
use super::shared_fixtures::Mock;
use std::sync::Arc;

// TODO(common): FakeGoogle, FakeStore는 main_mock, view_history_api도 쓴다. 이 파일로 옮긴다.

// ── /auth/refresh ────────────────────────────────────────────────────────────

/// `refresh_token_store`를 가짜로 끼운 Fx를 돌려준다. `determine_role`은 항상 "admin"이다.
pub fn fx_with_refresh_store(rt_store: &Mock) -> Fx {
    let store = rt_store.clone();
    Fx::tweaked(move |d| d.with_role_resolver(Arc::new(FixedRole(Some("admin".to_string())))).with_refresh_token_store(Arc::new(store)))
}
