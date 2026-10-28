//! 앱 의존성 묶음 `AppDeps`와 인증 방식 `AuthMode`. 설계: docs/rust-test-seams.md 3.2.
//!
//! `AppDeps::new(auth)`는 실제 구현을 기본값으로 채우고, 소비형 `with_xxx`로 하나씩 대역으로 바꾼다.
//! 무거운 의존성(DB 연결, 매니저)은 처음 쓸 때 만든다(Python `_LazyProxy`). 만들기에 실패하면 다음 호출에서 다시 시도한다.
//! 라우트는 `Arc<AppDeps>`를 state로 받아 `deps.refresh_store()` 같은 접근자로 꺼내 쓴다.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use sysdeps::{Clock, EnvFn, FsOps, RealFs, SystemClock};

use super::google::GoogleIdTokenVerifier;
use super::traits::{BookAccess, BookManagerApi, BookStub, CategoryMappingApi, GoogleVerifier, TaskRunner, ThreadRunner, UnavailableBookAccess, UnavailableBookManager, ViewHistoryApi};
use crate::auth::HTTPException;
use crate::auth::{EnvRoleResolver, JwtIssuer, RoleResolver, TokenIssuer};
use crate::book_manager::{BookManager, LoaderFileReader};
use crate::bookstore::BookstoreSearch;
use crate::category_mapping::CategoryMapping;
use crate::comics_manager::ComicsManager;
use crate::cover_management::CoverHooks;
use crate::es_manager::{ESManager, EsManagerApi, RealEsConnector};
use crate::refresh_token_store::{RefreshTokenStoreApi, create_refresh_token_store};

type TaskRuntimeFactory = Arc<dyn Fn() -> Result<tokio::runtime::Runtime, std::io::Error> + Send + Sync>;
use crate::view_history_store::create_view_history_store;

/// 테스트 중간에 바꿀 수 있는 인증 payload. `AuthMode::Shared`가 쓴다.
pub struct AuthSlots {
    pub auth: Mutex<Value>,
    pub admin: Mutex<Value>,
}

impl AuthSlots {
    pub fn new(auth: Value, admin: Value) -> Self {
        Self { auth: Mutex::new(auth), admin: Mutex::new(admin) }
    }
}

/// `require_auth`/`require_admin` 의존성을 어떻게 풀지(FastAPI `dependency_overrides`에 대응한다).
pub enum AuthMode {
    /// 실제 쿠키/Bearer 토큰 인증.
    Cookies,
    /// 둘 다 고정 payload.
    Fixed(Value),
    /// `require_auth`와 `require_admin`을 각각 고정.
    Split { auth: Value, admin: Value },
    /// `require_auth`만 고정하고 `require_admin`은 실제 인증.
    AuthOnly(Value),
    /// 테스트 중간에 바꾸는 값.
    Shared(Arc<AuthSlots>),
}

type Factory<T> = dyn Fn() -> Result<Arc<T>, String> + Send + Sync;
pub type BookstoreFactory = Arc<dyn Fn(&str) -> Option<Box<dyn BookstoreSearch>> + Send + Sync>;

/// 주입된 값 또는 처음 쓸 때 만드는 값. 만들기에 실패하면 캐시하지 않고 다음 호출에서 다시 시도한다.
pub struct Slot<T: ?Sized> {
    value: Arc<Mutex<Option<Arc<T>>>>,
    factory: Option<Arc<Factory<T>>>,
}

impl<T: ?Sized> Clone for Slot<T> {
    fn clone(&self) -> Self {
        Self { value: self.value.clone(), factory: self.factory.clone() }
    }
}

impl<T: ?Sized> Slot<T> {
    pub fn ready(value: Arc<T>) -> Self {
        Self { value: Arc::new(Mutex::new(Some(value))), factory: None }
    }

    pub fn lazy(factory: impl Fn() -> Result<Arc<T>, String> + Send + Sync + 'static) -> Self {
        Self { value: Arc::new(Mutex::new(None)), factory: Some(Arc::new(factory)) }
    }

    pub fn get(&self) -> Result<Arc<T>, String> {
        let mut guard = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = guard.as_ref() {
            return Ok(v.clone());
        }
        let factory = self.factory.as_ref().ok_or_else(|| "dependency is not configured".to_string())?;
        let made = factory()?;
        *guard = Some(made.clone());
        Ok(made)
    }
}

