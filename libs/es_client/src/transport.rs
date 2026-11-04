//! 저수준 ES 전송 [`EsTransport`]와 그 HTTP 구현 [`HttpTransport`]. [`crate::EsClient`]와 같은 HTTP(S) 전송 및 인증서 검증 규칙을 쓴다.
//!
//! 요청은 [`EsRequest`] variant 하나가 ES REST 호출 하나에 대응한다(검증한 서버는 ES 9.1.0이다).
//! 본문(`body`)은 ES REST 이름(`from`, `_source` 등)을 그대로 쓴다. `index`가 `None`이면 전송의 기본 색인이다.

use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::{EsError, base64, build_agent};

/// 저수준 전송: 요청 하나를 보내고 응답 JSON을 돌려준다.
pub trait EsTransport: Send + Sync {
    fn index_name(&self) -> &str;
    fn send(&self, request: &EsRequest) -> Result<Value, EsError>;
}

/// `_delete_by_query`·`_update_by_query`가 버전 충돌을 만났을 때.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflicts {
    Abort,
    Proceed,
}

impl Conflicts {
    fn as_str(self) -> &'static str {
        match self {
            Conflicts::Abort => "abort",
            Conflicts::Proceed => "proceed",
        }
    }
}

/// ES REST 호출 하나.
#[derive(Debug, Clone, PartialEq)]
pub enum EsRequest {
    /// `GET /`
    Info,
    /// `HEAD /{index}`. 200이면 `true`, 404이면 `false`를 돌려준다.
    IndexExists { index: Option<String> },
    /// `PUT /{index}`
    CreateIndex { index: Option<String>, body: Value },
    /// `DELETE /{index}`
    DeleteIndex { index: Option<String> },
    /// `POST /{index}/_refresh`
    RefreshIndex { index: Option<String> },
    /// `GET /{index}/_mapping`
    GetMapping { index: Option<String> },
    /// `PUT /{index}/_mapping`. 본문은 `{"properties": ...}`다.
    PutMapping { index: Option<String>, properties: Value },
    /// `POST /{index}/_search`. `scroll`이 있으면 scroll 검색을 연다.
    Search { index: Option<String>, body: Value, scroll: Option<String> },
    /// `POST /_search/scroll`
    Scroll { scroll_id: String, scroll: String },
    /// `DELETE /_search/scroll`
    ClearScroll { scroll_id: String },
    /// `POST /{index}/_msearch` 또는 `POST /_msearch`. 머리·본문 줄을 NDJSON으로 보낸다.
    MultiSearch { index: Option<String>, searches: Vec<Value> },
    /// `POST /_mget`. `source`가 있으면 그 필드만 받는다.
    MultiGet { docs: Vec<Value>, source: Option<Vec<String>> },
    /// `GET /{index}/_doc/{id}`
    Get { index: Option<String>, id: String },
    /// `POST /{index}/_count`
    Count { index: Option<String>, body: Value },
    /// `POST /{index}/_update/{id}`
    Update { index: Option<String>, id: String, body: Value, refresh: Option<bool> },
    /// `DELETE /{index}/_doc/{id}`
    Delete { index: Option<String>, id: String, refresh: Option<bool> },
    /// `POST /{index}/_delete_by_query`
    DeleteByQuery { index: Option<String>, body: Value, conflicts: Option<Conflicts>, refresh: Option<bool> },
    /// `POST /{index}/_update_by_query`
    UpdateByQuery { index: Option<String>, body: Value, conflicts: Option<Conflicts>, refresh: Option<bool> },
    /// `POST /_bulk`. 동작·문서 줄을 NDJSON으로 보낸다.
    Bulk { lines: Vec<Value>, timeout: Option<String>, refresh: Option<bool> },
}

impl EsRequest {
    /// 기본 색인 검색. scroll 없이 한 번에 받는다.
    pub fn search(body: Value) -> Self {
        EsRequest::Search { index: None, body, scroll: None }
    }

    /// 기본 색인 scroll 검색. `keep_alive` 동안 다음 쪽을 `Scroll`로 받는다.
    pub fn scroll_search(body: Value, keep_alive: &str) -> Self {
        EsRequest::Search { index: None, body, scroll: Some(keep_alive.to_string()) }
    }

