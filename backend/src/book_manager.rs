//! 책 관리자. Python `backend/book_manager.py`의 `BookManager`에 대응한다.
//!
//! # 구조
//!
//! - 이 파일: `BookManager`(상태), `BookManagerDeps`(생성 때 꽂는 대역과 설정), 주입 trait(`FileReader`), 상수, 생성자.
//! - `book_manager/*.rs`: 기능별 하위 모듈. 각 파일이 `impl BookManager { ... }` 블록 하나를 맡는다.
//!   하위 모듈은 이 파일의 비공개 필드(`self.fs`, `self.es` ...)에 바로 접근한다.
//! - `book_manager/api.rs`: `impl BookManagerApi for BookManager`. 라우트가 쓰는 trait 메서드를 inherent 메서드에 이어 준다.
//!
//! # 기능을 덧붙이는 방법
//!
//! 1. `book_manager/<기능>.rs`를 만들고 이 파일에 `mod <기능>;`을 한 줄 추가한다.
//! 2. 새 파일에서 `impl BookManager { pub async fn ... }`을 쓴다. 필요한 대역은 `self.fs`, `self.runner`, `self.es`, `self.reader()`로 쓴다.
//! 3. 새 설정이나 대역이 필요하면 이 파일의 `BookManagerDeps` 필드를 늘리지 말고, 새 파일에 설정 구조체를 두고
//!    `BookManagerDeps::with_ext(값)`으로 꽂는다. 읽을 때는 `self.ext::<타입>()`이다(없으면 `None`, 보통 `unwrap_or_default`).
//! 4. 매니저가 소유하는 캐시는 `self.state::<타입>()`이다. `Default`로 처음 쓸 때 만들고 `Arc`로 돌려준다(내부 가변성은 직접 둔다).
//! 5. 라우트가 부르는 메서드면 `book_manager/api.rs`에 `BookManagerApi` 구현을 한 줄 덧붙인다(trait 메서드는 `main/traits.rs`에 있다).
//!
//! 규칙(docs/rust-test-seams.md 3.0): 빌더는 소비형 `with_xxx`, 정수는 `i64`, 모든 trait는 `Send + Sync`,
//! `set_*`/`override_*`/전역 가드 금지. 한 번 만든 뒤에는 대역을 바꾸지 않는다.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use sysdeps::{CommandRunner, EnvFn, FsOps, RealFs, SystemRunner};

use crate::book::Book;
use crate::book_classifier::SharedBookClassifier;
use crate::classifier::Classify;
use crate::es_manager::{Docs, EsManagerApi};
use crate::pdf::{LopdfOpener, LopdfPageOps, PageCountCache, PdfOpener, PdfPageOps, PdfReaderCache};

pub use crate::comics_manager::RuntimeError;
pub use crate::main::{BookContent, BookManagerApi, BookResponse};

mod api;
mod backfill;
mod catalog;
mod category;
mod classify;
mod cover_list;
mod cover;
mod epub;
mod file_reader;
mod html;
mod mismatch;
mod paths;
mod pdf_cover;
mod pdf_edit;
mod preview;
mod reload;
mod seams;
mod update;
mod uncovered;
mod util;
mod validate;
mod xml_safe;

pub use cover_list::CoverPageOpts;
pub use uncovered::{CoverBooksPager, CoverPage};
pub use classify::{ApplyOptions, MODEL_CERTAIN_MIN_CONFIDENCE, MoveOpts, ProposeOptions};
pub use cover::{BookLookup, BookRef, Thumbnailer};
pub use file_reader::DefaultFileReader;
pub use seams::{BackfillThread, BookAdder, BookManagerError, CategoryNameCheck, PathIsRelativeTo, PreviewEpubValidator};
pub use xml_safe::{SafeXmlParser, XmlElement, _declare_unbound_prefixes, _prune_dead_ncx_navpoints, _safe_xml_parser, decode_category_cursor, encode_category_cursor};

