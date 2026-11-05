//! 카테고리 재적재: 파일 일괄 색인과 불일치 상세 다시 읽기.
//! `reload_category_mismatch_files`, `reload_book_root_files`, `reload_category`에 대응한다.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sysdeps::CommandError;

use super::{BookManager, HARD_PARSE_TIMEOUT_SECONDS, MismatchDetails, ReadFileError};
use search_manager::Docs;

/// `reload_category`의 loader 실행 시간 상한.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// 재적재 진행 보고. 보고마다 그때 아는 값만 채운다. 받는 쪽은 `merge`로 누적한다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReloadCounts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_count: Option<i64>,
}

impl ReloadCounts {
    fn running(indexed: i64, deleted: i64) -> Self {
        Self { before_count: None, indexed_count: Some(indexed), deleted_count: Some(deleted) }
    }

    /// `newer`에 있는 값으로 덮어쓴다.
    pub fn merge(&mut self, newer: ReloadCounts) {
        self.before_count = newer.before_count.or(self.before_count);
        self.indexed_count = newer.indexed_count.or(self.indexed_count);
        self.deleted_count = newer.deleted_count.or(self.deleted_count);
    }
}

/// 재적재 진행 보고를 받는 함수.
pub type ReloadProgressFn<'a> = &'a (dyn Fn(ReloadCounts) + Send + Sync);

/// 재적재하지 못한 항목. 파일 단위 실패면 `file_path`, 카테고리 단위 실패면 `category`가 있다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReloadFailure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    pub error: String,
}

impl ReloadFailure {
    pub fn file(file_path: &str, error: String) -> Self {
        Self { file_path: Some(file_path.to_string()), error, ..Self::default() }
    }

    pub fn category(category: &str, error: String) -> Self {
        Self { category: Some(category.to_string()), error, ..Self::default() }
    }

    fn other(error: String) -> Self {
        Self { error, ..Self::default() }
    }
}

/// 카테고리 하나의 불일치 재적재 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CategoryReload {
    pub category: String,
    pub indexed_count: i64,
    pub deleted_count: i64,
    pub failures: Vec<ReloadFailure>,
}

impl CategoryReload {
    fn rejected(category: &str, error: String) -> Self {
        Self { category: category.to_string(), failures: vec![ReloadFailure::category(category, error)], ..Self::default() }
    }
}

/// 불일치 재적재 전체 결과. 카테고리 하나만 재적재하면 `category`가 있다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MismatchReload {
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub category_count: usize,
    pub before_count: i64,
    pub after_count: i64,
    pub indexed_count: i64,
    pub deleted_count: i64,
    pub failed_count: i64,
    pub failures: Vec<ReloadFailure>,
    pub categories: Vec<CategoryReload>,
}

/// 디렉터리 재적재 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReloadedFiles {
    pub category: String,
    pub processed_count: i64,
}

impl BookManager {
    /// 파일 하나를 파싱하고 시간 상한을 적용한다.
    fn read_with_hard_timeout(&self, abs_path: PathBuf) -> Result<Docs, ReadFileError> {
        self.reader().try_read_file_with_timeout(abs_path, None, false, Duration::from_secs(HARD_PARSE_TIMEOUT_SECONDS))
    }

    /// 여러 파일을 배치로 파싱해 ES에 한 번에 색인한다. 반환: (file_path -> 새 book_id, 실패 목록).
    pub async fn bulk_index_files(&self, file_paths: &[&str], clean_existing: bool) -> (BTreeMap<String, i64>, Vec<ReloadFailure>) {
        self.bulk_index_files_with(file_paths, clean_existing, None, ReloadCounts::default()).await
    }

