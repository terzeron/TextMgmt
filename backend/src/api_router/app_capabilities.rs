//! 앱이 주입받는 능력(trait)과 실제·기본 구현. 설계: docs/rust-test-seams.md 3.0, 3.2.
//!
//! 규칙: 모든 trait는 `Send + Sync`이고 모든 메서드에 기본 구현이 있다. 기본값은 `Err(..unimplemented)`/빈 값이다.
//! 테스트 대역은 쓰는 메서드만 구현한다. facade 이름은 `XxxApi`, 좁은 능력은 역할 명사다.
//!
//! 이 파일의 `BookManagerApi`, `CategoryMappingApi`, `ViewHistoryApi`, `BookAccess`는 라우트를 붙이는 쪽이 필요로 하는
//! 메서드만 먼저 정의한 뼈대다. 새 라우트가 메서드를 더 필요로 하면 여기에 기본 구현과 함께 추가한다.
//! (실제 `BookManager`가 생기면 `backend::book_manager`가 이 trait를 `pub use`하거나 구현한다.)

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use serde_json::Value;

use crate::auth_service::{Claims, HttpStatusError};
use crate::view_history_store::{RecentViews, ViewHistoryError, ViewHistoryStore};
use db::category_mapping::CategoryMapping;

pub(crate) use crate::book_manager::UNIMPLEMENTED;

// ---------------------------------------------------------------------------
// Google 로그인
// ---------------------------------------------------------------------------

/// Google ID 토큰 검증. `TM_GOOGLE_CLIENT_ID`를 audience로 쓴다.
pub trait GoogleVerifier: Send + Sync {
    /// 설정된 OAuth 클라이언트 ID. 없으면 None.
    fn client_id(&self) -> Option<String> {
        std::env::var("TM_GOOGLE_CLIENT_ID").ok().filter(|v| !v.is_empty())
    }

    /// 검증에 성공하면 토큰의 claims를 돌려준다. 실패 사유는 문자열이다.
    fn verify(&self, _credential: &str, _audience: &str) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }
}

/// 자리표시자. Google 공개 키를 HTTPS로 가져오는 TLS 클라이언트가 아직 없어서 항상 "검증 불가"를 돌려준다.
/// 그래서 이 구현으로는 아무도 로그인할 수 없다(실패로 닫힌다).
pub struct UnavailableGoogleVerifier;

impl GoogleVerifier for UnavailableGoogleVerifier {
    fn verify(&self, _credential: &str, _audience: &str) -> Result<Value, String> {
        Err("Google ID token verification is not available in this build".to_string())
    }
}

// ---------------------------------------------------------------------------
// 조회 이력
// ---------------------------------------------------------------------------

/// 사용자별 조회 이력 저장소 facade. `ViewHistoryStore`가 구현한다.
pub trait ViewHistoryApi: Send + Sync {
    fn record_view(&self, _email: &str, _content_type: &str, _book_id: i64, _title: &str, _category: &str) -> Result<(), ViewHistoryError> {
        Err(ViewHistoryError::Storage(UNIMPLEMENTED.to_string()))
    }

    fn list_recent_views(&self, _limit: usize) -> Result<RecentViews, ViewHistoryError> {
        Err(ViewHistoryError::Storage(UNIMPLEMENTED.to_string()))
    }
}

impl ViewHistoryApi for ViewHistoryStore {
    fn record_view(&self, email: &str, content_type: &str, book_id: i64, title: &str, category: &str) -> Result<(), ViewHistoryError> {
        ViewHistoryStore::record_view(self, email, content_type, book_id, title, category)
    }

    fn list_recent_views(&self, limit: usize) -> Result<RecentViews, ViewHistoryError> {
        ViewHistoryStore::list_recent_views(self, limit)
    }
}

// ---------------------------------------------------------------------------
// 카테고리 매핑
// ---------------------------------------------------------------------------

/// 카테고리 매핑 facade의 오류. `Http`는 라우트가 그대로 응답으로 바꾼다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingError {
    Http(u16, String),
    Other(String),
}

impl fmt::Display for MappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MappingError::Http(status, detail) => write!(f, "{status}: {detail}"),
            MappingError::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for MappingError {}

fn unimplemented_mapping<T>() -> Result<T, MappingError> {
    Err(MappingError::Other(UNIMPLEMENTED.to_string()))
}

fn db_row_to_json(row: db::Row) -> Value {
    Value::Object(row.iter().map(|(key, value)| (key.clone(), Value::from(value))).collect())
}

