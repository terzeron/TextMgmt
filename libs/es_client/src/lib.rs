//! Elasticsearch REST 클라이언트(검색).
//!
//! HTTP와 TLS 검증 HTTPS를 지원한다. 자체 CA는 `TM_ES_CA_CERT` PEM 파일로 지정한다.
//! HTTP 요청은 모두 저수준 전송 [`transport`]를 거친다. [`EsClient`]는 그 위의 검색·색인 연산이고,
//! backend `ESManager`도 같은 전송 위에서 [`EsClient::from_transport`]로 색인 연산을 쓴다.
//! 접속 정보는 환경변수 `TM_ES_BOOK_INDEX`, `TM_ES_URL`, `TM_ES_USER`, `TM_ES_PASSWORD`에서 읽는다.

pub mod indexing;
pub mod transport;

use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::transport::{EsRequest, EsTransport, HttpTransport};

/// 목록 응답에 필요한 필드만 받는다. summary는 단건 조회에만 쓴다.
pub const LIST_SOURCE_FIELDS: [&str; 11] = ["category", "title", "author", "file_path", "file_type", "file_size", "line_count", "page_count", "isbn", "created_time", "updated_time"];

/// HTTP(S) agent를 만든다. CA를 지정하면 그 인증서를 TLS trust root로 사용한다.
pub fn build_agent(timeout: Duration, ca_cert_path: Option<&Path>) -> Result<ureq::Agent, String> {
    let mut builder = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false);
    if let Some(path) = ca_cert_path {
        let pem = fs::read(path).map_err(|error| format!("ES CA 인증서를 읽지 못했다 ({}): {error}", path.display()))?;
        let certs = ureq::tls::parse_pem(&pem)
            .filter_map(|item| match item {
                Ok(ureq::tls::PemItem::Certificate(cert)) => Some(Ok(cert)),
                Ok(ureq::tls::PemItem::PrivateKey(_)) => None,
                Err(error) => Some(Err(error)),
                _ => None,
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("ES CA 인증서 PEM을 해석하지 못했다: {error}"))?;
        if certs.is_empty() {
            return Err(format!("ES CA 인증서 PEM에 X.509 certificate가 없다: {}", path.display()));
        }
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::new_with_certs(&certs)).build();
        builder = builder.tls_config(tls);
    }
    Ok(builder.build().into())
}

const DEFAULT_MAX_RESULT_COUNT: usize = 10;
/// ES `max_result_window` 기본값. 이 이하는 scroll 없이 한 번에 받는다.
const MAX_RESULT_WINDOW: usize = 10_000;
pub const SIMILAR_SUMMARY_MAX_QUERY_TERMS: u32 = 25;
pub const SIMILAR_SUMMARY_MIN_DOC_FREQ: u32 = 3;
/// 제목·확장자·크기 검색 질의. 제목 단어가 많을수록 제목 boost를 키운다.
/// 공백 하나(`" "`)를 기준으로 나눈 조각 수. 빈 문자열도 1개다.
pub fn title_query(title: &str, file_type: &str, file_size: i64) -> Value {
    let words = title.split(' ').count() as f64;
    json!({"bool": {"should": [
        {"match": {"title": {"query": title, "boost": 1.2 + words.log2()}}},
        {"match": {"file_type": {"query": file_type, "boost": 1}}},
        {"match": {"file_size": {"query": file_size, "boost": 1}}},
    ]}})
}

/// 유사 문서 검색의 should 절.
/// summary 원문을 match로 넣으면 nori가 토큰 1,000여 개로 쪼개 3.5초 걸려서 more_like_this로 상위 용어만 쓴다.
pub fn similar_should_clauses(title: &str, author: &str, file_size: i64, summary: &str) -> Vec<Value> {
    let size = file_size as f64;
    vec![
        json!({"match": {"title": {"query": title, "boost": 20}}}),
        json!({"match": {"author": {"query": author, "boost": 15}}}),
        json!({"more_like_this": {"fields": ["summary"], "like": summary, "max_query_terms": SIMILAR_SUMMARY_MAX_QUERY_TERMS, "min_term_freq": 1, "min_doc_freq": SIMILAR_SUMMARY_MIN_DOC_FREQ, "boost": 3}}),
        json!({"range": {"file_size": {"gte": size * 0.9, "lte": size * 1.1, "boost": 1}}}),
    ]
}

