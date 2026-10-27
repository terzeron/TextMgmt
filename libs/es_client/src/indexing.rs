//! 색인 적재에 쓰는 ES 호출. `backend/es_manager.py`의 `ESManager` 중 loader가 쓰는 부분의 이식.
//!
//! 인덱스 만들기·매핑 보강, 문서 대량 입력, 기존 문서 조회, 중복·고아 문서 삭제를 맡는다.
//! HTTP와 TLS 인증서를 검증하는 HTTPS를 지원한다.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::{EsClient, EsError};

/// bulk 한 번에 보내는 문서 수.
pub const BULK_CHUNK_SIZE: usize = 100;
/// bulk 요청 재시도 횟수.
pub const BULK_MAX_RETRIES: u32 = 3;
/// `delete_by_ids`가 한 번에 지우는 id 수.
pub const DELETE_CHUNK_SIZE: usize = 10_000;
const SCROLL_PAGE_SIZE: usize = 5000;

/// loader가 쓰는 색인 조작. 테스트가 실제 ES 없이 적재 흐름을 확인할 수 있게 분리했다.
pub trait IndexStore {
    fn index_name(&self) -> &str;
    fn ping(&self) -> Result<(), EsError>;
    fn index_exists(&self) -> Result<bool, EsError>;
    fn create_index(&self) -> Result<(), EsError>;
    fn delete_index(&self) -> Result<(), EsError>;
    /// `{inode: file_path}`
    fn get_existing_paths(&self, doc_ids: &[i64]) -> Result<HashMap<i64, String>, EsError>;
    /// `{inode: {created_time, created_time_source, updated_time 중 있는 것}}`
    fn get_existing_created_time_metadata(&self, doc_ids: &[i64]) -> Result<HashMap<i64, BTreeMap<String, String>>, EsError>;
    /// 오류는 삼키고 0을 돌려준다(원본과 같다).
    fn delete_by_file_paths(&self, file_paths: &[String], exclude_ids: &[i64]) -> i64;
    fn get_doc_ids_by_path_prefix(&self, path_prefix: &str) -> HashSet<i64>;
    fn delete_by_ids(&self, ids: &[i64]) -> i64;
    fn insert(&self, docs: &[(i64, Value)]) -> Result<(), EsError>;
    fn refresh(&self) -> Result<(), EsError>;
}

/// 인덱스 설정: BM25 유사도, nori 분석기.
pub fn index_settings() -> Value {
    json!({
        "index": {"similarity": {"default": {"type": "BM25"}}},
        "analysis": {
            "tokenizer": {"nori_tokenizer": {"type": "nori_tokenizer", "decompound_mode": "discard"}},
            "filter": {"nori_posfilter": {"type": "nori_part_of_speech", "stoptags": [
                "EC", "EF", "EP", "ETM", "ETN",
                "JC", "JKB", "JKC", "JKG", "JKO", "JKQ", "JKS", "JKV", "JX",
                "SC", "SE", "SF", "SP", "SSC", "SSO", "SY",
                "XSA", "XSN", "XSV",
                "IC", "MAJ",
            ]}},
            "analyzer": {"nori_analyzer": {"type": "custom", "tokenizer": "nori_tokenizer", "filter": ["nori_posfilter", "lowercase"]}},
        },
    })
}

