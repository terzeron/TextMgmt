//! `EsTransport`의 실제 HTTP 구현. `libs/es_client`와 같은 HTTP(S) 전송 및 인증서 검증 규칙을 쓴다.
//!
//! `op`는 Python `Elasticsearch` 클라이언트 메서드 이름이고 `kwargs`는 같은 키워드 인자다.
//! 이 모듈이 그것을 ES REST 요청으로 바꾼다(검증한 서버는 ES 9.1.0이다).
//!
//! | op | 요청 |
//! |---|---|
//! | `info` | `GET /` |
//! | `indices.exists` | `HEAD /{index}` (200이면 `true`, 404이면 `false`) |
//! | `indices.create` / `delete` / `refresh` | `PUT /{index}` (본문 `body`) / `DELETE /{index}` / `POST /{index}/_refresh` |
//! | `indices.get_mapping` / `put_mapping` | `GET /{index}/_mapping` / `PUT /{index}/_mapping` (본문 `properties`) |
//! | `search` | `POST /{index}/_search` (`scroll`은 쿼리 문자열, `from_`는 `from`, `source`는 `_source`, 나머지는 본문) |
//! | `scroll` / `clear_scroll` | `POST` / `DELETE /_search/scroll` |
//! | `msearch` | `POST /{index}/_msearch` 또는 `POST /_msearch` (`searches`를 NDJSON으로) |
//! | `mget` | `POST /_mget?_source=...` (본문 `docs`) |
//! | `get` / `update` / `delete` | `GET` / `POST /{index}/_update/{id}` / `DELETE /{index}/_doc/{id}` (`refresh`는 쿼리 문자열) |
//! | `count` | `POST /{index}/_count` |
//! | `delete_by_query` / `update_by_query` | `POST /{index}/_delete_by_query` / `_update_by_query` (`conflicts`, `refresh`는 쿼리 문자열) |
//! | `bulk` | `POST /_bulk` (`body` 배열을 NDJSON으로, `timeout`, `refresh`는 쿼리 문자열) |

use std::path::Path;
use std::time::Duration;

use es_client::build_agent;
use serde_json::{Map, Value};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::{EsError, EsTransport};

/// 본문 대신 쿼리 문자열로 가는 인자. 이 순서로 붙인다.
const QUERY_PARAMS: [&str; 4] = ["scroll", "timeout", "conflicts", "refresh"];
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

fn param_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

impl HttpTransport {
    /// HTTP 또는 HTTPS endpoint를 쓴다. HTTPS는 기본 WebPKI roots로 검증한다.
    pub fn new(url: &str, user: &str, password: &str, index: &str, timeout: Duration) -> Self {
        Self::new_with_ca(url, user, password, index, timeout, None)
    }

    /// 사설 CA를 지정하면 해당 CA로 HTTPS 인증서를 검증한다.
    pub fn new_with_ca(url: &str, user: &str, password: &str, index: &str, timeout: Duration, ca_cert_path: Option<&Path>) -> Self {
        let auth = (!user.is_empty() || !password.is_empty()).then(|| format!("Basic {}", STANDARD.encode(format!("{user}:{password}"))));
        let agent = build_agent(timeout, ca_cert_path).map_err(EsError::Other);
        HttpTransport { agent, base: url.trim_end_matches('/').to_string(), auth, index: index.to_string() }
    }

