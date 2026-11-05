//! 색인 적재에 쓰는 ES 호출. loader가 쓴다.
//!
//! 인덱스 만들기·매핑 보강, 문서 대량 입력, 기존 문서 조회, 중복·고아 문서 삭제를 맡는다.
//! HTTP와 TLS 인증서를 검증하는 HTTPS를 지원한다.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::transport::{Conflicts, EsRequest};
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
    /// 지운 문서 수. 오류 처리(원본은 삼키고 0)는 호출자가 정한다.
    fn delete_by_file_paths(&self, file_paths: &[String], exclude_ids: &[i64]) -> Result<i64, EsError>;
    /// scroll 도중 실패하면 모은 일부 대신 오류를 돌려준다.
    fn get_doc_ids_by_path_prefix(&self, path_prefix: &str) -> Result<HashSet<i64>, EsError>;
    /// 청크 단위로 지운다. 중간 청크가 실패하면 앞 청크는 이미 지워진 채 오류를 돌려준다.
    fn delete_by_ids(&self, ids: &[i64]) -> Result<i64, EsError>;
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

impl EsClient {
    fn mget(&self, ids: &[i64], source: &[&str]) -> Result<Vec<Value>, EsError> {
        let response = self.send(&EsRequest::MultiGet { docs: mget_docs(&self.index, ids), source: Some(source.iter().map(|s| (*s).to_owned()).collect()) })?;
        Ok(response["docs"].as_array().cloned().unwrap_or_default())
    }

    fn mapping_properties(&self) -> Result<Value, EsError> {
        let mapping = self.send(&EsRequest::GetMapping { index: None })?;
        Ok(mapping[&self.index]["mappings"]["properties"].clone())
    }

    fn put_properties(&self, properties: Value) -> Result<(), EsError> {
        self.send(&EsRequest::PutMapping { index: None, properties }).map(|_| ())
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
        let response = self.send(&EsRequest::DeleteByQuery { index: None, body: json!({"query": query}), conflicts: Some(Conflicts::Proceed), refresh: Some(false) })?;
        Ok(response["deleted"].as_i64().unwrap_or(0))
    }
}

impl IndexStore for EsClient {
    fn index_name(&self) -> &str {
        &self.index
    }

    fn ping(&self) -> Result<(), EsError> {
        self.send(&EsRequest::Info).map(|_| ())
    }

    fn index_exists(&self) -> Result<bool, EsError> {
        Ok(self.send(&EsRequest::IndexExists { index: None })?.as_bool() == Some(true))
    }