fn text_with_keyword() -> Value {
    json!({"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}})
}

/// 인덱스 매핑.
pub fn index_mappings() -> Value {
    json!({"properties": {
        "category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}}},
        "title": text_with_keyword(),
        "author": text_with_keyword(),
        // EPUB의 dc:publisher. 전집 판정의 결정적 증거라 자동분류가 쓴다.
        "publisher": text_with_keyword(),
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
    }})
}

/// 경로 prefix 하위(자기 자신 포함) 문서를 매칭하는 쿼리. `""`, `"."`, `"/"`이면 전체다.
pub fn path_prefix_query(path_prefix: &str) -> Value {
    let p = path_prefix.trim_matches('/');
    if p.is_empty() || p == "." {
        return json!({"match_all": {}});
    }
    json!({"bool": {"should": [{"term": {"file_path": p}}, {"prefix": {"file_path": format!("{p}/")}}], "minimum_should_match": 1}})
}

fn mget_docs(index: &str, ids: &[i64]) -> Vec<Value> {
    ids.iter().map(|id| json!({"_index": index, "_id": id.to_string()})).collect()
}

fn transport(e: &EsError) -> bool {
    matches!(e, EsError::Transport(_))
}

impl EsClient {
    fn call_ndjson(&self, path: &str, body: &str) -> Result<Value, EsError> {
        let url = format!("{}{path}", self.base);
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(EsError::Transport(format!("ES URL은 http 또는 https여야 한다: {}", self.base)));
        }
        let mut resp = self.agent.post(&url).header("Authorization", &self.auth).content_type("application/x-ndjson").send(body.as_bytes()).map_err(|e| EsError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(|e| EsError::Transport(e.to_string()))?;
        if status >= 400 {
            return Err(EsError::Status { status, body: text });
        }
        serde_json::from_str(&text).map_err(|e| EsError::Parse(e.to_string()))
    }

    fn mget(&self, ids: &[i64], source: &[&str]) -> Result<Vec<Value>, EsError> {
        let body = json!({"docs": mget_docs(&self.index, ids)});
        let response = self.call("POST", &format!("/_mget?_source={}", source.join(",")), Some(&body))?;
        Ok(response["docs"].as_array().cloned().unwrap_or_default())
    }

    fn mapping_properties(&self) -> Result<Value, EsError> {
        let mapping = self.call("GET", &format!("/{}/_mapping", self.index), None)?;
        Ok(mapping[&self.index]["mappings"]["properties"].clone())
    }

    fn put_properties(&self, properties: Value) -> Result<(), EsError> {
        self.call("PUT", &format!("/{}/_mapping", self.index), Some(&json!({"properties": properties}))).map(|_| ())
    }

    /// 기존 인덱스에 빠진 필드(category.nori, created_time 계열, publisher)를 더한다. 실패해도 경고만 남기고 넘어간다.
    fn ensure_existing_index_mappings(&self) {
        let properties = match self.mapping_properties() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("WARNING Failed to read mapping of {}: {e}", self.index);
                return;
            }
        };
        let warn = |what: &str, r: Result<(), EsError>| {
            if let Err(e) = r {
                eprintln!("WARNING Failed to add {what}: {e}");
            }
        };
        if properties["category"]["fields"]["nori"].is_null() {
            warn("category.nori sub-field", self.put_properties(json!({"category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}}}})));
        }
        let mut missing = Map::new();
        if properties.get("created_time").is_none_or(Value::is_null) {
            missing.insert("created_time".into(), json!({"type": "date"}));
        }
        if properties.get("created_time_source").is_none_or(Value::is_null) {
            missing.insert("created_time_source".into(), json!({"type": "keyword"}));
        }
        if !missing.is_empty() {
            warn("created_time fields", self.put_properties(Value::Object(missing)));
        }
        if properties.get("publisher").is_none_or(Value::is_null) {
            warn("publisher field", self.put_properties(json!({"publisher": text_with_keyword()})));
        }
    }

    fn delete_by_query(&self, query: Value) -> Result<i64, EsError> {
        let response = self.call("POST", &format!("/{}/_delete_by_query?conflicts=proceed&refresh=false", self.index), Some(&json!({"query": query})))?;
        Ok(response["deleted"].as_i64().unwrap_or(0))
    }
}

impl IndexStore for EsClient {
    fn index_name(&self) -> &str {
        &self.index
    }

    fn ping(&self) -> Result<(), EsError> {
        self.call("GET", "/", None).map(|_| ())
    }

    fn index_exists(&self) -> Result<bool, EsError> {
        match self.call("GET", &format!("/{}", self.index), None) {
            Ok(_) => Ok(true),
            Err(EsError::Status { status: 404, .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn create_index(&self) -> Result<(), EsError> {
        if self.index_exists()? {
            self.ensure_existing_index_mappings();
            return Ok(());
        }
        match self.call("PUT", &format!("/{}", self.index), Some(&json!({"settings": index_settings(), "mappings": index_mappings()}))) {
            Ok(_) => Ok(()),
            Err(EsError::Status { status: 400, body }) if body.contains("resource_already_exists_exception") => {
                eprintln!("INFO Index {} already exists, skipping creation", self.index);
                self.ensure_existing_index_mappings();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn delete_index(&self) -> Result<(), EsError> {
        if self.index_exists()? {
            self.call("DELETE", &format!("/{}", self.index), None)?;
        }
        Ok(())
    }

    fn get_existing_paths(&self, doc_ids: &[i64]) -> Result<HashMap<i64, String>, EsError> {
        if doc_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut result = HashMap::new();
        for doc in self.mget(doc_ids, &["file_path"])? {
            if doc["found"].as_bool().unwrap_or(false)
                && let (Some(id), Some(path)) = (doc["_id"].as_str().and_then(|s| s.parse().ok()), doc["_source"]["file_path"].as_str())
            {
                result.insert(id, path.to_owned());
            }
        }
        Ok(result)
    }

    fn get_existing_created_time_metadata(&self, doc_ids: &[i64]) -> Result<HashMap<i64, BTreeMap<String, String>>, EsError> {
        if doc_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut result = HashMap::new();
        for doc in self.mget(doc_ids, &["created_time", "created_time_source", "updated_time"])? {
            if !doc["found"].as_bool().unwrap_or(false) {
                continue;
            }
            let metadata: BTreeMap<String, String> =
                ["created_time", "created_time_source", "updated_time"].iter().filter_map(|f| doc["_source"][*f].as_str().map(|v| ((*f).to_owned(), v.to_owned()))).collect();
            if !metadata.is_empty()
                && let Some(id) = doc["_id"].as_str().and_then(|s| s.parse().ok())
            {
                result.insert(id, metadata);
            }
        }
        Ok(result)
    }

    fn delete_by_file_paths(&self, file_paths: &[String], exclude_ids: &[i64]) -> i64 {
        if file_paths.is_empty() {
            return 0;
        }
        let mut query = json!({"bool": {"must": [{"terms": {"file_path": file_paths}}]}});
        if !exclude_ids.is_empty() {
            query["bool"]["must_not"] = json!([{"ids": {"values": exclude_ids.iter().map(i64::to_string).collect::<Vec<_>>()}}]);
        }
        match self.delete_by_query(query) {
            Ok(deleted) => {
                if deleted > 0 {
                    eprintln!("INFO delete_by_file_paths: {deleted} docs deleted for {} paths", file_paths.len());
                }
                deleted
            }
            Err(e) => {
                eprintln!("ERROR delete_by_file_paths error: {e}");
                0
            }
        }
    }

    fn get_doc_ids_by_path_prefix(&self, path_prefix: &str) -> HashSet<i64> {
        let mut ids = HashSet::new();
        let mut scroll_id: Option<String> = None;
        let outcome = (|| -> Result<(), EsError> {
            let body = json!({"query": path_prefix_query(path_prefix), "size": SCROLL_PAGE_SIZE, "_source": false});
            let mut response = self.call("POST", &format!("/{}/_search?scroll=10m", self.index), Some(&body))?;
            scroll_id = response["_scroll_id"].as_str().map(str::to_owned);
            loop {
                let hits = response["hits"]["hits"].as_array().cloned().unwrap_or_default();
                if hits.is_empty() {
                    return Ok(());
                }
                ids.extend(hits.iter().filter_map(|h| h["_id"].as_str().and_then(|s| s.parse::<i64>().ok())));
                response = self.call("POST", "/_search/scroll", Some(&json!({"scroll": "10m", "scroll_id": scroll_id})))?;
                scroll_id = response["_scroll_id"].as_str().map(str::to_owned).or(scroll_id.take());
            }
        })();
        if let Err(e) = outcome {
            eprintln!("ERROR get_doc_ids_by_path_prefix error: {e}");
        }
        if let Some(id) = scroll_id {
            let _ = self.call("DELETE", "/_search/scroll", Some(&json!({"scroll_id": id})));
        }
        ids
    }

    fn delete_by_ids(&self, ids: &[i64]) -> i64 {
        if ids.is_empty() {
            return 0;
        }
        let mut deleted = 0;
        for chunk in ids.chunks(DELETE_CHUNK_SIZE) {
            match self.delete_by_query(json!({"ids": {"values": chunk.iter().map(i64::to_string).collect::<Vec<_>>()}})) {
                Ok(n) => deleted += n,
                Err(e) => {
                    eprintln!("ERROR delete_by_ids error: {e}");
                    return deleted;
                }
            }
        }
        if deleted > 0 {
            eprintln!("INFO delete_by_ids: {deleted} docs deleted for {} ids", ids.len());
        }
        deleted
    }

    fn insert(&self, docs: &[(i64, Value)]) -> Result<(), EsError> {
        self.insert_with(docs, &|secs| std::thread::sleep(Duration::from_secs(secs)))
    }

    fn refresh(&self) -> Result<(), EsError> {
        self.call("POST", &format!("/{}/_refresh", self.index), None).map(|_| ())
    }
}

impl EsClient {
    /// 문서를 100건씩 bulk로 넣는다. 전송 오류는 `1, 2, 4`초 쉬며 3번까지 다시 시도한다. `sleep`은 쉬는 방법이다.
    pub fn insert_with(&self, docs: &[(i64, Value)], sleep: &dyn Fn(u64)) -> Result<(), EsError> {
        for chunk in docs.chunks(BULK_CHUNK_SIZE) {
            let mut body = String::new();
            for (id, doc) in chunk {
                body.push_str(&json!({"index": {"_index": self.index, "_id": id.to_string()}}).to_string());
                body.push('\n');
                body.push_str(&doc.to_string());
                body.push('\n');
            }
            eprintln!("INFO {} items inserted", chunk.len());
            for attempt in 0..BULK_MAX_RETRIES {
                match self.call_ndjson("/_bulk?timeout=60s&refresh=false", &body) {
                    Ok(response) => {
                        // 원본은 항목별 실패를 확인하지 않는다. 조용히 사라지지 않도록 경고를 남긴다.
                        if response["errors"].as_bool().unwrap_or(false) {
                            let failed = response["items"].as_array().map_or(0, |items| items.iter().filter(|i| i["index"]["error"].is_object()).count());
                            eprintln!("WARNING ES bulk 응답에 항목 오류가 있다: {failed}건");
                        }
                        break;
                    }
                    Err(e) if transport(&e) && attempt + 1 < BULK_MAX_RETRIES => {
                        let wait = 1u64 << attempt;
                        eprintln!("WARNING ES bulk 요청 실패 (시도 {}/{BULK_MAX_RETRIES}): {e}. {wait}초 후 재시도...", attempt + 1);
                        sleep(wait);
                    }
                    Err(e) => {
                        eprintln!("ERROR ES bulk 요청 최종 실패: {e}");
                        return Err(e);
                    }
                }
            }
        }
        Ok(())
    }
}
