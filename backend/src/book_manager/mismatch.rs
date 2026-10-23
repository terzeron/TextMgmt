//! 카테고리 불일치 요약·상세와 그 캐시. Python `get_category_mismatches`, `get_category_mismatch_details`,
//! `_scan_category_mismatches`, `_collect_es_category_paths`, `_iter_fs_category_paths`에 대응한다.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Instant;

use serde_json::{Map, Value, json};

use super::{BookManager, MAX_CATEGORY_RESULT_COUNT};
use crate::es_manager::Hit;

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

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

impl BookManager {
    /// 캐시를 직접 정한다(값 설정). 테스트가 TTL 안의 캐시를 만들 때 쓴다.
    pub fn set_mismatch_cache(&self, cache: Option<Value>, at: Instant) {
        let mut state = self.mismatch.lock().unwrap_or_else(|e| e.into_inner());
        state.cache = cache;
        state.time = Some(at);
    }

    pub(crate) fn clear_mismatch_cache(&self) {
        let mut state = self.mismatch.lock().unwrap_or_else(|e| e.into_inner());
        state.cache = None;
        state.time = None;
    }

    /// 불일치 요약 캐시를 비운다.
    pub fn _clear_mismatch_cache(&self) {
        self.clear_mismatch_cache();
    }

    /// 요약 응답의 이상 항목 총 건수. `anomaly_count`가 없으면 건수 비교만 하던 옛 응답으로 본다.
    pub fn _mismatch_item_count(&self, mismatch_data: &Value) -> i64 {
        let mut total = 0;
        for (key, legacy_field) in [("mismatches", "diff"), ("es_only", "es_count"), ("fs_only", "fs_count")] {
            for item in mismatch_data.get(key).and_then(Value::as_array).into_iter().flatten() {
                let count = item.get("anomaly_count").filter(|v| !v.is_null()).or_else(|| item.get(legacy_field)).and_then(Value::as_i64).unwrap_or(0);
                total += count.abs();
            }
        }
        total
    }

    /// 요약 응답에 나온 카테고리 이름들(중복 없이, 나온 순서대로).
    pub fn _mismatch_categories(&self, mismatch_data: &Value) -> Vec<String> {
        let mut categories: Vec<String> = Vec::new();
        for key in ["mismatches", "es_only", "fs_only"] {
            for item in mismatch_data.get(key).and_then(Value::as_array).into_iter().flatten() {
                if let Some(category) = item.get("category").and_then(Value::as_str)
                    && !category.is_empty()
                    && !categories.iter().any(|c| c == category)
                {
                    categories.push(category.to_string());
                }
            }
        }
        categories
    }

    pub(crate) fn mismatch_detail_item_count(details: &Value) -> i64 {
        ["fs_only", "es_only", "duplicates"].iter().map(|key| details.get(*key).and_then(Value::as_array).map_or(0, |a| a.len() as i64)).sum()
    }

