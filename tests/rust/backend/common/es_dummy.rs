//! ESManager 테스트용 가짜 저수준 ES 전송 계층. Python `tests/test_es_manager.py`의 `DummyES`/`DummyIndices`에 대응한다.
//! 설계: docs/rust-test-seams.md 3.3. 구 `EsApi`(call)와 `EsLike`(index_name/get/search)를 `EsTransport` 하나로 통일했다.
//!
//! 가정한 API (아직 구현 전이다):
//! - `backend::es_manager::EsTransport`: `fn index_name(&self) -> &str`, `fn call(&self, op: &str, kwargs: &Value) -> Result<Value, EsError>`
//!   (`get`, `search`는 `call("get")`, `call("search")`로 가는 기본 구현이다. Python `Elasticsearch` 클라이언트의 메서드 호출을 `op`와 키워드 인자 JSON으로 옮긴 것이다. `indices.exists`처럼 점으로 하위 객체를 쓴다)
//! - `backend::es_manager::EsError`: `MissingEnv`, `Transport(String)`, `Status { status: u16, body: String }`, `Parse`, `Unsupported`, `Other(String)`
//!   (구 `ElasticsearchError` 매핑: ConnectionError -> Transport, NotFoundError -> Status{404}, BadRequestError{message,status} -> Status{status, body:message}, RuntimeError -> Other)
//!
//! 사용: `use common::es_dummy::{DummyEs, runtime_error};` `ESManager::with_transport(DummyEs::new())`
//! 고수준 `EsManagerApi` 대역이 필요하면 `es_fake.rs`의 `FakeEsManager`를 쓴다.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use backend::es_manager::{EsError, EsTransport};
use serde_json::{json, Value};

pub type Handler = Arc<dyn Fn(&Value) -> Result<Value, EsError> + Send + Sync>;

/// `DummyES`와 `DummyIndices`의 상태 필드를 한곳에 모았다.
pub struct State {
    pub bulk_calls: usize,
    pub search_calls: usize,
    pub scroll_calls: usize,
    pub cleared_scroll: bool,
    pub exists_called: bool,
    pub created: bool,
    pub created_body: Value,
    pub deleted: bool,
    pub put_mapping_called: bool,
    pub put_mapping_properties: Value,
    /// `DummyIndices.mapping`. `exists` 키는 `indices.exists`의 응답이다.
    pub mapping: Value,
    /// 호출 순서대로 `(op, kwargs)`를 쌓는다. `MagicMock`의 `assert_called*`를 대신한다.
    pub calls: Vec<(String, Value)>,
}

pub struct DummyEs {
    index: String,
    overrides: Mutex<HashMap<String, Handler>>,
    state: Mutex<State>,
}

/// 구 `RuntimeError`.
pub fn runtime_error(message: &str) -> EsError {
    EsError::Other(message.to_string())
}

/// 구 `ConnectionError`.
pub fn connection_error(message: &str) -> EsError {
    EsError::Transport(message.to_string())
}

/// 구 `NotFoundError`.
pub fn not_found_error(message: &str) -> EsError {
    EsError::Status { status: 404, body: message.to_string() }
}

/// 구 `BadRequestError { message, status }`.
pub fn bad_request_error(message: &str, status: u16) -> EsError {
    EsError::Status { status, body: message.to_string() }
}

pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_none_or(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

impl DummyEs {
    /// 색인 이름은 `test`다.
    pub fn new() -> Arc<Self> {
        Self::with_index("test")
    }

    /// `index_name()`이 돌려줄 색인 이름을 정한다. 분류기 corpus 테스트는 `books`를 쓴다.
    pub fn with_index(index: &str) -> Arc<Self> {
        Arc::new(Self {
            index: index.to_string(),
            overrides: Mutex::new(HashMap::new()),
            state: Mutex::new(State {
                bulk_calls: 0,
                search_calls: 0,
                scroll_calls: 0,
                cleared_scroll: false,
                exists_called: false,
                created: false,
                created_body: Value::Null,
                deleted: false,
                put_mapping_called: false,
                put_mapping_properties: Value::Null,
                mapping: json!({}),
                calls: Vec::new(),
            }),
        })
    }

    /// Python에서 `es.search = fake`처럼 메서드를 갈아끼우는 일에 대응한다. 기본 동작과 상태 기록은 건너뛴다.
    pub fn on(&self, op: &str, handler: impl Fn(&Value) -> Result<Value, EsError> + Send + Sync + 'static) {
        self.overrides.lock().unwrap().insert(op.to_string(), Arc::new(handler));
    }

    /// 바꿔치기한 메서드를 기본 동작으로 되돌린다. Python `es.delete_by_query = DummyES().delete_by_query`에 대응한다.
    pub fn reset(&self, op: &str) {
        self.overrides.lock().unwrap().remove(op);
    }

    /// `op`에 항상 같은 응답을 준다. 분류기 corpus 테스트의 `get_result`/`search_result`에 대응한다.
    pub fn respond(&self, op: &str, value: Value) {
        self.on(op, move |_| Ok(value.clone()));
    }

    /// `op`를 부르면 항상 `Other(message)` 오류를 낸다. 구 `raise_on`에 대응한다.
    pub fn fail(&self, op: &str, message: &str) {
        let message = message.to_string();
        self.on(op, move |_| Err(EsError::Other(message.clone())));
    }

    pub fn state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    pub fn set_mapping(&self, mapping: Value) {
        self.state(|s| s.mapping = mapping);
    }

    /// 해당 `op`로 들어온 키워드 인자를 호출 순서대로 돌려준다.
    pub fn calls_of(&self, op: &str) -> Vec<Value> {
        self.state(|s| s.calls.iter().filter(|(o, _)| o == op).map(|(_, kw)| kw.clone()).collect())
    }

    fn default_search(&self, kw: &Value) -> Result<Value, EsError> {
        self.state(|s| s.search_calls += 1);
        let body = kw.get("body").filter(|b| !b.is_null());
        if let Some(body) = body {
            if truthy(body.get("aggs")) {
                return Ok(json!({"aggregations": {"unique_values": {"buckets": [{"key": "A", "doc_count": 2}]}}}));
            }
            if body.get("query").and_then(|q| q.get("match_all")).is_some_and(|v| !v.is_null()) {
                return Ok(json!({"_scroll_id": "scroll1", "hits": {"hits": [{"_source": {"category": "A", "file_path": "/a"}}, {"_source": {"category": "A", "file_path": "/b"}}]}}));
            }
        }
        if kw.get("scroll").is_some_and(|v| !v.is_null()) {
            return Ok(json!({"_scroll_id": "scroll1", "hits": {"max_score": 2.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 2.0}]}}));
        }
        let size = kw.get("size").and_then(Value::as_i64).unwrap_or(10);
        let query = kw.get("query").filter(|q| !q.is_null());
        if size == 1 && query.is_some_and(|q| truthy(q.get("bool").and_then(|b| b.get("filter")))) {
            return Ok(json!({"hits": {"hits": [{"_score": 7.0}]}}));
        }
        if truthy(kw.get("track_total_hits")) {
            return Ok(json!({"hits": {"total": {"value": 2}, "max_score": 5.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 5.0}]}}));
        }
        Ok(json!({"hits": {"max_score": 2.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 2.0}]}}))
    }

    /// Python 하위 클래스의 `super().search(...)`처럼 기본 동작을 직접 부른다. 호출 기록은 남기지 않는다.
    pub fn base_call(&self, op: &str, kw: &Value) -> Result<Value, EsError> {
        match op {
            "info" => Ok(json!({"ok": true})),
            "indices.exists" => {
                let exists = self.state(|s| {
                    s.exists_called = true;
                    s.mapping.get("exists").and_then(Value::as_bool).unwrap_or(false)
                });
                Ok(json!(exists))
            }
            "indices.create" => {
                self.state(|s| {
                    s.created = true;
                    s.created_body = kw["body"].clone();
                });
                Ok(json!({"acknowledged": true}))
            }
            "indices.get_mapping" => Ok(self.state(|s| s.mapping.clone())),
            "indices.put_mapping" => {
                self.state(|s| {
                    s.put_mapping_called = true;
                    s.put_mapping_properties = kw["properties"].clone();
                });
                Ok(json!({"acknowledged": true}))
            }
            "indices.delete" => {
                self.state(|s| s.deleted = true);
                Ok(json!({"acknowledged": true}))
            }
            "indices.refresh" => Ok(json!({"acknowledged": true})),
            "mget" => Ok(json!({"docs": [{"_id": "1", "found": true, "_source": {"file_path": "/a"}}, {"_id": "2", "found": false}]})),
            "bulk" => {
                let calls = self.state(|s| {
                    s.bulk_calls += 1;
                    s.bulk_calls
                });
                if calls == 1 {
                    Err(connection_error("boom"))
                } else {
                    Ok(json!({"errors": false}))
                }
            }
            "search" => self.default_search(kw),
            "scroll" => {
                self.state(|s| s.scroll_calls += 1);
                Ok(json!({"_scroll_id": "scroll2", "hits": {"max_score": 2.0, "hits": []}}))
            }
            "clear_scroll" => {
                self.state(|s| s.cleared_scroll = true);
                Ok(json!({"succeeded": true}))
            }
            "msearch" => Ok(json!({"responses": [{"hits": {"hits": [{"_score": 10.0}]}}, {"hits": {"total": {"value": 1}, "hits": [{"_id": "2", "_source": {"a": 2}, "_score": 5.0}]}}]})),
            "get" => Err(not_found_error("not found")),
            "count" => Ok(json!({"count": 3})),
            "delete_by_query" => Ok(json!({"deleted": 2, "failures": []})),
            "update_by_query" => Ok(json!({"updated": 4, "failures": []})),
            "update" => Ok(json!({"_shards": {"failed": 1}})),
            "delete" => Ok(json!({"result": "deleted"})),
            other => panic!("DummyEs가 모르는 호출이다: {other}"),
        }
    }
}

impl EsTransport for DummyEs {
    fn index_name(&self) -> &str {
        &self.index
    }

    fn call(&self, op: &str, kwargs: &Value) -> Result<Value, EsError> {
        self.state(|s| s.calls.push((op.to_string(), kwargs.clone())));
        let handler = self.overrides.lock().unwrap().get(op).cloned();
        match handler {
            Some(h) => h(kwargs),
            None => self.base_call(op, kwargs),
        }
    }
}
