//! 고수준 ES 대역 `FakeEsManager`(`EsManagerApi` 구현). 설계: docs/rust-test-seams.md 3.3, 5장.
//! `DummyEs` 4종(es_dummy, bm1, bm2, main), `FakeEs` 6종(book_fixtures, book_preview, epub_cover_set, classifier, classifier_corpus, classifier_service),
//! `MockEs` 3종(cover, main_bulk_rename, main_gaps)을 이 하나로 줄인다. 가장 완성도가 높은 book_manager_2의 `DummyEs`를 뼈대로 삼았다.
//!
//! 구성: 문서 맵 + 호출 기록 + 메서드별 오류 주입 + 메서드별 클로저 바꿔치기.
//! 저수준 `EsTransport` 대역은 `es_dummy.rs`의 `DummyEs`다.
//!
//! 가정한 API (아직 구현 전이다): `backend::es_manager::{EsError, EsManagerApi}`. 문서 3.3의 메서드는 그대로, 아래는 문서가 "나머지"로 둔 메서드를
//! 기존 fake(book_manager_2의 `DummyEs`)의 모양에 맞춰 `Result<_, EsError>`와 `i64`로 통일해 가정했다. 실제 trait 모양이 다르면 이 파일의 해당 메서드만 고친다.
//!
//! 사용: `let es = FakeEsManager::with_doc(doc);` `es.fail("update", "boom");` `assert_eq!(es.updates().len(), 1);`
//! 없는 메서드는 trait의 기본 구현(`Err(EsError::Unsupported)`)을 그대로 쓴다.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use backend::es_manager::{EsError, EsManagerApi};
use serde_json::{Value, json};

pub type Docs = BTreeMap<i64, Value>;
/// (문서 ID, `_source`, 점수)
pub type Hit = (i64, Value, f64);

type Fault = Arc<dyn Fn() -> EsError + Send + Sync>;
type SearchByIdHook = Arc<dyn Fn(i64) -> Result<Option<Value>, EsError> + Send + Sync>;
type UpdateHook = Arc<dyn Fn(i64, &Value) -> Result<bool, EsError> + Send + Sync>;
type DeleteHook = Arc<dyn Fn(i64) -> Result<bool, EsError> + Send + Sync>;
type InsertHook = Arc<dyn Fn(&Docs) -> Result<Vec<i64>, EsError> + Send + Sync>;
type SearchByCategoryHook = Arc<dyn Fn(&str, i64) -> Vec<Hit> + Send + Sync>;

/// 테스트가 직접 읽고 쓰는 상태. `FakeEsManager::with`로 연다. 클로저 안에서 fake를 다시 부르면 교착하므로 부르지 않는다.
pub struct State {
    /// ID로 찾는 문서.
    pub docs: BTreeMap<i64, Value>,
    /// `docs`에 없는 ID에 줄 문서. book_fixtures의 `FakeEs(doc)`처럼 어떤 ID든 같은 문서를 줄 때 쓴다.
    pub default_doc: Option<Value>,
    pub update_result: bool,
    pub delete_result: bool,
    pub insert_returns_empty: bool,
    pub delete_by_ids_returns: Option<i64>,
    pub aggregate: BTreeMap<String, i64>,
    pub counts: BTreeMap<String, i64>,
    /// `iter_all_category_file_paths`가 접어서 돌려줄 (category, file_path) 목록.
    pub all_file_paths: Vec<(String, String)>,
    pub category_docs: Vec<Hit>,
    pub keyword: Vec<Hit>,
    pub similar: Vec<Hit>,
    pub similar_paged: (Vec<Hit>, i64),
    pub keyword_paged: (Vec<Hit>, i64),
    pub latest_docs: (Vec<Hit>, i64),
    pub deleted_by_category: Value,
    pub rename_result: Value,
    pub backfill_result: Value,
    pub categories_pdf_stats: Value,
    pub category_pdf_stats: Value,
    /// `get_by_id_source`/`search_by_path_source`가 줄 `_source`. 키는 ID 문자열과 경로다.
    pub source_by_id: HashMap<String, Value>,
    pub source_by_path: HashMap<String, Value>,
    // 기록
    pub updates: Vec<(i64, Value)>,
    pub deleted: Vec<i64>,
    pub inserted: Vec<Docs>,
    pub deleted_ids: Vec<i64>,
    pub deleted_file_paths: Vec<(Vec<String>, Vec<i64>)>,
    pub create_index_calls: usize,
    pub refresh_calls: usize,
    /// (메서드 이름, 인자 JSON). `MagicMock`의 `assert_called*`를 대신한다.
    pub calls: Vec<(String, Value)>,
    pub record_threads: bool,
    pub call_threads: HashMap<String, ThreadId>,
    faults: HashMap<String, Fault>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            docs: BTreeMap::new(),
            default_doc: None,
            update_result: true,
            delete_result: true,
            insert_returns_empty: false,
            delete_by_ids_returns: None,
            aggregate: BTreeMap::from([("A".to_string(), 1)]),
            counts: BTreeMap::from([("A".to_string(), 1), ("B".to_string(), 0)]),
            all_file_paths: Vec::new(),
            category_docs: Vec::new(),
            keyword: Vec::new(),
            similar: Vec::new(),
            similar_paged: (Vec::new(), 0),
            keyword_paged: (Vec::new(), 0),
            latest_docs: (Vec::new(), 0),
            deleted_by_category: json!({"deleted": 2, "failures": []}),
            rename_result: json!({"updated": 3, "failures": []}),
            backfill_result: json!({"updated": 0, "skipped": 0, "failed": 0}),
            categories_pdf_stats: json!({}),
            category_pdf_stats: json!({}),
            source_by_id: HashMap::new(),
            source_by_path: HashMap::new(),
            updates: Vec::new(),
            deleted: Vec::new(),
            inserted: Vec::new(),
            deleted_ids: Vec::new(),
            deleted_file_paths: Vec::new(),
            create_index_calls: 0,
            refresh_calls: 0,
            calls: Vec::new(),
            record_threads: false,
            call_threads: HashMap::new(),
            faults: HashMap::new(),
        }
    }
}

