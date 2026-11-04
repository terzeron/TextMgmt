//! 콘텐츠 API 테스트용 세션. 실제 JWT와 역할 검증을 유지하고 MySQL만 대역으로 바꾼다.

use std::sync::Arc;

use backend::refresh_token_store::{RefreshTokenStoreApi, Rotation, StoreError};
use db::Row;
use serde_json::Value;

const ADMIN_FAMILY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const VIEWER_FAMILY: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

pub fn token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String {
    let family = if role == "admin" { ADMIN_FAMILY } else { VIEWER_FAMILY };
    backend::auth_service::create_session_jwt_token(email, role, name.unwrap_or_default(), picture.unwrap_or_default(), family)
}

pub fn store() -> Arc<dyn RefreshTokenStoreApi> {
    Arc::new(ContentSessions)
}

struct ContentSessions;

fn unsupported() -> StoreError {
    StoreError::Unsupported("content session fixture does not issue or revoke tokens".into())
}

#[allow(clippy::too_many_arguments)]
impl RefreshTokenStoreApi for ContentSessions {
    fn store_issued(&self, _: &str, _: &str, _: &str, _: i64, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), StoreError> {
        Err(unsupported())
    }

    fn rotate(&self, _: &str, _: &str, _: &str, _: &str, _: i64, _: i64, _: Option<&str>, _: Option<&str>) -> Result<Rotation, StoreError> {
        Err(unsupported())
    }

    fn revoke_family(&self, _: &str, _: &str) -> Result<(), StoreError> {
        Err(unsupported())
    }

    fn get_token_observation(&self, _: &str) -> Result<Option<Value>, StoreError> {
        Err(unsupported())
    }

    fn session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
        Ok([(ADMIN_FAMILY, "admin@test.com"), (VIEWER_FAMILY, "viewer@test.com")]
            .into_iter()
            .filter(|(fid, owner)| email.is_none_or(|e| e == *owner) && family_id.is_none_or(|f| f == *fid))
            .map(|(fid, owner)| Row::from([("family_id".into(), fid.into()), ("email".into(), owner.into()), ("valid_token_count".into(), 1_i64.into())]))
            .collect())
    }
}
