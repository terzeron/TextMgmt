//! 카테고리 불일치 요약·상세와 그 캐시: ES와 파일시스템의 카테고리 경로를 비교한다.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Instant;

use common::json_field_reader::str_field;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::BookManager;
use search_manager::{Hit, PagedHits};

/// ES 쪽 카테고리 하나의 경로 집합.
#[derive(Default)]
struct EsCategory {
    paths: BTreeSet<String>,
    duplicate_paths: BTreeSet<String>,
    doc_count: i64,
}

/// FS 쪽 카테고리 하나의 통계.
struct Stat {
    es_count: i64,
    fs_count: i64,
    es_only_count: i64,
    fs_only_count: i64,
    duplicate_count: i64,
}

/// `entry.is_dir(follow_symlinks=False)`와 `entry.is_file(follow_symlinks=False)`를 한 번의 stat으로 가른다.
enum EntryKind {
    Dir,
    File,
    Other,
}

fn entry_kind(path: &std::path::Path) -> EntryKind {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => EntryKind::Dir,
        Ok(m) if m.file_type().is_file() => EntryKind::File,
        _ => EntryKind::Other,
    }
}

/// 카테고리 하나의 ES·파일시스템 건수 비교.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CategoryMismatch {
    pub category: String,
    pub es_count: i64,
    pub fs_count: i64,
    pub es_only_count: i64,
    pub fs_only_count: i64,
    pub duplicate_count: i64,
    /// 경로 단위 이상 항목 수. 건수 차이(`diff`)는 서로 상쇄될 수 있어 이 값으로 판단한다.
    pub anomaly_count: i64,
    pub diff: i64,
}

/// 카테고리별 불일치 요약. 디렉터리가 없으면 `es_only`, ES 문서가 없으면 `fs_only`, 둘 다 있으면 `mismatches`다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MismatchSummary {
    pub mismatches: Vec<CategoryMismatch>,
    pub es_only: Vec<CategoryMismatch>,
    pub fs_only: Vec<CategoryMismatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl MismatchSummary {
    fn all(&self) -> impl Iterator<Item = &CategoryMismatch> {
        self.mismatches.iter().chain(&self.es_only).chain(&self.fs_only)
    }

    /// 이상 항목 총 건수.
    pub fn item_count(&self) -> i64 {
        self.all().map(|item| item.anomaly_count.abs()).sum()
    }

    /// 요약에 나온 카테고리 이름들(중복 없이, 나온 순서대로).
    pub fn categories(&self) -> Vec<String> {
        let mut categories: Vec<String> = Vec::new();
        for item in self.all() {
            if !item.category.is_empty() && !categories.contains(&item.category) {
                categories.push(item.category.clone());
            }
        }
        categories
    }
}

/// ES에만 있는 문서. 파일이 사라졌다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EsOnlyFile {
    pub book_id: i64,
    pub title: String,
    pub file_type: String,
    pub file_path: String,
}

/// 파일시스템에만 있는 파일. 색인되지 않았다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FsOnlyFile {
    pub file_name: String,
    pub file_path: String,
}

/// 같은 경로를 가리키는 ES 문서 하나.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuplicateDoc {
    pub book_id: i64,
    pub title: String,
    pub author: String,
    pub file_type: String,
    /// 문서 ID가 지금 파일의 inode와 같다.
    pub file_linked: bool,
}

/// ES 문서가 여럿 가리키는 경로.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuplicatePath {
    pub file_path: String,
    pub file_exists: bool,
    pub docs: Vec<DuplicateDoc>,
}

/// 카테고리 하나의 불일치 상세.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MismatchDetails {
    pub es_only: Vec<EsOnlyFile>,
    pub fs_only: Vec<FsOnlyFile>,
    pub duplicates: Vec<DuplicatePath>,
    pub fs_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl MismatchDetails {
    /// 고칠 항목 수.
    pub fn item_count(&self) -> i64 {
        (self.fs_only.len() + self.es_only.len() + self.duplicates.len()) as i64
    }
}

