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

mod http;

pub use http::HttpTransport;

/// (문서 ID, `_source`, 점수)
pub type Hit = (i64, Value, f64);
pub type Docs = BTreeMap<i64, Value>;
/// (히트, 전체 건수, 다음 `search_after`)
pub type PagedHits = (Vec<Hit>, i64, Option<Vec<Value>>);
/// 커버 파일 한 페이지: ([(문서 id, 문서, 정렬값)], 전체 건수).
pub type CoverFileHits = (Vec<(i64, Value, Vec<Value>)>, i64);

#[derive(Debug, Clone, PartialEq)]
pub enum EsError {
    /// 필수 환경 변수가 없다.
    MissingEnv(String),
    /// 연결 실패나 시간 초과 같은 전송 계층 오류.
    Transport(String),
    /// ES가 오류 상태 코드를 돌려줬다.
    Status {
        status: u16,
        body: String,
    },
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

impl EsError {
    /// 문서나 색인이 없다(404).
    pub fn is_not_found(&self) -> bool {
        matches!(self, EsError::Status { status: 404, .. })
    }

    /// 연결을 만들지 못했거나 응답을 해석하지 못했다. Python `ConnectionError`, `ConnectionTimeout`, `SerializationError`에 대응한다.
    pub fn is_connection(&self) -> bool {
        matches!(self, EsError::Transport(_) | EsError::Parse(_))
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
    /// 사용할 색인 이름. Python은 `ESManager.index_name`으로 따로 들고 있었다.
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
    /// 필수 환경 변수가 없다. Python은 `sys.exit(-1)`이다.
    SystemExit(i32),
    Elasticsearch(EsError),
}

impl fmt::Display for ESManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ESManagerError::SystemExit(code) => write!(f, "종료 코드 {code}"),
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

/// `search_similar_docs`와 `_paged`의 인자. 기본값은 Python 시그니처와 같다.
#[derive(Debug, Clone, PartialEq)]
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

impl SimilarDocsArgs {
    /// `EsManagerApi`가 받는 JSON 인자에서 만든다. 없는 키는 기본값이다.
    pub fn from_value(args: &Value) -> Self {
        let d = SimilarDocsArgs::default();
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let int = |key: &str, default: i64| args.get(key).and_then(Value::as_i64).unwrap_or(default);
        SimilarDocsArgs {
            category: text("category"),
            title: text("title"),
            author: text("author"),
            file_type: text("file_type"),
            file_size: int("file_size", d.file_size),
            summary: text("summary"),
            max_result_count: int("max_result_count", d.max_result_count),
            exclude_id: args.get("exclude_id").and_then(Value::as_i64),
            size: int("size", d.size),
            offset: int("offset", d.offset),
        }
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

/// ES sort 값을 클라이언트에 내보낼 불투명 커서 문자열로 만든다. Python `encode_category_cursor`와 같다
/// (JSON을 URL-safe base64로 감싼다. 비ASCII는 `\u` 이스케이프 없이 UTF-8 그대로라 바이트열은 Python과 다를 수 있으나 서로 복원된다).
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

/// `_search`를 대신하는 함수. `(query, sort, max_result_count, source_fields)`를 받아 히트를 돌려준다.
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

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_none_or(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

impl ESManager {
    pub const DEFAULT_MAX_RESULT_COUNT: i64 = 10;
    /// 목록 응답에는 summary가 없으므로 받아오지 않는다. summary는 `search_by_id`(단건)만 쓴다.
    pub const LIST_SOURCE_FIELDS: [&'static str; 11] = ["category", "title", "author", "file_path", "file_type", "file_size", "line_count", "page_count", "isbn", "created_time", "updated_time"];
    /// 커버를 그릴 수 있는 형식.
    pub const COVER_FILE_TYPES: [&'static str; 2] = ["epub", "pdf"];
    pub const REPRESENTATIVE_CANDIDATES: i64 = 5;
    /// summary 원문을 match로 넣으면 느려서 more_like_this로 상위 용어만 쓴다.
    pub const SIMILAR_SUMMARY_MAX_QUERY_TERMS: u32 = 25;
    pub const SIMILAR_SUMMARY_MIN_DOC_FREQ: u32 = 3;

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

    /// `_search` 반환을 직접 주입한다. `_search`를 거치는 검색 메서드(`search_by_title` 등)에만 영향을 준다.
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
                return Err(ESManagerError::SystemExit(-1));
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
            match es.call("info", &json!({})) {
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

    // ---- 색인 ----

    pub fn do_exist_index(&self) -> Result<bool, EsError> {
        log::debug!("do_exist_index()");
        Ok(truthy(Some(&self.es.call("indices.exists", &json!({"index": self.index()}))?)))
    }

    /// 주어진 ID 목록의 기존 file_path를 조회한다. 반환: {inode: file_path}
    pub fn get_existing_paths(&self, doc_ids: &[i64]) -> Result<HashMap<i64, String>, EsError> {
        if doc_ids.is_empty() {
            return Ok(HashMap::new());
        }
        log::debug!("get_existing_paths({} ids)", doc_ids.len());
        let response = self.es.call("mget", &json!({"docs": self.mget_docs(doc_ids), "source": ["file_path"]}))?;
        let mut result = HashMap::new();
        for doc in response["docs"].as_array().ok_or_else(|| parse_error("docs가 없다"))? {
            if doc["found"].as_bool().unwrap_or(false) {
                let path = doc["_source"]["file_path"].as_str().ok_or_else(|| parse_error("_source.file_path가 없다"))?;
                result.insert(hit_id(doc)?, path.to_string());
            }
        }
        Ok(result)
    }

    /// 주어진 ID 목록의 기존 created_time 관련 필드와 updated_time을 조회한다.
    pub fn get_existing_created_time_metadata(&self, doc_ids: &[i64]) -> Result<HashMap<i64, Value>, EsError> {
        if doc_ids.is_empty() {
            return Ok(HashMap::new());
        }
        log::debug!("get_existing_created_time_metadata({} ids)", doc_ids.len());
        let response = self.es.call("mget", &json!({"docs": self.mget_docs(doc_ids), "source": ["created_time", "created_time_source", "updated_time"]}))?;
        let mut result = HashMap::new();
        for doc in response["docs"].as_array().ok_or_else(|| parse_error("docs가 없다"))? {
            if !doc["found"].as_bool().unwrap_or(false) {
                continue;
            }
            let mut metadata = Map::new();
            for field in ["created_time", "created_time_source", "updated_time"] {
                if let Some(value) = doc["_source"].get(field) {
                    metadata.insert(field.to_string(), value.clone());
                }
            }
            if !metadata.is_empty() {
                result.insert(hit_id(doc)?, Value::Object(metadata));
            }
        }
        Ok(result)
    }

    fn mget_docs(&self, doc_ids: &[i64]) -> Vec<Value> {
        doc_ids.iter().map(|id| json!({"_index": self.index(), "_id": id.to_string()})).collect()
    }

    pub fn create_index(&self) -> Result<Value, EsError> {
        log::debug!("create_index()");
        let settings = json!({
            "index": {"similarity": {"default": {"type": "BM25"}}},
            "analysis": {
                "tokenizer": {"nori_tokenizer": {"type": "nori_tokenizer", "decompound_mode": "discard"}},
                "filter": {"nori_posfilter": {"type": "nori_part_of_speech", "stoptags": [
                    // 어미
                    "EC", "EF", "EP", "ETM", "ETN",
                    // 조사
                    "JC", "JKB", "JKC", "JKG", "JKO", "JKQ", "JKS", "JKV", "JX",
                    // 기호
                    "SC", "SE", "SF", "SP", "SSC", "SSO", "SY",
                    // 접미사
                    "XSA", "XSN", "XSV",
                    // 기타
                    "IC", "MAJ",
                ]}},
                "analyzer": {"nori_analyzer": {"type": "custom", "tokenizer": "nori_tokenizer", "filter": ["nori_posfilter", "lowercase"]}},
            },
        });
        let mappings = json!({"properties": {
            "category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}}},
            "title": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
            "author": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
            // EPUB의 dc:publisher. 전집 판정의 결정적 증거라 자동분류가 쓴다.
            "publisher": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
            "file_path": {"type": "keyword"},
            "file_type": {"type": "keyword"},
            "file_size": {"type": "unsigned_long"},
            "line_count": {"type": "integer"},
            "page_count": {"type": "integer"},
            "isbn": {"type": "keyword"},
            "summary": {"type": "text", "analyzer": "nori_analyzer"},
            "created_time": {"type": "date"},
            "created_time_source": {"type": "keyword"},
            "updated_time": {"type": "date"},
        }});

        if self.do_exist_index()? {
            self.ensure_existing_index_mappings();
            return Ok(json!({"acknowledged": true}));
        }
        match self.es.call("indices.create", &json!({"index": self.index(), "body": {"settings": settings, "mappings": mappings}})) {
            Err(EsError::Status { status: 400, body }) if body.contains("resource_already_exists_exception") => {
                log::info!("Index {} already exists, skipping creation", self.index());
                self.ensure_existing_index_mappings();
                Ok(json!({"acknowledged": true}))
            }
            other => other,
        }
    }

    /// 매핑을 한 번만 읽어 세 필드를 모두 점검한다(Python은 필드마다 읽었다).
    fn ensure_existing_index_mappings(&self) {
        match self.es.call("indices.get_mapping", &json!({"index": self.index()})) {
            Ok(mapping) => {
                self.apply_category_nori(&mapping);
                self.apply_created_time(&mapping);
                self.apply_publisher(&mapping);
            }
            Err(e) => log::warn!("Failed to read mapping of index {}: {e}", self.index()),
        }
    }

    fn index_mappings<'a>(&self, mapping: &'a Value) -> Option<&'a Value> {
        mapping.get(self.index())?.get("mappings")
    }

    fn put_properties(&self, properties: Value) -> Result<(), EsError> {
        self.es.call("indices.put_mapping", &json!({"index": self.index(), "properties": properties})).map(|_| ())
    }

    /// 기존 인덱스에 category.nori 서브필드가 없으면 추가한다. 오류는 삼킨다.
    pub fn _ensure_category_nori_subfield(&self) {
        match self.es.call("indices.get_mapping", &json!({"index": self.index()})) {
            Ok(mapping) => self.apply_category_nori(&mapping),
            Err(e) => log::warn!("Failed to add category.nori sub-field: {e}"),
        }
    }

    fn apply_category_nori(&self, mapping: &Value) {
        let Some(properties) = self.index_mappings(mapping).and_then(|m| m.get("properties")) else {
            log::warn!("Failed to add category.nori sub-field: 매핑에 properties가 없다");
            return;
        };
        if properties["category"]["fields"].get("nori").is_some() {
            return;
        }
        log::info!("Adding category.nori sub-field to index {}", self.index());
        match self.put_properties(json!({"category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}}}})) {
            Ok(()) => log::info!("category.nori sub-field added successfully"),
            Err(e) => log::warn!("Failed to add category.nori sub-field: {e}"),
        }
    }

    /// 기존 인덱스에 created_time 관련 필드가 없으면 추가한다. 오류는 삼킨다.
    pub fn _ensure_created_time_field(&self) {
        match self.es.call("indices.get_mapping", &json!({"index": self.index()})) {
            Ok(mapping) => self.apply_created_time(&mapping),
            Err(e) => log::warn!("Failed to add created_time fields: {e}"),
        }
    }

    fn apply_created_time(&self, mapping: &Value) {
        let Some(mappings) = self.index_mappings(mapping) else {
            log::warn!("Failed to add created_time fields: 매핑이 없다");
            return;
        };
        let properties = mappings.get("properties");
        let has = |name: &str| properties.is_some_and(|p| p.get(name).is_some());
        let mut missing = Map::new();
        if !has("created_time") {
            missing.insert("created_time".into(), json!({"type": "date"}));
        }
        if !has("created_time_source") {
            missing.insert("created_time_source".into(), json!({"type": "keyword"}));
        }
        if missing.is_empty() {
            return;
        }
        log::info!("Adding created_time fields to index {}: {:?}", self.index(), missing.keys().collect::<Vec<_>>());
        match self.put_properties(Value::Object(missing)) {
            Ok(()) => log::info!("created_time fields added successfully"),
            Err(e) => log::warn!("Failed to add created_time fields: {e}"),
        }
    }

    /// 기존 인덱스에 publisher 필드가 없으면 추가한다. 오류는 삼킨다.
    pub fn _ensure_publisher_field(&self) {
        match self.es.call("indices.get_mapping", &json!({"index": self.index()})) {
            Ok(mapping) => self.apply_publisher(&mapping),
            Err(e) => log::warn!("Failed to add publisher field: {e}"),
        }
    }

    fn apply_publisher(&self, mapping: &Value) {
        let Some(mappings) = self.index_mappings(mapping) else {
            log::warn!("Failed to add publisher field: 매핑이 없다");
            return;
        };
        if mappings.get("properties").is_some_and(|p| p.get("publisher").is_some()) {
            return;
        }
        log::info!("Adding publisher field to index {}", self.index());
        match self.put_properties(json!({"publisher": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}}})) {
            Ok(()) => log::info!("publisher field added successfully"),
            Err(e) => log::warn!("Failed to add publisher field: {e}"),
        }
    }

    pub fn delete_index(&self) -> Result<(), EsError> {
        log::debug!("delete_index()");
        if self.do_exist_index()? {
            self.es.call("indices.delete", &json!({"index": self.index()}))?;
        }
        Ok(())
    }

    /// 인덱스를 refresh해 최근 변경사항을 검색 가능하게 한다.
    pub fn refresh(&self) -> Result<(), EsError> {
        log::debug!("refresh()");
        self.es.call("indices.refresh", &json!({"index": self.index()})).map(|_| ())
    }

    // ---- 검색 기반 ----

    fn normalized_hits(response: &Value, max_score: f64) -> Result<Vec<Hit>, EsError> {
        hits_of(response).iter().map(|hit| Ok((hit_id(hit)?, source_of(hit), normalize(hit["_score"].as_f64().unwrap_or(0.0), max_score, false)))).collect()
    }

    /// 질의를 실행하고 점수를 최고점 기준 0~100으로 정규화한다. `max_score`가 없으면 빈 결과다.
    /// 1만 건 이하는 scroll 없이 한 번에, 초과하면 scroll로 읽는다.
    pub fn _search(&self, query: &Value, sort: Option<&Value>, max_result_count: i64, source_fields: Option<&[&str]>) -> Result<Vec<Hit>, EsError> {
        if let Some(stub) = &self.search_stub {
            let owned: Option<Vec<String>> = source_fields.map(|fields| fields.iter().map(|f| f.to_string()).collect());
            return stub(query, sort, max_result_count, owned.as_deref());
        }
        let max_result_count = resolve_count(max_result_count);
        // max_result_window 오류를 피하려고 크기를 자른다.
        let size = max_result_count.min(MAX_RESULT_WINDOW);
        log::debug!("_search(max_result_count={max_result_count}, size={size}, query='{query}')");

        if max_result_count <= MAX_RESULT_WINDOW {
            let mut kwargs = json!({"index": self.index(), "query": query, "size": size, "track_scores": true});
            if let Some(sort) = sort {
                kwargs["sort"] = sort.clone();
            }
            if let Some(fields) = source_fields {
                kwargs["source"] = json!(fields);
            }
            let response = self.es.call("search", &kwargs)?;
            let Some(max_score) = response["hits"]["max_score"].as_f64() else { return Ok(Vec::new()) };
            let mut result = Self::normalized_hits(&response, max_score)?;
            result.truncate(max_result_count as usize);
            return Ok(result);
        }

        let mut scroll_id: Option<String> = None;
        let outcome = self.scroll_search(query, sort, size, max_result_count, &mut scroll_id);
        if let Some(id) = scroll_id
            && let Err(e) = self.es.call("clear_scroll", &json!({"scroll_id": id}))
        {
            log::debug!("Failed to clear scroll: {e}");
        }
        outcome
    }

    fn scroll_search(&self, query: &Value, sort: Option<&Value>, size: i64, max_result_count: i64, scroll_id: &mut Option<String>) -> Result<Vec<Hit>, EsError> {
        let mut kwargs = json!({"index": self.index(), "query": query, "scroll": "10m", "track_scores": true, "size": size});
        if let Some(sort) = sort {
            kwargs["sort"] = sort.clone();
        }
        let mut response = self.es.call("search", &kwargs)?;
        if let Some(id) = response["_scroll_id"].as_str() {
            *scroll_id = Some(id.to_string());
        }
        let limit = max_result_count as usize;
        let mut result: Vec<Hit> = Vec::new();
        let Some(max_score) = response["hits"]["max_score"].as_f64() else { return Ok(Vec::new()) };
        for hit in hits_of(&response) {
            result.push((hit_id(hit)?, source_of(hit), normalize(hit["_score"].as_f64().unwrap_or(0.0), max_score, false)));
            if result.len() >= limit {
                return Ok(result);
            }
        }
        while !hits_of(&response).is_empty() {
            response = self.es.call("scroll", &json!({"scroll_id": scroll_id.clone().unwrap_or_default(), "scroll": "10m"}))?;
            if let Some(id) = response["_scroll_id"].as_str() {
                *scroll_id = Some(id.to_string());
            }
            let Some(max_score) = response["hits"]["max_score"].as_f64() else { return Ok(Vec::new()) };
            for hit in hits_of(&response) {
                result.push((hit_id(hit)?, source_of(hit), normalize(hit["_score"].as_f64().unwrap_or(0.0), max_score, false)));
                if result.len() >= limit {
                    return Ok(result);
                }
            }
        }
        result.truncate(limit);
        Ok(result)
    }

    /// `(결과, 전체 건수)`를 돌려주는 from/size 페이지 검색. `ref_score`가 0이면 결과 안의 max_score를 기준으로 쓴다.
    pub fn _search_paged(&self, query: &Value, sort: Option<&Value>, size: i64, offset: i64, ref_score: f64, source_fields: Option<&[&str]>) -> Result<(Vec<Hit>, i64), EsError> {
        let size = size.min(MAX_RESULT_WINDOW);
        log::debug!("_search_paged(size={size}, offset={offset}, query='{query}')");
        let mut kwargs = json!({"index": self.index(), "query": query, "from_": offset, "size": size, "track_scores": true, "track_total_hits": true});
        if let Some(sort) = sort {
            kwargs["sort"] = sort.clone();
        }
        if let Some(fields) = source_fields {
            kwargs["source"] = json!(fields);
        }
        let response = self.es.call("search", &kwargs)?;
        let total = total_of(&response)?;
        let base_score = if ref_score > 0.0 { ref_score } else { response["hits"]["max_score"].as_f64().unwrap_or(0.0) };
        if base_score <= 0.0 {
            return Ok((Vec::new(), total));
        }
        let result =
            hits_of(&response).iter().map(|hit| Ok((hit_id(hit)?, source_of(hit), normalize(hit["_score"].as_f64().unwrap_or(0.0), base_score, true)))).collect::<Result<Vec<Hit>, EsError>>()?;
        Ok((result, total))
    }

    pub fn search_by_title(&self, title: &str, file_type: &str, file_size: i64, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        let max_result_count = resolve_count(max_result_count);
        log::debug!("search_by_title(max_result_count={max_result_count}, title='{title}', file_type='{file_type}', file_size={file_size})");
        // Python `title.split(" ")`처럼 공백 하나를 기준으로 센다. 빈 문자열도 1개다.
        let words = title.split(' ').count() as f64;
        let query = json!({"bool": {"should": [
            {"match": {"title": {"query": title, "boost": 1.2 + words.log2()}}},
            {"match": {"file_type": {"query": file_type, "boost": 1}}},
            {"match": {"file_size": {"query": file_size, "boost": 1}}},
        ]}});
        self._search(&query, None, max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    pub fn search_by_category(&self, category: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        let max_result_count = resolve_count(max_result_count);
        log::debug!("search_by_category(category='{category}')");
        let sort = json!(["author.keyword", "title.keyword"]);
        self._search(&json!({"term": {"category": category}}), Some(&sort), max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    /// 카테고리 내 문서를 search_after 커서로 한 페이지 조회한다. 오류는 삼키고 빈 결과를 돌려준다.
    /// 반환: (문서 목록, 전체 건수, 다음 커서 | None)
    pub fn search_by_category_paged(&self, category: &str, size: i64, search_after: Option<&[Value]>) -> (Vec<Hit>, i64, Option<Vec<Value>>) {
        log::debug!("search_by_category_paged(category='{category}', size={size}, search_after={search_after:?})");
        let mut kwargs =
            json!({"index": self.index(), "query": {"term": {"category": category}}, "sort": Self::category_sort(), "size": size, "track_total_hits": true, "source": Self::LIST_SOURCE_FIELDS});
        if let Some(after) = search_after.filter(|a| !a.is_empty()) {
            kwargs["search_after"] = json!(after);
        }
        let parsed = self.es.call("search", &kwargs).and_then(|response| {
            let hits = hits_of(&response);
            let total = total_of(&response)?;
            let result = hits.iter().map(|hit| Ok((hit_id(hit)?, source_of(hit), 0.0))).collect::<Result<Vec<Hit>, EsError>>()?;
            // 마지막 페이지(요청 크기 미만)면 다음 커서를 주지 않는다.
            let next = if hits.len() as i64 == size { hits.last().and_then(|hit| hit["sort"].as_array().cloned()) } else { None };
            Ok((result, total, next))
        });
        parsed.unwrap_or_else(|e| {
            log::error!("search_by_category_paged error: {e}");
            (Vec::new(), 0, None)
        })
    }

    fn cover_type_filter() -> Value {
        json!({"terms": {"file_type": Self::COVER_FILE_TYPES}})
    }

    /// 커버 형식(epub, pdf) 문서가 있는 카테고리별로 (문서 수, 최근 수정 시각, 최근 생성 시각)을 센다. 시각은 epoch 밀리초다.
    /// 오류는 삼키고 빈 결과를 돌려준다.
    pub fn aggregate_cover_categories(&self) -> HashMap<String, (i64, Option<f64>, Option<f64>)> {
        let mut stats = HashMap::new();
        let mut after: Option<Value> = None;
        loop {
            let mut composite = json!({"size": 2000, "sources": [{"category": {"terms": {"field": "category"}}}]});
            if let Some(after) = &after {
                composite["after"] = after.clone();
            }
            let sub_aggs = json!({"updated": {"max": {"field": "updated_time"}}, "created": {"max": {"field": "created_time"}}});
            let kwargs = json!({"index": self.index(), "size": 0, "query": {"bool": {"filter": [Self::cover_type_filter()]}}, "aggs": {"categories": {"composite": composite, "aggs": sub_aggs}}, "track_total_hits": false});
            let response = match self.es.call("search", &kwargs) {
                Ok(response) => response,
                Err(e) => {
                    log::error!("aggregate_cover_categories error: {e}");
                    return HashMap::new();
                }
            };
            let aggregation = &response["aggregations"]["categories"];
            let buckets = aggregation["buckets"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            for bucket in buckets {
                let Some(category) = bucket["key"]["category"].as_str() else { continue };
                stats.insert(category.to_string(), (bucket["doc_count"].as_i64().unwrap_or(0), bucket["updated"]["value"].as_f64(), bucket["created"]["value"].as_f64()));
            }
            after = aggregation.get("after_key").filter(|k| truthy(Some(k))).cloned();
            if buckets.is_empty() || after.is_none() {
                return stats;
            }
        }
    }

    pub fn _exclude_category_queries(categories: Option<&[&str]>) -> Vec<Value> {
        categories.unwrap_or(&[]).iter().filter(|c| !c.is_empty()).map(|c| category_or_children(c)).collect()
    }

    /// 주어진 카테고리에 직접 속한 커버 형식 파일을 최신순으로 한 페이지 조회한다. 오류는 삼키고 빈 결과를 돌려준다.
    /// 반환: ([(문서 id, 문서, 정렬값)], 전체 건수). 전체 건수는 search_after와 무관하다.
    pub fn search_cover_files_paged(&self, categories: &[&str], size: i64, search_after: Option<&[Value]>, exclude_categories: Option<&[&str]>) -> (Vec<(i64, Value, Vec<Value>)>, i64) {
        if categories.is_empty() {
            return (Vec::new(), 0);
        }
        log::debug!("search_cover_files_paged(categories={}, size={size}, search_after={search_after:?})", categories.len());
        let mut query = json!({"bool": {"filter": [Self::cover_type_filter(), {"terms": {"category": categories}}]}});
        let excluded = Self::_exclude_category_queries(exclude_categories);
        if !excluded.is_empty() {
            query["bool"]["must_not"] = json!(excluded);
        }
        let mut kwargs = json!({"index": self.index(), "query": query, "sort": Self::latest_sort(), "size": size, "track_total_hits": true, "source": Self::LIST_SOURCE_FIELDS});
        if let Some(after) = search_after.filter(|a| !a.is_empty()) {
            kwargs["search_after"] = json!(after);
        }
        let parsed = self.es.call("search", &kwargs).and_then(|response| {
            let hits = hits_of(&response).iter().map(|hit| Ok((hit_id(hit)?, source_of(hit), hit["sort"].as_array().cloned().unwrap_or_default()))).collect::<Result<Vec<_>, EsError>>()?;
            Ok((hits, total_of(&response)?))
        });
        parsed.unwrap_or_else(|e| {
            log::error!("search_cover_files_paged error: {e}");
            (Vec::new(), 0)
        })
    }

    /// 작품 디렉토리마다 대표 후보(파일 경로순 앞쪽 몇 권)를 한 번에 조회한다. 오류는 삼키고 빈 결과를 돌려준다.
    pub fn search_group_representatives(&self, group_keys: &[&str], exclude_categories: Option<&[&str]>) -> HashMap<String, Vec<(i64, Value)>> {
        if group_keys.is_empty() {
            return HashMap::new();
        }
        let excluded = Self::_exclude_category_queries(exclude_categories);
        let mut searches: Vec<Value> = Vec::new();
        for key in group_keys {
            let mut query = json!({"bool": {"filter": [Self::cover_type_filter()], "should": [{"term": {"category": key}}, {"prefix": {"category": format!("{key}/")}}], "minimum_should_match": 1}});
            if !excluded.is_empty() {
                query["bool"]["must_not"] = json!(excluded);
            }
            searches.push(json!({}));
            searches.push(json!({"query": query, "sort": [{"file_path": "asc"}], "size": Self::REPRESENTATIVE_CANDIDATES, "_source": Self::LIST_SOURCE_FIELDS}));
        }
        let response = match self.es.call("msearch", &json!({"index": self.index(), "searches": searches})) {
            Ok(response) => response,
            Err(e) => {
                log::error!("search_group_representatives error: {e}");
                return HashMap::new();
            }
        };
        let mut result = HashMap::new();
        for (key, item) in group_keys.iter().zip(response["responses"].as_array().map(Vec::as_slice).unwrap_or(&[])) {
            if item.get("error").is_some() {
                log::error!("search_group_representatives({key}) error: {}", item["error"]);
                continue;
            }
            let rows = hits_of(item).iter().map(|hit| Ok((hit_id(hit)?, source_of(hit)))).collect::<Result<Vec<_>, EsError>>();
            match rows {
                Ok(rows) => {
                    result.insert(key.to_string(), rows);
                }
                Err(e) => log::error!("search_group_representatives({key}) error: {e}"),
            }
        }
        result
    }

    fn keyword_query(keyword: &str) -> Value {
        json!({"bool": {"should": [
            {"match": {"title": {"query": keyword, "boost": 10}}},
            {"match": {"author": {"query": keyword, "boost": 5}}},
            {"match": {"category.nori": {"query": keyword, "boost": 3}}},
            {"match": {"summary": {"query": keyword, "boost": 1}}},
        ], "minimum_should_match": 1}})
    }

    pub fn search_by_keyword(&self, keyword: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        let max_result_count = resolve_count(max_result_count);
        log::debug!("search_by_keyword(keyword='{keyword}', max_result_count={max_result_count})");
        self._search(&Self::keyword_query(keyword), None, max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    /// 이름과 추출한 저자만 사용해 유사 파일을 찾는다.
    pub fn search_similar_names(&self, name: &str, author: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        let wildcard_name = name.replace('\\', "\\\\").replace('*', "\\*").replace('?', "\\?");
        let mut should = vec![json!({"match": {"title": {"query": name, "fuzziness": "AUTO", "boost": 8}}}), json!({"wildcard": {"file_path": {"value": format!("*{wildcard_name}*"), "boost": 6}}})];
        if !author.is_empty() {
            should.push(json!({"match": {"author": {"query": author, "fuzziness": "AUTO", "boost": 10}}}));
        }
        let query = json!({"bool": {"should": should, "minimum_should_match": 1}});
        self._search(&query, None, max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    pub fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, exclude_categories: Option<&[&str]>, category: Option<&str>) -> Result<(Vec<Hit>, i64), EsError> {
        log::debug!("search_by_keyword_paged(keyword='{keyword}', size={size}, offset={offset}, exclude_categories={exclude_categories:?}, category={category:?})");
        let mut query = json!({"bool": {"must": [Self::keyword_query(keyword)]}});
        if let Some(excluded) = exclude_categories.filter(|e| !e.is_empty()) {
            query["bool"]["must_not"] = json!(excluded.iter().map(|c| json!({"prefix": {"category": c}})).collect::<Vec<_>>());
        }
        if let Some(category) = category.filter(|c| !c.is_empty()) {
            query["bool"]["filter"] = json!([{"term": {"category": category}}]);
        }
        self._search_paged(&query, None, size, offset, 0.0, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    /// 커버 형식 문서를 최신순으로 조회한다. 오류는 삼키고 빈 결과를 돌려준다.
    pub fn search_latest_docs(&self, max_result_count: i64, exclude_categories: Option<&[&str]>) -> (Vec<Hit>, i64) {
        log::debug!("search_latest_docs(max_result_count={max_result_count}, exclude_categories={exclude_categories:?})");
        let size = max_result_count.clamp(1, MAX_RESULT_WINDOW);
        let mut query = json!({"bool": {"must": [{"match_all": {}}], "filter": [Self::cover_type_filter()]}});
        let excluded = Self::_exclude_category_queries(exclude_categories);
        if !excluded.is_empty() {
            query["bool"]["must_not"] = json!(excluded);
        }
        let kwargs = json!({"index": self.index(), "query": query, "sort": Self::latest_sort(), "size": size, "track_total_hits": true, "source": Self::LIST_SOURCE_FIELDS});
        let parsed = self.es.call("search", &kwargs).and_then(|response| {
            let hits = hits_of(&response).iter().map(|hit| Ok((hit_id(hit)?, source_of(hit), 0.0))).collect::<Result<Vec<Hit>, EsError>>()?;
            Ok((hits, total_of(&response)?))
        });
        parsed.unwrap_or_else(|e| {
            log::error!("search_latest_docs error: {e}");
            (Vec::new(), 0)
        })
    }

    // ---- 유사 문서 ----

    /// 유사 문서 검색의 should 절. `search_similar_docs`와 `_paged`가 공유한다.
    /// summary 원문을 match로 넣으면 nori가 토큰 1,000여 개로 쪼개 3.5초 걸려서 more_like_this로 상위 용어만 쓴다.
    fn similar_should_clauses(title: &str, author: &str, file_size: i64, summary: &str) -> Vec<Value> {
        let size = file_size as f64;
        vec![
            json!({"match": {"title": {"query": title, "boost": 20}}}),
            json!({"match": {"author": {"query": author, "boost": 15}}}),
            json!({"more_like_this": {"fields": ["summary"], "like": summary, "max_query_terms": Self::SIMILAR_SUMMARY_MAX_QUERY_TERMS, "min_term_freq": 1, "min_doc_freq": Self::SIMILAR_SUMMARY_MIN_DOC_FREQ, "boost": 3}}),
            json!({"range": {"file_size": {"gte": size * 0.9, "lte": size * 1.1, "boost": 1}}}),
        ]
    }

    pub fn search_similar_docs(&self, args: &SimilarDocsArgs) -> Result<Vec<Hit>, EsError> {
        let max_result_count = resolve_count(args.max_result_count);
        log::debug!("search_similar_docs(title='{}', author='{}', size={}, max_result_count={max_result_count})", args.title, args.author, args.file_size);
        let mut query = json!({"bool": {"should": Self::similar_should_clauses(&args.title, &args.author, args.file_size, &args.summary), "minimum_should_match": 1}});
        if let Some(exclude_id) = args.exclude_id {
            query["bool"]["must_not"] = json!([{"term": {"_id": exclude_id.to_string()}}]);
        }
        self._search(&query, None, max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    pub fn search_similar_docs_paged(&self, args: &SimilarDocsArgs) -> Result<(Vec<Hit>, i64), EsError> {
        log::debug!("search_similar_docs_paged(title='{}', author='{}', size={}, offset={})", args.title, args.author, args.size, args.offset);
        let should_clauses = Self::similar_should_clauses(&args.title, &args.author, args.file_size, &args.summary);
        // exclude_id가 있으면 msearch로 self-score와 본 검색을 한 번의 왕복으로 실행한다.
        if let Some(exclude_id) = args.exclude_id {
            return self._search_paged_with_self_score(&should_clauses, exclude_id, args.size, args.offset);
        }
        let query = json!({"bool": {"should": should_clauses, "minimum_should_match": 1}});
        self._search_paged(&query, None, args.size, args.offset, 0.0, None)
    }

    /// msearch로 self-score 쿼리와 본 검색을 한 번의 왕복으로 실행한다. 점수는 원본 문서의 self-score 기준이다.
    pub fn _search_paged_with_self_score(&self, should_clauses: &[Value], exclude_id: i64, size: i64, offset: i64) -> Result<(Vec<Hit>, i64), EsError> {
        log::debug!("_search_paged_with_self_score(exclude_id={exclude_id}, size={size}, offset={offset})");
        let size = size.min(MAX_RESULT_WINDOW);
        // self-score 쿼리: 원본 문서만 매칭한다. 점수만 쓰고 본문을 버리므로 _source를 통째로 끈다.
        let self_score_query = json!({"bool": {"should": should_clauses, "filter": [{"term": {"_id": exclude_id.to_string()}}]}});
        // 본 검색 쿼리: 원본은 제외한다.
        let search_query = json!({"bool": {"should": should_clauses, "minimum_should_match": 1, "must_not": [{"term": {"_id": exclude_id.to_string()}}]}});
        let searches = json!([
            {"index": self.index()},
            {"size": 1, "query": self_score_query, "_source": false},
            {"index": self.index()},
            {"size": size, "from": offset, "query": search_query, "track_scores": true, "track_total_hits": true, "_source": Self::LIST_SOURCE_FIELDS},
        ]);
        let response = self.es.call("msearch", &json!({"searches": searches}))?;
        let responses = response["responses"].as_array().filter(|r| r.len() >= 2).ok_or_else(|| parse_error("msearch responses가 2개 미만이다"))?;

        let base_score = if responses[0].get("error").is_some() {
            log::error!("msearch self-score query error: {}", responses[0]["error"]);
            0.0
        } else {
            hits_of(&responses[0]).first().and_then(|hit| hit["_score"].as_f64()).unwrap_or(0.0)
        };

        let search_resp = &responses[1];
        if search_resp.get("error").is_some() {
            log::error!("msearch search query error: {}", search_resp["error"]);
            return Ok((Vec::new(), 0));
        }
        let total = total_of(search_resp)?;
        if base_score <= 0.0 {
            return Ok((Vec::new(), total));
        }
        let result =
            hits_of(search_resp).iter().map(|hit| Ok((hit_id(hit)?, source_of(hit), normalize(hit["_score"].as_f64().unwrap_or(0.0), base_score, true)))).collect::<Result<Vec<Hit>, EsError>>()?;
        Ok((result, total))
    }

    // ---- 단건 ----

    /// `_source`를 읽는다. 없으면 `None`이다.
    fn get_source(&self, id: &str) -> Result<Option<Value>, EsError> {
        match self.es.call("get", &json!({"index": self.index(), "id": id})) {
            Ok(response) => Ok(Some(source_of(&response))),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 문서 하나의 `_source`. 없으면 `{}`를 반환하고, 그 밖의 ES 오류는 전달한다.
    pub fn search_by_id(&self, doc_id: i64) -> Result<Value, EsError> {
        log::debug!("search_by_id(doc_id={doc_id})");
        self.get_source(&doc_id.to_string()).map(|source| source.unwrap_or_else(|| json!({})))
    }

    pub fn search_and_aggregate_by_category(&self) -> Result<Value, EsError> {
        log::debug!("search_and_aggregate_by_category()");
        let body = json!({"size": 1, "aggs": {"unique_values": {"terms": {"field": "category", "size": 10000}}}});
        let result = self.es.call("search", &json!({"index": self.index(), "body": body}))?;
        let buckets = result["aggregations"]["unique_values"]["buckets"].as_array().ok_or_else(|| parse_error("aggregations.unique_values.buckets가 없다"))?;
        let mut counts = Map::new();
        for bucket in buckets {
            let key = bucket["key"].as_str().map(str::to_string).unwrap_or_else(|| bucket["key"].to_string());
            counts.insert(key, bucket["doc_count"].clone());
        }
        Ok(Value::Object(counts))
    }

    fn pdf_stats_of(pdfs: &Value) -> Value {
        let int = |value: &Value| value["value"].as_f64().unwrap_or(0.0) as i64;
        json!({"file_count": pdfs["doc_count"], "page_count": int(&pdfs["total_pages"]), "total_file_size": int(&pdfs["total_file_size"])})
    }

    fn pdf_aggs() -> Value {
        json!({"filter": {"term": {"file_type": "pdf"}}, "aggs": {"total_pages": {"sum": {"field": "page_count"}}, "total_file_size": {"sum": {"field": "file_size"}}}})
    }

    /// 카테고리와 하위 카테고리의 PDF 수, 페이지 합계, 파일 크기를 계산한다.
    pub fn search_category_pdf_stats(&self, category: &str) -> Result<Value, EsError> {
        let query = json!({"bool": {"filter": [category_or_children(category)]}});
        let response = self.es.call("search", &json!({"index": self.index(), "query": query, "size": 0, "aggs": {"pdfs": Self::pdf_aggs()}}))?;
        Ok(Self::pdf_stats_of(&response["aggregations"]["pdfs"]))
    }

    /// 여러 카테고리와 하위 경로의 PDF 수·페이지 합계를 한 번에 집계한다.
    pub fn search_categories_pdf_stats(&self, categories: &[&str]) -> Result<Value, EsError> {
        if categories.is_empty() {
            return Ok(json!({}));
        }
        let filters: Map<String, Value> = categories.iter().map(|c| (c.to_string(), category_or_children(c))).collect();
        let aggs = json!({"by_category": {"filters": {"filters": filters}, "aggs": {"pdfs": Self::pdf_aggs()}}});
        let response = self.es.call("search", &json!({"index": self.index(), "query": {"match_all": {}}, "size": 0, "aggs": aggs}))?;
        let buckets = response["aggregations"]["by_category"]["buckets"].as_object().ok_or_else(|| parse_error("aggregations.by_category.buckets가 없다"))?;
        Ok(Value::Object(buckets.iter().map(|(category, bucket)| (category.clone(), Self::pdf_stats_of(&bucket["pdfs"]))).collect()))
    }

    /// 전체 문서를 (category, file_path, 문서 수)로 훑는다. 검색이 아니라 열거라서 composite 집계를 쓴다.
    /// 같은 경로를 가리키는 중복 문서는 문서 수로 센다.
    pub fn iter_all_category_file_paths(&self, batch_size: usize) -> CategoryFilePaths<'_> {
        log::debug!("iter_all_category_file_paths(batch_size={batch_size})");
        CategoryFilePaths { manager: self, batch_size, after: None, buffer: VecDeque::new(), done: false }
    }

    // ---- 삭제 ----

    /// 주어진 file_path 목록의 기존 문서를 삭제한다(중복 방지용). `exclude_ids`는 지우지 않는다. 오류는 삼키고 0을 돌려준다.
    pub fn delete_by_file_paths(&self, file_paths: &[&str], exclude_ids: Option<&[i64]>) -> i64 {
        if file_paths.is_empty() {
            return 0;
        }
        let mut query = json!({"bool": {"must": [{"terms": {"file_path": file_paths}}]}});
        if let Some(ids) = exclude_ids.filter(|ids| !ids.is_empty()) {
            query["bool"]["must_not"] = json!([{"ids": {"values": ids.iter().map(i64::to_string).collect::<Vec<_>>()}}]);
        }
        match self.es.call("delete_by_query", &json!({"index": self.index(), "body": {"query": query}, "conflicts": "proceed", "refresh": false})) {
            Ok(result) => {
                let deleted = result["deleted"].as_i64().unwrap_or(0);
                if deleted > 0 {
                    log::info!("delete_by_file_paths: {deleted} docs deleted for {} paths", file_paths.len());
                }
                deleted
            }
            Err(e) => {
                log::error!("delete_by_file_paths error: {e}");
                0
            }
        }
    }

    /// 경로 prefix 하위(자기 자신 포함) 문서를 매칭하는 쿼리. prefix가 ""/"."/"/"이면 전체(match_all)다.
    pub fn _path_prefix_query(path_prefix: &str) -> Value {
        let p = path_prefix.trim_matches('/');
        if p.is_empty() || p == "." {
            return json!({"match_all": {}});
        }
        json!({"bool": {"should": [{"term": {"file_path": p}}, {"prefix": {"file_path": format!("{p}/")}}], "minimum_should_match": 1}})
    }

    /// 경로 prefix 하위 문서의 `_id`(inode) 집합. scroll로 읽고 `_source`는 받지 않는다. 오류는 삼키고 읽은 만큼 돌려준다.
    pub fn get_doc_ids_by_path_prefix(&self, path_prefix: &str) -> HashSet<i64> {
        let query = Self::_path_prefix_query(path_prefix);
        let mut ids = HashSet::new();
        let mut scroll_id: Option<String> = None;
        if let Err(e) = self.collect_ids(&query, &mut ids, &mut scroll_id) {
            log::error!("get_doc_ids_by_path_prefix error: {e}");
        }
        self.clear_scroll_quietly(scroll_id);
        ids
    }

    fn collect_ids(&self, query: &Value, ids: &mut HashSet<i64>, scroll_id: &mut Option<String>) -> Result<(), EsError> {
        let mut response = self.es.call("search", &json!({"index": self.index(), "query": query, "scroll": "10m", "size": 5000, "source": false}))?;
        loop {
            if let Some(id) = response["_scroll_id"].as_str() {
                *scroll_id = Some(id.to_string());
            }
            let hits = hits_of(&response);
            if hits.is_empty() {
                return Ok(());
            }
            for hit in hits {
                ids.insert(hit_id(hit)?);
            }
            response = self.es.call("scroll", &json!({"scroll_id": scroll_id.clone().unwrap_or_default(), "scroll": "10m"}))?;
        }
    }

    fn clear_scroll_quietly(&self, scroll_id: Option<String>) {
        if let Some(id) = scroll_id
            && let Err(e) = self.es.call("clear_scroll", &json!({"scroll_id": id}))
        {
            log::debug!("Failed to clear scroll: {e}");
        }
    }

    /// 주어진 `_id`(inode) 목록의 문서를 삭제한다. 오류가 나면 그때까지 지운 수를 돌려준다.
    pub fn delete_by_ids(&self, ids: &[i64], chunk_size: usize) -> i64 {
        if ids.is_empty() {
            return 0;
        }
        let mut deleted = 0;
        for chunk in ids.chunks(chunk_size.max(1)) {
            let query = json!({"ids": {"values": chunk.iter().map(i64::to_string).collect::<Vec<_>>()}});
            match self.es.call("delete_by_query", &json!({"index": self.index(), "body": {"query": query}, "conflicts": "proceed", "refresh": false})) {
                Ok(result) => deleted += result["deleted"].as_i64().unwrap_or(0),
                Err(e) => {
                    log::error!("delete_by_ids error: {e}");
                    return deleted;
                }
            }
        }
        if deleted > 0 {
            log::info!("delete_by_ids: {deleted} docs deleted for {} ids", ids.len());
        }
        deleted
    }

    // ---- 쓰기 ----

    /// 문서를 100건씩 bulk로 넣는다. 연결 오류는 `2**attempt`초씩 쉬며 `max_retries`번까지 시도한다.
    /// `num_docs` 이상 처리하면 그 묶음에서 멈춘다.
    pub fn insert(&self, data: &[(i64, Value)], num_docs: usize, max_retries: u32) -> Result<Vec<i64>, EsError> {
        log::debug!("insert() {} items", data.len());
        let mut doc_ids = Vec::new();
        let mut data_count = 0usize;
        for chunk in data.chunks(100) {
            let mut body = Vec::with_capacity(chunk.len() * 2);
            for (inode, doc) in chunk {
                body.push(json!({"index": {"_index": self.index(), "_id": inode.to_string()}}));
                body.push(doc.clone());
                doc_ids.push(*inode);
                data_count += 1;
            }
            log::info!("{} items inserted", body.len() / 2);
            let kwargs = json!({"body": body, "timeout": "60s", "refresh": false});
            for attempt in 0..max_retries {
                match self.es.call("bulk", &kwargs) {
                    Ok(response) => {
                        if response["errors"].as_bool().unwrap_or(false) {
                            log::warn!("ES bulk 응답에 항목 오류가 있다");
                        }
                        break;
                    }
                    Err(e) if e.is_connection() => {
                        if attempt + 1 < max_retries {
                            let wait = 2f64.powi(attempt as i32);
                            log::warn!("ES bulk 요청 실패 (시도 {}/{max_retries}): {e}. {wait}초 후 재시도...", attempt + 1);
                            self.clock.sleep(wait);
                        } else {
                            log::error!("ES bulk 요청 최종 실패: {e}");
                            return Err(e);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            if data_count >= num_docs {
                break;
            }
        }
        Ok(doc_ids)
    }

    /// 필드를 갱신한다(`refresh=true`). 샤드 실패가 있으면 `false`다.
    pub fn update(&self, doc_id: i64, fields: &UpdateFields) -> Result<bool, EsError> {
        log::debug!(
            "update(doc_id={doc_id}, title='{}', author='{}', file_path='{}', file_type='{}', file_size={}, category='{}')",
            fields.title,
            fields.author,
            fields.file_path,
            fields.file_type,
            fields.file_size,
            fields.category
        );
        let result = self.es.call("update", &json!({"index": self.index(), "id": doc_id.to_string(), "body": {"doc": fields.to_doc()}, "refresh": true}))?;
        Ok(result["_shards"]["failed"].as_i64().is_none_or(|failed| failed <= 0))
    }

    pub fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        log::debug!("delete(doc_id={doc_id})");
        let result = self.es.call("delete", &json!({"index": self.index(), "id": doc_id.to_string(), "refresh": true}))?;
        Ok(result["result"] == "deleted")
    }

    // ---- 카테고리 ----

    fn category_query(category: &str, prefix: bool) -> Value {
        if prefix { category_or_children(category) } else { json!({"term": {"category": category}}) }
    }

    /// 특정 카테고리의 문서 수. `prefix`이면 하위 카테고리(`category/*`)도 센다.
    pub fn count_by_category(&self, category: &str, prefix: bool) -> Result<i64, EsError> {
        log::debug!("count_by_category(category='{category}', prefix={prefix})");
        let result = self.es.call("count", &json!({"index": self.index(), "query": Self::category_query(category, prefix)}))?;
        result["count"].as_i64().ok_or_else(|| parse_error("count가 없다"))
    }

    /// 여러 카테고리의 문서 수를 msearch로 한 번에 조회한다. 카테고리가 없으면 호출 없이 `{}`다(Python은 빈 msearch를 보낸다).
    pub fn count_by_categories(&self, categories: &[&str], prefix: bool) -> Result<Value, EsError> {
        if categories.is_empty() {
            return Ok(json!({}));
        }
        if let [category] = categories {
            return Ok(json!({ *category: self.count_by_category(category, prefix)? }));
        }
        log::debug!("count_by_categories(categories={categories:?}, prefix={prefix})");
        let mut searches = Vec::with_capacity(categories.len() * 2);
        for category in categories {
            searches.push(json!({"index": self.index()}));
            searches.push(json!({"size": 0, "track_total_hits": true, "query": Self::category_query(category, prefix)}));
        }
        let response = self.es.call("msearch", &json!({"searches": searches}))?;
        let responses = response["responses"].as_array().ok_or_else(|| parse_error("msearch responses가 없다"))?;
        let mut result = Map::new();
        for (category, item) in categories.iter().zip(responses) {
            if item.get("error").is_some() {
                log::error!("msearch error for category '{category}': {}", item["error"]);
                result.insert(category.to_string(), json!(0));
            } else {
                result.insert(category.to_string(), json!(total_of(item)?));
            }
        }
        Ok(Value::Object(result))
    }

    /// 특정 카테고리('A'와 'A/*')의 모든 문서를 새 카테고리로 바꾸고 file_path의 카테고리 prefix도 갱신한다.
    /// 반환: `{"updated": n, "failures": [...]}`
    pub fn rename_category(&self, old_category: &str, new_category: &str) -> Result<Value, EsError> {
        log::debug!("rename_category(old='{old_category}', new='{new_category}')");
        let old_prefix = format!("{old_category}/");
        let new_prefix = format!("{new_category}/");
        // 디렉토리 rename은 하위 카테고리까지 통째로 옮기므로 ES도 'A'와 'A/*'를 함께 갱신해야 한다.
        let script = json!({
            "source": "
                if (ctx._source.category == params.old_category) {
                    ctx._source.category = params.new_category;
                } else if (ctx._source.category.startsWith(params.old_prefix)) {
                    ctx._source.category = params.new_prefix + ctx._source.category.substring(params.old_prefix.length());
                }
                if (ctx._source.file_path != null && ctx._source.file_path.startsWith(params.old_prefix)) {
                    ctx._source.file_path = params.new_prefix + ctx._source.file_path.substring(params.old_prefix.length());
                }
            ",
            "lang": "painless",
            "params": {"old_category": old_category, "new_category": new_category, "old_prefix": old_prefix, "new_prefix": new_prefix},
        });
        let query = json!({"bool": {"should": [{"term": {"category": old_category}}, {"prefix": {"category": old_prefix}}], "minimum_should_match": 1}});
        let result = self.es.call("update_by_query", &json!({"index": self.index(), "query": query, "script": script, "conflicts": "abort", "refresh": true}))?;
        Ok(json!({"updated": result["updated"].as_i64().unwrap_or(0), "failures": result.get("failures").cloned().unwrap_or_else(|| json!([]))}))
    }

    /// 특정 카테고리의 모든 문서를 삭제한다. `prefix`이면 하위 카테고리도 지운다. 반환: `{"deleted": n, "failures": [...]}`
    pub fn delete_by_category(&self, category: &str, prefix: bool) -> Result<Value, EsError> {
        log::debug!("delete_by_category(category='{category}', prefix={prefix})");
        let result = self.es.call("delete_by_query", &json!({"index": self.index(), "query": Self::category_query(category, prefix), "conflicts": "abort", "refresh": true}))?;
        Ok(json!({"deleted": result["deleted"].as_i64().unwrap_or(0), "failures": result.get("failures").cloned().unwrap_or_else(|| json!([]))}))
    }

    // ---- 생성 시각 채우기 ----

    /// 경로를 `Path.resolve(strict=False)`처럼 정규화한다. 없는 꼬리는 `..`와 `.`만 문자열로 처리한다.
    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        let mut missing = io::Error::from(io::ErrorKind::NotFound);
        for ancestor in path.ancestors() {
            match self.fs.canonicalize(ancestor) {
                Ok(mut resolved) => {
                    let tail = path.strip_prefix(ancestor).unwrap_or(Path::new(""));
                    for component in tail.components() {
                        match component {
                            Component::ParentDir => {
                                resolved.pop();
                            }
                            Component::Normal(name) => resolved.push(name),
                            _ => {}
                        }
                    }
                    return Ok(resolved);
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => missing = e,
                Err(e) => return Err(e),
            }
        }
        Err(missing)
    }

    /// 파일 생성 시각(ISO 8601)과 출처. 리눅스 statx의 btime이 있으면 `statx_btime`, 없으면 `st_ctime`이다.
    fn created_time_with_source(&self, path: &Path) -> io::Result<(String, &'static str)> {
        let metadata = self.fs.metadata(path)?;
        let (time, source) = match metadata.created() {
            Ok(created) => (DateTime::<Local>::from(created), "statx_btime"),
            Err(_) => {
                (DateTime::from_timestamp(metadata.ctime(), metadata.ctime_nsec() as u32).map(|utc| utc.with_timezone(&Local)).ok_or_else(|| io::Error::other("ctime을 해석할 수 없다"))?, "st_ctime")
            }
        };
        let local = time.naive_local();
        // Python `datetime.isoformat()`처럼 마이크로초가 0이면 소수부를 생략한다.
        let micros = local.and_utc().timestamp_subsec_micros();
        let iso = if micros == 0 { local.format("%Y-%m-%dT%H:%M:%S").to_string() } else { local.format("%Y-%m-%dT%H:%M:%S%.6f").to_string() };
        Ok((iso, source))
    }

    /// created_time이 없는 기존 ES 문서를 실제 파일 stat 기준으로 채운다. 반환: `{"updated", "skipped", "failed"}`
    pub fn backfill_created_time(&self, path_prefix: &Path, batch_size: usize) -> Value {
        log::info!("backfill_created_time(index={}) start", self.index());
        let (mut updated, mut skipped, mut failed) = (0i64, 0i64, 0i64);
        let query = json!({"bool": {"should": [{"bool": {"must_not": [{"exists": {"field": "created_time"}}]}}, {"bool": {"must_not": [{"exists": {"field": "created_time_source"}}]}}], "minimum_should_match": 1}});
        let resolved_prefix = self.resolve(path_prefix).unwrap_or_else(|_| path_prefix.to_path_buf());

        let resolve_path = |stored: &str| -> Option<PathBuf> {
            if stored.is_empty() {
                return None;
            }
            let candidate = Path::new(stored);
            let full = if candidate.is_absolute() { candidate.to_path_buf() } else { path_prefix.join(stored) };
            self.resolve(&full).ok().filter(|resolved| resolved.starts_with(&resolved_prefix))
        };

        let mut update_hits = |hits: &[Value]| {
            let mut bulk_body: Vec<Value> = Vec::new();
            let mut pending = 0i64;
            for hit in hits {
                let stored = hit["_source"]["file_path"].as_str().unwrap_or("");
                let Some(file_path) = resolve_path(stored).filter(|p| self.fs.is_file(p).unwrap_or(false)) else {
                    skipped += 1;
                    continue;
                };
                let Ok((created_time, source)) = self.created_time_with_source(&file_path) else {
                    failed += 1;
                    continue;
                };
                bulk_body.push(json!({"update": {"_index": self.index(), "_id": hit["_id"]}}));
                bulk_body.push(json!({"doc": {"created_time": created_time, "created_time_source": source}}));
                pending += 1;
            }
            if bulk_body.is_empty() {
                return;
            }
            match self.es.call("bulk", &json!({"body": bulk_body, "timeout": "60s", "refresh": false})) {
                Err(e) => {
                    log::error!("backfill_created_time bulk error: {e}");
                    failed += pending;
                }
                Ok(bulk_result) if bulk_result["errors"].as_bool().unwrap_or(false) => {
                    let item_failures = bulk_result["items"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().filter(|item| truthy(item["update"].get("error"))).count() as i64;
                    failed += item_failures;
                    updated += pending - item_failures;
                }
                Ok(_) => updated += pending,
            }
        };

        let mut scroll_id: Option<String> = None;
        let scan = (|| -> Result<(), EsError> {
            let mut response = self.es.call("search", &json!({"index": self.index(), "query": query, "scroll": "10m", "size": batch_size, "source": ["file_path"]}))?;
            scroll_id = response["_scroll_id"].as_str().map(str::to_string);
            while !hits_of(&response).is_empty() {
                update_hits(hits_of(&response));
                response = self.es.call("scroll", &json!({"scroll_id": scroll_id.clone().unwrap_or_default(), "scroll": "10m"}))?;
                scroll_id = response["_scroll_id"].as_str().map(str::to_string);
            }
            Ok(())
        })();
        if let Err(e) = scan {
            log::error!("backfill_created_time error: {e}");
            failed += 1;
        }
        self.clear_scroll_quietly(scroll_id);
        if updated > 0
            && let Err(e) = self.refresh()
        {
            log::warn!("backfill_created_time refresh error: {e}");
        }
        let result = json!({"updated": updated, "skipped": skipped, "failed": failed});
        log::info!("backfill_created_time(index={}) done: {result}", self.index());
        result
    }
}

/// `iter_all_category_file_paths`가 돌려주는 반복자. 한 번에 한 쪽(batch)씩 읽는다. 오류가 나면 그 오류를 한 번 내고 끝난다.
pub struct CategoryFilePaths<'a> {
    manager: &'a ESManager,
    batch_size: usize,
    after: Option<Value>,
    buffer: VecDeque<(String, String, i64)>,
    done: bool,
}

impl CategoryFilePaths<'_> {
    fn fetch(&mut self) -> Result<(), EsError> {
        let mut composite = json!({
            "size": self.batch_size,
            // missing_bucket을 켜야 category나 file_path가 빠진 문서가 조용히 사라지지 않는다.
            "sources": [{"category": {"terms": {"field": "category", "missing_bucket": true}}}, {"file_path": {"terms": {"field": "file_path", "missing_bucket": true}}}],
        });
        if let Some(after) = &self.after {
            composite["after"] = after.clone();
        }
        let response = self.manager.es.call("search", &json!({"index": self.manager.index(), "size": 0, "aggs": {"paths": {"composite": composite}}}))?;
        let aggregation = &response["aggregations"]["paths"];
        let buckets = aggregation["buckets"].as_array().ok_or_else(|| parse_error("aggregations.paths.buckets가 없다"))?;
        if buckets.is_empty() {
            self.done = true;
            return Ok(());
        }
        for bucket in buckets {
            let key = &bucket["key"];
            self.buffer.push_back((key["category"].as_str().unwrap_or("").to_string(), key["file_path"].as_str().unwrap_or("").to_string(), bucket["doc_count"].as_i64().unwrap_or(0)));
        }
        self.after = aggregation.get("after_key").filter(|k| truthy(Some(k))).cloned();
        self.done = self.after.is_none();
        Ok(())
    }
}

impl Iterator for CategoryFilePaths<'_> {
    type Item = Result<(String, String, i64), EsError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(row) = self.buffer.pop_front() {
                return Some(Ok(row));
            }
            if self.done {
                return None;
            }
            if let Err(e) = self.fetch() {
                self.done = true;
                return Some(Err(e));
            }
        }
    }
}

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
        let response = self.es.call("search", &json!({"index": self.index(), "query": {"term": {"file_path": path}}, "size": 1}))?;
        Ok(hits_of(&response).first().map(source_of))
    }

    fn search_by_category(&self, category: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        ESManager::search_by_category(self, category, max_result_count)
    }

    fn search_by_category_paged(&self, category: &str, size: i64, search_after: Option<&[Value]>) -> Result<PagedHits, EsError> {
        Ok(ESManager::search_by_category_paged(self, category, size, search_after))
    }

    fn count_by_categories(&self, categories: &[String], prefix: bool) -> Result<BTreeMap<String, i64>, EsError> {
        let counts = ESManager::count_by_categories(self, &texts(categories), prefix)?;
        Ok(counts.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0))).collect()).unwrap_or_default())
    }

    fn count_by_category(&self, category: &str, prefix: bool) -> Result<i64, EsError> {
        ESManager::count_by_category(self, category, prefix)
    }

    fn rename_category(&self, old_category: &str, new_category: &str) -> Result<Value, EsError> {
        ESManager::rename_category(self, old_category, new_category)
    }

    fn delete_by_category(&self, category: &str, prefix: bool) -> Result<Value, EsError> {
        ESManager::delete_by_category(self, category, prefix)
    }

    fn search_by_keyword(&self, keyword: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        ESManager::search_by_keyword(self, keyword, max_result_count)
    }

    /// 인자 키: `keyword`, `size`(기본 10), `offset`(기본 0), `exclude_categories`(문자열 배열), `category`.
    fn search_by_keyword_paged(&self, args: &Value) -> Result<(Vec<Hit>, i64), EsError> {
        let exclude: Option<Vec<&str>> = args.get("exclude_categories").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect());
        ESManager::search_by_keyword_paged(
            self,
            args.get("keyword").and_then(Value::as_str).unwrap_or(""),
            args.get("size").and_then(Value::as_i64).unwrap_or(10),
            args.get("offset").and_then(Value::as_i64).unwrap_or(0),
            exclude.as_deref(),
            args.get("category").and_then(Value::as_str),
        )
    }

    /// 인자 키는 `SimilarDocsArgs`의 필드 이름이다.
    fn search_similar_docs(&self, args: &Value) -> Result<Vec<Hit>, EsError> {
        ESManager::search_similar_docs(self, &SimilarDocsArgs::from_value(args))
    }

    fn search_similar_docs_paged(&self, args: &Value) -> Result<(Vec<Hit>, i64), EsError> {
        ESManager::search_similar_docs_paged(self, &SimilarDocsArgs::from_value(args))
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
