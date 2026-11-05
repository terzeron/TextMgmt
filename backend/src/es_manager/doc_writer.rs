//! 문서 삭제·삽입·수정과 카테고리 이름 바꾸기·삭제.

use super::*;
use es_client::indexing::IndexStore;

impl ESManager {
    /// 주어진 file_path 목록의 기존 문서를 삭제한다(중복 방지용). `exclude_ids`는 지우지 않는다.
    pub fn delete_by_file_paths(&self, file_paths: &[String], exclude_ids: Option<&[i64]>) -> Result<i64, EsError> {
        self.index_store().delete_by_file_paths(file_paths, exclude_ids.unwrap_or_default())
    }

    pub(super) fn clear_scroll_quietly(&self, scroll_id: Option<String>) {
        if let Some(id) = scroll_id
            && let Err(e) = self.es.send(&EsRequest::ClearScroll { scroll_id: id })
        {
            log::debug!("Failed to clear scroll: {e}");
        }
    }

    /// 주어진 `_id`(inode) 목록의 문서를 `chunk_size`개씩 삭제한다. 중간 묶음이 실패하면 앞 묶음은 지워진 채 오류를 돌려준다.
    pub fn delete_by_ids(&self, ids: &[i64], chunk_size: usize) -> Result<i64, EsError> {
        self.index_store().delete_by_ids_chunked(ids, chunk_size)
    }

    // ---- 쓰기 ----

    /// 문서를 100건씩 bulk로 넣고 넣은 문서 id를 돌려준다. 재시도 규칙은 `EsClient::insert_with`와 같고, 쉬는 시간은 `Clock`으로 잰다.
    pub fn insert(&self, data: &[(i64, Value)]) -> Result<Vec<i64>, EsError> {
        log::debug!("insert() {} items", data.len());
        self.index_store().insert_with(data, &|secs| self.clock.sleep(secs as f64))?;
        Ok(data.iter().map(|(id, _)| *id).collect())
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
        let result = self.es.send(&EsRequest::Update { index: None, id: doc_id.to_string(), body: json!({"doc": fields.to_doc()}), refresh: Some(true) })?;
        Ok(result["_shards"]["failed"].as_i64().is_none_or(|failed| failed <= 0))
    }

    pub fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        log::debug!("delete(doc_id={doc_id})");
        let result = self.es.send(&EsRequest::Delete { index: None, id: doc_id.to_string(), refresh: Some(true) })?;
        Ok(result["result"] == "deleted")
    }

    // ---- 카테고리 ----

    fn category_query(category: &str, prefix: bool) -> Value {
        if prefix { category_or_children(category) } else { json!({"term": {"category": category}}) }
    }

    /// 특정 카테고리의 문서 수. `prefix`이면 하위 카테고리(`category/*`)도 센다.
    pub fn count_by_category(&self, category: &str, prefix: bool) -> Result<i64, EsError> {
        log::debug!("count_by_category(category='{category}', prefix={prefix})");
        let result = self.es.send(&EsRequest::Count { index: None, body: json!({"query": Self::category_query(category, prefix)}) })?;
        result["count"].as_i64().ok_or_else(|| parse_error("count가 없다"))
    }

    /// 여러 카테고리의 문서 수를 msearch로 한 번에 조회한다. 카테고리가 없으면 호출 없이 빈 결과다.
    pub fn count_by_categories(&self, categories: &[&str], prefix: bool) -> Result<BTreeMap<String, i64>, EsError> {
        if categories.is_empty() {
            return Ok(BTreeMap::new());
        }
        if let [category] = categories {
            return Ok(BTreeMap::from([(category.to_string(), self.count_by_category(category, prefix)?)]));
        }
        log::debug!("count_by_categories(categories={categories:?}, prefix={prefix})");
        let mut searches = Vec::with_capacity(categories.len() * 2);
        for category in categories {
            searches.push(json!({"index": self.index()}));
            searches.push(json!({"size": 0, "track_total_hits": true, "query": Self::category_query(category, prefix)}));
        }
        let response = self.es.send(&EsRequest::MultiSearch { index: None, searches })?;
        let responses = response["responses"].as_array().ok_or_else(|| parse_error("msearch responses가 없다"))?;
        let mut result = BTreeMap::new();
        for (category, item) in categories.iter().zip(responses) {
            let count = if item.get("error").is_some() {
                log::error!("msearch error for category '{category}': {}", item["error"]);
                0
            } else {
                total_of(item)?
            };
            result.insert(category.to_string(), count);
        }
        Ok(result)
    }

    /// 특정 카테고리('A'와 'A/*')의 모든 문서를 새 카테고리로 바꾸고 file_path의 카테고리 prefix도 갱신한다.
    pub fn rename_category(&self, old_category: &str, new_category: &str) -> Result<UpdateByQueryResult, EsError> {
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
        let result = self.es.send(&EsRequest::UpdateByQuery { index: None, body: json!({"query": query, "script": script}), conflicts: Some(Conflicts::Abort), refresh: Some(true) })?;
        serde_json::from_value(result).map_err(|e| parse_error(format!("update_by_query 응답: {e}")))
    }

    /// 특정 카테고리의 모든 문서를 삭제한다. `prefix`이면 하위 카테고리도 지운다.
    pub fn delete_by_category(&self, category: &str, prefix: bool) -> Result<DeleteByQueryResult, EsError> {
        log::debug!("delete_by_category(category='{category}', prefix={prefix})");
        let result = self.es.send(&EsRequest::DeleteByQuery { index: None, body: json!({"query": Self::category_query(category, prefix)}), conflicts: Some(Conflicts::Abort), refresh: Some(true) })?;
        serde_json::from_value(result).map_err(|e| parse_error(format!("delete_by_query 응답: {e}")))
    }
}
