//! Elasticsearch 접근 계층의 trait와 오류. 구현(`ESManager`)은 `libs/es_client`를 감싸서 이 trait를 만족시킨다.
//! 설계: docs/rust-test-seams.md 3.3. 모든 메서드의 기본 구현은 `Err(EsError::Unsupported)`라서, 테스트 대역은 쓰는 메서드만 구현한다.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use chrono::{DateTime, Local};
use serde_json::{Map, Value, json};
use sysdeps::{Clock, EnvFn, FsOps, RealFs, SystemClock};

pub use es_client::EsError;
pub use es_client::transport::{Conflicts, EsRequest, EsTransport, HttpTransport};

/// `update_by_query` 결과. `failures`는 ES가 준 실패 항목 그대로다.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct UpdateByQueryResult {
    pub updated: i64,
    pub failures: Vec<Value>,
}

/// `delete_by_query` 결과. `failures`는 ES가 준 실패 항목 그대로다.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct DeleteByQueryResult {
    pub deleted: i64,
    pub failures: Vec<Value>,
}

/// 검색 결과 한 건.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// 문서 ID
    pub id: i64,
    /// `_source`
    pub source: Value,
    pub score: f64,
}

impl Hit {
    pub fn new(id: i64, source: Value, score: f64) -> Self {
        Self { id, source, score }
    }
}

pub type Docs = BTreeMap<i64, Value>;

/// `search_after`로 넘기는 검색 결과 한 페이지.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PagedHits {
    pub hits: Vec<Hit>,
    /// 전체 건수
    pub total: i64,
    /// 다음 페이지의 `search_after` 값
    pub search_after: Option<Vec<Value>>,
}

/// 정렬값을 함께 받은 커버 파일 문서 한 건.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverFileHit {
    /// 문서 ID
    pub id: i64,
    pub source: Value,
    /// ES `sort` 값 `[수정, 생성, 경로]`
    pub sort: Vec<Value>,
}

impl CoverFileHit {
    pub fn new(id: i64, source: Value, sort: Vec<Value>) -> Self {
        Self { id, source, sort }
    }
}

/// 커버 파일 한 페이지.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoverFileHits {
    pub hits: Vec<CoverFileHit>,
    /// 전체 건수
    pub total: i64,
}

/// 고수준 facade. ES 검색·색인·관리 메서드를 묶는다.
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
    fn rename_category(&self, _old_category: &str, _new_category: &str) -> Result<UpdateByQueryResult, EsError> {
        Err(EsError::Unsupported)
    }
    fn delete_by_category(&self, _category: &str, _prefix: bool) -> Result<DeleteByQueryResult, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_by_keyword(&self, _keyword: &str, _max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_by_keyword_paged(&self, _args: &KeywordSearchArgs) -> Result<(Vec<Hit>, i64), EsError> {
        Err(EsError::Unsupported)
    }
    fn search_similar_docs(&self, _args: &SimilarDocsArgs) -> Result<Vec<Hit>, EsError> {
        Err(EsError::Unsupported)
    }
    fn search_similar_docs_paged(&self, _args: &SimilarDocsArgs) -> Result<(Vec<Hit>, i64), EsError> {
        Err(EsError::Unsupported)
    }
    fn search_latest_docs(&self, _max_result_count: i64, _exclude_categories: Option<&[String]>) -> Result<(Vec<Hit>, i64), EsError> {
        Err(EsError::Unsupported)
    }
    fn backfill_created_time(&self, _path_prefix: &Path) -> Result<Value, EsError> {
        Err(EsError::Unsupported)
    }

    /// 커버 형식 문서가 있는 카테고리별 (문서 수, 최근 수정 시각, 최근 생성 시각). 시각은 epoch 밀리초이고 없으면 `i64::MIN`에 해당하는 값이다.
    fn aggregate_cover_categories(&self) -> Result<HashMap<String, (usize, f64, f64)>, EsError> {
        Err(EsError::Unsupported)
    }

    /// 카테고리에 직접 속한 커버 형식 파일을 최신순으로 한 페이지. `search_after`는 정렬값 `(수정, 생성, 경로)`다.
    /// 반환: ([(문서 id, 문서, 정렬값)], 전체 건수).
    fn search_cover_files_paged(&self, _categories: &[String], _size: i64, _search_after: Option<(i64, i64, String)>, _exclude: &[String]) -> Result<CoverFileHits, EsError> {
        Err(EsError::Unsupported)
    }

    /// 작품 디렉토리마다 대표 후보(경로순 앞쪽 몇 권).
    fn search_group_representatives(&self, _keys: &[String], _exclude: &[String]) -> Result<HashMap<String, Vec<(i64, Value)>>, EsError> {
        Err(EsError::Unsupported)
    }
}