    fn index_of<'a>(&'a self, kwargs: &'a Value) -> String {
        encode(kwargs.get("index").and_then(Value::as_str).unwrap_or(&self.index))
    }

    fn id_of(kwargs: &Value) -> Result<String, EsError> {
        kwargs.get("id").and_then(param_text).map(|id| encode(&id)).ok_or_else(|| EsError::Other("id 인자가 없다".to_string()))
    }

    /// 본문이 되는 인자: `body`가 있으면 그것을 바탕으로 하고, 쿼리 문자열용 인자와 `index`, `id`를 뺀 나머지를 합친다.
    fn payload(kwargs: &Value, rename: bool) -> Value {
        let mut body = match kwargs.get("body") {
            Some(Value::Object(o)) => o.clone(),
            _ => Map::new(),
        };
        for (key, value) in kwargs.as_object().into_iter().flatten() {
            if matches!(key.as_str(), "index" | "id" | "body") || QUERY_PARAMS.contains(&key.as_str()) {
                continue;
            }
            let key = match (rename, key.as_str()) {
                (true, "from_") => "from",
                (true, "source") => "_source",
                (_, other) => other,
            };
            body.insert(key.to_string(), value.clone());
        }
        Value::Object(body)
    }

    fn with_query(mut request: Request, kwargs: &Value) -> Request {
        for key in QUERY_PARAMS {
            if let Some(text) = kwargs.get(key).and_then(param_text) {
                request.query.push((key.to_string(), text));
            }
        }
        request
    }

    fn plan(&self, op: &str, kwargs: &Value) -> Result<Request, EsError> {
        let index = self.index_of(kwargs);
        let request = match op {
            "info" => Request::new("GET", "/"),
            "indices.exists" => Request::new("HEAD", format!("/{index}")),
            "indices.create" => Request::new("PUT", format!("/{index}")).json(Self::payload(kwargs, false)),
            "indices.delete" => Request::new("DELETE", format!("/{index}")),
            "indices.refresh" => Request::new("POST", format!("/{index}/_refresh")),
            "indices.get_mapping" => Request::new("GET", format!("/{index}/_mapping")),
            "indices.put_mapping" => Request::new("PUT", format!("/{index}/_mapping")).json(Self::payload(kwargs, false)),
            "search" => Self::with_query(Request::new("POST", format!("/{index}/_search")).json(Self::payload(kwargs, true)), kwargs),
            "scroll" => Request::new("POST", "/_search/scroll").json(serde_json::json!({"scroll_id": kwargs["scroll_id"], "scroll": kwargs["scroll"]})),
            "clear_scroll" => Request::new("DELETE", "/_search/scroll").json(serde_json::json!({"scroll_id": kwargs["scroll_id"]})),
            "msearch" => {
                let path = if kwargs.get("index").is_some() { format!("/{index}/_msearch") } else { "/_msearch".to_string() };
                let mut request = Request::new("POST", path);
                request.body = Body::Ndjson(kwargs["searches"].as_array().cloned().unwrap_or_default());
                request
            }
            "mget" => {
                let path = if kwargs.get("index").is_some() { format!("/{index}/_mget") } else { "/_mget".to_string() };
                let mut request = Request::new("POST", path).json(serde_json::json!({"docs": kwargs["docs"]}));
                if let Some(fields) = kwargs.get("source").and_then(Value::as_array) {
                    let joined: Vec<&str> = fields.iter().filter_map(Value::as_str).collect();
                    request.query.push(("_source".to_string(), joined.join(",")));
                }
                request
            }
            "get" => Request::new("GET", format!("/{index}/_doc/{}", Self::id_of(kwargs)?)),
            "count" => Request::new("POST", format!("/{index}/_count")).json(Self::payload(kwargs, false)),
            "update" => Self::with_query(Request::new("POST", format!("/{index}/_update/{}", Self::id_of(kwargs)?)).json(Self::payload(kwargs, false)), kwargs),
            "delete" => Self::with_query(Request::new("DELETE", format!("/{index}/_doc/{}", Self::id_of(kwargs)?)), kwargs),
            "delete_by_query" => Self::with_query(Request::new("POST", format!("/{index}/_delete_by_query")).json(Self::payload(kwargs, false)), kwargs),
            "update_by_query" => Self::with_query(Request::new("POST", format!("/{index}/_update_by_query")).json(Self::payload(kwargs, false)), kwargs),
            "bulk" => {
                let mut request = Self::with_query(Request::new("POST", "/_bulk"), kwargs);
                request.body = Body::Ndjson(kwargs["body"].as_array().cloned().unwrap_or_default());
                request
            }
            _ => return Err(EsError::Unsupported),
        };
        Ok(request)
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

    fn call(&self, op: &str, kwargs: &Value) -> Result<Value, EsError> {
        if !self.base.starts_with("http://") && !self.base.starts_with("https://") {
            return Err(EsError::Transport(format!("ES URL은 http 또는 https여야 한다: {}", self.base)));
        }
        let request = self.plan(op, kwargs)?;
        let (status, text) = self.execute(request)?;
        if op == "indices.exists" {
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