    /// `bulk_index_files`에 진행 보고를 붙인 판. `progress_base`는 이 배치 이전까지 확정된 누적 카운트다.
    pub async fn bulk_index_files_with(
        &self,
        file_paths: &[&str],
        clean_existing: bool,
        on_progress: Option<ReloadProgressFn<'_>>,
        progress_base: ReloadCounts,
    ) -> (BTreeMap<String, i64>, Vec<ReloadFailure>) {
        if let Some(hook) = self.seams().bulk_index_hook {
            let names: Vec<String> = file_paths.iter().map(|p| p.to_string()).collect();
            return hook(&names, clean_existing);
        }
        let mut merged = Docs::new();
        let mut id_by_path: Vec<(String, i64)> = Vec::new();
        let mut failures: Vec<ReloadFailure> = Vec::new();
        let base_indexed = progress_base.indexed_count.unwrap_or(0);

        for file_path in file_paths {
            // 파일 1건 파싱이 가장 오래 걸릴 수 있어, 파일마다 진행률을 보고한다.
            if let Some(report) = on_progress {
                report(ReloadCounts { indexed_count: Some(base_indexed + id_by_path.len() as i64), ..progress_base });
            }
            let abs_path = match self.resolve_path(&self.path_prefix.join(file_path)) {
                Ok(p) if matches!(self.is_inside_root(&p), Ok(true)) && self.fs.is_file(&p).unwrap_or(false) => p,
                _ => {
                    failures.push(ReloadFailure::file(file_path, format!("파일을 찾을 수 없습니다: {file_path}")));
                    continue;
                }
            };
            let data = match self.read_with_hard_timeout(abs_path) {
                Ok(data) => data,
                Err(ReadFileError::Timeout(reason)) => {
                    log::error!("파일 파싱 타임아웃: {file_path} ({reason})");
                    failures.push(ReloadFailure::file(file_path, format!("파싱 시간 초과: {file_path}")));
                    continue;
                }
                Err(ReadFileError::Other(reason)) => {
                    log::error!("파일 파싱 실패: {file_path} ({reason})");
                    failures.push(ReloadFailure::file(file_path, format!("파일 읽기 실패: {file_path}")));
                    continue;
                }
            };
            if data.len() != 1 {
                failures.push(ReloadFailure::file(file_path, format!("지원하지 않는 파일 형식입니다: {file_path}")));
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
                failures.push(ReloadFailure::file(&file_path, "ES 적재 실패".to_string()));
            }
        }
        (indexed, failures)
    }

    pub async fn reload_category_mismatch_details(&self, category: &str, details: &MismatchDetails) -> CategoryReload {
        self.reload_category_mismatch_details_with(category, details, None, ReloadCounts::default()).await
    }