/// 유사 문서 검색 질의. `exclude_id`가 있으면 그 문서를 결과에서 뺀다.
pub fn similar_query(title: &str, author: &str, file_size: i64, summary: &str, exclude_id: Option<i64>) -> Value {
    let mut query = json!({"bool": {"should": similar_should_clauses(title, author, file_size, summary), "minimum_should_match": 1}});
    if let Some(id) = exclude_id {
        query["bool"]["must_not"] = json!([{"term": {"_id": id.to_string()}}]);
    }
    query
}

const ENV_KEYS: [&str; 4] = ["TM_ES_BOOK_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"];

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

impl EsError {
    /// 문서나 색인이 없다(404).
    pub fn is_not_found(&self) -> bool {
        matches!(self, EsError::Status { status: 404, .. })
    }

    /// 연결을 만들지 못했거나 응답을 해석하지 못했다. 연결 실패, 시간 초과, 응답 해석 실패가 여기에 든다.
    pub fn is_connection(&self) -> bool {
        matches!(self, EsError::Transport(_) | EsError::Parse(_))
    }
}

/// 검색 결과 한 건. `score`는 최고점을 100으로 맞춘 값이다.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: i64,
    pub source: Value,
    pub score: f64,
}

/// 도서 검색. 테스트가 실제 ES 없이 CLI를 확인할 수 있게 분리했다.
pub trait BookSearch {
    fn search_by_title(&self, title: &str, file_type: &str, file_size: i64) -> Result<Vec<Hit>, EsError>;
    fn search_similar_docs(&self, summary: &str) -> Result<Vec<Hit>, EsError>;
}

/// 검색·색인 연산. 요청은 모두 [`EsTransport`]로 보낸다.
pub struct EsClient {
    transport: Arc<dyn EsTransport>,
    pub index: String,
}

/// `EsClient`가 직접 접속할 때 쓰는 요청 시간 상한.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in input.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

impl EsClient {
    pub fn new(url: &str, user: &str, password: &str, index: &str) -> Self {
        Self::new_with_ca(url, user, password, index, None).expect("기본 TLS agent 설정은 실패하지 않는다")
    }

    pub fn new_with_ca(url: &str, user: &str, password: &str, index: &str, ca_cert_path: Option<&Path>) -> Result<Self, EsError> {
        let transport = HttpTransport::try_new_with_ca(url, user, password, index, CLIENT_TIMEOUT, ca_cert_path).map_err(EsError::Transport)?;
        Ok(Self::from_transport(Arc::new(transport)))
    }

    /// 이미 있는 전송 위에서 연산을 쓴다. 색인 이름은 전송의 기본 색인이다.
    pub fn from_transport(transport: Arc<dyn EsTransport>) -> Self {
        let index = transport.index_name().to_owned();
        EsClient { transport, index }
    }

    pub(crate) fn send(&self, request: &EsRequest) -> Result<Value, EsError> {
        self.transport.send(request)
    }

    /// 환경변수에서 접속 정보를 읽고 지정된 색인을 사용한다.
    pub fn from_env_with_index(index: &str) -> Result<Self, EsError> {
        let get = |k: &str| std::env::var(k).map_err(|_| EsError::MissingEnv(k.to_owned()));
        let ca_cert_path = std::env::var_os("TM_ES_CA_CERT").map(std::path::PathBuf::from);
        Self::new_with_ca(&get("TM_ES_URL")?, &get("TM_ES_USER")?, &get("TM_ES_PASSWORD")?, index, ca_cert_path.as_deref())
    }

    pub fn from_env() -> Result<Self, EsError> {
        let get = |k: &str| std::env::var(k).map_err(|_| EsError::MissingEnv(k.to_owned()));
        // 정해진 순서로 확인해 처음 빠진 변수를 알린다.
        for key in ENV_KEYS {
            get(key)?;
        }
        Self::from_env_with_index(&get("TM_ES_BOOK_INDEX")?)
    }