/// 카테고리 단위 ES 조회 상한. ES `max_result_window`(기본 10000)와 같다.
pub const MAX_CATEGORY_RESULT_COUNT: i64 = 10000;
/// 커서 기반 카테고리 조회의 요청당 최대 페이지 크기.
pub const MAX_CATEGORY_PAGE_SIZE: i64 = MAX_CATEGORY_RESULT_COUNT;
/// 카테고리 불일치 재적재 때 파일을 묶어서 색인하는 배치 크기.
pub const BULK_REINDEX_BATCH_SIZE: usize = 200;
pub const DUPLICATE_NAME_MAX_INDEX: usize = 1000;
/// 카테고리 불일치 요약 캐시의 유효 시간(초).
pub const MISMATCH_CACHE_TTL_SECONDS: u64 = 600;
/// 대량 재적재 때 파일 하나를 파싱하는 최대 시간(초).
pub const HARD_PARSE_TIMEOUT_SECONDS: u64 = 150;
pub const MAX_LATEST_BOOK_COUNT: i64 = 2000;
pub const CREATED_TIME_BACKFILL_ENV: &str = "TM_BACKFILL_CREATED_TIME_ON_STARTUP";
/// 캐시 파일 보존 기간(초).
pub const CACHE_MAX_AGE_SECONDS: u64 = 86400;
/// EPUB 뷰어용 이미지의 긴 변 상한(픽셀).
pub const EPUB_IMAGE_MAX_EDGE: u32 = 1600;

/// 진행 보고 콜백. 값은 JSON 객체다.
pub type ProgressFn<'a> = &'a (dyn Fn(Value) + Send + Sync);

/// 파일 하나를 읽어 ES 문서로 만든다. Python `Loader.read_file`에 대응한다(구 `ReadFile`).
///
/// 실제 구현은 `loader` crate의 `read_file`을 감싸야 한다. 이 crate는 `loader`에 의존하지 않으므로
/// 조립하는 쪽(바이너리)이 `BookManagerDeps::with_file_reader`로 꽂는다. 꽂지 않으면 `DefaultFileReader`가 쓰인다.
/// `FileReader::try_read_file`이 돌려주는 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadFileError {
    /// 파서가 시간 상한을 넘었다(Python `ParserTimeout`).
    Timeout(String),
    Other(String),
}

pub trait FileReader: Send + Sync {
    /// 파일이 아니거나 지원하지 않는 형식이면 빈 묶음이다.
    fn read_file(&self, path: &Path, path_prefix: Option<&Path>) -> Docs;

    /// `skip_text`는 본문 추출을 건너뛴다(만화). 기본 구현은 `read_file`에 맡긴다.
    fn read_file_with_options(&self, path: &Path, path_prefix: Option<&Path>, skip_text: bool) -> Docs {
        let _ = skip_text;
        self.read_file(path, path_prefix)
    }

    /// 실패 사유를 구분해 읽는다. 기본 구현은 실패를 빈 묶음으로 표현하는 `read_file_with_options`에 맡긴다.
    fn try_read_file(&self, path: &Path, path_prefix: Option<&Path>, skip_text: bool) -> Result<Docs, ReadFileError> {
        Ok(self.read_file_with_options(path, path_prefix, skip_text))
    }
}

pub type ClassifierFactory = Arc<dyn Fn() -> Arc<dyn Classify> + Send + Sync>;
/// 서점 득표·정책·보정을 가진 분류 서비스를 돌려준다. `None` 이면 분류기가 없는 것이다. 서비스는 매니저 안에서 한 번만 만든다.
pub type ClassifierServiceFactory = Arc<dyn Fn() -> Option<Arc<SharedBookClassifier>> + Send + Sync>;
pub type MismatchOverride = Arc<dyn Fn(bool) -> Value + Send + Sync>;
pub type DetailsOverride = Arc<dyn Fn(&str) -> Value + Send + Sync>;
pub type IterFilesOverride = Arc<dyn Fn(&str, bool) -> Vec<PathBuf> + Send + Sync>;
pub type HighConfidenceOverride = Arc<dyn Fn(&dyn Any, f64) -> bool + Send + Sync>;

/// 하위 모듈이 덧붙이는 설정과 대역을 담는 칸. 타입 하나에 값 하나다.
#[derive(Clone, Default)]
struct Extensions {
    map: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Extensions {
    fn insert<T: Any + Send + Sync>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Arc::new(value));
    }

    fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.map.get(&TypeId::of::<T>()).cloned().and_then(|v| v.downcast::<T>().ok())
    }
}

