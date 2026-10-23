//! Elasticsearch 접근 계층의 trait와 오류. 구현(`ESManager`)은 `libs/es_client`를 감싸서 이 trait를 만족시킨다.
//! 설계: docs/rust-test-seams.md 3.3. 모든 메서드의 기본 구현은 `Err(EsError::Unsupported)`라서, 테스트 대역은 쓰는 메서드만 구현한다.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde_json::Value;

/// (문서 ID, `_source`, 점수)
pub type Hit = (i64, Value, f64);
pub type Docs = BTreeMap<i64, Value>;
/// (히트, 전체 건수, 다음 `search_after`)
pub type PagedHits = (Vec<Hit>, i64, Option<Vec<Value>>);

#[derive(Debug, Clone, PartialEq)]
pub enum EsError {
    /// 필수 환경 변수가 없다.
    MissingEnv(String),
    /// 연결 실패나 시간 초과 같은 전송 계층 오류.
    Transport(String),
    /// ES가 오류 상태 코드를 돌려줬다.
    Status { status: u16, body: String },
    /// 응답 본문을 해석할 수 없다.
    Parse(String),
    /// 이 구현에 없는 연산.
    Unsupported,
    Other(String),
}

impl fmt::Display for EsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EsError::MissingEnv(name) => write!(f, "환경 변수 {name}이(가) 없다"),
            EsError::Transport(msg) => write!(f, "ES 연결 오류: {msg}"),
            EsError::Status { status, body } => write!(f, "ES 응답 오류 {status}: {body}"),
            EsError::Parse(msg) => write!(f, "ES 응답 해석 실패: {msg}"),
            EsError::Unsupported => write!(f, "지원하지 않는 연산"),
            EsError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for EsError {}

/// 저수준 전송: 연산 이름과 인자(JSON)를 받아 응답 JSON을 돌려준다. Python `Elasticsearch` 클라이언트 호출 하나에 대응한다.
pub trait EsTransport: Send + Sync {
    fn index_name(&self) -> &str;
    fn call(&self, op: &str, kwargs: &Value) -> Result<Value, EsError>;
}

/// 고수준 facade. Python `ESManager`의 공개 메서드에 대응한다.
pub trait EsManagerApi: Send + Sync {
    fn create_index(&self) -> Result<Value, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_by_id(&self, _doc_id: i64) -> Result<Option<Value>, EsError> {
        Err(EsError::Unsupported)
    }
    fn update(&self, _doc_id: i64, _fields: &Value) -> Result<bool, EsError> {
        Err(EsError::Unsupported)
    }
    fn delete(&self, _doc_id: i64) -> Result<bool, EsError> {
        Err(EsError::Unsupported)
    }
    fn insert(&self, _docs: &Docs) -> Result<Vec<i64>, EsError> {
        Err(EsError::Unsupported)
    }
    fn delete_by_file_paths(&self, _paths: &[String], _exclude_ids: Option<&[i64]>) -> Result<i64, EsError> {
        Err(EsError::Unsupported)
    }
    fn delete_by_ids(&self, _ids: &[i64], _chunk_size: usize) -> Result<i64, EsError> {
        Err(EsError::Unsupported)
    }
    fn refresh(&self) -> Result<(), EsError> {
        Err(EsError::Unsupported)
    }
    fn search_and_aggregate_by_category(&self) -> Result<BTreeMap<String, i64>, EsError> {
        Err(EsError::Unsupported)
    }
    /// (카테고리, 파일 경로, 문서 ID)
    fn iter_all_category_file_paths(&self, _batch: usize) -> Result<Vec<(String, String, i64)>, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_categories_pdf_stats(&self, _categories: &[String]) -> Result<Value, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_category_pdf_stats(&self, _category: &str) -> Result<Value, EsError> {
        Err(EsError::Unsupported)
    }
    fn get_by_id_source(&self, _id: &str) -> Result<Option<Value>, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_by_path_source(&self, _path: &str) -> Result<Option<Value>, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_by_category(&self, _category: &str, _max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        Err(EsError::Unsupported)
    }
    /// (히트, 전체 건수, 다음 `search_after`)
    fn search_by_category_paged(&self, _category: &str, _size: i64, _search_after: Option<&[Value]>) -> Result<PagedHits, EsError> {
        Err(EsError::Unsupported)
    }
    fn count_by_categories(&self, _categories: &[String], _prefix: bool) -> Result<BTreeMap<String, i64>, EsError> {
        Err(EsError::Unsupported)
    }
    fn count_by_category(&self, _category: &str, _prefix: bool) -> Result<i64, EsError> {
        Err(EsError::Unsupported)
    }
    fn rename_category(&self, _old_category: &str, _new_category: &str) -> Result<Value, EsError> {
        Err(EsError::Unsupported)
    }
    fn delete_by_category(&self, _category: &str, _prefix: bool) -> Result<Value, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_by_keyword(&self, _keyword: &str, _max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_by_keyword_paged(&self, _args: &Value) -> Result<(Vec<Hit>, i64), EsError> {
        Err(EsError::Unsupported)
    }
    fn search_similar_docs(&self, _args: &Value) -> Result<Vec<Hit>, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_similar_docs_paged(&self, _args: &Value) -> Result<(Vec<Hit>, i64), EsError> {
        Err(EsError::Unsupported)
    }
    fn search_latest_docs(&self, _max_result_count: i64, _exclude_categories: Option<&[String]>) -> Result<(Vec<Hit>, i64), EsError> {
        Err(EsError::Unsupported)
    }
    fn backfill_created_time(&self, _path_prefix: &Path) -> Result<Value, EsError> {
        Err(EsError::Unsupported)
    }
}