fn reload_counts(value: &Value) -> Result<Vec<(&str, i64)>, String> {
    value
        .as_object()
        .ok_or_else(|| "재적재 진행 수치가 객체가 아닙니다".to_string())?
        .iter()
        .map(|(key, value)| value.as_i64().map(|count| (key.as_str(), count)).ok_or_else(|| format!("재적재 진행 수치가 정수가 아닙니다: {key}")))
        .collect()
}

/// 카테고리 매핑 facade. 동기 메서드다. 지금은 viewer 접근 제어에 필요한 조회만 있다.
pub trait CategoryMappingApi: Send + Sync {
    fn acquire_reload_lock(&self, _content_type: &str, _category: Option<&str>, _reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        unimplemented_mapping()
    }
    fn heartbeat_reload_lock(&self, _content_type: &str, _category: Option<&str>, _counts: &Value) -> Result<(), String> {
        Err(UNIMPLEMENTED.into())
    }
    fn complete_reload_lock(&self, _content_type: &str, _status: &str, _error: Option<&str>, _category: Option<&str>, _counts: &Value) -> Result<(), String> {
        Err(UNIMPLEMENTED.into())
    }
    fn get_reload_status(&self, _content_type: &str, _category: Option<&str>) -> Result<Option<Value>, MappingError> {
        unimplemented_mapping()
    }
    fn rename_category(&self, _old: &str, _new: &str, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    /// 카테고리 → 키워드 목록.
    fn get_all_mappings(&self, _content_type: &str) -> Result<BTreeMap<String, Vec<String>>, MappingError> {
        unimplemented_mapping()
    }
    fn get_keywords(&self, _category: &str, _content_type: &str) -> Result<Vec<String>, MappingError> {
        unimplemented_mapping()
    }
    fn set_keywords(&self, _category: &str, _keywords: &[String], _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn add_keyword(&self, _category: &str, _keyword: &str, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn remove_keyword(&self, _category: &str, _keyword: &str, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn delete_category(&self, _category: &str, _content_type: &str, _prefix: bool) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn update_all_mappings(&self, _mappings: &Value, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn set_hidden(&self, _category: &str, _hidden: bool, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn set_latest_excluded(&self, _category: &str, _excluded: bool, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn get_hidden_categories(&self, _content_type: &str) -> Result<Vec<String>, MappingError> {
        unimplemented_mapping()
    }

    fn get_latest_excluded_categories(&self, _content_type: &str) -> Result<Vec<String>, MappingError> {
        unimplemented_mapping()
    }

    fn get_classify_proposal_status(&self, _content_type: &str, _stale_seconds: i64) -> Result<Value, MappingError> {
        unimplemented_mapping()
    }
    fn set_classify_proposal_status(&self, _status: Value, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn merge_classify_proposal_status(&self, _fields: Value, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn try_start_classify_proposal(&self, _status: Value, content_type: &str, _stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        Ok((false, self.get_classify_proposal_status(content_type, 900)?))
    }
    fn try_begin_classify_apply(&self, _apply_token: &str, content_type: &str, _stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        Ok((false, self.get_classify_proposal_status(content_type, 900)?))
    }
    fn clear_classify_proposal_items(&self, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn get_classify_proposal_items(&self, _content_type: &str) -> Result<Vec<Value>, MappingError> {
        unimplemented_mapping()
    }
    fn delete_classify_proposal_item(&self, _file_path: &str, _content_type: &str) -> Result<u64, MappingError> {
        unimplemented_mapping()
    }
    fn delete_applied_classify_proposal_items(&self, _content_type: &str) -> Result<u64, MappingError> {
        unimplemented_mapping()
    }
    fn add_classify_proposal_items(&self, _items: &[Value], _source_category: &str, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn update_classify_proposal_item_payloads(&self, _items: &[Value], _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn update_classify_proposal_item_status(&self, _file_path: &str, _apply_status: &str, _apply_error: Option<&str>, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
}

impl CategoryMappingApi for CategoryMapping {
    fn acquire_reload_lock(&self, content_type: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        CategoryMapping::acquire_reload_lock(self, content_type, category, reload_source)
            .map(|(acquired, error, status)| (acquired, error, status.map(db_row_to_json)))
            .map_err(|e| MappingError::Other(e.to_string()))
    }

    fn heartbeat_reload_lock(&self, content_type: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let counts = reload_counts(counts)?;
        CategoryMapping::heartbeat_reload_lock(self, content_type, category, &counts).map_err(|error| error.to_string())
    }

    fn complete_reload_lock(&self, content_type: &str, status: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let counts = reload_counts(counts)?;
        CategoryMapping::complete_reload_lock(self, content_type, status, error, category, &counts).map_err(|error| error.to_string())
    }

    fn get_reload_status(&self, content_type: &str, category: Option<&str>) -> Result<Option<Value>, MappingError> {
        CategoryMapping::get_reload_status(self, content_type, category).map(|row| row.map(db_row_to_json)).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn rename_category(&self, old: &str, new: &str, content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::rename_category(self, old, new, content_type))
    }
    fn get_all_mappings(&self, content_type: &str) -> Result<BTreeMap<String, Vec<String>>, MappingError> {
        CategoryMapping::get_all_mappings(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn get_keywords(&self, category: &str, content_type: &str) -> Result<Vec<String>, MappingError> {
        CategoryMapping::get_keywords(self, category, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn set_keywords(&self, category: &str, keywords: &[String], content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::set_keywords(self, category, &keywords.iter().map(String::as_str).collect::<Vec<_>>(), content_type))
    }
    fn add_keyword(&self, category: &str, keyword: &str, content_type: &str) -> Result<bool, MappingError> {
        CategoryMapping::add_keyword(self, category, keyword, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn remove_keyword(&self, category: &str, keyword: &str, content_type: &str) -> Result<bool, MappingError> {
        CategoryMapping::remove_keyword(self, category, keyword, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn delete_category(&self, category: &str, content_type: &str, prefix: bool) -> Result<bool, MappingError> {
        CategoryMapping::delete_category(self, category, content_type, prefix).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn update_all_mappings(&self, mappings: &Value, content_type: &str) -> Result<bool, MappingError> {
        let mappings = serde_json::from_value(mappings.clone()).map_err(|e| MappingError::Other(e.to_string()))?;
        Ok(CategoryMapping::update_all_mappings(self, &mappings, content_type))
    }
    fn set_hidden(&self, category: &str, hidden: bool, content_type: &str) -> Result<bool, MappingError> {
        CategoryMapping::set_hidden(self, category, hidden, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn set_latest_excluded(&self, category: &str, excluded: bool, content_type: &str) -> Result<bool, MappingError> {
        CategoryMapping::set_latest_excluded(self, category, excluded, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn get_hidden_categories(&self, content_type: &str) -> Result<Vec<String>, MappingError> {
        CategoryMapping::get_hidden_categories(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }

    fn get_latest_excluded_categories(&self, content_type: &str) -> Result<Vec<String>, MappingError> {
        CategoryMapping::get_latest_excluded_categories(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }

    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, MappingError> {
        CategoryMapping::get_classify_proposal_status(self, content_type, stale_seconds).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn set_classify_proposal_status(&self, status: Value, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::set_classify_proposal_status(self, &status, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::merge_classify_proposal_status(self, &fields, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        CategoryMapping::try_start_classify_proposal(self, &status, content_type, stale_seconds).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        CategoryMapping::try_begin_classify_apply(self, apply_token, content_type, stale_seconds).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn clear_classify_proposal_items(&self, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::clear_classify_proposal_items(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn get_classify_proposal_items(&self, content_type: &str) -> Result<Vec<Value>, MappingError> {
        CategoryMapping::get_classify_proposal_items(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn delete_classify_proposal_item(&self, file_path: &str, content_type: &str) -> Result<u64, MappingError> {
        CategoryMapping::delete_classify_proposal_item(self, file_path, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn delete_applied_classify_proposal_items(&self, content_type: &str) -> Result<u64, MappingError> {
        CategoryMapping::delete_applied_classify_proposal_items(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn add_classify_proposal_items(&self, items: &[Value], source_category: &str, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::add_classify_proposal_items(self, items, source_category, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn update_classify_proposal_item_payloads(&self, items: &[Value], content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::update_classify_proposal_item_payloads(self, items, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::update_classify_proposal_item_status(self, file_path, apply_status, apply_error, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// viewer 접근 확인
// ---------------------------------------------------------------------------

/// `BookAccess`가 돌려주는 책 요약.
#[derive(Debug, Clone, PartialEq)]
pub struct BookStub {
    pub title: String,
    pub category: Option<String>,
}

/// 책을 찾고, 없으면 404, viewer가 볼 수 없는 카테고리면 403 오류를 돌려준다.
#[async_trait]
pub trait BookAccess: Send + Sync {
    async fn get_book_and_ensure_viewer_access(&self, _book_id: i64, _payload: &Claims, _content_type: &str) -> Result<BookStub, HttpStatusError> {
        Err(HttpStatusError::new(500, UNIMPLEMENTED))
    }
}

/// 설정이 없을 때 쓰는 접근 확인. 기본 구현(미구현 오류)이다.
pub struct UnavailableBookAccess;

impl BookAccess for UnavailableBookAccess {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;

    struct EmptyMapping;
    struct EmptyGoogleVerifier;
    struct EmptyBookAccess;
    struct EmptyViewHistory;

    impl CategoryMappingApi for EmptyMapping {}
    impl GoogleVerifier for EmptyGoogleVerifier {}
    impl BookAccess for EmptyBookAccess {}
    impl ViewHistoryApi for EmptyViewHistory {}

    #[test]
    fn google_verifier_and_view_history_defaults_are_explicit() {
        let expected_client_id = std::env::var("TM_GOOGLE_CLIENT_ID").ok().filter(|value| !value.is_empty());
        assert_eq!(EmptyGoogleVerifier.client_id(), expected_client_id);
        assert_eq!(EmptyGoogleVerifier.verify("token", "audience").unwrap_err(), UNIMPLEMENTED);

        assert!(EmptyViewHistory.record_view("a@b", "book", 1, "title", "category").is_err());
        assert!(EmptyViewHistory.list_recent_views(5).is_err());
        assert!(UnavailableGoogleVerifier.verify("token", "audience").is_err());
    }

    #[tokio::test]
    async fn unavailable_book_access_fails_closed() {
        assert!(EmptyBookAccess.get_book_and_ensure_viewer_access(1, &Claims::default(), "book").await.is_err());
        assert!(UnavailableBookAccess.get_book_and_ensure_viewer_access(1, &Claims::default(), "book").await.is_err());
    }

    #[test]
    fn category_mapping_defaults_report_unimplemented() {
        let mapping = EmptyMapping;
        let counts = json!({"indexed_count": 1});
        let item = json!({"path": "/books/a.epub"});

        assert!(mapping.acquire_reload_lock("book", None, None).is_err());
        assert_eq!(mapping.heartbeat_reload_lock("book", None, &counts).unwrap_err(), UNIMPLEMENTED);
        assert_eq!(mapping.complete_reload_lock("book", "done", None, None, &counts).unwrap_err(), UNIMPLEMENTED);
        assert!(mapping.get_reload_status("book", None).is_err());
        assert!(mapping.rename_category("old", "new", "book").is_err());
        assert!(mapping.get_all_mappings("book").is_err());
        assert!(mapping.get_keywords("fiction", "book").is_err());
        assert!(mapping.set_keywords("fiction", &[], "book").is_err());
        assert!(mapping.add_keyword("fiction", "word", "book").is_err());
        assert!(mapping.remove_keyword("fiction", "word", "book").is_err());
        assert!(mapping.delete_category("fiction", "book", false).is_err());
        assert!(mapping.update_all_mappings(&json!({}), "book").is_err());
        assert!(mapping.set_hidden("fiction", true, "book").is_err());
        assert!(mapping.set_latest_excluded("fiction", true, "book").is_err());
        assert!(mapping.get_hidden_categories("book").is_err());
        assert!(mapping.get_latest_excluded_categories("book").is_err());
        assert!(mapping.get_classify_proposal_status("book", 60).is_err());
        assert!(mapping.set_classify_proposal_status(json!({}), "book").is_err());
        assert!(mapping.merge_classify_proposal_status(json!({}), "book").is_err());
        assert!(mapping.try_start_classify_proposal(json!({}), "book", 60).is_err());
        assert!(mapping.try_begin_classify_apply("token", "book", 60).is_err());
        assert!(mapping.clear_classify_proposal_items("book").is_err());
        assert!(mapping.get_classify_proposal_items("book").is_err());
        assert!(mapping.delete_classify_proposal_item("/a", "book").is_err());
        assert!(mapping.delete_applied_classify_proposal_items("book").is_err());
        assert!(mapping.add_classify_proposal_items(std::slice::from_ref(&item), "old", "book").is_err());
        assert!(mapping.update_classify_proposal_item_payloads(&[item], "book").is_err());
        assert!(mapping.update_classify_proposal_item_status("/a", "failed", None, "book").is_err());
    }

    #[test]
    fn mapping_errors_format_http_and_other_messages() {
        assert_eq!(MappingError::Http(409, "busy".into()).to_string(), "409: busy");
        assert_eq!(MappingError::Other("broken".into()).to_string(), "broken");
    }

    #[test]
    fn row_and_reload_progress_converters_cover_all_value_types_and_errors() {
        let mut row = db::Row::new();
        row.insert("null".into(), db::DbValue::Null);
        row.insert("number".into(), db::DbValue::Int(7));
        row.insert("text".into(), db::DbValue::Text("value".into()));
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 27).unwrap().and_hms_opt(1, 2, 3).unwrap();
        row.insert("date".into(), db::DbValue::DateTime(date));
        assert_eq!(db_row_to_json(row), json!({"null": null, "number": 7, "text": "value", "date": "2026-10-27 01:02:03"}));

        assert_eq!(reload_counts(&json!({"done": 2, "failed": 0})).unwrap(), vec![("done", 2), ("failed", 0)]);
        assert!(reload_counts(&json!([])).unwrap_err().contains("객체가 아닙니다"));
        assert!(reload_counts(&json!({"done": "2"})).unwrap_err().contains("정수가 아닙니다: done"));
    }

    #[test]
    fn concrete_category_mapping_adapter_forwards_database_errors() {
        let mapping = CategoryMapping::with_connection_provider(Arc::new(|| Err(db::DbError::Other("offline".into()))));
        let counts = json!({"indexed_count": 1});
        let item = json!({"file_path": "/books/a.epub"});

        assert!(CategoryMappingApi::acquire_reload_lock(&mapping, "book", None, None).is_err());
        assert!(CategoryMappingApi::heartbeat_reload_lock(&mapping, "book", None, &counts).unwrap_err().contains("offline"));
        assert!(CategoryMappingApi::complete_reload_lock(&mapping, "book", "done", None, None, &counts).unwrap_err().contains("offline"));
        assert!(CategoryMappingApi::get_reload_status(&mapping, "book", None).is_err());
        assert!(!CategoryMappingApi::rename_category(&mapping, "old", "new", "book").unwrap());
        assert!(CategoryMappingApi::get_all_mappings(&mapping, "book").is_err());
        assert!(CategoryMappingApi::get_keywords(&mapping, "fiction", "book").is_err());
        assert!(!CategoryMappingApi::set_keywords(&mapping, "fiction", &[], "book").unwrap());
        assert!(CategoryMappingApi::add_keyword(&mapping, "fiction", "word", "book").is_err());
        assert!(CategoryMappingApi::remove_keyword(&mapping, "fiction", "word", "book").is_err());
        assert!(CategoryMappingApi::delete_category(&mapping, "fiction", "book", false).is_err());
        assert!(!CategoryMappingApi::update_all_mappings(&mapping, &json!({}), "book").unwrap());
        assert!(!CategoryMappingApi::set_hidden(&mapping, "fiction", true, "book").unwrap());
        assert!(!CategoryMappingApi::set_latest_excluded(&mapping, "fiction", true, "book").unwrap());
        assert!(CategoryMappingApi::get_hidden_categories(&mapping, "book").is_err());
        assert!(CategoryMappingApi::get_latest_excluded_categories(&mapping, "book").is_err());
        assert!(CategoryMappingApi::get_classify_proposal_status(&mapping, "book", 60).is_err());
        assert!(CategoryMappingApi::set_classify_proposal_status(&mapping, json!({}), "book").is_err());
        assert!(CategoryMappingApi::merge_classify_proposal_status(&mapping, json!({"progress": 1}), "book").is_err());
        assert!(CategoryMappingApi::try_start_classify_proposal(&mapping, json!({}), "book", 60).is_err());
        assert!(CategoryMappingApi::try_begin_classify_apply(&mapping, "token", "book", 60).is_err());
        assert!(CategoryMappingApi::clear_classify_proposal_items(&mapping, "book").is_err());
        assert!(CategoryMappingApi::get_classify_proposal_items(&mapping, "book").is_err());
        assert!(CategoryMappingApi::delete_classify_proposal_item(&mapping, "/a", "book").is_err());
        assert!(CategoryMappingApi::delete_applied_classify_proposal_items(&mapping, "book").is_err());
        assert!(CategoryMappingApi::add_classify_proposal_items(&mapping, std::slice::from_ref(&item), "old", "book").is_err());
        assert!(CategoryMappingApi::update_classify_proposal_item_payloads(&mapping, &[item], "book").is_err());
        assert!(CategoryMappingApi::update_classify_proposal_item_status(&mapping, "/a", "failed", None, "book").is_err());
    }
}