    /// 접속을 확인한다. 실패하면 `2**(n+1)`초(최대 10초)씩 쉬며 `max_retries`번 시도한다.
    pub fn connect_with_retry(&self, max_retries: u32, sleep: &dyn Fn(u64)) -> Result<Value, EsError> {
        let mut last = None;
        for attempt in 0..max_retries {
            match self.send(&EsRequest::Info) {
                Ok(info) => return Ok(info),
                Err(e) => {
                    if attempt + 1 < max_retries {
                        sleep((2u64 << attempt).min(10));
                    }
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| EsError::Transport("시도 횟수가 0이다".into())))
    }

    /// 질의를 실행하고 점수를 최고점 기준 0~100으로 정규화한다. `max_score`가 없으면 빈 결과다.
    pub fn search(&self, query: Value, max_result_count: usize, source_fields: Option<&[&str]>) -> Result<Vec<Hit>, EsError> {
        if max_result_count > MAX_RESULT_WINDOW {
            return self.search_scroll(&query, max_result_count);
        }
        let mut body = json!({"query": query, "size": max_result_count, "track_scores": true});
        if let Some(fields) = source_fields {
            body["_source"] = json!(fields);
        }
        let response = self.send(&EsRequest::search(body))?;
        let hits = &response["hits"];
        let Some(max_score) = hits["max_score"].as_f64() else { return Ok(Vec::new()) };
        let rows = hits["hits"].as_array().ok_or_else(|| EsError::Parse("hits.hits가 없다".into()))?;
        rows.iter().map(|hit| to_hit(hit, max_score)).collect()
    }

    /// 10,000건을 넘는 요청은 ES scroll context로 이어 읽는다.
    fn search_scroll(&self, query: &Value, max_result_count: usize) -> Result<Vec<Hit>, EsError> {
        let mut scroll_id: Option<String> = None;
        let outcome: Result<Vec<Hit>, EsError> = (|| {
            let body = json!({"query": query, "size": MAX_RESULT_WINDOW, "track_scores": true});
            let mut response = self.send(&EsRequest::scroll_search(body, "10m"))?;
            scroll_id = response.get("_scroll_id").and_then(Value::as_str).map(str::to_owned);
            let mut result = Vec::new();
            loop {
                let Some(max_score) = response["hits"]["max_score"].as_f64() else {
                    return Ok(Vec::new());
                };
                let rows = response["hits"]["hits"].as_array().ok_or_else(|| EsError::Parse("hits.hits가 없다".into()))?;
                if rows.is_empty() {
                    break;
                }
                for hit in rows {
                    result.push(to_hit(hit, max_score)?);
                    if result.len() >= max_result_count {
                        return Ok(result);
                    }
                }
                let mut next = self.send(&EsRequest::Scroll { scroll_id: scroll_id.clone().unwrap_or_default(), scroll: "10m".to_string() })?;
                scroll_id = Some(next.get("_scroll_id").and_then(Value::as_str).ok_or_else(|| EsError::Parse("scroll 응답에 _scroll_id가 없다".into()))?.to_owned());
                if next["hits"]["max_score"].as_f64().is_none() {
                    return Ok(Vec::new());
                }
                // 다음 반복에서 현재 페이지 점수를 기준으로 정규화한다.
                response = std::mem::take(&mut next);
            }
            Ok(result)
        })();
        if let Some(id) = scroll_id {
            let _ = self.send(&EsRequest::ClearScroll { scroll_id: id });
        }
        outcome
    }

    /// 제목·확장자·크기로 찾는다. 기본 건수는 10건이다.
    pub fn search_by_title(&self, title: &str, file_type: &str, file_size: i64, max_result_count: Option<usize>) -> Result<Vec<Hit>, EsError> {
        let n = max_result_count.unwrap_or(DEFAULT_MAX_RESULT_COUNT);
        self.search(title_query(title, file_type, file_size), n, Some(&LIST_SOURCE_FIELDS))
    }

    pub fn search_similar_docs(&self, title: &str, author: &str, file_size: i64, summary: &str, max_result_count: Option<usize>, exclude_id: Option<i64>) -> Result<Vec<Hit>, EsError> {
        let n = max_result_count.unwrap_or(DEFAULT_MAX_RESULT_COUNT);
        self.search(similar_query(title, author, file_size, summary, exclude_id), n, Some(&LIST_SOURCE_FIELDS))
    }
}

/// 검색 hit 하나를 읽고 점수를 `max_score` 기준 0~100으로 정규화한다.
fn to_hit(hit: &Value, max_score: f64) -> Result<Hit, EsError> {
    let id = hit["_id"].as_str().and_then(|s| s.parse().ok()).ok_or_else(|| EsError::Parse(format!("_id를 정수로 읽지 못했다: {}", hit["_id"])))?;
    let score = hit["_score"].as_f64().unwrap_or(0.0);
    Ok(Hit { id, source: hit["_source"].clone(), score: if max_score > 0.0 { score * 100.0 / max_score } else { 0.0 } })
}

impl BookSearch for EsClient {
    fn search_by_title(&self, title: &str, file_type: &str, file_size: i64) -> Result<Vec<Hit>, EsError> {
        EsClient::search_by_title(self, title, file_type, file_size, None)
    }

    fn search_similar_docs(&self, summary: &str) -> Result<Vec<Hit>, EsError> {
        EsClient::search_similar_docs(self, "", "", 0, summary, None, None)
    }
}