impl BookManager {
    /// 캐시를 직접 정한다(값 설정). 테스트가 TTL 안의 캐시를 만들 때 쓴다.
    pub fn set_mismatch_cache(&self, cache: Option<MismatchSummary>, at: Instant) {
        let mut state = self.mismatch.lock().unwrap_or_else(|e| e.into_inner());
        state.cache = cache;
        state.time = Some(at);
    }

    /// 불일치 요약 캐시를 비운다.
    pub fn clear_mismatch_cache(&self) {
        let mut state = self.mismatch.lock().unwrap_or_else(|e| e.into_inner());
        state.cache = None;
        state.time = None;
    }

    /// 카테고리의 ES 문서를 모두 모은다(커서를 따라 끝까지).
    fn search_all_docs_by_category(&self, category: &str) -> Result<Vec<Hit>, String> {
        let mut result: Vec<Hit> = Vec::new();
        let mut search_after: Option<Vec<Value>> = None;
        loop {
            let PagedHits { hits: page, search_after: next, .. } = self.es.search_by_category_paged(category, self.max_category_result_count, search_after.as_deref()).map_err(|e| e.to_string())?;
            result.extend(page);
            match next {
                Some(next) => search_after = Some(next),
                None => return Ok(result),
            }
        }
    }

    /// 카테고리별 ES 문서의 상대 경로 집합, 문서 수, 중복 경로를 모은다.
    fn collect_es_category_paths(&self) -> Result<HashMap<String, EsCategory>, String> {
        let mut collected: HashMap<String, EsCategory> = HashMap::new();
        for (category, file_path, doc_count) in self.es.iter_all_category_file_paths(10000).map_err(|e| e.to_string())? {
            let entry = collected.entry(category).or_default();
            entry.doc_count += doc_count;
            let rel_path = self.normalize_stored_file_path(&file_path);
            // 같은 경로에 문서가 여럿이거나, 서로 다른 원본 경로가 같은 상대 경로로 정규화되면 중복이다.
            if doc_count > 1 || entry.paths.contains(&rel_path) {
                entry.duplicate_paths.insert(rel_path.clone());
            }
            entry.paths.insert(rel_path);
        }
        Ok(collected)
    }

    /// 디렉터리마다 (카테고리, 색인 대상 파일의 상대 경로 집합)을 깊이 제한 없이 만든다. 디렉터리 하나씩 `visit`에 넘긴다.
    fn for_each_fs_category(&self, mut visit: impl FnMut(String, BTreeSet<String>)) {
        let mut stack: Vec<(PathBuf, String)> = vec![(self.path_prefix.clone(), Self::ROOT_CATEGORY.to_string())];
        while let Some((dir_path, category)) = stack.pop() {
            let Ok(entries) = self.fs.read_dir(&dir_path) else { continue };
            let mut paths: BTreeSet<String> = BTreeSet::new();
            for entry in entries {
                let name = crate::path_validator::file_name_of(&entry).to_string_lossy().into_owned();
                let qualified = if category == Self::ROOT_CATEGORY { name.clone() } else { format!("{category}/{name}") };
                match entry_kind(&entry) {
                    EntryKind::Dir => {
                        if !book_model::ignore_rules::is_ignored_segment(&name) {
                            stack.push((entry, qualified));
                        }
                    }
                    EntryKind::File => {
                        if self.is_indexable_file_path(&entry) {
                            paths.insert(qualified);
                        }
                    }
                    EntryKind::Other => {}
                }
            }
            visit(category, paths);
        }
    }