    /// 로그와 테스트 대역이 쓰는 요청 이름. ES API 이름(`indices.exists`, `search` 등)과 같다.
    pub fn name(&self) -> &'static str {
        match self {
            EsRequest::Info => "info",
            EsRequest::IndexExists { .. } => "indices.exists",
            EsRequest::CreateIndex { .. } => "indices.create",
            EsRequest::DeleteIndex { .. } => "indices.delete",
            EsRequest::RefreshIndex { .. } => "indices.refresh",
            EsRequest::GetMapping { .. } => "indices.get_mapping",
            EsRequest::PutMapping { .. } => "indices.put_mapping",
            EsRequest::Search { .. } => "search",
            EsRequest::Scroll { .. } => "scroll",
            EsRequest::ClearScroll { .. } => "clear_scroll",
            EsRequest::MultiSearch { .. } => "msearch",
            EsRequest::MultiGet { .. } => "mget",
            EsRequest::Get { .. } => "get",
            EsRequest::Count { .. } => "count",
            EsRequest::Update { .. } => "update",
            EsRequest::Delete { .. } => "delete",
            EsRequest::DeleteByQuery { .. } => "delete_by_query",
            EsRequest::UpdateByQuery { .. } => "update_by_query",
            EsRequest::Bulk { .. } => "bulk",
        }
    }

    /// 요청 내용을 JSON 하나로 펼친 보기. 본문 필드에 `index`, `id`, 쿼리 문자열 인자를 더한다. 로그와 테스트 검사용이다.
    pub fn params(&self) -> Value {
        let mut out = Map::new();
        let mut put = |key: &str, value: Value| {
            if !value.is_null() {
                out.insert(key.to_string(), value);
            }
        };
        let merge = |out: &mut Map<String, Value>, body: &Value| {
            if let Value::Object(fields) = body {
                out.extend(fields.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        };
        match self {
            EsRequest::Info => {}
            EsRequest::IndexExists { index } | EsRequest::DeleteIndex { index } | EsRequest::RefreshIndex { index } | EsRequest::GetMapping { index } => put("index", serde_json::json!(index)),
            EsRequest::CreateIndex { index, body } => {
                put("index", serde_json::json!(index));
                put("body", body.clone());
            }
            EsRequest::PutMapping { index, properties } => {
                put("index", serde_json::json!(index));
                put("properties", properties.clone());
            }
            EsRequest::Search { index, body, scroll } => {
                put("index", serde_json::json!(index));
                put("scroll", serde_json::json!(scroll));
                merge(&mut out, body);
            }
            EsRequest::Scroll { scroll_id, scroll } => {
                put("scroll_id", serde_json::json!(scroll_id));
                put("scroll", serde_json::json!(scroll));
            }
            EsRequest::ClearScroll { scroll_id } => put("scroll_id", serde_json::json!(scroll_id)),
            EsRequest::MultiSearch { index, searches } => {
                put("index", serde_json::json!(index));
                put("searches", serde_json::json!(searches));
            }
            EsRequest::MultiGet { docs, source } => {
                put("docs", serde_json::json!(docs));
                put("_source", serde_json::json!(source));
            }
            EsRequest::Get { index, id } => {
                put("index", serde_json::json!(index));
                put("id", serde_json::json!(id));
            }
            EsRequest::Count { index, body } => {
                put("index", serde_json::json!(index));
                merge(&mut out, body);
            }
            EsRequest::Update { index, id, body, refresh } => {
                put("index", serde_json::json!(index));
                put("id", serde_json::json!(id));
                put("refresh", serde_json::json!(refresh));
                merge(&mut out, body);
            }
            EsRequest::Delete { index, id, refresh } => {
                put("index", serde_json::json!(index));
                put("id", serde_json::json!(id));
                put("refresh", serde_json::json!(refresh));
            }
            EsRequest::DeleteByQuery { index, body, conflicts, refresh } | EsRequest::UpdateByQuery { index, body, conflicts, refresh } => {
                put("index", serde_json::json!(index));
                put("conflicts", serde_json::json!(conflicts.map(Conflicts::as_str)));
                put("refresh", serde_json::json!(refresh));
                merge(&mut out, body);
            }
            EsRequest::Bulk { lines, timeout, refresh } => {
                put("body", serde_json::json!(lines));
                put("timeout", serde_json::json!(timeout));
                put("refresh", serde_json::json!(refresh));
            }
        }
        Value::Object(out)
    }
}
/// 응답 본문 상한. ureq 기본(10MB)은 1만 건 검색에 모자랄 수 있다.
const RESPONSE_LIMIT: u64 = 512 * 1024 * 1024;

pub struct HttpTransport {
    agent: Result<ureq::Agent, EsError>,
    base: String,
    auth: Option<String>,
    index: String,
}

enum Body {
    None,
    Json(Value),
    Ndjson(Vec<Value>),
}

struct Request {
    method: &'static str,
    path: String,
    query: Vec<(String, String)>,
    body: Body,
}

impl Request {
    fn new(method: &'static str, path: impl Into<String>) -> Self {
        Request { method, path: path.into(), query: Vec::new(), body: Body::None }
    }

    fn json(mut self, body: Value) -> Self {
        self.body = Body::Json(body);
        self
    }
}

/// RFC 3986 unreserved 문자만 그대로 두고 나머지는 퍼센트 인코딩한다.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl HttpTransport {
    /// HTTP 또는 HTTPS endpoint를 쓴다. HTTPS는 기본 WebPKI roots로 검증한다.
    pub fn new(url: &str, user: &str, password: &str, index: &str, timeout: Duration) -> Self {
        Self::new_with_ca(url, user, password, index, timeout, None)
    }

    /// 사설 CA를 지정하면 해당 CA로 HTTPS 인증서를 검증한다.
    pub fn new_with_ca(url: &str, user: &str, password: &str, index: &str, timeout: Duration, ca_cert_path: Option<&Path>) -> Self {
        let auth = (!user.is_empty() || !password.is_empty()).then(|| format!("Basic {}", base64(format!("{user}:{password}").as_bytes())));
        let agent = build_agent(timeout, ca_cert_path).map_err(EsError::Other);
        HttpTransport { agent, base: url.trim_end_matches('/').to_string(), auth, index: index.to_string() }
    }

    fn index_path(&self, index: &Option<String>) -> String {
        encode(index.as_deref().unwrap_or(&self.index))
    }

    fn plan(&self, request: &EsRequest) -> Request {
        let query = |pairs: &[(&str, Option<String>)]| -> Vec<(String, String)> { pairs.iter().filter_map(|(k, v)| v.clone().map(|v| (k.to_string(), v))).collect() };
        let flag = |b: &Option<bool>| b.map(|b| b.to_string());
        match request {
            EsRequest::Info => Request::new("GET", "/"),
            EsRequest::IndexExists { index } => Request::new("HEAD", format!("/{}", self.index_path(index))),
            EsRequest::CreateIndex { index, body } => Request::new("PUT", format!("/{}", self.index_path(index))).json(body.clone()),
            EsRequest::DeleteIndex { index } => Request::new("DELETE", format!("/{}", self.index_path(index))),
            EsRequest::RefreshIndex { index } => Request::new("POST", format!("/{}/_refresh", self.index_path(index))),
            EsRequest::GetMapping { index } => Request::new("GET", format!("/{}/_mapping", self.index_path(index))),
            EsRequest::PutMapping { index, properties } => Request::new("PUT", format!("/{}/_mapping", self.index_path(index))).json(serde_json::json!({"properties": properties})),
            EsRequest::Search { index, body, scroll } => {
                let mut r = Request::new("POST", format!("/{}/_search", self.index_path(index))).json(body.clone());
                r.query = query(&[("scroll", scroll.clone())]);
                r
            }
            EsRequest::Scroll { scroll_id, scroll } => Request::new("POST", "/_search/scroll").json(serde_json::json!({"scroll_id": scroll_id, "scroll": scroll})),
            EsRequest::ClearScroll { scroll_id } => Request::new("DELETE", "/_search/scroll").json(serde_json::json!({"scroll_id": scroll_id})),
            EsRequest::MultiSearch { index, searches } => {
                let path = match index {
                    Some(_) => format!("/{}/_msearch", self.index_path(index)),
                    None => "/_msearch".to_string(),
                };
                let mut r = Request::new("POST", path);
                r.body = Body::Ndjson(searches.clone());
                r
            }
            EsRequest::MultiGet { docs, source } => {
                let mut r = Request::new("POST", "/_mget").json(serde_json::json!({"docs": docs}));
                r.query = query(&[("_source", source.as_ref().map(|fields| fields.join(",")))]);
                r
            }
            EsRequest::Get { index, id } => Request::new("GET", format!("/{}/_doc/{}", self.index_path(index), encode(id))),
            EsRequest::Count { index, body } => Request::new("POST", format!("/{}/_count", self.index_path(index))).json(body.clone()),
            EsRequest::Update { index, id, body, refresh } => {
                let mut r = Request::new("POST", format!("/{}/_update/{}", self.index_path(index), encode(id))).json(body.clone());
                r.query = query(&[("refresh", flag(refresh))]);
                r
            }
            EsRequest::Delete { index, id, refresh } => {
                let mut r = Request::new("DELETE", format!("/{}/_doc/{}", self.index_path(index), encode(id)));
                r.query = query(&[("refresh", flag(refresh))]);
                r
            }
            EsRequest::DeleteByQuery { index, body, conflicts, refresh } | EsRequest::UpdateByQuery { index, body, conflicts, refresh } => {
                let endpoint = if matches!(request, EsRequest::DeleteByQuery { .. }) { "_delete_by_query" } else { "_update_by_query" };
                let mut r = Request::new("POST", format!("/{}/{endpoint}", self.index_path(index))).json(body.clone());
                r.query = query(&[("conflicts", conflicts.map(|c| c.as_str().to_string())), ("refresh", flag(refresh))]);
                r
            }
            EsRequest::Bulk { lines, timeout, refresh } => {
                let mut r = Request::new("POST", "/_bulk");
                r.query = query(&[("timeout", timeout.clone()), ("refresh", flag(refresh))]);
                r.body = Body::Ndjson(lines.clone());
                r
            }
        }
    }

    /// 요청을 보내고 `(상태 코드, 본문 텍스트)`를 돌려준다. 상태 코드 오류는 호출한 쪽이 해석한다.
    fn execute(&self, request: Request) -> Result<(u16, String), EsError> {
        let agent = self.agent.as_ref().map_err(Clone::clone)?;
        let mut url = format!("{}{}", self.base, request.path);
        if !request.query.is_empty() {
            let pairs: Vec<String> = request.query.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
            url.push('?');
            url.push_str(&pairs.join("&"));
        }
        let mut builder = ureq::http::Request::builder().method(request.method).uri(&url).header("Accept", "application/json");
        if let Some(auth) = &self.auth {
            builder = builder.header("Authorization", auth);
        }
        let transport_err = |e: &dyn std::fmt::Display| EsError::Transport(e.to_string());
        let result = match request.body {
            Body::None => agent.run(builder.body(()).map_err(|e| transport_err(&e))?),
            Body::Json(value) => agent.run(builder.header("Content-Type", "application/json").body(value.to_string().into_bytes()).map_err(|e| transport_err(&e))?),
            Body::Ndjson(lines) => {
                let mut text = String::new();
                for line in &lines {
                    text.push_str(&line.to_string());
                    text.push('\n');
                }
                agent.run(builder.header("Content-Type", "application/x-ndjson").body(text.into_bytes()).map_err(|e| transport_err(&e))?)
            }
        };
        let mut response = result.map_err(|e| transport_err(&e))?;
        let status = response.status().as_u16();
        let text = response.body_mut().with_config().limit(RESPONSE_LIMIT).read_to_string().map_err(|e| transport_err(&e))?;
        Ok((status, text))
    }
}

impl EsTransport for HttpTransport {
    fn index_name(&self) -> &str {
        &self.index
    }

    fn send(&self, request: &EsRequest) -> Result<Value, EsError> {
        if !self.base.starts_with("http://") && !self.base.starts_with("https://") {
            return Err(EsError::Transport(format!("ES URL은 http 또는 https여야 한다: {}", self.base)));
        }
        let (status, text) = self.execute(self.plan(request))?;
        if matches!(request, EsRequest::IndexExists { .. }) {
            return match status {
                200..=299 => Ok(Value::Bool(true)),
                404 => Ok(Value::Bool(false)),
                _ => Err(EsError::Status { status, body: text }),
            };
        }
        if status >= 400 {
            return Err(EsError::Status { status, body: text });
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| EsError::Parse(e.to_string()))
    }
}
