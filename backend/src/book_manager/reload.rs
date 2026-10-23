//! 카테고리 재적재. Python `_bulk_index_files`, `_reload_category_mismatch_details`, `reload_category_mismatches`,
//! `reload_category_mismatch_files`, `_reload_root_files`, `reload_category`에 대응한다.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use sysdeps::CommandError;

use super::{BookManager, HARD_PARSE_TIMEOUT_SECONDS, ProgressFn};
use crate::es_manager::Docs;

/// `reload_category`의 loader 실행 시간 상한.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(600);

fn count_of(base: Option<&Value>, key: &str) -> i64 {
    base.and_then(|b| b.get(key)).and_then(Value::as_i64).unwrap_or(0)
}

/// 진행 보고에 쓸 누적 건수 객체. 이전 카테고리까지 확정된 값(`base`)에 덮어쓴다.
fn progress_payload(base: Option<&Value>, indexed: i64, deleted: Option<i64>) -> Value {
    let mut payload: Map<String, Value> = base.and_then(Value::as_object).cloned().unwrap_or_default();
    payload.insert("indexed_count".into(), json!(indexed));
    if let Some(deleted) = deleted {
        payload.insert("deleted_count".into(), json!(deleted));
    }
    Value::Object(payload)
}

impl BookManager {
    /// 파일 하나를 파싱한다. 파싱은 별도 스레드에서 돌리고 `HARD_PARSE_TIMEOUT_SECONDS`를 넘기면 포기한다
    /// (파서가 멈춰도 재적재 전체가 서지 않게 한다. 스레드는 죽일 수 없어 끝날 때까지 남는다).
    fn read_with_hard_timeout(&self, abs_path: PathBuf) -> Result<Docs, ()> {
        let reader = self.reader();
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("book-parse".to_string()).spawn(move || {
            let _ = tx.send(reader.read_file(&abs_path, None));
        });
        if spawned.is_err() {
            return Err(());
        }
        match rx.recv_timeout(Duration::from_secs(HARD_PARSE_TIMEOUT_SECONDS)) {
            Ok(docs) => Ok(docs),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(()),
            // 파서가 panic했다. 읽지 못한 파일로 본다.
            Err(mpsc::RecvTimeoutError::Disconnected) => Ok(Docs::new()),
        }
    }

    /// 여러 파일을 배치로 파싱해 ES에 한 번에 색인한다. 반환: (file_path -> 새 book_id, 실패 목록).
    pub async fn _bulk_index_files(&self, file_paths: &[&str], clean_existing: bool) -> (BTreeMap<String, i64>, Vec<Value>) {
        self._bulk_index_files_with(file_paths, clean_existing, None, None).await
    }

    /// `_bulk_index_files`에 진행 보고를 붙인 판. `progress_base`는 이 배치 이전까지 확정된 누적 카운트다.
    pub async fn _bulk_index_files_with(&self, file_paths: &[&str], clean_existing: bool, on_progress: Option<ProgressFn<'_>>, progress_base: Option<&Value>) -> (BTreeMap<String, i64>, Vec<Value>) {
        let mut merged = Docs::new();
        let mut id_by_path: Vec<(String, i64)> = Vec::new();
        let mut failures: Vec<Value> = Vec::new();
        let base_indexed = count_of(progress_base, "indexed_count");

        for file_path in file_paths {
            // 파일 1건 파싱이 가장 오래 걸릴 수 있어, 파일마다 진행률을 보고한다.
            if let Some(report) = on_progress {
                report(progress_payload(progress_base, base_indexed + id_by_path.len() as i64, None));
            }
            let abs_path = match self.resolve_path(&self.path_prefix.join(file_path)) {
                Ok(p) if matches!(self.is_inside_root(&p), Ok(true)) && self.fs.is_file(&p).unwrap_or(false) => p,
                _ => {
                    failures.push(json!({"file_path": file_path, "error": format!("파일을 찾을 수 없습니다: {file_path}")}));
                    continue;
                }
            };
            let data = match self.read_with_hard_timeout(abs_path) {
                Ok(data) => data,
                Err(()) => {
                    log::error!("파일 파싱 타임아웃: {file_path}");
                    failures.push(json!({"file_path": file_path, "error": format!("파싱 시간 초과: {file_path}")}));
                    continue;
                }
            };
            if data.len() != 1 {
                failures.push(json!({"file_path": file_path, "error": format!("지원하지 않는 파일 형식입니다: {file_path}")}));
                continue;
            }
            if let Some((book_id, doc)) = data.into_iter().next() {
                merged.insert(book_id, doc);
                id_by_path.push((file_path.to_string(), book_id));
            }
        }

        if merged.is_empty() {
            return (BTreeMap::new(), failures);
        }

        if clean_existing {
            let stale_paths: Vec<String> = merged.values().filter_map(|doc| doc.get("file_path").and_then(Value::as_str).map(str::to_string)).collect();
            let exclude: Vec<i64> = merged.keys().copied().collect();
            if let Err(e) = self.es.delete_by_file_paths(&stale_paths, Some(&exclude)) {
                log::warn!("_bulk_index_files: 기존 문서 정리 실패: {e}");
            }
        }

        let indexed_ids: Vec<i64> = match self.es.insert(&merged) {
            Ok(ids) => ids,
            Err(e) => {
                log::error!("_bulk_index_files: ES 적재 실패: {e}");
                Vec::new()
            }
        };
        let mut indexed = BTreeMap::new();
        for (file_path, book_id) in id_by_path {
            if indexed_ids.contains(&book_id) {
                indexed.insert(file_path, book_id);
            } else {
                failures.push(json!({"file_path": file_path, "error": "ES 적재 실패"}));
            }
        }
        (indexed, failures)
    }

