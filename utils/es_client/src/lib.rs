//! Elasticsearch REST 클라이언트. `backend/es_manager.py`의 `ESManager` 중 검색 부분의 이식.
//!
//! 접속 정보는 환경변수 `TM_ES_BOOK_INDEX`, `TM_ES_URL`, `TM_ES_USER`, `TM_ES_PASSWORD`에서 읽는다.

use std::fmt;
use std::time::Duration;

use serde_json::{Value, json};
use ureq::tls::TlsConfig;

/// 목록 응답에 필요한 필드만 받는다. summary는 단건 조회에만 쓴다(원본 `LIST_SOURCE_FIELDS`).
pub const LIST_SOURCE_FIELDS: [&str; 11] = ["category", "title", "author", "file_path", "file_type", "file_size", "line_count", "page_count", "isbn", "created_time", "updated_time"];

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
        let mut builder = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).http_status_as_error(false);
        if url.to_lowercase().starts_with("https") {
            // ECK 자체서명 인증서: 전송 구간은 TLS로 암호화하되 인증서 검증은 생략한다.
            builder = builder.tls_config(TlsConfig::builder().disable_verification(true).build());
        }
        EsClient { agent: builder.build().into(), base: url.trim_end_matches('/').to_owned(), auth: format!("Basic {}", base64(format!("{user}:{password}").as_bytes())), index: index.to_owned() }
    }

    pub fn from_env() -> Result<Self, EsError> {
        let get = |k: &str| std::env::var(k).map_err(|_| EsError::MissingEnv(k.to_owned()));
        // 원본과 같은 순서로 확인해 처음 빠진 변수를 알린다.
        for key in ENV_KEYS {
            get(key)?;
        }
        Ok(Self::new(&get("TM_ES_URL")?, &get("TM_ES_USER")?, &get("TM_ES_PASSWORD")?, &get("TM_ES_BOOK_INDEX")?))
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, EsError> {
        let url = format!("{}{path}", self.base);
        let result = match (method, body) {
            ("POST", Some(b)) => self.agent.post(&url).header("Authorization", &self.auth).send_json(b),
            _ => self.agent.get(&url).header("Authorization", &self.auth).call(),
        };
        let mut resp = result.map_err(|e| EsError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(|e| EsError::Transport(e.to_string()))?;
        if status >= 400 {
            return Err(EsError::Status { status, body: text });
        }
        serde_json::from_str(&text).map_err(|e| EsError::Parse(e.to_string()))
    }

    /// 접속을 확인한다. 실패하면 `2**(n+1)`초(최대 10초)씩 쉬며 `max_retries`번 시도한다.
    pub fn connect_with_retry(&self, max_retries: u32, sleep: &dyn Fn(u64)) -> Result<Value, EsError> {
        let mut last = None;
        for attempt in 0..max_retries {
            match self.call("GET", "/", None) {
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
        // max_result_window 오류를 피하려고 크기를 자른다.
        let size = max_result_count.min(MAX_RESULT_WINDOW);
        let mut body = json!({"query": query, "size": size, "track_scores": true});
        if let Some(fields) = source_fields {
            body["_source"] = json!(fields);
        }
        let response = self.call("POST", &format!("/{}/_search", self.index), Some(&body))?;
        let hits = &response["hits"];
        let Some(max_score) = hits["max_score"].as_f64() else { return Ok(Vec::new()) };
        let rows = hits["hits"].as_array().ok_or_else(|| EsError::Parse("hits.hits가 없다".into()))?;
        let mut result = Vec::with_capacity(rows.len());
        for hit in rows {
            let id = hit["_id"].as_str().and_then(|s| s.parse().ok()).ok_or_else(|| EsError::Parse(format!("_id를 정수로 읽지 못했다: {}", hit["_id"])))?;
            let score = hit["_score"].as_f64().unwrap_or(0.0);
            result.push(Hit { id, source: hit["_source"].clone(), score: if max_score > 0.0 { score * 100.0 / max_score } else { 0.0 } });
        }
        result.truncate(max_result_count);
        Ok(result)
    }

    fn effective_count(max_result_count: Option<usize>) -> Result<usize, EsError> {
        let n = max_result_count.unwrap_or(DEFAULT_MAX_RESULT_COUNT);
        if n > MAX_RESULT_WINDOW {
            // 원본은 이 경우 scroll로 이어서 읽는다. 검색 점검용으로는 필요 없어 옮기지 않았다.
            return Err(EsError::Unsupported(format!("{MAX_RESULT_WINDOW}건을 넘는 검색(scroll)은 지원하지 않는다: {n}")));
        }
        Ok(n)
    }

    /// 제목·확장자·크기로 찾는다. 기본 건수는 10건이다.
    pub fn search_by_title(&self, title: &str, file_type: &str, file_size: i64, max_result_count: Option<usize>) -> Result<Vec<Hit>, EsError> {
        let n = Self::effective_count(max_result_count)?;
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
        let n = Self::effective_count(max_result_count)?;
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

impl BookSearch for EsClient {
    fn search_by_title(&self, title: &str, file_type: &str, file_size: i64) -> Result<Vec<Hit>, EsError> {
        EsClient::search_by_title(self, title, file_type, file_size, None)
    }

    fn search_similar_docs(&self, summary: &str) -> Result<Vec<Hit>, EsError> {
        EsClient::search_similar_docs(self, "", "", 0, summary, None, None)
    }
}