#[derive(Default)]
struct Hooks {
    search_by_id: Option<SearchByIdHook>,
    update: Option<UpdateHook>,
    delete: Option<DeleteHook>,
    insert: Option<InsertHook>,
    search_by_category: Option<SearchByCategoryHook>,
}

pub struct FakeEsManager {
    state: Mutex<State>,
    hooks: Mutex<Hooks>,
}

impl FakeEsManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State::default()), hooks: Mutex::new(Hooks::default()) })
    }

    /// 어떤 ID든 `doc`을 돌려주는 fake. book_fixtures, bm1, bm2의 `FakeEs(doc)`/`DummyEs(doc)`.
    pub fn with_doc(doc: Value) -> Arc<Self> {
        let es = Self::new();
        es.set_doc(Some(doc));
        es
    }

    /// 상태를 읽거나 바꾼다.
    pub fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    // ── 설정 ──

    /// `es.doc = ...`. `None`이면 없는 문서다.
    pub fn set_doc(&self, doc: Option<Value>) {
        self.with(|s| s.default_doc = doc);
    }

    /// ID 하나에 문서를 넣는다.
    pub fn put_doc(&self, id: i64, doc: Value) {
        self.with(|s| {
            s.docs.insert(id, doc);
        });
    }

    pub fn set_update_result(&self, ok: bool) {
        self.with(|s| s.update_result = ok);
    }

    pub fn set_delete_result(&self, ok: bool) {
        self.with(|s| s.delete_result = ok);
    }

    pub fn set_counts(&self, counts: &[(&str, i64)]) {
        self.with(|s| s.counts = counts.iter().map(|(k, v)| (k.to_string(), *v)).collect());
    }

    pub fn set_aggregate(&self, pairs: &[(&str, i64)]) {
        self.with(|s| s.aggregate = pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect());
    }

    pub fn set_category_docs(&self, docs: Vec<Hit>) {
        self.with(|s| s.category_docs = docs);
    }

    pub fn set_latest_docs(&self, docs: Vec<Hit>, total: i64) {
        self.with(|s| s.latest_docs = (docs, total));
    }

    pub fn set_categories_pdf_stats(&self, value: Value) {
        self.with(|s| s.categories_pdf_stats = value);
    }

    pub fn set_category_pdf_stats(&self, value: Value) {
        self.with(|s| s.category_pdf_stats = value);
    }

    /// `get_by_id_source(id)`가 줄 `_source`. 분류기 조회 테스트의 `by_inode`.
    pub fn put_source_by_id(&self, id: &str, source: Value) {
        self.with(|s| {
            s.source_by_id.insert(id.to_string(), source);
        });
    }

    /// `search_by_path_source(path)`가 줄 `_source`. 분류기 조회 테스트의 `by_path`.
    pub fn put_source_by_path(&self, path: &str, source: Value) {
        self.with(|s| {
            s.source_by_path.insert(path.to_string(), source);
        });
    }

    pub fn record_call_threads(&self) {
        self.with(|s| s.record_threads = true);
    }

    // ── 오류 주입 ──

    /// 이 메서드를 부르면 `Other(message)` 오류를 낸다. 메서드 이름은 trait 메서드 이름이다(`"update"`, `"insert"`, `"refresh"` ...).
    pub fn fail(&self, method: &str, message: &str) {
        let message = message.to_string();
        self.fail_with(method, move || EsError::Other(message.clone()));
    }

    /// 이 메서드를 부르면 `make()`가 만든 오류를 낸다.
    pub fn fail_with(&self, method: &str, make: impl Fn() -> EsError + Send + Sync + 'static) {
        self.with(|s| {
            s.faults.insert(method.to_string(), Arc::new(make));
        });
    }

    pub fn clear_fault(&self, method: &str) {
        self.with(|s| {
            s.faults.remove(method);
        });
    }

    // ── 클로저 바꿔치기. Python `es.search_by_id = lambda ...` ──

    pub fn on_search_by_id(&self, f: impl Fn(i64) -> Result<Option<Value>, EsError> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().search_by_id = Some(Arc::new(f));
    }

    pub fn on_update(&self, f: impl Fn(i64, &Value) -> Result<bool, EsError> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().update = Some(Arc::new(f));
    }

    pub fn on_delete(&self, f: impl Fn(i64) -> Result<bool, EsError> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().delete = Some(Arc::new(f));
    }

    pub fn on_insert(&self, f: impl Fn(&Docs) -> Result<Vec<i64>, EsError> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().insert = Some(Arc::new(f));
    }

    pub fn on_search_by_category(&self, f: impl Fn(&str, i64) -> Vec<Hit> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().search_by_category = Some(Arc::new(f));
    }

    // ── 기록 읽기 ──

    pub fn updates(&self) -> Vec<(i64, Value)> {
        self.with(|s| s.updates.clone())
    }

    pub fn deleted(&self) -> Vec<i64> {
        self.with(|s| s.deleted.clone())
    }

    pub fn inserted(&self) -> Vec<Docs> {
        self.with(|s| s.inserted.clone())
    }

    pub fn deleted_ids(&self) -> Vec<i64> {
        self.with(|s| s.deleted_ids.clone())
    }

    pub fn deleted_file_paths(&self) -> Vec<(Vec<String>, Vec<i64>)> {
        self.with(|s| s.deleted_file_paths.clone())
    }

    pub fn refresh_count(&self) -> usize {
        self.with(|s| s.refresh_calls)
    }

    pub fn create_index_count(&self) -> usize {
        self.with(|s| s.create_index_calls)
    }

    pub fn call_threads(&self) -> HashMap<String, ThreadId> {
        self.with(|s| s.call_threads.clone())
    }

    /// 호출 순서대로 (메서드 이름, 인자 JSON).
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.with(|s| s.calls.clone())
    }

    /// 한 메서드의 인자 JSON 목록.
    pub fn calls_of(&self, method: &str) -> Vec<Value> {
        self.with(|s| s.calls.iter().filter(|(m, _)| m == method).map(|(_, a)| a.clone()).collect())
    }

    pub fn call_count(&self, method: &str) -> usize {
        self.calls_of(method).len()
    }

    /// 호출을 기록하고 주입한 오류가 있으면 돌려준다. 모든 trait 메서드가 가장 먼저 부른다.
    fn enter(&self, method: &str, args: Value) -> Result<(), EsError> {
        let mut s = self.state.lock().unwrap();
        s.calls.push((method.to_string(), args));
        if s.record_threads {
            s.call_threads.insert(method.to_string(), std::thread::current().id());
        }
        match s.faults.get(method) {
            Some(make) => Err(make()),
            None => Ok(()),
        }
    }
}