    /// ES 문서와 파일시스템 파일을 경로 단위로 비교해 카테고리별 이상 항목 수를 센다.
    fn scan_category_mismatches(&self) -> Result<MismatchSummary, String> {
        if let Some(observer) = &self.scan_observer {
            observer();
        }
        let mut es_entries = self.collect_es_category_paths()?;
        let mut stats: BTreeMap<String, Stat> = BTreeMap::new();

        self.for_each_fs_category(|category, fs_paths| {
            let entry = es_entries.remove(&category);
            if entry.is_none() && fs_paths.is_empty() {
                return;
            }
            let empty = EsCategory::default();
            let es = entry.as_ref().unwrap_or(&empty);
            stats.insert(
                category,
                Stat {
                    es_count: es.doc_count,
                    fs_count: fs_paths.len() as i64,
                    es_only_count: es.paths.difference(&fs_paths).count() as i64,
                    fs_only_count: fs_paths.difference(&es.paths).count() as i64,
                    duplicate_count: es.duplicate_paths.len() as i64,
                },
            );
        });

        // 디렉터리가 사라진 카테고리 - ES 문서 전부가 고아다.
        for (category, entry) in es_entries {
            if !is_scannable_category(&category) {
                continue;
            }
            stats.insert(category, Stat { es_count: entry.doc_count, fs_count: 0, es_only_count: entry.paths.len() as i64, fs_only_count: 0, duplicate_count: entry.duplicate_paths.len() as i64 });
        }

        let mut result = MismatchSummary::default();
        for (category, stat) in stats {
            let anomaly_count = stat.es_only_count + stat.fs_only_count + stat.duplicate_count;
            if anomaly_count == 0 {
                continue;
            }
            let item = CategoryMismatch {
                category,
                es_count: stat.es_count,
                fs_count: stat.fs_count,
                es_only_count: stat.es_only_count,
                fs_only_count: stat.fs_only_count,
                duplicate_count: stat.duplicate_count,
                anomaly_count,
                diff: (stat.es_count - stat.fs_count).abs(),
            };
            let list = if stat.fs_count == 0 {
                &mut result.es_only
            } else if stat.es_count == 0 {
                &mut result.fs_only
            } else {
                &mut result.mismatches
            };
            list.push(item);
        }
        result.mismatches.sort_by_key(|item| std::cmp::Reverse(item.anomaly_count));
        log::info!("get_category_mismatches: 이상 카테고리 {}개, 이상 항목 {}건", result.all().count(), result.item_count());
        Ok(result)
    }

    /// 요약 스캔을 돌려 캐시를 갱신한다. 스캔은 동시에 하나만 돈다.
    fn refresh_mismatch_cache(&self) -> Result<MismatchSummary, String> {
        let _scan = self.scan_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = self.mismatch.lock().unwrap_or_else(|e| e.into_inner()).fresh().cloned() {
            return Ok(cached);
        }
        let result = self.scan_category_mismatches()?;
        let mut state = self.mismatch.lock().unwrap_or_else(|e| e.into_inner());
        state.cache = Some(result.clone());
        state.time = Some(Instant::now());
        Ok(result)
    }

    /// 카테고리별 이상 항목 요약. ES 오류는 `Err`다. TTL 안이면 캐시를 쓴다. `force_refresh`는 오래된 값을 쓰면 안 되는 호출자용이다.
    pub fn try_get_category_mismatches(&self, force_refresh: bool) -> Result<MismatchSummary, String> {
        if let Some(f) = &self.get_category_mismatches_override {
            return Ok(f(force_refresh));
        }
        if force_refresh {
            self.clear_mismatch_cache();
        } else if let Some(cached) = self.mismatch.lock().unwrap_or_else(|e| e.into_inner()).fresh().cloned() {
            return Ok(cached);
        }
        self.refresh_mismatch_cache()
    }

    /// `try_get_category_mismatches`의 값 판. ES 오류는 빈 요약에 `error`를 담아 돌려준다(캐시하지 않는다).
    pub fn get_category_mismatches(&self, force_refresh: bool) -> MismatchSummary {
        self.try_get_category_mismatches(force_refresh).unwrap_or_else(|e| {
            log::error!("get_category_mismatches 실패: {e}");
            MismatchSummary { error: Some(e), ..MismatchSummary::default() }
        })
    }