    /// 카테고리 하나의 불일치(`fs_only`, `duplicates`, `es_only`)를 재적재·정리한다.
    /// `progress_base`는 이전 카테고리까지 확정된 누적 카운트로, 보고에는 항상 이 값을 더한 전역 누적치를 싣는다.
    pub async fn reload_category_mismatch_details_with(&self, category: &str, details: &MismatchDetails, on_progress: Option<ReloadProgressFn<'_>>, progress_base: ReloadCounts) -> CategoryReload {
        let mut result = CategoryReload { category: category.to_string(), ..CategoryReload::default() };
        let mut ids_to_delete: BTreeSet<i64> = BTreeSet::new();
        let mut running_indexed = progress_base.indexed_count.unwrap_or(0);
        let mut running_deleted = progress_base.deleted_count.unwrap_or(0);
        let empty_path = || ReloadFailure::file("", "file_path가 비어있습니다".to_string());

        let mut fs_only_paths: Vec<String> = Vec::new();
        for item in &details.fs_only {
            if item.file_path.is_empty() {
                result.failures.push(empty_path());
            } else {
                fs_only_paths.push(item.file_path.clone());
            }
        }

        let linked_ids_of = |item: &super::DuplicatePath| -> BTreeSet<i64> { item.docs.iter().filter(|d| d.file_linked).map(|d| d.book_id).collect() };
        let mut dup_reindex_paths: Vec<String> = Vec::new();
        for item in &details.duplicates {
            if item.file_exists && linked_ids_of(item).is_empty() {
                if item.file_path.is_empty() {
                    result.failures.push(empty_path());
                } else {
                    dup_reindex_paths.push(item.file_path.clone());
                }
            }
        }

        let mut indexed_book_id_by_path: HashMap<String, i64> = HashMap::new();
        for (paths, clean_existing) in [(&fs_only_paths, true), (&dup_reindex_paths, false)] {
            for batch in paths.chunks(self.bulk_reindex_batch_size) {
                let batch_refs: Vec<&str> = batch.iter().map(String::as_str).collect();
                let base = ReloadCounts::running(running_indexed, running_deleted);
                let (indexed, batch_failures) = self.bulk_index_files_with(&batch_refs, clean_existing, on_progress, base).await;
                result.indexed_count += indexed.len() as i64;
                indexed_book_id_by_path.extend(indexed.iter().map(|(k, v)| (k.clone(), *v)));
                result.failures.extend(batch_failures);
                // 배치 도중 보고한 잠정값을 실제 색인 건수로 정정한다.
                running_indexed += indexed.len() as i64;
                if let Some(report) = on_progress {
                    report(ReloadCounts::running(running_indexed, running_deleted));
                }
            }
        }

        for item in &details.duplicates {
            let mut linked = linked_ids_of(item);
            if item.file_exists && linked.is_empty() {
                let Some(new_book_id) = indexed_book_id_by_path.get(&item.file_path) else {
                    continue; // 색인 실패 - failures에 이미 기록됨
                };
                linked.insert(*new_book_id);
            }
            for doc in &item.docs {
                if item.file_exists && linked.contains(&doc.book_id) {
                    continue;
                }
                ids_to_delete.insert(doc.book_id);
            }
        }

        ids_to_delete.extend(details.es_only.iter().map(|item| item.book_id));

        if !ids_to_delete.is_empty() {
            let ids: Vec<i64> = ids_to_delete.iter().copied().collect();
            let deleted = match self.es.delete_by_ids(&ids, 10000) {
                Ok(n) => n,
                Err(e) => {
                    log::error!("_reload_category_mismatch_details: ES 삭제 실패: {e}");
                    0
                }
            };
            result.deleted_count += deleted;
            running_deleted += deleted;
            if let Some(report) = on_progress {
                report(ReloadCounts::running(running_indexed, running_deleted));
            }
            if deleted < ids.len() as i64 {
                result.failures.push(ReloadFailure::category(category, format!("ES 문서 {}건 중 {}건만 삭제됨", ids.len(), deleted)));
            }
        }
        result
    }

    /// 현재 카테고리 불일치 항목을 파일 단위로 ES에 재적재·정리한다.
    pub async fn reload_category_mismatches(&self, on_progress: Option<ReloadProgressFn<'_>>) -> Result<MismatchReload, String> {
        self.reload_category_mismatches_for("book", on_progress).await
    }

    pub async fn reload_category_mismatches_for(&self, content_type: &str, on_progress: Option<ReloadProgressFn<'_>>) -> Result<MismatchReload, String> {
        log::info!("reload_category_mismatches 시작: content_type='{content_type}'");
        let before = match self.try_get_category_mismatches(true) {
            Ok(v) => v,
            Err(e) => return Err(format!("불일치 조회 실패: {e}")),
        };
        let categories = before.categories();
        let before_count = before.item_count();
        let (mut indexed_total, mut deleted_total) = (0i64, 0i64);
        let mut failures: Vec<ReloadFailure> = Vec::new();
        let mut category_results: Vec<CategoryReload> = Vec::new();
        if let Some(report) = on_progress {
            report(ReloadCounts { before_count: Some(before_count), ..ReloadCounts::default() });
        }

        for category in &categories {
            if !self.is_safe_category_name(category) {
                category_results.push(CategoryReload::rejected(category, "잘못된 카테고리 경로입니다".to_string()));
                continue;
            }
            let details = self.get_category_mismatch_details(category);
            if let Some(error) = &details.error {
                category_results.push(CategoryReload::rejected(category, format!("불일치 상세 조회 실패: {error}")));
                continue;
            }
            let base = ReloadCounts::running(indexed_total, deleted_total);
            let category_result = self.reload_category_mismatch_details_with(category, &details, on_progress, base).await;
            indexed_total += category_result.indexed_count;
            deleted_total += category_result.deleted_count;
            category_results.push(category_result);
            if let Some(report) = on_progress {
                report(ReloadCounts::running(indexed_total, deleted_total));
            }
        }

        if let Err(e) = self.es.refresh() {
            failures.push(ReloadFailure::other(format!("ES refresh 실패: {e}")));
        }

        let after_count = self.try_get_category_mismatches(true).map(|summary| summary.item_count()).unwrap_or(0);
        for category_result in &category_results {
            failures.extend(category_result.failures.iter().cloned());
        }
        let failed_count = failures.len() as i64;
        log::info!(
            "reload_category_mismatches 완료: content_type='{content_type}', categories={}, indexed={indexed_total}, deleted={deleted_total}, failed={failed_count}, before={before_count}, after={after_count}",
            categories.len()
        );
        Ok(MismatchReload {
            content_type: content_type.to_string(),
            category: None,
            category_count: categories.len(),
            before_count,
            after_count,
            indexed_count: indexed_total,
            deleted_count: deleted_total,
            failed_count,
            failures,
            categories: category_results,
        })
    }

