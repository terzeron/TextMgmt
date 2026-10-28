//! Elasticsearch REST 클라이언트. `backend/es_manager.py`의 `ESManager` 중 검색 부분의 이식.
//!
//! HTTP와 TLS 검증 HTTPS를 지원한다. 자체 CA는 `TM_ES_CA_CERT` PEM 파일로 지정한다.
//! 접속 정보는 환경변수 `TM_ES_BOOK_INDEX`, `TM_ES_URL`, `TM_ES_USER`, `TM_ES_PASSWORD`에서 읽는다.

pub mod indexing;

use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

/// 목록 응답에 필요한 필드만 받는다. summary는 단건 조회에만 쓴다(원본 `LIST_SOURCE_FIELDS`).
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
const SIMILAR_SUMMARY_MAX_QUERY_TERMS: u32 = 25;
const SIMILAR_SUMMARY_MIN_DOC_FREQ: u32 = 3;
const ENV_KEYS: [&str; 4] = ["TM_ES_BOOK_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"];

#[derive(Debug)]
pub enum EsError {
    /// 필요한 환경변수가 없다.
    MissingEnv(String),
    /// 연결·전송 실패.
    Transport(String),
    /// ES가 오류 상태로 응답했다.
    Status { status: u16, body: String },
    /// 응답 형식이 예상과 다르다.
    Parse(String),
    /// 아직 옮기지 않은 경로.
    Unsupported(String),
}

impl fmt::Display for EsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EsError::MissingEnv(k) => write!(f, "The environment variable {k} is not set."),
            EsError::Transport(m) => write!(f, "ES 연결 실패: {m}"),
            EsError::Status { status, body } => write!(f, "ES 응답 오류 {status}: {body}"),
            EsError::Parse(m) => write!(f, "ES 응답 해석 실패: {m}"),
            EsError::Unsupported(m) => write!(f, "지원하지 않는 요청: {m}"),
        }
    }
}

impl std::error::Error for EsError {}

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

/// 문서 갱신에 쓰는 필드. 비어 있거나 0인 값은 보내지 않는다(원본 `ESManager.update`와 같다).
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
    fn to_doc(&self) -> Value {
        let mut doc = serde_json::Map::new();
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

/// 문서 쓰기·조회·삭제. 테스트가 실제 ES 없이 CLI를 확인할 수 있게 분리했다.
pub trait BookStore {
    fn update(&self, doc_id: i64, fields: &UpdateFields) -> Result<bool, EsError>;
    fn search_by_id(&self, doc_id: i64) -> Result<Value, EsError>;
    fn delete(&self, doc_id: i64) -> Result<bool, EsError>;
}

/// ES REST 호출의 HTTP 메서드.
#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
    Put,
    Delete,
}

pub struct EsClient {
    agent: ureq::Agent,
    base: String,
    auth: String,
    pub index: String,
}