/// `BookManager`를 만들 때 꽂는 대역과 설정. 소비형 빌더다.
#[derive(Clone)]
pub struct BookManagerDeps {
    path_prefix: PathBuf,
    es: Arc<dyn EsManagerApi>,
    fs: Arc<dyn FsOps>,
    runner: Arc<dyn CommandRunner>,
    pdf_opener: Arc<dyn PdfOpener>,
    pdf_pages: Arc<dyn PdfPageOps>,
    file_reader: Option<Arc<dyn FileReader>>,
    classifier_factory: Option<ClassifierFactory>,
    classifier_service_factory: Option<ClassifierServiceFactory>,
    bulk_reindex_batch_size: usize,
    scan_observer: Option<Arc<dyn Fn() + Send + Sync>>,
    get_category_mismatches_override: Option<MismatchOverride>,
    get_category_mismatch_details_override: Option<DetailsOverride>,
    iter_category_indexable_files_override: Option<IterFilesOverride>,
    is_high_confidence_override: Option<HighConfidenceOverride>,
    loader_program: String,
    duplicate_name_max_index: usize,
    max_category_result_count: i64,
    ext: Extensions,
}

impl BookManagerDeps {
    /// 실제 파일시스템, 실제 외부 명령, `lopdf` PDF 구현이 기본값이다.
    pub fn new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>) -> Self {
        Self {
            path_prefix,
            es,
            fs: Arc::new(RealFs),
            runner: Arc::new(SystemRunner),
            pdf_opener: Arc::new(LopdfOpener),
            pdf_pages: Arc::new(LopdfPageOps),
            file_reader: None,
            classifier_factory: None,
            classifier_service_factory: None,
            bulk_reindex_batch_size: BULK_REINDEX_BATCH_SIZE,
            scan_observer: None,
            get_category_mismatches_override: None,
            get_category_mismatch_details_override: None,
            iter_category_indexable_files_override: None,
            is_high_confidence_override: None,
            loader_program: "loader".to_string(),
            duplicate_name_max_index: DUPLICATE_NAME_MAX_INDEX,
            max_category_result_count: MAX_CATEGORY_RESULT_COUNT,
            ext: Extensions::default(),
        }
    }

    pub fn with_fs(mut self, fs: Arc<dyn FsOps>) -> Self {
        self.fs = fs;
        self
    }

    pub fn with_runner(mut self, runner: Arc<dyn CommandRunner>) -> Self {
        self.runner = runner;
        self
    }

    pub fn with_pdf_opener(mut self, opener: Arc<dyn PdfOpener>) -> Self {
        self.pdf_opener = opener;
        self
    }

    pub fn with_pdf_pages(mut self, pages: Arc<dyn PdfPageOps>) -> Self {
        self.pdf_pages = pages;
        self
    }

    pub fn with_file_reader(mut self, reader: Arc<dyn FileReader>) -> Self {
        self.file_reader = Some(reader);
        self
    }

    pub fn with_classifier_factory(mut self, factory: ClassifierFactory) -> Self {
        self.classifier_factory = Some(factory);
        self
    }

    /// 분류 서비스(서점 득표·정책·보정 포함)를 꽂는다. 안 꽂으면 `TM_CLASSIFIER_MODEL_DIR` 의 모델로 기본 서비스를 만든다.
    /// `with_classifier_factory`(모델 단독 대역)가 있으면 그쪽이 먼저다.
    pub fn with_classifier_service_factory(mut self, factory: ClassifierServiceFactory) -> Self {
        self.classifier_service_factory = Some(factory);
        self
    }

    /// 불일치 재적재가 한 번에 색인하는 파일 수. 0이면 1로 본다.
    pub fn with_bulk_reindex_batch_size(mut self, n: usize) -> Self {
        self.bulk_reindex_batch_size = n.max(1);
        self
    }

    /// 불일치 요약 스캔을 시작하기 직전에 부른다(스캔 횟수 관찰, 지연 주입).
    pub fn with_scan_observer(mut self, observer: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.scan_observer = Some(observer);
        self
    }

    /// `get_category_mismatches`를 통째로 바꾼다. 캐시를 거치지 않는다.
    pub fn with_get_category_mismatches_override(mut self, f: MismatchOverride) -> Self {
        self.get_category_mismatches_override = Some(f);
        self
    }

    pub fn with_get_category_mismatch_details_override(mut self, f: DetailsOverride) -> Self {
        self.get_category_mismatch_details_override = Some(f);
        self
    }

    pub fn with_iter_category_indexable_files_override(mut self, f: IterFilesOverride) -> Self {
        self.iter_category_indexable_files_override = Some(f);
        self
    }

    /// 모델 점수가 '확실'인지 판정하는 함수를 바꾼다. 첫 인자는 분류기(`Arc<dyn Classify>`를 `&dyn Any`로 본 것)다.
    pub fn with_is_high_confidence_override(mut self, f: HighConfidenceOverride) -> Self {
        self.is_high_confidence_override = Some(f);
        self
    }

    /// 카테고리 전체 재적재(`reload_category`)가 실행할 loader 실행 파일. 기본은 PATH의 `loader`다.
    pub fn with_loader_program(mut self, program: &str) -> Self {
        self.loader_program = program.to_string();
        self
    }

    /// `name (N).ext` 번호를 붙일 수 있는 최대 N. 이 값까지 다 차면 옮기지 않고 오류를 낸다(기본 1000).
    pub fn with_duplicate_name_max_index(mut self, n: usize) -> Self {
        self.duplicate_name_max_index = n;
        self
    }

    /// 카테고리 단위 ES 조회 상한(기본 `MAX_CATEGORY_RESULT_COUNT`). 값 설정이다.
    pub fn with_max_category_result_count(mut self, n: i64) -> Self {
        self.max_category_result_count = n.max(1);
        self
    }

    /// 하위 모듈이 정의한 설정이나 대역을 꽂는다. 같은 타입을 다시 꽂으면 덮어쓴다.
    pub fn with_ext<T: Any + Send + Sync>(mut self, value: T) -> Self {
        self.ext.insert(value);
        self
    }
}