    /// 선택 카테고리의 현재 불일치 항목만 파일 단위로 ES에 재적재·정리한다.
    pub async fn reload_category_mismatch_files(&self, category: &str, on_progress: Option<ReloadProgressFn<'_>>) -> Result<MismatchReload, String> {
        self.reload_category_mismatch_files_for(category, "book", on_progress).await
    }

    pub async fn reload_category_mismatch_files_for(&self, category: &str, content_type: &str, on_progress: Option<ReloadProgressFn<'_>>) -> Result<MismatchReload, String> {
        log::info!("reload_category_mismatch_files 시작: category='{category}', content_type='{content_type}'");
        if category.is_empty() {
            return Err("카테고리 이름이 비어있습니다".to_string());
        }
        if !self.is_safe_category_name(category) {
            return Err("잘못된 카테고리 경로입니다".to_string());
        }

        self.clear_mismatch_cache();
        let before_details = self.get_category_mismatch_details(category);
        if let Some(error) = &before_details.error {
            return Err(format!("불일치 상세 조회 실패: {error}"));
        }
        let before_count = before_details.item_count();
        if let Some(report) = on_progress {
            report(ReloadCounts { before_count: Some(before_count), ..ReloadCounts::default() });
        }
        let category_result = self.reload_category_mismatch_details_with(category, &before_details, on_progress, ReloadCounts::default()).await;
        let mut failures: Vec<ReloadFailure> = Vec::new();
        if let Err(e) = self.es.refresh() {
            failures.push(ReloadFailure::other(format!("ES refresh 실패: {e}")));
        }

        self.clear_mismatch_cache();
        let after_count = self.get_category_mismatch_details(category).item_count();
        failures.extend(category_result.failures.iter().cloned());
        Ok(MismatchReload {
            content_type: content_type.to_string(),
            category: Some(category.to_string()),
            category_count: 1,
            before_count,
            after_count,
            indexed_count: category_result.indexed_count,
            deleted_count: category_result.deleted_count,
            failed_count: failures.len() as i64,
            failures,
            categories: vec![category_result],
        })
    }

    /// `_root`는 실제 디렉터리가 아니라 최상위 파일 묶음이라, loader 대신 최상위 파일만 직접 색인한다.
    /// 책 루트의 최상위 파일만 다시 색인한다.
    pub async fn reload_book_root_files(&self) -> Result<ReloadedFiles, String> {
        self.reload_root_files("book").await
    }