fn base64(input: &[u8]) -> String {
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
        let agent = build_agent(Duration::from_secs(10), ca_cert_path).map_err(EsError::Transport)?;
        Ok(EsClient { agent, base: url.trim_end_matches('/').to_owned(), auth: format!("Basic {}", base64(format!("{user}:{password}").as_bytes())), index: index.to_owned() })
    }

    /// 환경변수에서 접속 정보를 읽고 지정된 색인을 사용한다.
    pub fn from_env_with_index(index: &str) -> Result<Self, EsError> {
        let get = |k: &str| std::env::var(k).map_err(|_| EsError::MissingEnv(k.to_owned()));
        let ca_cert_path = std::env::var_os("TM_ES_CA_CERT").map(std::path::PathBuf::from);
        Self::new_with_ca(&get("TM_ES_URL")?, &get("TM_ES_USER")?, &get("TM_ES_PASSWORD")?, index, ca_cert_path.as_deref())
    }

    pub fn from_env() -> Result<Self, EsError> {
        let get = |k: &str| std::env::var(k).map_err(|_| EsError::MissingEnv(k.to_owned()));
        // 원본과 같은 순서로 확인해 처음 빠진 변수를 알린다.
        for key in ENV_KEYS {
            get(key)?;
        }
        Self::from_env_with_index(&get("TM_ES_BOOK_INDEX")?)
    }

    /// 요청 URL. http·https가 아니면 보내지 않는다.
    fn url(&self, path: &str) -> Result<String, EsError> {
        let url = format!("{}{path}", self.base);
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(EsError::Transport(format!("ES URL은 http 또는 https여야 한다: {}", self.base)));
        }
        Ok(url)
    }

    /// 응답 상태를 확인하고 본문을 JSON으로 읽는다.
    fn finish(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, EsError> {
        let mut resp = result.map_err(|e| EsError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(|e| EsError::Transport(e.to_string()))?;
        if status >= 400 {
            return Err(EsError::Status { status, body: text });
        }
        serde_json::from_str(&text).map_err(|e| EsError::Parse(e.to_string()))
    }

    fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value, EsError> {
        let url = self.url(path)?;
        let auth = &self.auth;
        Self::finish(match (method, body) {
            (Method::Post, Some(b)) => self.agent.post(&url).header("Authorization", auth).send_json(b),
            (Method::Post, None) => self.agent.post(&url).header("Authorization", auth).send_empty(),
            (Method::Put, Some(b)) => self.agent.put(&url).header("Authorization", auth).send_json(b),
            (Method::Put, None) => self.agent.put(&url).header("Authorization", auth).send_empty(),
            (Method::Delete, Some(b)) => self.agent.delete(&url).header("Authorization", auth).force_send_body().send_json(b),
            (Method::Delete, None) => self.agent.delete(&url).header("Authorization", auth).call(),
            (Method::Get, _) => self.agent.get(&url).header("Authorization", auth).call(),
        })
    }

    /// 접속을 확인한다. 실패하면 `2**(n+1)`초(최대 10초)씩 쉬며 `max_retries`번 시도한다.
    pub fn connect_with_retry(&self, max_retries: u32, sleep: &dyn Fn(u64)) -> Result<Value, EsError> {
        let mut last = None;
        for attempt in 0..max_retries {
            match self.call(Method::Get, "/", None) {
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
        let response = self.call(Method::Post, &format!("/{}/_search", self.index), Some(&body))?;
        let hits = &response["hits"];
        let Some(max_score) = hits["max_score"].as_f64() else { return Ok(Vec::new()) };
        let rows = hits["hits"].as_array().ok_or_else(|| EsError::Parse("hits.hits가 없다".into()))?;
        rows.iter().map(|hit| to_hit(hit, max_score)).collect()
    }

    /// Python `_search`처럼 10,000건을 넘는 요청은 ES scroll context로 이어 읽는다.
    fn search_scroll(&self, query: &Value, max_result_count: usize) -> Result<Vec<Hit>, EsError> {
        let mut scroll_id: Option<String> = None;
        let outcome: Result<Vec<Hit>, EsError> = (|| {
            let body = json!({"query": query, "size": MAX_RESULT_WINDOW, "track_scores": true});
            let mut response = self.call(Method::Post, &format!("/{}/_search?scroll=10m", self.index), Some(&body))?;
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
                let mut next = self.call(Method::Post, "/_search/scroll", Some(&json!({"scroll":"10m", "scroll_id":scroll_id.as_deref().unwrap_or("")})))?;
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
            let _ = self.call(Method::Delete, "/_search/scroll", Some(&json!({"scroll_id": id})));
        }
        outcome
    }

    /// 제목·확장자·크기로 찾는다. 기본 건수는 10건이다.
    pub fn search_by_title(&self, title: &str, file_type: &str, file_size: i64, max_result_count: Option<usize>) -> Result<Vec<Hit>, EsError> {
        let n = max_result_count.unwrap_or(DEFAULT_MAX_RESULT_COUNT);
        // Python `title.split(" ")`처럼 공백 하나를 기준으로 센다. 빈 문자열도 1개다.
        let words = title.split(' ').count() as f64;
        let query = json!({"bool": {"should": [
            {"match": {"title": {"query": title, "boost": 1.2 + words.log2()}}},
            {"match": {"file_type": {"query": file_type, "boost": 1}}},
            {"match": {"file_size": {"query": file_size, "boost": 1}}},
        ]}});
        self.search(query, n, Some(&LIST_SOURCE_FIELDS))
    }

    pub fn search_similar_docs(&self, title: &str, author: &str, file_size: i64, summary: &str, max_result_count: Option<usize>, exclude_id: Option<i64>) -> Result<Vec<Hit>, EsError> {
        let n = max_result_count.unwrap_or(DEFAULT_MAX_RESULT_COUNT);
        let size = file_size as f64;
        let mut query = json!({"bool": {
            "should": [
                {"match": {"title": {"query": title, "boost": 20}}},
                {"match": {"author": {"query": author, "boost": 15}}},
                {"more_like_this": {"fields": ["summary"], "like": summary, "max_query_terms": SIMILAR_SUMMARY_MAX_QUERY_TERMS, "min_term_freq": 1, "min_doc_freq": SIMILAR_SUMMARY_MIN_DOC_FREQ, "boost": 3}},
                {"range": {"file_size": {"gte": size * 0.9, "lte": size * 1.1, "boost": 1}}},
            ],
            "minimum_should_match": 1,
        }});
        if let Some(id) = exclude_id {
            query["bool"]["must_not"] = json!([{"term": {"_id": id.to_string()}}]);
        }
        self.search(query, n, Some(&LIST_SOURCE_FIELDS))
    }
}

impl EsClient {
    /// 문서의 일부 필드를 갱신한다(`refresh=true`). 샤드 실패가 있으면 `false`다. 없는 문서는 오류다.
    pub fn update(&self, doc_id: i64, fields: &UpdateFields) -> Result<bool, EsError> {
        let body = json!({"doc": fields.to_doc()});
        let response = self.call(Method::Post, &format!("/{}/_update/{doc_id}?refresh=true", self.index), Some(&body))?;
        Ok(response["_shards"]["failed"].as_i64().is_none_or(|failed| failed <= 0))
    }

    /// 문서 하나의 `_source`. 없으면 빈 객체다.
    pub fn search_by_id(&self, doc_id: i64) -> Result<Value, EsError> {
        match self.call(Method::Get, &format!("/{}/_doc/{doc_id}", self.index), None) {
            Ok(response) => Ok(response["_source"].clone()),
            Err(EsError::Status { status: 404, .. }) => Ok(json!({})),
            Err(e) => Err(e),
        }
    }

    /// 문서를 지운다(`refresh=true`). 결과가 `deleted`일 때만 `true`다. 없는 문서는 오류다.
    pub fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        let response = self.call(Method::Delete, &format!("/{}/_doc/{doc_id}?refresh=true", self.index), None)?;
        Ok(response["result"] == "deleted")
    }
}

/// 검색 hit 하나를 읽고 점수를 `max_score` 기준 0~100으로 정규화한다.
fn to_hit(hit: &Value, max_score: f64) -> Result<Hit, EsError> {
    let id = hit["_id"].as_str().and_then(|s| s.parse().ok()).ok_or_else(|| EsError::Parse(format!("_id를 정수로 읽지 못했다: {}", hit["_id"])))?;
    let score = hit["_score"].as_f64().unwrap_or(0.0);
    Ok(Hit { id, source: hit["_source"].clone(), score: if max_score > 0.0 { score * 100.0 / max_score } else { 0.0 } })
}

impl BookStore for EsClient {
    fn update(&self, doc_id: i64, fields: &UpdateFields) -> Result<bool, EsError> {
        EsClient::update(self, doc_id, fields)
    }

    fn search_by_id(&self, doc_id: i64) -> Result<Value, EsError> {
        EsClient::search_by_id(self, doc_id)
    }

    fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        EsClient::delete(self, doc_id)
    }
}

impl BookSearch for EsClient {
    fn search_by_title(&self, title: &str, file_type: &str, file_size: i64) -> Result<Vec<Hit>, EsError> {
        EsClient::search_by_title(self, title, file_type, file_size, None)
    }

    fn search_similar_docs(&self, summary: &str) -> Result<Vec<Hit>, EsError> {
        EsClient::search_similar_docs(self, "", "", 0, summary, None, None)
    }
}