/// 불일치 요약 캐시. `time`이 없으면 캐시가 없는 것과 같다.
#[derive(Default)]
struct MismatchState {
    cache: Option<Value>,
    time: Option<Instant>,
}

impl MismatchState {
    fn fresh(&self) -> Option<&Value> {
        match (&self.cache, self.time) {
            (Some(cache), Some(time)) if time.elapsed() < Duration::from_secs(MISMATCH_CACHE_TTL_SECONDS) => Some(cache),
            _ => None,
        }
    }
}

pub struct BookManager {
    path_prefix: PathBuf,
    es: Arc<dyn EsManagerApi>,
    fs: Arc<dyn FsOps>,
    runner: Arc<dyn CommandRunner>,
    pdf_opener: Arc<dyn PdfOpener>,
    pdf_pages: Arc<dyn PdfPageOps>,
    file_reader: Option<Arc<dyn FileReader>>,
    classifier_factory: Option<ClassifierFactory>,
    classifier_service_factory: Option<ClassifierServiceFactory>,
    bulk_reindex_batch_size: usize,
    scan_observer: Option<Arc<dyn Fn() + Send + Sync>>,
    get_category_mismatches_override: Option<MismatchOverride>,
    get_category_mismatch_details_override: Option<DetailsOverride>,
    iter_category_indexable_files_override: Option<IterFilesOverride>,
    is_high_confidence_override: Option<HighConfidenceOverride>,
    loader_program: String,
    duplicate_name_max_index: usize,
    max_category_result_count: i64,
    ext: Extensions,
    /// 분류 서비스. 처음 쓸 때 한 번 만든다(모델 로드가 무거워 매니저 생성에서는 읽지 않는다).
    classifier_service: std::sync::OnceLock<Option<Arc<SharedBookClassifier>>>,
    state: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
    mismatch: Mutex<MismatchState>,
    scan_lock: Mutex<()>,
    pdf_cache: PdfReaderCache,
    page_count_cache: PageCountCache,
    backfill_thread: Mutex<Option<Arc<dyn BackfillThread>>>,
}

impl BookManager {
    pub const CACHE_MAX_AGE_SECONDS: u64 = CACHE_MAX_AGE_SECONDS;
    /// 이미지 축소 정책 버전. 캐시 파일 이름에 붙여 정책이 바뀌면 옛 산출물이 계속 나가지 않게 한다.
    pub const EPUB_IMAGE_POLICY_VERSION: u32 = 3;
    pub const EPUB_IMAGE_MAX_EDGE: u32 = EPUB_IMAGE_MAX_EDGE;
    /// 최상위 파일을 가리키는 카테고리 이름.
    pub const ROOT_CATEGORY: &'static str = "_root";