    async fn reload_root_files(&self, content_type: &str) -> Result<ReloadedFiles, String> {
        log::info!("_reload_root_files 시작: content_type='{content_type}'");
        let entries = match self.fs.read_dir(&self.path_prefix) {
            Ok(entries) => entries,
            Err(e) => {
                log::error!("_reload_root_files: 최상위 디렉토리 스캔 실패 - {e}");
                return Err(format!("최상위 디렉토리를 읽을 수 없습니다: {e}"));
            }
        };
        let mut file_names: Vec<String> =
            entries.iter().filter(|p| Self::is_regular_file_nofollow(p) && self.is_indexable_file_path(p)).map(|p| Self::file_name_of(p).to_string_lossy().into_owned()).collect();
        file_names.sort();

        let mut processed = 0usize;
        let mut failures: Vec<ReloadFailure> = Vec::new();
        for batch in file_names.chunks(self.bulk_reindex_batch_size) {
            let refs: Vec<&str> = batch.iter().map(String::as_str).collect();
            let (indexed, batch_failures) = self.bulk_index_files(&refs, true).await;
            processed += indexed.len();
            failures.extend(batch_failures);
        }
        if let Err(e) = self.es.refresh() {
            failures.push(ReloadFailure::other(format!("ES refresh 실패: {e}")));
        }

        self.clear_mismatch_cache();
        if let Some(first) = failures.first() {
            log::error!("_reload_root_files 실패: processed={processed}, failed={} - {first:?}", failures.len());
            return Err(format!("최상위 파일 재적재 실패 {}건: {}", failures.len(), first.error));
        }
        log::info!("_reload_root_files 완료: processed={processed}");
        Ok(ReloadedFiles { category: Self::ROOT_CATEGORY.to_string(), processed_count: processed as i64 })
    }

    /// 카테고리 전체를 ES에 재적재한다(loader를 `--recursive --reload`로 실행). `content_type`은 `"comic"`이면 만화 색인을 쓴다.
    pub async fn reload_category(&self, category: &str, content_type: Option<&str>) -> Result<ReloadedFiles, String> {
        let content_type = content_type.unwrap_or("book");
        log::info!("reload_category 시작: category='{category}', content_type='{content_type}'");
        if category.is_empty() {
            return Err("카테고리 이름이 비어있습니다".to_string());
        }
        if category.contains("..") {
            return Err("카테고리 이름에 '..'는 사용할 수 없습니다".to_string());
        }
        if category == Self::ROOT_CATEGORY {
            return self.reload_root_files(content_type).await;
        }

        let abs_dir = match self.resolve_path(&self.path_prefix.join(category)) {
            Ok(p) if matches!(self.is_inside_root(&p), Ok(true)) => p,
            _ => return Err(format!("잘못된 경로입니다: {category}")),
        };
        if !abs_dir.is_dir() {
            log::error!("reload_category: 디렉토리 없음 - {}", abs_dir.display());
            return Err(format!("디렉토리를 찾을 수 없습니다: {category}"));
        }

        let index_name = if content_type == "comic" { "comics" } else { "book" };
        log::info!("reload_category: loader 실행 - index={index_name}, path={}", abs_dir.display());
        let argv = vec![self.loader_program.clone(), "--recursive".to_string(), "--reload".to_string(), index_name.to_string(), abs_dir.to_string_lossy().into_owned()];
        let output = match self.runner.run(&argv, Some(RELOAD_TIMEOUT)) {
            Ok(output) => output,
            Err(CommandError::Timeout) => {
                log::error!("reload_category: 타임아웃 (10분 초과) - category='{category}'");
                return Err("재적재 시간이 초과되었습니다 (10분)".to_string());
            }
            Err(e) => {
                log::error!("reload_category: subprocess 실행 실패 - {e}");
                return Err(format!("재적재 실행 실패: {e}"));
            }
        };
        let stdout_text = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if output.returncode != 0 {
            log::error!("reload_category: 실패 (exit {}) - stderr: {stderr_text}", output.returncode);
            return Err(format!("재적재 실패 (exit {}): {stderr_text}", output.returncode));
        }
        if !stderr_text.is_empty() {
            log::warn!("reload_category: stderr 출력 - {stderr_text}");
        }

        let processed = regex::Regex::new(r"총\s+(\d+)개\s+파일\s+처리됨").ok().and_then(|re| re.captures(&stdout_text)).and_then(|c| c[1].parse::<i64>().ok()).unwrap_or(0);
        log::info!("reload_category 완료: category='{category}', processed={processed}");
        self.clear_mismatch_cache();
        Ok(ReloadedFiles { category: category.to_string(), processed_count: processed })
    }
}
