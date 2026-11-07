//! 파일 시각으로 `created_time`을 채운다.

use super::*;

impl ESManager {
    /// 경로를 `Path.resolve(strict=False)`처럼 정규화한다. 없는 꼬리는 `..`와 `.`만 문자열로 처리한다.
    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        let mut missing = io::Error::from(io::ErrorKind::NotFound);
        for ancestor in path.ancestors() {
            match self.fs.canonicalize(ancestor) {
                Ok(mut resolved) => {
                    let tail = path.strip_prefix(ancestor).unwrap_or(Path::new(""));
                    for component in tail.components() {
                        match component {
                            Component::ParentDir => {
                                resolved.pop();
                            }
                            Component::Normal(name) => resolved.push(name),
                            _ => {}
                        }
                    }
                    return Ok(resolved);
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => missing = e,
                Err(e) => return Err(e),
            }
        }
        Err(missing)
    }

    /// 파일 생성 시각(ISO 8601)과 출처. 리눅스 statx의 btime이 있으면 `statx_btime`, 없으면 `st_ctime`이다.
    fn created_time_with_source(&self, path: &Path) -> io::Result<(String, &'static str)> {
        let metadata = self.fs.metadata(path)?;
        let (time, source) = match metadata.created() {
            Ok(created) => (DateTime::<Local>::from(created), "statx_btime"),
            Err(_) => {
                (DateTime::from_timestamp(metadata.ctime(), metadata.ctime_nsec() as u32).map(|utc| utc.with_timezone(&Local)).ok_or_else(|| io::Error::other("ctime을 해석할 수 없다"))?, "st_ctime")
            }
        };
        let local = time.naive_local();
        // ISO 8601: 마이크로초가 0이면 소수부를 생략한다.
        let micros = local.and_utc().timestamp_subsec_micros();
        let iso = if micros == 0 { local.format("%Y-%m-%dT%H:%M:%S").to_string() } else { local.format("%Y-%m-%dT%H:%M:%S%.6f").to_string() };
        Ok((iso, source))
    }

    /// created_time이 없는 기존 ES 문서를 실제 파일 stat 기준으로 채운다.
    pub fn backfill_created_time(&self, path_prefix: &Path, batch_size: usize) -> BackfillReport {
        log::info!("backfill_created_time(index={}) start", self.index());
        let (mut updated, mut skipped, mut failed) = (0i64, 0i64, 0i64);
        let query = json!({"bool": {"should": [{"bool": {"must_not": [{"exists": {"field": "created_time"}}]}}, {"bool": {"must_not": [{"exists": {"field": "created_time_source"}}]}}], "minimum_should_match": 1}});
        let resolved_prefix = self.resolve(path_prefix).unwrap_or_else(|_| path_prefix.to_path_buf());

        let resolve_path = |stored: &str| -> Option<PathBuf> {
            if stored.is_empty() {
                return None;
            }
            let candidate = Path::new(stored);
            let full = if candidate.is_absolute() { candidate.to_path_buf() } else { path_prefix.join(stored) };
            self.resolve(&full).ok().filter(|resolved| resolved.starts_with(&resolved_prefix))
        };

        let mut update_hits = |hits: &[Value]| {
            let mut bulk_body: Vec<Value> = Vec::new();
            let mut pending = 0i64;
            for hit in hits {
                let stored = hit["_source"]["file_path"].as_str().unwrap_or("");
                let Some(file_path) = resolve_path(stored).filter(|p| self.fs.is_file(p).unwrap_or(false)) else {
                    skipped += 1;
                    continue;
                };
                let Ok((created_time, source)) = self.created_time_with_source(&file_path) else {
                    failed += 1;
                    continue;
                };
                bulk_body.push(json!({"update": {"_index": self.index(), "_id": hit["_id"]}}));
                bulk_body.push(json!({"doc": {"created_time": created_time, "created_time_source": source}}));
                pending += 1;
            }
            if bulk_body.is_empty() {
                return;
            }
            match self.es.send(&EsRequest::Bulk { lines: bulk_body.clone(), timeout: Some("60s".to_string()), refresh: Some(false) }) {
                Err(e) => {
                    log::error!("backfill_created_time bulk error: {e}");
                    failed += pending;
                }
                Ok(bulk_result) if bulk_result["errors"].as_bool().unwrap_or(false) => {
                    let item_failures = bulk_result["items"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().filter(|item| has_content(item["update"].get("error"))).count() as i64;
                    failed += item_failures;
                    updated += pending - item_failures;
                }
                Ok(_) => updated += pending,
            }
        };

        let mut scroll_id: Option<String> = None;
        let scan = (|| -> Result<(), EsError> {
            let mut response = self.es.send(&EsRequest::scroll_search(json!({"query": query, "size": batch_size, "_source": ["file_path"]}), "10m"))?;
            scroll_id = response["_scroll_id"].as_str().map(str::to_string);
            while !hits_of(&response).is_empty() {
                update_hits(hits_of(&response));
                response = self.es.send(&EsRequest::Scroll { scroll_id: scroll_id.clone().unwrap_or_default(), scroll: "10m".to_string() })?;
                scroll_id = response["_scroll_id"].as_str().map(str::to_string);
            }
            Ok(())
        })();
        if let Err(e) = scan {
            log::error!("backfill_created_time error: {e}");
            failed += 1;
        }
        self.clear_scroll_quietly(scroll_id);
        if updated > 0
            && let Err(e) = self.refresh()
        {
            log::warn!("backfill_created_time refresh error: {e}");
        }
        let result = BackfillReport { updated, skipped, failed };
        log::info!("backfill_created_time(index={}) done: {result:?}", self.index());
        result
    }
}