    /// 대역을 꽂아 만든다. ES 색인 생성(`create_index`)과 백필은 하지 않는다(`from_env`가 한다).
    pub fn with_deps(deps: BookManagerDeps) -> BookManager {
        let seams = deps.seams();
        BookManager {
            pdf_cache: PdfReaderCache::new(seams.pdf_cache_limits.unwrap_or_default()),
            page_count_cache: PageCountCache::new(seams.page_count_cache_max.unwrap_or(PageCountCache::DEFAULT_MAX)),
            path_prefix: deps.path_prefix,
            es: deps.es,
            fs: deps.fs,
            runner: deps.runner,
            pdf_opener: deps.pdf_opener,
            pdf_pages: deps.pdf_pages,
            file_reader: deps.file_reader,
            classifier_factory: deps.classifier_factory,
            classifier_service_factory: deps.classifier_service_factory,
            classifier_service: std::sync::OnceLock::new(),
            bulk_reindex_batch_size: deps.bulk_reindex_batch_size,
            scan_observer: deps.scan_observer,
            get_category_mismatches_override: deps.get_category_mismatches_override,
            get_category_mismatch_details_override: deps.get_category_mismatch_details_override,
            iter_category_indexable_files_override: deps.iter_category_indexable_files_override,
            is_high_confidence_override: deps.is_high_confidence_override,
            loader_program: deps.loader_program,
            duplicate_name_max_index: deps.duplicate_name_max_index,
            max_category_result_count: deps.max_category_result_count,
            ext: deps.ext,
            state: Mutex::new(HashMap::new()),
            mismatch: Mutex::new(MismatchState::default()),
            scan_lock: Mutex::new(()),
            backfill_thread: Mutex::new(None),
        }
    }

    /// 환경 변수로 만든다. `TM_BOOK_DIR`이 없으면 오류다. `es_factory`는 색인 이름(`TM_ES_BOOK_INDEX`, 없으면 빈 문자열)을 받는다.
    pub fn from_env(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>) -> Result<BookManager, RuntimeError> {
        Self::from_env_with(env, es_factory, |deps| deps)
    }

    /// `from_env`와 같으나, 기본 대역 묶음을 `customize`로 고쳐 쓴다(예: loader 기반 `FileReader`를 꽂는다).
    pub fn from_env_with(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>, customize: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Result<BookManager, RuntimeError> {
        let Some(dir) = env("TM_BOOK_DIR") else {
            return Err(RuntimeError("The environment variable TM_BOOK_DIR is not set.".to_string()));
        };
        let path_prefix = PathBuf::from(dir);
        log::debug!("{}", path_prefix.display());
        let index_name = env("TM_ES_BOOK_INDEX").unwrap_or_default();
        let es = es_factory(&index_name);
        es.create_index().map_err(|e| RuntimeError(format!("create_index failed: {e}")))?;
        let mut deps = BookManagerDeps::new(path_prefix, es);
        if let Some(program) = env("TM_LOADER_BIN").filter(|p| !p.is_empty()) {
            deps = deps.with_loader_program(&program);
        }
        let manager = BookManager::with_deps(customize(deps));
        manager.start_created_time_backfill(matches!(env(CREATED_TIME_BACKFILL_ENV).unwrap_or_default().trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on"), &index_name);
        Ok(manager)
    }

    pub fn path_prefix(&self) -> PathBuf {
        self.path_prefix.clone()
    }

    /// 책 루트를 바꾼다. 테스트가 프로덕션 디렉터리 전체 스캔을 피하려고 쓴다. 생성 뒤 대역 교체가 아니라 값 설정이다.
    pub fn set_path_prefix(&mut self, path: &Path) {
        self.path_prefix = path.to_path_buf();
        self.clear_mismatch_cache();
    }

    pub fn es_manager(&self) -> &dyn EsManagerApi {
        &*self.es
    }

    /// 하위 모듈이 `BookManagerDeps::with_ext`로 꽂은 값.
    pub fn ext<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.ext.get::<T>()
    }

    /// 하위 모듈이 소유하는 가변 상태. 처음 쓸 때 `Default`로 만든다.
    pub fn state<T: Default + Any + Send + Sync>(&self) -> Arc<T> {
        let mut slots = self.state.lock().unwrap();
        let slot = slots.entry(TypeId::of::<T>()).or_insert_with(|| Arc::new(T::default()));
        slot.clone().downcast::<T>().expect("TypeId가 같으면 타입도 같다")
    }

    /// 파일 읽기 대역. 꽂지 않았으면 `DefaultFileReader`다.
    fn reader(&self) -> Arc<dyn FileReader> {
        match &self.file_reader {
            Some(reader) => reader.clone(),
            None => Arc::new(DefaultFileReader::new(self.path_prefix.clone())),
        }
    }

    /// ES 문서에서 `Book`을 만든다. 전역 `Book::path_prefix`를 쓰지 않고 이 매니저의 루트를 쓴다.
    fn make_book(&self, book_id: i64, info: &Value, score: f64) -> Book {
        util::book_from_doc(&self.path_prefix, book_id, info, score)
    }
}