// ---------------------------------------------------------------------------
// 접속
// ---------------------------------------------------------------------------

/// `Elasticsearch(...)` 생성자에 넘기던 인자.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectOptions {
    pub hosts: Vec<String>,
    /// (사용자, 비밀번호)
    pub basic_auth: (String, String),
    /// 초 단위 요청 제한 시간.
    pub request_timeout: f64,
    pub retry_on_timeout: bool,
    /// HTTPS 서버 인증서 검증 여부. Rust transport는 검증을 끄지 않는다.
    pub verify_certs: Option<bool>,
    pub ssl_show_warn: Option<bool>,
    /// ECK 같은 사설 CA 인증서 PEM 경로.
    pub ca_cert_path: Option<PathBuf>,
    /// 사용할 색인 이름.
    pub index: String,
}

/// `ConnectOptions`로 전송 계층을 만든다. 테스트는 `DummyEs`를 돌려주는 커넥터를 쓴다.
pub trait EsConnector {
    fn connect(&self, options: &ConnectOptions) -> Arc<dyn EsTransport>;
}

/// 실제 ES에 `HttpTransport`로 붙는 커넥터.
pub struct RealEsConnector;

impl EsConnector for RealEsConnector {
    fn connect(&self, options: &ConnectOptions) -> Arc<dyn EsTransport> {
        let host = options.hosts.first().map(String::as_str).unwrap_or("");
        Arc::new(HttpTransport::new_with_ca(
            host,
            &options.basic_auth.0,
            &options.basic_auth.1,
            &options.index,
            Duration::from_secs_f64(options.request_timeout.max(0.001)),
            options.ca_cert_path.as_deref(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ESManagerError {
    /// 필수 환경 변수가 없다.
    FatalConfigError(i32),
    Elasticsearch(EsError),
}

impl fmt::Display for ESManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ESManagerError::FatalConfigError(code) => write!(f, "종료 코드 {code}"),
            ESManagerError::Elasticsearch(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ESManagerError {}

impl From<EsError> for ESManagerError {
    fn from(e: EsError) -> Self {
        ESManagerError::Elasticsearch(e)
    }
}

const ENV_KEYS: [&str; 4] = ["TM_ES_BOOK_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"];
const CONNECT_MAX_RETRIES: u32 = 5;
const MAX_RESULT_WINDOW: i64 = 10_000;

// ---------------------------------------------------------------------------
// 인자 묶음
// ---------------------------------------------------------------------------

/// `search_similar_docs`와 `_paged`의 인자와 기본값.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SimilarDocsArgs {
    pub category: String,
    pub title: String,
    pub author: String,
    pub file_type: String,
    pub file_size: i64,
    pub summary: String,
    /// 음수면 `DEFAULT_MAX_RESULT_COUNT`다. (`search_similar_docs`만 쓴다)
    pub max_result_count: i64,
    pub exclude_id: Option<i64>,
    /// `search_similar_docs_paged`만 쓴다.
    pub size: i64,
    pub offset: i64,
}

impl Default for SimilarDocsArgs {
    fn default() -> Self {
        SimilarDocsArgs {
            category: String::new(),
            title: String::new(),
            author: String::new(),
            file_type: String::new(),
            file_size: 0,
            summary: String::new(),
            max_result_count: -1,
            exclude_id: None,
            size: 10,
            offset: 0,
        }
    }
}

/// 키워드 검색 한 쪽의 인자.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct KeywordSearchArgs {
    pub keyword: String,
    pub size: i64,
    pub offset: i64,
    pub exclude_categories: Option<Vec<String>>,
    /// 이 카테고리 안에서만 찾는다.
    pub category: Option<String>,
}

impl Default for KeywordSearchArgs {
    fn default() -> Self {
        KeywordSearchArgs { keyword: String::new(), size: 10, offset: 0, exclude_categories: None, category: None }
    }
}

/// `update`에 넘기는 필드. 비어 있거나 0인 값은 보내지 않는다.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateFields {
    pub category: String,
    pub title: String,
    pub author: String,
    pub file_path: String,
    pub file_type: String,
    pub file_size: i64,
    pub summary: String,
    pub updated_time: String,
}

impl UpdateFields {
    /// `EsManagerApi::update`가 받는 JSON 필드에서 만든다.
    pub fn from_value(fields: &Value) -> Self {
        let text = |key: &str| fields.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        UpdateFields {
            category: text("category"),
            title: text("title"),
            author: text("author"),
            file_path: text("file_path"),
            file_type: text("file_type"),
            file_size: fields.get("file_size").and_then(Value::as_i64).unwrap_or(0),
            summary: text("summary"),
            updated_time: text("updated_time"),
        }
    }

    fn to_doc(&self) -> Value {
        let mut doc = Map::new();
        for (key, value) in [("category", &self.category), ("title", &self.title), ("author", &self.author), ("file_path", &self.file_path), ("file_type", &self.file_type)] {
            if !value.is_empty() {
                doc.insert(key.into(), json!(value));
            }
        }
        if self.file_size != 0 {
            doc.insert("file_size".into(), json!(self.file_size));
        }
        for (key, value) in [("summary", &self.summary), ("updated_time", &self.updated_time)] {
            if !value.is_empty() {
                doc.insert(key.into(), json!(value));
            }
        }
        Value::Object(doc)
    }
}

// ---------------------------------------------------------------------------
// 커서
// ---------------------------------------------------------------------------

/// ES sort 값을 클라이언트에 내보낼 불투명 커서 문자열로 만든다
/// (JSON을 URL-safe base64로 감싼다. 비ASCII는 `\u` 이스케이프 없이 UTF-8 그대로다).
pub fn encode_category_cursor(sort_values: &[Value]) -> String {
    URL_SAFE.encode(Value::Array(sort_values.to_vec()).to_string())
}

/// 커서를 sort 값으로 되돌린다. 형식이 잘못되면 `None`이다.
pub fn decode_category_cursor(cursor: &str) -> Option<Vec<Value>> {
    let bytes = URL_SAFE.decode(cursor).ok()?;
    match serde_json::from_slice::<Value>(&bytes).ok()? {
        Value::Array(values) => Some(values),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// ESManager
// ---------------------------------------------------------------------------

/// `search`를 대신하는 함수. `(query, sort, max_result_count, source_fields)`를 받아 히트를 돌려준다.
pub type SearchStub = Arc<dyn Fn(&Value, Option<&Value>, i64, Option<&[String]>) -> Result<Vec<Hit>, EsError> + Send + Sync>;

pub struct ESManager {
    es: Arc<dyn EsTransport>,
    clock: Arc<dyn Clock>,
    fs: Arc<dyn FsOps>,
    search_stub: Option<SearchStub>,
}

fn parse_error(message: impl Into<String>) -> EsError {
    EsError::Parse(message.into())
}

fn hit_id(hit: &Value) -> Result<i64, EsError> {
    let id = &hit["_id"];
    id.as_str().and_then(|s| s.parse().ok()).or_else(|| id.as_i64()).ok_or_else(|| parse_error(format!("_id를 정수로 읽지 못했다: {id}")))
}

fn hits_of(response: &Value) -> &[Value] {
    response["hits"]["hits"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn total_of(response: &Value) -> Result<i64, EsError> {
    response["hits"]["total"]["value"].as_i64().ok_or_else(|| parse_error("hits.total.value가 없다"))
}

fn source_of(hit: &Value) -> Value {
    hit.get("_source").cloned().unwrap_or(Value::Null)
}

/// 점수를 `base` 기준 0~100으로 바꾼다. `cap`이면 100을 넘기지 않는다.
fn normalize(score: f64, base: f64, cap: bool) -> f64 {
    if base <= 0.0 {
        return 0.0;
    }
    let value = score * 100.0 / base;
    if cap { value.min(100.0) } else { value }
}

fn resolve_count(max_result_count: i64) -> i64 {
    if max_result_count < 0 { ESManager::DEFAULT_MAX_RESULT_COUNT } else { max_result_count }
}

/// 카테고리 하나와 그 하위(`category/`)를 함께 매칭하는 절.
fn category_or_children(category: &str) -> Value {
    json!({"bool": {"should": [{"term": {"category": category}}, {"prefix": {"category": format!("{category}/")}}], "minimum_should_match": 1}})
}

use crate::has_content;

impl ESManager {
    pub const DEFAULT_MAX_RESULT_COUNT: i64 = 10;
    /// 목록 응답에는 summary가 없으므로 받아오지 않는다. summary는 `search_by_id`(단건)만 쓴다.
    pub const LIST_SOURCE_FIELDS: [&'static str; 11] = es_client::LIST_SOURCE_FIELDS;
    /// 커버를 그릴 수 있는 형식.
    pub const COVER_FILE_TYPES: [&'static str; 2] = ["epub", "pdf"];
    pub const REPRESENTATIVE_CANDIDATES: i64 = 5;
    /// summary 원문을 match로 넣으면 느려서 more_like_this로 상위 용어만 쓴다. 질의는 `es_client`가 만든다.
    pub const SIMILAR_SUMMARY_MAX_QUERY_TERMS: u32 = es_client::SIMILAR_SUMMARY_MAX_QUERY_TERMS;
    pub const SIMILAR_SUMMARY_MIN_DOC_FREQ: u32 = es_client::SIMILAR_SUMMARY_MIN_DOC_FREQ;

    /// 카테고리 목록의 전순서 정렬. `_id`를 tie-breaker로 쓰면 이 클러스터에서 질의가 통째로 실패한다.
    pub fn category_sort() -> Value {
        json!([{"title.keyword": "asc"}, {"file_path": "asc"}])
    }

    pub fn latest_sort() -> Value {
        json!([{"updated_time": {"order": "desc", "missing": "_last"}}, {"created_time": {"order": "desc", "missing": "_last"}}, {"file_path": {"order": "asc"}}])
    }

    /// 이미 만든 전송 계층으로 매니저를 만든다. 색인 이름은 `es.index_name()`이다.
    pub fn with_transport(es: Arc<dyn EsTransport>) -> ESManager {
        ESManager { es, clock: Arc::new(SystemClock), fs: Arc::new(RealFs), search_stub: None }
    }

    /// `insert` 재시도 대기에 쓸 시계를 바꾼다.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> ESManager {
        self.clock = clock;
        self
    }

    /// `backfill_created_time`이 쓸 파일시스템을 바꾼다.
    pub fn with_fs(mut self, fs: Arc<dyn FsOps>) -> ESManager {
        self.fs = fs;
        self
    }

    /// `search` 반환을 직접 주입한다. `search`를 거치는 검색 메서드(`search_by_title` 등)에만 영향을 준다.
    pub fn with_search_stub(mut self, stub: SearchStub) -> ESManager {
        self.search_stub = Some(stub);
        self
    }

    /// 환경 변수로 접속하고 `info`로 확인한다. 실패하면 `2**(n+1)`초(최대 10초)씩 쉬며 5번 시도한다.
    pub fn connect(env: EnvFn, connector: &dyn EsConnector, clock: &dyn Clock) -> Result<ESManager, ESManagerError> {
        Self::connect_index(env, "", connector, clock)
    }

    /// `connect`와 같으나 색인 이름을 직접 정한다(빈 문자열이면 `TM_ES_BOOK_INDEX`).
    pub fn connect_index(env: EnvFn, index_name: &str, connector: &dyn EsConnector, clock: &dyn Clock) -> Result<ESManager, ESManagerError> {
        for key in ENV_KEYS {
            if env(key).is_none() {
                log::error!("The environment variable {key} is not set.");
                return Err(ESManagerError::FatalConfigError(-1));
            }
        }
        let get = |key: &str| env(key).unwrap_or_default();
        let url = get("TM_ES_URL");
        let https = url.to_lowercase().starts_with("https");
        let index = if index_name.is_empty() { get("TM_ES_BOOK_INDEX") } else { index_name.to_string() };
        // HTTPS는 검증을 유지한다. 사설 CA가 있으면 전송기에 전달한다.
        let options = ConnectOptions {
            hosts: vec![url],
            basic_auth: (get("TM_ES_USER"), get("TM_ES_PASSWORD")),
            request_timeout: 10.0,
            retry_on_timeout: true,
            verify_certs: https.then_some(true),
            ssl_show_warn: None,
            ca_cert_path: env("TM_ES_CA_CERT").map(PathBuf::from),
            index,
        };
        let es = connector.connect(&options);
        for attempt in 0..CONNECT_MAX_RETRIES {
            match es.send(&EsRequest::Info) {
                Ok(_) => {
                    log::info!("Elasticsearch 연결 성공");
                    break;
                }
                Err(e) if e.is_connection() => {
                    if attempt + 1 < CONNECT_MAX_RETRIES {
                        let wait = (2u64 << attempt).min(10);
                        log::warn!("ES 연결 실패 (시도 {}/{}): {e}. {wait}초 후 재시도...", attempt + 1, CONNECT_MAX_RETRIES);
                        clock.sleep(wait as f64);
                    } else {
                        log::error!("ES 연결 최종 실패: {e}");
                        return Err(e.into());
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(ESManager::with_transport(es))
    }

    /// 프로세스 환경 변수로 실제 ES에 접속한다.
    pub fn from_env() -> Result<ESManager, ESManagerError> {
        Self::connect(&|key| std::env::var(key).ok(), &RealEsConnector, &SystemClock)
    }

    fn index(&self) -> &str {
        self.es.index_name()
    }
}

mod created_time_backfiller;
mod doc_searcher;
mod doc_writer;
mod index_manager;

pub use doc_searcher::CategoryFilePaths;

// ---------------------------------------------------------------------------
// EsManagerApi
// ---------------------------------------------------------------------------

fn texts(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

impl EsManagerApi for ESManager {
    fn create_index(&self) -> Result<Value, EsError> {
        ESManager::create_index(self)
    }

    fn search_by_id(&self, doc_id: i64) -> Result<Option<Value>, EsError> {
        self.get_source(&doc_id.to_string())
    }

    fn update(&self, doc_id: i64, fields: &Value) -> Result<bool, EsError> {
        ESManager::update(self, doc_id, &UpdateFields::from_value(fields))
    }

    fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        ESManager::delete(self, doc_id)
    }

    fn insert(&self, docs: &Docs) -> Result<Vec<i64>, EsError> {
        let data: Vec<(i64, Value)> = docs.iter().map(|(id, doc)| (*id, doc.clone())).collect();
        ESManager::insert(self, &data, usize::MAX, 3)
    }

    fn delete_by_file_paths(&self, paths: &[String], exclude_ids: Option<&[i64]>) -> Result<i64, EsError> {
        Ok(ESManager::delete_by_file_paths(self, &texts(paths), exclude_ids))
    }

    fn delete_by_ids(&self, ids: &[i64], chunk_size: usize) -> Result<i64, EsError> {
        Ok(ESManager::delete_by_ids(self, ids, chunk_size))
    }

    fn refresh(&self) -> Result<(), EsError> {
        ESManager::refresh(self)
    }

    fn search_and_aggregate_by_category(&self) -> Result<BTreeMap<String, i64>, EsError> {
        let counts = ESManager::search_and_aggregate_by_category(self)?;
        Ok(counts.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0))).collect()).unwrap_or_default())
    }

    fn iter_all_category_file_paths(&self, batch: usize) -> Result<Vec<(String, String, i64)>, EsError> {
        ESManager::iter_all_category_file_paths(self, batch).collect()
    }

    fn search_categories_pdf_stats(&self, categories: &[String]) -> Result<Value, EsError> {
        ESManager::search_categories_pdf_stats(self, &texts(categories))
    }

    fn search_category_pdf_stats(&self, category: &str) -> Result<Value, EsError> {
        ESManager::search_category_pdf_stats(self, category)
    }

    fn get_by_id_source(&self, id: &str) -> Result<Option<Value>, EsError> {
        self.get_source(id)
    }

    /// 문서 3.3의 `EsLookup::search_by_path`에 대응한다. file_path 정확 일치 첫 문서의 `_source`다.
    fn search_by_path_source(&self, path: &str) -> Result<Option<Value>, EsError> {
        let response = self.es.send(&EsRequest::search(json!({"query": {"term": {"file_path": path}}, "size": 1})))?;
        Ok(hits_of(&response).first().map(source_of))
    }

    fn search_by_category(&self, category: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        ESManager::search_by_category(self, category, max_result_count)
    }

    fn search_by_category_paged(&self, category: &str, size: i64, search_after: Option<&[Value]>) -> Result<PagedHits, EsError> {
        Ok(ESManager::search_by_category_paged(self, category, size, search_after))
    }

    fn count_by_categories(&self, categories: &[String], prefix: bool) -> Result<BTreeMap<String, i64>, EsError> {
        ESManager::count_by_categories(self, &texts(categories), prefix)
    }

    fn count_by_category(&self, category: &str, prefix: bool) -> Result<i64, EsError> {
        ESManager::count_by_category(self, category, prefix)
    }

    fn rename_category(&self, old_category: &str, new_category: &str) -> Result<UpdateByQueryResult, EsError> {
        ESManager::rename_category(self, old_category, new_category)
    }

    fn delete_by_category(&self, category: &str, prefix: bool) -> Result<DeleteByQueryResult, EsError> {
        ESManager::delete_by_category(self, category, prefix)
    }

    fn search_by_keyword(&self, keyword: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        ESManager::search_by_keyword(self, keyword, max_result_count)
    }

    fn search_by_keyword_paged(&self, args: &KeywordSearchArgs) -> Result<(Vec<Hit>, i64), EsError> {
        let exclude = args.exclude_categories.as_deref().map(texts);
        ESManager::search_by_keyword_paged(self, &args.keyword, args.size, args.offset, exclude.as_deref(), args.category.as_deref())
    }

    fn search_similar_docs(&self, args: &SimilarDocsArgs) -> Result<Vec<Hit>, EsError> {
        ESManager::search_similar_docs(self, args)
    }

    fn search_similar_docs_paged(&self, args: &SimilarDocsArgs) -> Result<(Vec<Hit>, i64), EsError> {
        ESManager::search_similar_docs_paged(self, args)
    }

    fn search_latest_docs(&self, max_result_count: i64, exclude_categories: Option<&[String]>) -> Result<(Vec<Hit>, i64), EsError> {
        let exclude = exclude_categories.map(texts);
        Ok(ESManager::search_latest_docs(self, max_result_count, exclude.as_deref()))
    }

    fn backfill_created_time(&self, path_prefix: &Path) -> Result<Value, EsError> {
        Ok(ESManager::backfill_created_time(self, path_prefix, 500))
    }

    fn aggregate_cover_categories(&self) -> Result<HashMap<String, (usize, f64, f64)>, EsError> {
        // 시각이 없는 카테고리는 ES 정렬값과 같은 `i64::MIN`으로 채운다(가장 뒤).
        let missing = i64::MIN as f64;
        Ok(ESManager::aggregate_cover_categories(self)
            .into_iter()
            .map(|(category, (count, updated, created))| (category, (count.max(0) as usize, updated.unwrap_or(missing), created.unwrap_or(missing))))
            .collect())
    }

    fn search_cover_files_paged(&self, categories: &[String], size: i64, search_after: Option<(i64, i64, String)>, exclude: &[String]) -> Result<CoverFileHits, EsError> {
        let (categories, exclude) = (texts(categories), texts(exclude));
        let after: Option<Vec<Value>> = search_after.map(|(updated, created, path)| vec![json!(updated), json!(created), json!(path)]);
        Ok(ESManager::search_cover_files_paged(self, &categories, size, after.as_deref(), Some(&exclude)))
    }

    fn search_group_representatives(&self, keys: &[String], exclude: &[String]) -> Result<HashMap<String, Vec<(i64, Value)>>, EsError> {
        let (keys, exclude) = (texts(keys), texts(exclude));
        Ok(ESManager::search_group_representatives(self, &keys, Some(&exclude)))
    }
}
