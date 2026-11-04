//! 문서 삭제·삽입·수정과 카테고리 이름 바꾸기·삭제.

use super::*;

impl ESManager {
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
    pub fn path_prefix_query(path_prefix: &str) -> Value {
        let p = path_prefix.trim_matches('/');
        if p.is_empty() || p == "." {
            return json!({"match_all": {}});
        }
        json!({"bool": {"should": [{"term": {"file_path": p}}, {"prefix": {"file_path": format!("{p}/")}}], "minimum_should_match": 1}})
    }

    /// 경로 prefix 하위 문서의 `_id`(inode) 집합. scroll로 읽고 `_source`는 받지 않는다. 오류는 삼키고 읽은 만큼 돌려준다.
    pub fn get_doc_ids_by_path_prefix(&self, path_prefix: &str) -> HashSet<i64> {
        let query = Self::path_prefix_query(path_prefix);
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

    pub(super) fn clear_scroll_quietly(&self, scroll_id: Option<String>) {
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

    /// 여러 카테고리의 문서 수를 msearch로 한 번에 조회한다. 카테고리가 없으면 호출 없이 `{}`다.
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
}