pub struct AppDeps {
    auth: AuthMode,
    role_resolver: Arc<dyn RoleResolver>,
    tokens: Arc<dyn TokenIssuer>,
    google: Arc<dyn GoogleVerifier>,
    refresh_store: Slot<dyn RefreshTokenStoreApi>,
    view_history: Slot<dyn ViewHistoryApi>,
    category_mapping: Slot<dyn CategoryMappingApi>,
    book_manager: Slot<dyn BookManagerApi>,
    comics_manager: Slot<dyn BookManagerApi>,
    book_access: Arc<dyn BookAccess>,
    fs: Arc<dyn FsOps>,
    clock: Arc<dyn Clock>,
    task_runner: Arc<dyn TaskRunner>,
    task_runtime_factory: TaskRuntimeFactory,
    cover_hooks: CoverHooks,
    reload_progress_flush_interval: Duration,
    bookstore_factory: BookstoreFactory,
}

/// 재적재 진행 카운트를 DB에 기록하는 기본 주기(Python `RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS`).
pub const RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS: u64 = 30;

impl AppDeps {
    /// 나머지는 실제 구현(또는 환경 변수 기본값)으로 채운다.
    ///
    /// 테스트와 부분 조립에서 쓸 빈 의존성 묶음이다.
    pub fn new(auth: AuthMode) -> Self {
        Self {
            auth,
            role_resolver: Arc::new(EnvRoleResolver),
            tokens: Arc::new(JwtIssuer),
            google: Arc::new(GoogleIdTokenVerifier::default()),
            refresh_store: Slot::lazy(|| create_refresh_token_store().map(|s| Arc::new(s) as Arc<dyn RefreshTokenStoreApi>).map_err(|e| e.to_string())),
            view_history: Slot::lazy(|| create_view_history_store().map(|s| Arc::new(s) as Arc<dyn ViewHistoryApi>).map_err(|e| e.to_string())),
            category_mapping: Slot::lazy(|| CategoryMapping::new().map(|s| Arc::new(s) as Arc<dyn CategoryMappingApi>).map_err(|e| e.to_string())),
            book_manager: Slot::ready(Arc::new(UnavailableBookManager)),
            comics_manager: Slot::ready(Arc::new(UnavailableBookManager)),
            book_access: Arc::new(UnavailableBookAccess),
            fs: Arc::new(RealFs),
            clock: Arc::new(SystemClock),
            task_runner: Arc::new(ThreadRunner),
            task_runtime_factory: Arc::new(|| tokio::runtime::Builder::new_current_thread().enable_all().build()),
            cover_hooks: CoverHooks::default(),
            reload_progress_flush_interval: Duration::from_secs(RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS),
            bookstore_factory: Arc::new(crate::bookstore::create_bookstore_search),
        }
    }

    /// 운영용 기본 조립. 환경 변수는 각 구현이 쓸 때 읽는다.
    pub fn from_env() -> Self {
        let mut deps = Self::new(AuthMode::Cookies);
        deps.book_manager = Slot::lazy(create_runtime_book_manager);
        deps.comics_manager = Slot::lazy(create_runtime_comics_manager);
        deps.book_access = Arc::new(RuntimeBookAccess::new(deps.book_manager.clone(), deps.comics_manager.clone(), deps.category_mapping.clone()));
        deps
    }

    pub fn with_book_manager(mut self, m: Arc<dyn BookManagerApi>) -> Self {
        self.book_manager = Slot::ready(m);
        self
    }

    /// BookManager 지연 초기화 factory를 지정한다.
    pub fn with_book_manager_factory(mut self, factory: impl Fn() -> Result<Arc<dyn BookManagerApi>, String> + Send + Sync + 'static) -> Self {
        self.book_manager = Slot::lazy(factory);
        self
    }

    pub fn with_comics_manager(mut self, m: Arc<dyn BookManagerApi>) -> Self {
        self.comics_manager = Slot::ready(m);
        self
    }

    pub fn with_category_mapping(mut self, m: Arc<dyn CategoryMappingApi>) -> Self {
        self.category_mapping = Slot::ready(m);
        self
    }

    /// CategoryMapping 지연 초기화 factory를 지정한다.
    pub fn with_category_mapping_factory(mut self, factory: impl Fn() -> Result<Arc<dyn CategoryMappingApi>, String> + Send + Sync + 'static) -> Self {
        self.category_mapping = Slot::lazy(factory);
        self
    }