impl EsManagerApi for FakeEsManager {
    fn create_index(&self) -> Result<Value, EsError> {
        self.enter("create_index", json!([]))?;
        self.with(|s| s.create_index_calls += 1);
        Ok(json!({"acknowledged": true}))
    }

    fn search_by_id(&self, doc_id: i64) -> Result<Option<Value>, EsError> {
        self.enter("search_by_id", json!([doc_id]))?;
        let hook = self.hooks.lock().unwrap().search_by_id.clone();
        if let Some(hook) = hook {
            return hook(doc_id);
        }
        Ok(self.with(|s| s.docs.get(&doc_id).cloned().or_else(|| s.default_doc.clone())))
    }

    fn update(&self, doc_id: i64, fields: &Value) -> Result<bool, EsError> {
        self.enter("update", json!([doc_id, fields]))?;
        self.with(|s| s.updates.push((doc_id, fields.clone())));
        let hook = self.hooks.lock().unwrap().update.clone();
        match hook {
            Some(hook) => hook(doc_id, fields),
            None => Ok(self.with(|s| s.update_result)),
        }
    }

    fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        self.enter("delete", json!([doc_id]))?;
        self.with(|s| s.deleted.push(doc_id));
        let hook = self.hooks.lock().unwrap().delete.clone();
        match hook {
            Some(hook) => hook(doc_id),
            None => Ok(self.with(|s| s.delete_result)),
        }
    }

    fn insert(&self, docs: &Docs) -> Result<Vec<i64>, EsError> {
        self.enter("insert", json!(docs))?;
        let hook = self.hooks.lock().unwrap().insert.clone();
        if let Some(hook) = hook {
            return hook(docs);
        }
        self.with(|s| {
            s.inserted.push(docs.clone());
            if s.insert_returns_empty { Ok(Vec::new()) } else { Ok(docs.keys().copied().collect()) }
        })
    }

    fn delete_by_file_paths(&self, paths: &[String], exclude_ids: Option<&[i64]>) -> Result<i64, EsError> {
        self.enter("delete_by_file_paths", json!([paths, exclude_ids]))?;
        self.with(|s| s.deleted_file_paths.push((paths.to_vec(), exclude_ids.unwrap_or(&[]).to_vec())));
        Ok(0)
    }

    fn delete_by_ids(&self, ids: &[i64], chunk_size: usize) -> Result<i64, EsError> {
        self.enter("delete_by_ids", json!([ids, chunk_size]))?;
        self.with(|s| {
            if let Some(n) = s.delete_by_ids_returns {
                return Ok(n);
            }
            s.deleted_ids.extend_from_slice(ids);
            Ok(ids.len() as i64)
        })
    }

    fn refresh(&self) -> Result<(), EsError> {
        self.enter("refresh", json!([]))?;
        self.with(|s| s.refresh_calls += 1);
        Ok(())
    }

    fn search_and_aggregate_by_category(&self) -> Result<BTreeMap<String, i64>, EsError> {
        self.enter("search_and_aggregate_by_category", json!([]))?;
        Ok(self.with(|s| s.aggregate.clone()))
    }

    /// 실제 composite 집계처럼 (category, file_path) 조합별로 접어서 문서 수와 함께 준다.
    fn iter_all_category_file_paths(&self, batch: usize) -> Result<Vec<(String, String, i64)>, EsError> {
        self.enter("iter_all_category_file_paths", json!([batch]))?;
        Ok(self.with(|s| {
            let mut counts: BTreeMap<(String, String), i64> = BTreeMap::new();
            for key in &s.all_file_paths {
                *counts.entry(key.clone()).or_insert(0) += 1;
            }
            counts.into_iter().map(|((category, file_path), n)| (category, file_path, n)).collect()
        }))
    }

    fn search_categories_pdf_stats(&self, categories: &[String]) -> Result<Value, EsError> {
        self.enter("search_categories_pdf_stats", json!([categories]))?;
        Ok(self.with(|s| s.categories_pdf_stats.clone()))
    }

    fn search_category_pdf_stats(&self, category: &str) -> Result<Value, EsError> {
        self.enter("search_category_pdf_stats", json!([category]))?;
        Ok(self.with(|s| s.category_pdf_stats.clone()))
    }

    fn get_by_id_source(&self, id: &str) -> Result<Option<Value>, EsError> {
        self.enter("get_by_id_source", json!([id]))?;
        Ok(self.with(|s| s.source_by_id.get(id).cloned()))
    }

    fn search_by_path_source(&self, path: &str) -> Result<Option<Value>, EsError> {
        self.enter("search_by_path_source", json!([path]))?;
        Ok(self.with(|s| s.source_by_path.get(path).cloned()))
    }

    // ── 문서가 "나머지"로 둔 메서드 (모양은 파일 머리말 참고) ──

    fn search_by_category(&self, category: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        self.enter("search_by_category", json!([category, max_result_count]))?;
        let hook = self.hooks.lock().unwrap().search_by_category.clone();
        match hook {
            Some(hook) => Ok(hook(category, max_result_count)),
            None => Ok(self.with(|s| s.category_docs.clone())),
        }
    }

    /// `search_after`의 첫 값을 시작 위치로 읽는 단순 페이지 나누기. 다음 쪽이 있으면 `[다음 시작 위치]`를 돌려준다.
    fn search_by_category_paged(&self, category: &str, size: i64, search_after: Option<&[Value]>) -> Result<(Vec<Hit>, i64, Option<Vec<Value>>), EsError> {
        self.enter("search_by_category_paged", json!([category, size, search_after]))?;
        Ok(self.with(|s| {
            let start = search_after.and_then(|a| a.first()).and_then(Value::as_u64).unwrap_or(0) as usize;
            let size = size.max(0) as usize;
            let end = (start + size).min(s.category_docs.len());
            let page: Vec<Hit> = s.category_docs.get(start..end).unwrap_or(&[]).to_vec();
            let next_start = start + page.len();
            let next = if page.len() == size && next_start < s.category_docs.len() { Some(vec![json!(next_start)]) } else { None };
            (page, s.category_docs.len() as i64, next)
        }))
    }

    fn count_by_categories(&self, categories: &[String], prefix: bool) -> Result<BTreeMap<String, i64>, EsError> {
        self.enter("count_by_categories", json!([categories, prefix]))?;
        Ok(self.with(|s| {
            let total = |cat: &str| -> i64 {
                if !prefix {
                    return s.counts.get(cat).copied().unwrap_or(0);
                }
                s.counts.iter().filter(|(c, _)| c.as_str() == cat || c.starts_with(&format!("{cat}/"))).map(|(_, n)| *n).sum()
            };
            categories.iter().map(|c| (c.clone(), total(c))).collect()
        }))
    }

    fn count_by_category(&self, category: &str, prefix: bool) -> Result<i64, EsError> {
        self.enter("count_by_category", json!([category, prefix]))?;
        Ok(self.with(|s| s.counts.get(category).copied().unwrap_or(0)))
    }

    fn rename_category(&self, old_category: &str, new_category: &str) -> Result<Value, EsError> {
        self.enter("rename_category", json!([old_category, new_category]))?;
        Ok(self.with(|s| s.rename_result.clone()))
    }

    fn delete_by_category(&self, category: &str, prefix: bool) -> Result<Value, EsError> {
        self.enter("delete_by_category", json!([category, prefix]))?;
        Ok(self.with(|s| s.deleted_by_category.clone()))
    }

    fn search_by_keyword(&self, keyword: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        self.enter("search_by_keyword", json!([keyword, max_result_count]))?;
        Ok(self.with(|s| s.keyword.clone()))
    }

    /// 인자는 `{keyword, size, offset, exclude_categories, category}` 형태의 JSON으로 받는다고 가정한다.
    fn search_by_keyword_paged(&self, args: &Value) -> Result<(Vec<Hit>, i64), EsError> {
        self.enter("search_by_keyword_paged", args.clone())?;
        Ok(self.with(|s| s.keyword_paged.clone()))
    }

    fn search_similar_docs(&self, args: &Value) -> Result<Vec<Hit>, EsError> {
        self.enter("search_similar_docs", args.clone())?;
        Ok(self.with(|s| s.similar.clone()))
    }

    fn search_similar_docs_paged(&self, args: &Value) -> Result<(Vec<Hit>, i64), EsError> {
        self.enter("search_similar_docs_paged", args.clone())?;
        Ok(self.with(|s| s.similar_paged.clone()))
    }

    fn search_latest_docs(&self, max_result_count: i64, exclude_categories: Option<&[String]>) -> Result<(Vec<Hit>, i64), EsError> {
        self.enter("search_latest_docs", json!([max_result_count, exclude_categories]))?;
        Ok(self.with(|s| s.latest_docs.clone()))
    }

    fn backfill_created_time(&self, path_prefix: &Path) -> Result<Value, EsError> {
        self.enter("backfill_created_time", json!([path_prefix.to_string_lossy()]))?;
        Ok(self.with(|s| s.backfill_result.clone()))
    }
}