    /// 특정 카테고리의 ES 문서와 파일시스템 파일을 비교해 불일치 항목을 돌려준다. ES 오류는 `Err`다.
    pub fn try_get_category_mismatch_details(&self, category: &str) -> Result<MismatchDetails, String> {
        if !self.is_safe_category_name(category) {
            log::warn!("get_category_mismatch_details: unsafe category ignored: {category:?}");
            return Ok(MismatchDetails::default());
        }

        // 1. ES 문서 목록(file_path 기준, 중복 감지 포함)
        let doc_list = self.search_all_docs_by_category(category)?;
        let mut es_files: BTreeMap<String, &Hit> = BTreeMap::new();
        let mut path_docs: BTreeMap<String, Vec<&Hit>> = BTreeMap::new();
        for hit in &doc_list {
            let rel_path = self.normalize_stored_file_path(str_field(&hit.source, "file_path"));
            path_docs.entry(rel_path.clone()).or_default().push(hit);
            es_files.insert(rel_path, hit);
        }

        // 2. 파일시스템 파일 목록
        let mut fs_files: BTreeSet<String> = BTreeSet::new();
        if let Ok(entries) = self.fs.read_dir(&self.category_dir(category)) {
            for entry in entries {
                if crate::path_validator::is_regular_file_nofollow(&entry) && self.is_indexable_file_path(&entry) {
                    let name = crate::path_validator::file_name_of(&entry).to_string_lossy().into_owned();
                    fs_files.insert(if category == Self::ROOT_CATEGORY { name } else { format!("{category}/{name}") });
                }
            }
        }

        // 3. 비교
        let es_only: Vec<EsOnlyFile> = es_files
            .iter()
            .filter(|(path, _)| !fs_files.contains(path.as_str()))
            .map(|(path, hit)| EsOnlyFile { book_id: hit.id, title: str_field(&hit.source, "title").to_string(), file_type: str_field(&hit.source, "file_type").to_string(), file_path: path.clone() })
            .collect();
        let fs_only: Vec<FsOnlyFile> =
            fs_files.iter().filter(|p| !es_files.contains_key(p.as_str())).map(|path| FsOnlyFile { file_name: path.rsplit('/').next().unwrap_or(path).to_string(), file_path: path.clone() }).collect();

        // 4. 중복 ES 문서 감지(같은 file_path에 여러 book_id)
        let mut duplicates = Vec::new();
        for (path, docs) in &path_docs {
            if docs.len() <= 1 {
                continue;
            }
            let file_exists = fs_files.contains(path);
            // 파일의 실제 inode를 조회해 ES 문서 _id(=inode)와 비교한다.
            let linked_id: Option<i64> = if file_exists { self.fs.metadata(&self.path_prefix.join(path)).ok().map(|m| m.ino() as i64) } else { None };
            let docs = docs
                .iter()
                .map(|hit| DuplicateDoc {
                    book_id: hit.id,
                    title: str_field(&hit.source, "title").to_string(),
                    author: str_field(&hit.source, "author").to_string(),
                    file_type: str_field(&hit.source, "file_type").to_string(),
                    file_linked: linked_id == Some(hit.id),
                })
                .collect();
            duplicates.push(DuplicatePath { file_path: path.clone(), file_exists, docs });
        }
        Ok(MismatchDetails { es_only, fs_only, duplicates, fs_count: fs_files.len(), error: None })
    }

    /// `try_get_category_mismatch_details`의 값 판. ES 오류는 빈 상세에 `error`를 담아 돌려준다.
    pub fn get_category_mismatch_details(&self, category: &str) -> MismatchDetails {
        if let Some(f) = &self.get_category_mismatch_details_override {
            return f(category);
        }
        self.try_get_category_mismatch_details(category).unwrap_or_else(|e| {
            log::error!("get_category_mismatch_details 실패: {category}: {e}");
            MismatchDetails { error: Some(e), ..MismatchDetails::default() }
        })
    }
}

/// FS 스캔이 들어가지 않는 디렉터리는 ES 쪽에서도 빼야 한다.
fn is_scannable_category(category: &str) -> bool {
    !category.split('/').any(book_model::ignore_rules::is_ignored_segment)
}