    pub fn with_view_history_store(mut self, s: Arc<dyn ViewHistoryApi>) -> Self {
        self.view_history = Slot::ready(s);
        self
    }

    pub fn with_book_access(mut self, a: Arc<dyn BookAccess>) -> Self {
        self.book_access = a;
        self
    }

    pub fn with_refresh_token_store(mut self, s: Arc<dyn RefreshTokenStoreApi>) -> Self {
        self.refresh_store = Slot::ready(s);
        self
    }

    pub fn with_google_verifier(mut self, v: Arc<dyn GoogleVerifier>) -> Self {
        self.google = v;
        self
    }

    pub fn with_role_resolver(mut self, r: Arc<dyn RoleResolver>) -> Self {
        self.role_resolver = r;
        self
    }

    pub fn with_token_issuer(mut self, t: Arc<dyn TokenIssuer>) -> Self {
        self.tokens = t;
        self
    }

    pub fn with_fs(mut self, fs: Arc<dyn FsOps>) -> Self {
        self.fs = fs;
        self
    }

    pub fn with_clock(mut self, c: Arc<dyn Clock>) -> Self {
        self.clock = c;
        self
    }

    pub fn with_task_runner(mut self, r: Arc<dyn TaskRunner>) -> Self {
        self.task_runner = r;
        self
    }

    /// 백그라운드 작업용 Tokio runtime 생성기를 바꾼다.
    pub fn with_task_runtime_factory(mut self, factory: impl Fn() -> Result<tokio::runtime::Runtime, std::io::Error> + Send + Sync + 'static) -> Self {
        self.task_runtime_factory = Arc::new(factory);
        self
    }

    pub(crate) fn task_runtime_factory(&self) -> TaskRuntimeFactory {
        self.task_runtime_factory.clone()
    }

    /// 표지 관리 라우트가 쓰는 검색·내려받기·시계 등 바깥 의존성.
    pub fn with_cover_hooks(mut self, hooks: CoverHooks) -> Self {
        self.cover_hooks = hooks;
        self
    }

    pub fn with_reload_progress_flush_interval(mut self, d: Duration) -> Self {
        self.reload_progress_flush_interval = d;
        self
    }

    pub fn with_bookstore_factory(mut self, factory: BookstoreFactory) -> Self {
        self.bookstore_factory = factory;
        self
    }

    // ---- 접근자: 라우트가 쓴다 ----

    pub fn auth_mode(&self) -> &AuthMode {
        &self.auth
    }

    pub fn role_resolver(&self) -> &dyn RoleResolver {
        self.role_resolver.as_ref()
    }

    pub fn tokens(&self) -> &dyn TokenIssuer {
        self.tokens.as_ref()
    }

    pub fn google(&self) -> Arc<dyn GoogleVerifier> {
        self.google.clone()
    }

    pub fn refresh_store(&self) -> Result<Arc<dyn RefreshTokenStoreApi>, String> {
        self.refresh_store.get()
    }

    pub fn view_history(&self) -> Result<Arc<dyn ViewHistoryApi>, String> {
        self.view_history.get()
    }

    pub fn category_mapping(&self) -> Result<Arc<dyn CategoryMappingApi>, String> {
        self.category_mapping.get()
    }

    pub fn book_manager(&self) -> Result<Arc<dyn BookManagerApi>, String> {
        self.book_manager.get()
    }

    pub fn comics_manager(&self) -> Result<Arc<dyn BookManagerApi>, String> {
        self.comics_manager.get()
    }

    pub fn book_access(&self) -> Arc<dyn BookAccess> {
        self.book_access.clone()
    }

    pub fn fs(&self) -> Arc<dyn FsOps> {
        self.fs.clone()
    }

    pub fn bookstore(&self, name: &str) -> Option<Box<dyn BookstoreSearch>> {
        (self.bookstore_factory)(name)
    }

    pub fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }

    pub fn task_runner(&self) -> Arc<dyn TaskRunner> {
        self.task_runner.clone()
    }

    pub fn cover_hooks(&self) -> CoverHooks {
        self.cover_hooks.clone()
    }

    pub fn reload_progress_flush_interval(&self) -> Duration {
        self.reload_progress_flush_interval
    }
}