    /// 카테고리의 ES 문서를 모두 모은다(커서를 따라 끝까지).
    fn search_all_docs_by_category(&self, category: &str) -> Result<Vec<Hit>, String> {
        let mut result: Vec<Hit> = Vec::new();
        let mut search_after: Option<Vec<Value>> = None;
        loop {
            let (page, _total, next) = self.es.search_by_category_paged(category, MAX_CATEGORY_RESULT_COUNT, search_after.as_deref()).map_err(|e| e.to_string())?;
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

    /// FS 스캔이 들어가지 않는 디렉터리는 ES 쪽에서도 빼야 한다.
    fn is_scannable_category(category: &str) -> bool {
        !category.split('/').any(corpus_layout::is_ignored_segment)
    }

    /// 디렉터리마다 (카테고리, 색인 대상 파일의 상대 경로 집합)을 깊이 제한 없이 만든다. 디렉터리 하나씩 `visit`에 넘긴다.
    fn for_each_fs_category(&self, mut visit: impl FnMut(String, BTreeSet<String>)) {
        let mut stack: Vec<(PathBuf, String)> = vec![(self.path_prefix.clone(), Self::ROOT_CATEGORY.to_string())];
        while let Some((dir_path, category)) = stack.pop() {
            let Ok(entries) = self.fs.read_dir(&dir_path) else { continue };
            let mut paths: BTreeSet<String> = BTreeSet::new();
            for entry in entries {
                let name = Self::file_name_of(&entry).to_string_lossy().into_owned();
                let qualified = if category == Self::ROOT_CATEGORY { name.clone() } else { format!("{category}/{name}") };
                match entry_kind(&entry) {
                    EntryKind::Dir => {
                        if !corpus_layout::is_ignored_segment(&name) {
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
    fn scan_category_mismatches(&self) -> Result<Value, String> {
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
            if !Self::is_scannable_category(&category) {
                continue;
            }
            stats.insert(category, Stat { es_count: entry.doc_count, fs_count: 0, es_only_count: entry.paths.len() as i64, fs_only_count: 0, duplicate_count: entry.duplicate_paths.len() as i64 });
        }

        let (mut mismatches, mut es_only, mut fs_only) = (Vec::new(), Vec::new(), Vec::new());
        for (category, stat) in &stats {
            let anomaly_count = stat.es_only_count + stat.fs_only_count + stat.duplicate_count;
            if anomaly_count == 0 {
                continue;
            }
            let item = json!({
                "category": category, "es_count": stat.es_count, "fs_count": stat.fs_count, "es_only_count": stat.es_only_count,
                "fs_only_count": stat.fs_only_count, "duplicate_count": stat.duplicate_count, "anomaly_count": anomaly_count,
                "diff": (stat.es_count - stat.fs_count).abs(),
            });
            if stat.fs_count == 0 {
                es_only.push(item);
            } else if stat.es_count == 0 {
                fs_only.push(item);
            } else {
                mismatches.push(item);
            }
        }
        mismatches.sort_by_key(|item| std::cmp::Reverse(item["anomaly_count"].as_i64().unwrap_or(0)));
        let result = json!({"mismatches": mismatches, "es_only": es_only, "fs_only": fs_only});
        log::info!("get_category_mismatches: 이상 카테고리 {}개, 이상 항목 {}건", mismatches_len(&result), self._mismatch_item_count(&result));
        Ok(result)
    }

    /// 요약 스캔을 돌려 캐시를 갱신한다. 스캔은 동시에 하나만 돈다.
    fn refresh_mismatch_cache(&self) -> Result<Value, String> {
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
    pub fn try_get_category_mismatches(&self, force_refresh: bool) -> Result<Value, String> {
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
    pub fn get_category_mismatches(&self, force_refresh: bool) -> Value {
        self.try_get_category_mismatches(force_refresh).unwrap_or_else(|e| {
            log::error!("get_category_mismatches 실패: {e}");
            json!({"mismatches": [], "es_only": [], "fs_only": [], "error": e})
        })
    }

    /// 특정 카테고리의 ES 문서와 파일시스템 파일을 비교해 불일치 항목을 돌려준다. ES 오류는 `Err`다.
    pub fn try_get_category_mismatch_details(&self, category: &str) -> Result<Value, String> {
        let empty_result = json!({"es_only": [], "fs_only": [], "duplicates": [], "fs_count": 0});
        if !self.is_safe_category_name(category) {
            log::warn!("get_category_mismatch_details: unsafe category ignored: {category:?}");
            return Ok(empty_result);
        }

        // 1. ES 문서 목록(file_path 기준, 중복 감지 포함)
        let doc_list = self.search_all_docs_by_category(category)?;
        let mut es_files: BTreeMap<String, Value> = BTreeMap::new();
        let mut path_docs: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for (book_id, doc, _score) in &doc_list {
            let rel_path = self.normalize_stored_file_path(str_of(doc, "file_path"));
            let mut with_id = Map::new();
            with_id.insert("book_id".to_string(), json!(book_id));
            if let Some(fields) = doc.as_object() {
                with_id.extend(fields.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            let with_id = Value::Object(with_id);
            path_docs.entry(rel_path.clone()).or_default().push(with_id.clone());
            es_files.insert(rel_path, with_id);
        }

        // 2. 파일시스템 파일 목록
        let mut fs_files: BTreeSet<String> = BTreeSet::new();
        if let Ok(entries) = self.fs.read_dir(&self.category_dir(category)) {
            for entry in entries {
                if Self::is_regular_file_nofollow(&entry) && self.is_indexable_file_path(&entry) {
                    let name = Self::file_name_of(&entry).to_string_lossy().into_owned();
                    fs_files.insert(if category == Self::ROOT_CATEGORY { name } else { format!("{category}/{name}") });
                }
            }
        }

        // 3. 비교
        let es_paths: BTreeSet<&String> = es_files.keys().collect();
        let es_only: Vec<Value> = es_paths
            .iter()
            .filter(|p| !fs_files.contains(p.as_str()))
            .map(|path| {
                let info = &es_files[path.as_str()];
                json!({"book_id": info["book_id"], "title": str_of(info, "title"), "file_type": str_of(info, "file_type"), "file_path": path})
            })
            .collect();
        let fs_only: Vec<Value> = fs_files.iter().filter(|p| !es_files.contains_key(p.as_str())).map(|path| json!({"file_name": path.rsplit('/').next().unwrap_or(path), "file_path": path})).collect();

        // 4. 중복 ES 문서 감지(같은 file_path에 여러 book_id)
        let mut duplicates = Vec::new();
        for (path, docs) in &path_docs {
            if docs.len() <= 1 {
                continue;
            }
            let file_exists = fs_files.contains(path);
            // 파일의 실제 inode를 조회해 ES 문서 _id(=inode)와 비교한다.
            let linked_id: Option<i64> = if file_exists { self.fs.metadata(&self.path_prefix.join(path)).ok().map(|m| m.ino() as i64) } else { None };
            let dup_docs: Vec<Value> = docs
                .iter()
                .map(|d| json!({"book_id": d["book_id"], "title": str_of(d, "title"), "author": str_of(d, "author"), "file_type": str_of(d, "file_type"), "file_linked": d["book_id"].as_i64() == linked_id && linked_id.is_some()}))
                .collect();
            duplicates.push(json!({"file_path": path, "file_exists": file_exists, "docs": dup_docs}));
        }
        Ok(json!({"es_only": es_only, "fs_only": fs_only, "duplicates": duplicates, "fs_count": fs_files.len()}))
    }

    /// `try_get_category_mismatch_details`의 값 판. ES 오류는 빈 상세에 `error`를 담아 돌려준다.
    pub fn get_category_mismatch_details(&self, category: &str) -> Value {
        if let Some(f) = &self.get_category_mismatch_details_override {
            return f(category);
        }
        self.try_get_category_mismatch_details(category).unwrap_or_else(|e| {
            log::error!("get_category_mismatch_details 실패: {category}: {e}");
            json!({"es_only": [], "fs_only": [], "duplicates": [], "fs_count": 0, "error": e})
        })
    }
}

fn mismatches_len(result: &Value) -> usize {
    ["mismatches", "es_only", "fs_only"].iter().map(|k| result[*k].as_array().map_or(0, Vec::len)).sum()
}