    fn create_index(&self) -> Result<(), EsError> {
        if self.index_exists()? {
            self.ensure_existing_index_mappings();
            return Ok(());
        }
        match self.send(&EsRequest::CreateIndex { index: None, body: json!({"settings": index_settings(), "mappings": index_mappings()}) }) {
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
            self.send(&EsRequest::DeleteIndex { index: None })?;
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

    fn delete_by_file_paths(&self, file_paths: &[String], exclude_ids: &[i64]) -> Result<i64, EsError> {
        if file_paths.is_empty() {
            return Ok(0);
        }
        let mut query = json!({"bool": {"must": [{"terms": {"file_path": file_paths}}]}});
        if !exclude_ids.is_empty() {
            query["bool"]["must_not"] = json!([{"ids": {"values": exclude_ids.iter().map(i64::to_string).collect::<Vec<_>>()}}]);
        }
        let deleted = self.delete_by_query(query)?;
        if deleted > 0 {
            eprintln!("INFO delete_by_file_paths: {deleted} docs deleted for {} paths", file_paths.len());
        }
        Ok(deleted)
    }

    fn get_doc_ids_by_path_prefix(&self, path_prefix: &str) -> Result<HashSet<i64>, EsError> {
        let mut ids = HashSet::new();
        let mut scroll_id: Option<String> = None;
        let outcome = (|| -> Result<(), EsError> {
            let body = json!({"query": path_prefix_query(path_prefix), "size": SCROLL_PAGE_SIZE, "_source": false});
            let mut response = self.send(&EsRequest::scroll_search(body, "10m"))?;
            scroll_id = response["_scroll_id"].as_str().map(str::to_owned);
            loop {
                let hits = response["hits"]["hits"].as_array().cloned().unwrap_or_default();
                if hits.is_empty() {
                    return Ok(());
                }
                ids.extend(hits.iter().filter_map(|h| h["_id"].as_str().and_then(|s| s.parse::<i64>().ok())));
                response = self.send(&EsRequest::Scroll { scroll_id: scroll_id.clone().unwrap_or_default(), scroll: "10m".to_string() })?;
                scroll_id = response["_scroll_id"].as_str().map(str::to_owned).or(scroll_id.take());
            }
        })();
        if let Some(id) = scroll_id {
            let _ = self.send(&EsRequest::ClearScroll { scroll_id: id });
        }
        outcome.map(|()| ids)
    }

    fn delete_by_ids(&self, ids: &[i64]) -> Result<i64, EsError> {
        self.delete_by_ids_chunked(ids, DELETE_CHUNK_SIZE)
    }

    fn insert(&self, docs: &[(i64, Value)]) -> Result<(), EsError> {
        self.insert_with(docs, &|secs| std::thread::sleep(Duration::from_secs(secs)))
    }

    fn refresh(&self) -> Result<(), EsError> {
        self.send(&EsRequest::RefreshIndex { index: None }).map(|_| ())
    }
}

impl EsClient {
    /// `ids`를 `chunk_size`개씩 나눠 지운다. 중간 묶음이 실패하면 앞 묶음은 이미 지워진 채 오류를 돌려준다.
    pub fn delete_by_ids_chunked(&self, ids: &[i64], chunk_size: usize) -> Result<i64, EsError> {
        let mut deleted = 0;
        for chunk in ids.chunks(chunk_size.max(1)) {
            deleted += self.delete_by_query(json!({"ids": {"values": chunk.iter().map(i64::to_string).collect::<Vec<_>>()}}))?;
        }
        if deleted > 0 {
            eprintln!("INFO delete_by_ids: {deleted} docs deleted for {} ids", ids.len());
        }
        Ok(deleted)
    }

    /// 문서를 100건씩 bulk로 넣는다. 연결·응답 해석 오류([`EsError::is_connection`])는 `1, 2, 4`초 쉬며 3번까지 다시 시도한다.
    /// 문서마다 `_id`를 정해 넣으므로 다시 보내도 중복되지 않는다. `sleep`은 쉬는 방법이다.
    pub fn insert_with(&self, docs: &[(i64, Value)], sleep: &dyn Fn(u64)) -> Result<(), EsError> {
        for chunk in docs.chunks(BULK_CHUNK_SIZE) {
            let mut lines = Vec::with_capacity(chunk.len() * 2);
            for (id, doc) in chunk {
                lines.push(json!({"index": {"_index": self.index, "_id": id.to_string()}}));
                lines.push(doc.clone());
            }
            eprintln!("INFO {} items inserted", chunk.len());
            let request = EsRequest::Bulk { lines, timeout: Some("60s".to_string()), refresh: Some(false) };
            for attempt in 0..BULK_MAX_RETRIES {
                match self.send(&request) {
                    Ok(response) => {
                        // 항목별 실패는 요청 전체를 실패로 보지 않는다. 조용히 사라지지 않도록 경고를 남긴다.
                        if response["errors"].as_bool().unwrap_or(false) {
                            let failed = response["items"].as_array().map_or(0, |items| items.iter().filter(|i| i["index"]["error"].is_object()).count());
                            eprintln!("WARNING ES bulk 응답에 항목 오류가 있다: {failed}건");
                        }
                        break;
                    }
                    Err(e) if e.is_connection() && attempt + 1 < BULK_MAX_RETRIES => {
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