struct RuntimeBookAccess {
    books: Slot<dyn BookManagerApi>,
    comics: Slot<dyn BookManagerApi>,
    categories: Slot<dyn CategoryMappingApi>,
}

impl RuntimeBookAccess {
    fn new(books: Slot<dyn BookManagerApi>, comics: Slot<dyn BookManagerApi>, categories: Slot<dyn CategoryMappingApi>) -> Self {
        Self { books, comics, categories }
    }
}

#[async_trait::async_trait]
impl BookAccess for RuntimeBookAccess {
    async fn get_book_and_ensure_viewer_access(&self, book_id: i64, payload: &Value, content_type: &str) -> Result<BookStub, HTTPException> {
        let manager = if content_type == "comic" { &self.comics } else { &self.books };
        let manager = manager.get().map_err(|error| HTTPException::new(503, &error))?;
        let book = manager.get_book(book_id).await.map_err(|error| HTTPException::new(404, &error.to_string()))?;

        if payload.get("role").and_then(Value::as_str) == Some("viewer") {
            let categories = self.categories.get().map_err(|error| HTTPException::new(503, &error))?;
            let hidden = categories.get_hidden_categories(content_type).map_err(|error| HTTPException::new(500, &error.to_string()))?;
            if super::routes_books::is_hidden(&book.category, &hidden) {
                return Err(HTTPException::new(403, "접근 권한이 없는 카테고리입니다."));
            }
        }

        Ok(BookStub { title: book.title, category: Some(book.category) })
    }
}

fn build_book_manager(env: EnvFn<'_>, es: Arc<dyn EsManagerApi>) -> Result<Arc<dyn BookManagerApi>, String> {
    let book_root = env("TM_BOOK_DIR").map(PathBuf::from).unwrap_or_default();
    let comics_root = env("TM_COMICS_DIR").map(PathBuf::from).unwrap_or_default();
    let reader = Arc::new(LoaderFileReader::new(book_root, comics_root));
    BookManager::from_env_with_es(env, es, |deps| deps.with_file_reader(reader)).map(|manager| Arc::new(manager) as Arc<dyn BookManagerApi>).map_err(|e| e.to_string())
}

fn build_comics_manager(env: EnvFn<'_>, es: Arc<dyn EsManagerApi>) -> Result<Arc<dyn BookManagerApi>, String> {
    let book_root = env("TM_BOOK_DIR").map(PathBuf::from).unwrap_or_default();
    let comics_root = env("TM_COMICS_DIR").map(PathBuf::from).unwrap_or_default();
    let reader = Arc::new(LoaderFileReader::new(book_root, comics_root));
    ComicsManager::from_env_with_es(env, es, |deps| deps.with_file_reader(reader)).map(|manager| Arc::new(manager) as Arc<dyn BookManagerApi>).map_err(|e| e.to_string())
}

fn connect_runtime_es(env: EnvFn<'_>, index_name: &str) -> Result<Arc<dyn EsManagerApi>, String> {
    ESManager::connect_index(env, index_name, &RealEsConnector, &SystemClock).map(|es| Arc::new(es) as Arc<dyn EsManagerApi>).map_err(|e| e.to_string())
}

fn create_runtime_book_manager() -> Result<Arc<dyn BookManagerApi>, String> {
    let env = |key: &str| std::env::var(key).ok();
    if env("TM_BOOK_DIR").is_none() {
        return Err("The environment variable TM_BOOK_DIR is not set.".to_string());
    }
    let index_name = env("TM_ES_BOOK_INDEX").unwrap_or_default();
    build_book_manager(&env, connect_runtime_es(&env, &index_name)?)
}