    pub async fn _reload_category_mismatch_details(&self, category: &str, details: &Value) -> Value {
        self._reload_category_mismatch_details_with(category, details, None, None).await
    }

    /// 카테고리 하나의 불일치(`fs_only`, `duplicates`, `es_only`)를 재적재·정리한다.
    /// `progress_base`는 이전 카테고리까지 확정된 누적 카운트로, 보고에는 항상 이 값을 더한 전역 누적치를 싣는다.
    pub async fn _reload_category_mismatch_details_with(&self, category: &str, details: &Value, on_progress: Option<ProgressFn<'_>>, progress_base: Option<&Value>) -> Value {
        let mut indexed_count = 0i64;
        let mut deleted_count = 0i64;
        let mut failures: Vec<Value> = Vec::new();
        let mut ids_to_delete: BTreeSet<i64> = BTreeSet::new();
        let mut running_indexed = count_of(progress_base, "indexed_count");
        let mut running_deleted = count_of(progress_base, "deleted_count");
        let list = |key: &str| -> Vec<Value> { details.get(key).and_then(Value::as_array).cloned().unwrap_or_default() };

        let mut fs_only_paths: Vec<String> = Vec::new();
        for item in list("fs_only") {
            match item.get("file_path").and_then(Value::as_str).filter(|p| !p.is_empty()) {
                Some(path) => fs_only_paths.push(path.to_string()),
                None => failures.push(json!({"file_path": item.get("file_path").cloned().unwrap_or(Value::Null), "error": "file_path가 비어있습니다"})),
            }
        }

        let linked_ids_of = |item: &Value| -> BTreeSet<i64> {
            item.get("docs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|d| d.get("file_linked").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|d| d.get("book_id").and_then(Value::as_i64))
                .collect()
        };
        let duplicate_items = list("duplicates");
        let mut dup_reindex_paths: Vec<String> = Vec::new();
        for item in &duplicate_items {
            let file_exists = item.get("file_exists").and_then(Value::as_bool).unwrap_or(false);
            if file_exists && linked_ids_of(item).is_empty() {
                match item.get("file_path").and_then(Value::as_str).filter(|p| !p.is_empty()) {
                    Some(path) => dup_reindex_paths.push(path.to_string()),
                    None => failures.push(json!({"file_path": item.get("file_path").cloned().unwrap_or(Value::Null), "error": "file_path가 비어있습니다"})),
                }
            }
        }

        let mut indexed_book_id_by_path: HashMap<String, i64> = HashMap::new();
        for (paths, clean_existing) in [(&fs_only_paths, true), (&dup_reindex_paths, false)] {
            for batch in paths.chunks(self.bulk_reindex_batch_size) {
                let batch_refs: Vec<&str> = batch.iter().map(String::as_str).collect();
                let base = json!({"indexed_count": running_indexed, "deleted_count": running_deleted});
                let (indexed, batch_failures) = self._bulk_index_files_with(&batch_refs, clean_existing, on_progress, Some(&base)).await;
                indexed_count += indexed.len() as i64;
                indexed_book_id_by_path.extend(indexed.iter().map(|(k, v)| (k.clone(), *v)));
                failures.extend(batch_failures);
                // 배치 도중 보고한 잠정값을 실제 색인 건수로 정정한다.
                running_indexed += indexed.len() as i64;
                if let Some(report) = on_progress {
                    report(json!({"indexed_count": running_indexed, "deleted_count": running_deleted}));
                }
            }
        }

        for item in &duplicate_items {
            let file_exists = item.get("file_exists").and_then(Value::as_bool).unwrap_or(false);
            let mut linked = linked_ids_of(item);
            if file_exists && linked.is_empty() {
                let Some(new_book_id) = item.get("file_path").and_then(Value::as_str).and_then(|p| indexed_book_id_by_path.get(p)) else {
                    continue; // 색인 실패 - failures에 이미 기록됨
                };
                linked.insert(*new_book_id);
            }
            for doc in item.get("docs").and_then(Value::as_array).into_iter().flatten() {
                let Some(book_id) = doc.get("book_id").and_then(Value::as_i64) else { continue };
                if file_exists && linked.contains(&book_id) {
                    continue;
                }
                ids_to_delete.insert(book_id);
            }
        }

        for item in list("es_only") {
            if let Some(book_id) = item.get("book_id").and_then(Value::as_i64) {
                ids_to_delete.insert(book_id);
            }
        }

        if !ids_to_delete.is_empty() {
            let ids: Vec<i64> = ids_to_delete.iter().copied().collect();
            let deleted = match self.es.delete_by_ids(&ids, 10000) {
                Ok(n) => n,
                Err(e) => {
                    log::error!("_reload_category_mismatch_details: ES 삭제 실패: {e}");
                    0
                }
            };
            deleted_count += deleted;
            running_deleted += deleted;
            if let Some(report) = on_progress {
                report(json!({"indexed_count": running_indexed, "deleted_count": running_deleted}));
            }
            if deleted < ids.len() as i64 {
                failures.push(json!({"category": category, "error": format!("ES 문서 {}건 중 {}건만 삭제됨", ids.len(), deleted)}));
            }
        }

        json!({"category": category, "indexed_count": indexed_count, "deleted_count": deleted_count, "failures": failures})
    }

    /// 현재 카테고리 불일치 항목을 파일 단위로 ES에 재적재·정리한다.
    pub async fn reload_category_mismatches(&self, on_progress: Option<ProgressFn<'_>>) -> (Value, Option<String>) {
        self.reload_category_mismatches_for("book", on_progress).await
    }

    pub async fn reload_category_mismatches_for(&self, content_type: &str, on_progress: Option<ProgressFn<'_>>) -> (Value, Option<String>) {
        log::info!("reload_category_mismatches 시작: content_type='{content_type}'");
        let before = match self.try_get_category_mismatches(true) {
            Ok(v) => v,
            Err(e) => return (json!({}), Some(format!("불일치 조회 실패: {e}"))),
        };
        let categories = self._mismatch_categories(&before);
        let before_count = self._mismatch_item_count(&before);
        let (mut indexed_total, mut deleted_total) = (0i64, 0i64);
        let mut failures: Vec<Value> = Vec::new();
        let mut category_results: Vec<Value> = Vec::new();
        if let Some(report) = on_progress {
            report(json!({"before_count": before_count}));
        }

        for category in &categories {
            if !self.is_safe_category_name(category) {
                category_results.push(json!({"category": category, "indexed_count": 0, "deleted_count": 0, "failures": [{"category": category, "error": "잘못된 카테고리 경로입니다"}]}));
                continue;
            }
            let details = self.get_category_mismatch_details(category);
            if let Some(error) = details.get("error").and_then(Value::as_str) {
                category_results.push(json!({"category": category, "indexed_count": 0, "deleted_count": 0, "failures": [{"category": category, "error": format!("불일치 상세 조회 실패: {error}")}]}));
                continue;
            }
            let base = json!({"indexed_count": indexed_total, "deleted_count": deleted_total});
            let category_result = self._reload_category_mismatch_details_with(category, &details, on_progress, Some(&base)).await;
            indexed_total += category_result["indexed_count"].as_i64().unwrap_or(0);
            deleted_total += category_result["deleted_count"].as_i64().unwrap_or(0);
            category_results.push(category_result);
            if let Some(report) = on_progress {
                report(json!({"indexed_count": indexed_total, "deleted_count": deleted_total}));
            }
        }

        if let Err(e) = self.es.refresh() {
            failures.push(json!({"error": format!("ES refresh 실패: {e}")}));
        }

        let after_count = self.try_get_category_mismatches(true).map(|v| self._mismatch_item_count(&v)).unwrap_or(0);
        let mut failed_count = failures.len() as i64;
        for category_result in &category_results {
            let list = category_result["failures"].as_array().cloned().unwrap_or_default();
            failed_count += list.len() as i64;
            failures.extend(list);
        }
        log::info!(
            "reload_category_mismatches 완료: content_type='{content_type}', categories={}, indexed={indexed_total}, deleted={deleted_total}, failed={failed_count}, before={before_count}, after={after_count}",
            categories.len()
        );
        (
            json!({
                "content_type": content_type, "category_count": categories.len(), "before_count": before_count, "after_count": after_count,
                "indexed_count": indexed_total, "deleted_count": deleted_total, "failed_count": failed_count, "failures": failures, "categories": category_results,
            }),
            None,
        )
    }

    /// 선택 카테고리의 현재 불일치 항목만 파일 단위로 ES에 재적재·정리한다.
    pub async fn reload_category_mismatch_files(&self, category: &str, on_progress: Option<ProgressFn<'_>>) -> (Value, Option<String>) {
        self.reload_category_mismatch_files_for(category, "book", on_progress).await
    }

    pub async fn reload_category_mismatch_files_for(&self, category: &str, content_type: &str, on_progress: Option<ProgressFn<'_>>) -> (Value, Option<String>) {
        log::info!("reload_category_mismatch_files 시작: category='{category}', content_type='{content_type}'");
        if category.is_empty() {
            return (json!({}), Some("카테고리 이름이 비어있습니다".to_string()));
        }
        if !self.is_safe_category_name(category) {
            return (json!({}), Some("잘못된 카테고리 경로입니다".to_string()));
        }

        self.clear_mismatch_cache();
        let before_details = self.get_category_mismatch_details(category);
        if let Some(error) = before_details.get("error").and_then(Value::as_str) {
            return (json!({}), Some(format!("불일치 상세 조회 실패: {error}")));
        }
        let before_count = Self::mismatch_detail_item_count(&before_details);
        if let Some(report) = on_progress {
            report(json!({"before_count": before_count}));
        }
        let category_result = self._reload_category_mismatch_details_with(category, &before_details, on_progress, None).await;
        let mut failures: Vec<Value> = Vec::new();
        if let Err(e) = self.es.refresh() {
            failures.push(json!({"error": format!("ES refresh 실패: {e}")}));
        }

        self.clear_mismatch_cache();
        let after_count = Self::mismatch_detail_item_count(&self.get_category_mismatch_details(category));
        failures.extend(category_result["failures"].as_array().cloned().unwrap_or_default());
        let failed_count = failures.len();
        (
            json!({
                "content_type": content_type, "category": category, "category_count": 1, "before_count": before_count, "after_count": after_count,
                "indexed_count": category_result["indexed_count"], "deleted_count": category_result["deleted_count"],
                "failed_count": failed_count, "failures": failures, "categories": [category_result],
            }),
            None,
        )
    }

    /// `_root`는 실제 디렉터리가 아니라 최상위 파일 묶음이라, loader 대신 최상위 파일만 직접 색인한다.
    async fn reload_root_files(&self, content_type: &str) -> (Value, Option<String>) {
        log::info!("_reload_root_files 시작: content_type='{content_type}'");
        let entries = match self.fs.read_dir(&self.path_prefix) {
            Ok(entries) => entries,
            Err(e) => {
                log::error!("_reload_root_files: 최상위 디렉토리 스캔 실패 - {e}");
                return (json!({}), Some(format!("최상위 디렉토리를 읽을 수 없습니다: {e}")));
            }
        };
        let mut file_names: Vec<String> =
            entries.iter().filter(|p| Self::is_regular_file_nofollow(p) && self.is_indexable_file_path(p)).map(|p| Self::file_name_of(p).to_string_lossy().into_owned()).collect();
        file_names.sort();

        let mut processed = 0usize;
        let mut failures: Vec<Value> = Vec::new();
        for batch in file_names.chunks(self.bulk_reindex_batch_size) {
            let refs: Vec<&str> = batch.iter().map(String::as_str).collect();
            let (indexed, batch_failures) = self._bulk_index_files(&refs, true).await;
            processed += indexed.len();
            failures.extend(batch_failures);
        }
        if let Err(e) = self.es.refresh() {
            failures.push(json!({"error": format!("ES refresh 실패: {e}")}));
        }

        self.clear_mismatch_cache();
        if let Some(first) = failures.first() {
            log::error!("_reload_root_files 실패: processed={processed}, failed={} - {first}", failures.len());
            let reason = first.get("error").and_then(Value::as_str).unwrap_or("None");
            return (json!({}), Some(format!("최상위 파일 재적재 실패 {}건: {reason}", failures.len())));
        }
        log::info!("_reload_root_files 완료: processed={processed}");
        (json!({"category": "_root", "processed_count": processed}), None)
    }

    /// 카테고리 전체를 ES에 재적재한다(loader를 `--recursive --reload`로 실행). `content_type`은 `"comic"`이면 만화 색인을 쓴다.
    pub async fn reload_category(&self, category: &str, content_type: Option<&str>) -> (Value, Option<String>) {
        let content_type = content_type.unwrap_or("book");
        log::info!("reload_category 시작: category='{category}', content_type='{content_type}'");
        if category.is_empty() {
            return (json!({}), Some("카테고리 이름이 비어있습니다".to_string()));
        }
        if category.contains("..") {
            return (json!({}), Some("카테고리 이름에 '..'는 사용할 수 없습니다".to_string()));
        }
        if category == Self::ROOT_CATEGORY {
            return self.reload_root_files(content_type).await;
        }

        let abs_dir = match self.resolve_path(&self.path_prefix.join(category)) {
            Ok(p) if matches!(self.is_inside_root(&p), Ok(true)) => p,
            _ => return (json!({}), Some(format!("잘못된 경로입니다: {category}"))),
        };
        if !abs_dir.is_dir() {
            log::error!("reload_category: 디렉토리 없음 - {}", abs_dir.display());
            return (json!({}), Some(format!("디렉토리를 찾을 수 없습니다: {category}")));
        }

        let index_name = if content_type == "comic" { "comics" } else { "book" };
        log::info!("reload_category: loader 실행 - index={index_name}, path={}", abs_dir.display());
        let argv = vec![self.loader_program.clone(), "--recursive".to_string(), "--reload".to_string(), index_name.to_string(), abs_dir.to_string_lossy().into_owned()];
        let output = match self.runner.run(&argv, Some(RELOAD_TIMEOUT)) {
            Ok(output) => output,
            Err(CommandError::Timeout) => {
                log::error!("reload_category: 타임아웃 (10분 초과) - category='{category}'");
                return (json!({}), Some("재적재 시간이 초과되었습니다 (10분)".to_string()));
            }
            Err(e) => {
                log::error!("reload_category: subprocess 실행 실패 - {e}");
                return (json!({}), Some(format!("재적재 실행 실패: {e}")));
            }
        };
        let stdout_text = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if output.returncode != 0 {
            log::error!("reload_category: 실패 (exit {}) - stderr: {stderr_text}", output.returncode);
            return (json!({}), Some(format!("재적재 실패 (exit {}): {stderr_text}", output.returncode)));
        }
        if !stderr_text.is_empty() {
            log::warn!("reload_category: stderr 출력 - {stderr_text}");
        }

        let processed = regex::Regex::new(r"총\s+(\d+)개\s+파일\s+처리됨").ok().and_then(|re| re.captures(&stdout_text)).and_then(|c| c[1].parse::<i64>().ok()).unwrap_or(0);
        log::info!("reload_category 완료: category='{category}', processed={processed}");
        self.clear_mismatch_cache();
        (json!({"category": category, "processed_count": processed}), None)
    }
}