fn create_runtime_comics_manager() -> Result<Arc<dyn BookManagerApi>, String> {
    let env = |key: &str| std::env::var(key).ok();
    if env("TM_COMICS_DIR").is_none() {
        return Err("The environment variable TM_COMICS_DIR is not set.".to_string());
    }
    let index_name = env("TM_ES_COMICS_INDEX").ok_or_else(|| "The environment variable TM_ES_COMICS_INDEX is not set.".to_string())?;
    build_comics_manager(&env, connect_runtime_es(&env, &index_name)?)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};

    use super::{RuntimeBookAccess, Slot, build_book_manager, build_comics_manager};
    use crate::book::{Book, BookLookupError};
    use crate::es_manager::{EsError, EsManagerApi};
    use crate::main::{BookAccess, BookManagerApi, BookStub, CategoryMappingApi};
    use async_trait::async_trait;

    struct CountingEs(AtomicUsize);

    impl EsManagerApi for CountingEs {
        fn create_index(&self) -> Result<Value, EsError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"acknowledged": true}))
        }
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let values: HashMap<String, String> = pairs.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
        move |key| values.get(key).cloned()
    }

    #[test]
    fn runtime_book_manager_uses_loader_ready_manager_and_es() {
        let es = std::sync::Arc::new(CountingEs(AtomicUsize::new(0)));
        let get = env(&[("TM_BOOK_DIR", "/books"), ("TM_COMICS_DIR", "/comics")]);
        let manager = build_book_manager(&get, es.clone()).unwrap();
        assert_eq!(manager.path_prefix(), std::path::Path::new("/books"));
        assert_eq!(es.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn runtime_comics_manager_uses_comics_root_and_es() {
        let es = std::sync::Arc::new(CountingEs(AtomicUsize::new(0)));
        let get = env(&[("TM_BOOK_DIR", "/books"), ("TM_COMICS_DIR", "/comics"), ("TM_ES_COMICS_INDEX", "comics")]);
        let manager = build_comics_manager(&get, es.clone()).unwrap();
        assert_eq!(manager.path_prefix(), std::path::Path::new("/comics"));
        assert_eq!(es.0.load(Ordering::SeqCst), 1);
    }

    struct FixedBookManager(Book);

    #[async_trait]
    impl BookManagerApi for FixedBookManager {
        async fn get_book(&self, _id: i64) -> Result<Book, BookLookupError> {
            Ok(self.0.clone())
        }
    }

    struct FixedHiddenCategories(Vec<String>);

    impl CategoryMappingApi for FixedHiddenCategories {
        fn get_hidden_categories(&self, _content_type: &str) -> Result<Vec<String>, crate::main::MappingError> {
            Ok(self.0.clone())
        }
    }

    fn test_book(category: &str) -> Book {
        let timestamp = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
        Book {
            book_id: 1,
            category: category.into(),
            title: "title".into(),
            author: "author".into(),
            file_path: PathBuf::from("/books/title.epub"),
            file_type: "epub".into(),
            file_size: 1,
            line_count: 0,
            page_count: 0,
            isbn: String::new(),
            summary: String::new(),
            created_time: timestamp,
            updated_time: timestamp,
            score: 0.0,
        }
    }

    #[tokio::test]
    async fn runtime_book_access_denies_hidden_category_and_descendants_for_viewers() {
        let manager: Arc<dyn BookManagerApi> = Arc::new(FixedBookManager(test_book("소설/판타지/무협")));
        let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FixedHiddenCategories(vec!["소설/판타지".into()]));
        let access = RuntimeBookAccess::new(Slot::ready(manager.clone()), Slot::ready(manager.clone()), Slot::ready(mapping));

        let result = access.get_book_and_ensure_viewer_access(1, &json!({"role": "viewer"}), "book").await;

        assert_eq!(result.unwrap_err().status_code, 403);
    }

    #[tokio::test]
    async fn runtime_book_access_allows_admin_without_loading_hidden_categories() {
        let manager: Arc<dyn BookManagerApi> = Arc::new(FixedBookManager(test_book("소설/판타지")));
        let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FixedHiddenCategories(vec!["소설".into()]));
        let access = RuntimeBookAccess::new(Slot::ready(manager.clone()), Slot::ready(manager), Slot::ready(mapping));

        let book = access.get_book_and_ensure_viewer_access(1, &json!({"role": "admin"}), "book").await.unwrap();

        assert_eq!(book, BookStub { title: "title".into(), category: Some("소설/판타지".into()) });
    }

    #[tokio::test]
    async fn runtime_book_access_does_not_hide_an_empty_category() {
        let manager: Arc<dyn BookManagerApi> = Arc::new(FixedBookManager(test_book("")));
        let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FixedHiddenCategories(vec![String::new()]));
        let access = RuntimeBookAccess::new(Slot::ready(manager.clone()), Slot::ready(manager), Slot::ready(mapping));

        let result = access.get_book_and_ensure_viewer_access(1, &json!({"role": "viewer"}), "book").await;

        assert!(result.is_ok());
    }
}
