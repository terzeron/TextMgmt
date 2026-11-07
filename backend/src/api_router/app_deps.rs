//! 앱 의존성 묶음 `AppDeps`와 인증 방식 `AuthMode`. 설계: docs/rust-test-seams.md 3.2.
//!
//! `AppDeps::new(auth)`는 실제 구현을 기본값으로 채우고, 소비형 `with_xxx`로 하나씩 대역으로 바꾼다.
//! 무거운 의존성(DB 연결, 매니저)은 처음 쓸 때 만든다. 만들기에 실패하면 다음 호출에서 다시 시도한다.
//! 라우트는 `Arc<AppDeps>`를 state로 받아 `deps.refresh_store()` 같은 접근자로 꺼내 쓴다.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysdeps::{Clock, EnvFn, FsOps, RealFs, SystemClock};

use auth_manager::auth_service::{Claims, HttpStatusError};
use auth_manager::auth_service::{EnvRoleResolver, JwtIssuer, RoleResolver, TokenIssuer};
use auth_manager::category_access_checker::{BookAccess, BookLookup, BookStub, UnavailableBookAccess, ViewerBookAccess};
use auth_manager::google_token_verifier::{GoogleIdTokenVerifier, GoogleVerifier};
use auth_manager::refresh_token_store::{RefreshTokenStoreApi, create_refresh_token_store};
use book_manager::comics_manager::ComicsManager;
use book_manager::{BookManager, LoaderFileReader};
use book_manager::{BookManagerApi, UnavailableBookManager};
use bookstore::store_clients::BookstoreSearch;
use cover_manager::cover_management::CoverHooks;
use db::category_mapping::CategoryMapping;
use db::category_mapping::CategoryMappingApi;
use db::view_history_store::ViewHistoryApi;
use search_manager::{ESManager, EsManagerApi, RealEsConnector};
use sysdeps::task_runner::{TaskRunner, ThreadRunner};

type TaskRuntimeFactory = Arc<dyn Fn() -> Result<tokio::runtime::Runtime, std::io::Error> + Send + Sync>;
use db::view_history_store::create_view_history_store;

/// 테스트 중간에 바꿀 수 있는 인증 payload. `AuthMode::Shared`가 쓴다.
pub struct AuthSlots {
    pub auth: Mutex<Claims>,
    pub admin: Mutex<Claims>,
}

impl AuthSlots {
    pub fn new(auth: Claims, admin: Claims) -> Self {
        Self { auth: Mutex::new(auth), admin: Mutex::new(admin) }
    }
}

/// `require_auth`/`require_admin` 의존성을 어떻게 풀지(테스트가 실제 인증을 대역으로 바꿀 때 쓴다).
pub enum AuthMode {
    /// 실제 쿠키/Bearer 토큰 인증.
    Cookies,
    /// 둘 다 고정 payload.
    Fixed(Claims),
    /// `require_auth`와 `require_admin`을 각각 고정.
    Split { auth: Claims, admin: Claims },
    /// `require_auth`만 고정하고 `require_admin`은 실제 인증.
    AuthOnly(Claims),
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

/// 재적재 진행 카운트를 DB에 기록하는 기본 주기.
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
            bookstore_factory: Arc::new(bookstore::store_clients::create_bookstore_search),
        }
    }

    /// 운영용 기본 조립. 환경 변수는 각 구현이 쓸 때 읽는다.
    pub fn from_env() -> Self {
        let mut deps = Self::new(AuthMode::Cookies);
        deps.book_manager = Slot::lazy(create_runtime_book_manager);
        deps.comics_manager = Slot::lazy(create_runtime_comics_manager);
        deps.book_access = Arc::new(runtime_book_access(deps.book_manager.clone(), deps.comics_manager.clone(), deps.category_mapping.clone()));
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

/// 운영용 책 찾기: 만화면 만화 관리자, 아니면 책 관리자에서 찾는다.
struct ManagerBookLookup {
    books: Slot<dyn BookManagerApi>,
    comics: Slot<dyn BookManagerApi>,
}

#[async_trait::async_trait]
impl BookLookup for ManagerBookLookup {
    async fn find_book(&self, book_id: i64, content_type: &str) -> Result<BookStub, HttpStatusError> {
        let manager = if content_type == "comic" { &self.comics } else { &self.books };
        let manager = manager.get().map_err(|error| HttpStatusError::new(503, &error))?;
        let book = manager.get_book(book_id).await.map_err(|error| HttpStatusError::new(404, &error.to_string()))?;
        Ok(BookStub { title: book.title, category: Some(book.category) })
    }
}

/// 운영용 책 접근 확인. 정책은 `auth_manager::category_access_checker`가 적용한다.
fn runtime_book_access(books: Slot<dyn BookManagerApi>, comics: Slot<dyn BookManagerApi>, categories: Slot<dyn CategoryMappingApi>) -> ViewerBookAccess {
    ViewerBookAccess::new(Arc::new(ManagerBookLookup { books, comics }), Arc::new(move || categories.get()))
}

/// 책·만화 디렉터리를 아는 loader 기반 파일 reader.
fn loader_file_reader(env: EnvFn<'_>) -> Arc<LoaderFileReader> {
    let book_root = env("TM_BOOK_DIR").map(PathBuf::from).unwrap_or_default();
    let comics_root = env("TM_COMICS_DIR").map(PathBuf::from).unwrap_or_default();
    Arc::new(LoaderFileReader::new(book_root, comics_root))
}

fn build_book_manager(env: EnvFn<'_>, es: Arc<dyn EsManagerApi>) -> Result<Arc<dyn BookManagerApi>, String> {
    let reader = loader_file_reader(env);
    BookManager::from_env_with_es(env, es, |deps| deps.with_file_reader(reader)).map(|manager| Arc::new(manager) as Arc<dyn BookManagerApi>).map_err(|e| e.to_string())
}

fn build_comics_manager(env: EnvFn<'_>, es: Arc<dyn EsManagerApi>) -> Result<Arc<dyn BookManagerApi>, String> {
    let reader = loader_file_reader(env);
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

    use super::{Claims, Slot, build_book_manager, build_comics_manager, runtime_book_access};
    use crate::api_router::{BookAccess, BookManagerApi, BookStub, CategoryMappingApi};
    use async_trait::async_trait;
    use book_manager::book_metadata::{Book, BookLookupError};
    use search_manager::{EsError, EsManagerApi};

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
        fn get_hidden_categories(&self, _content_type: &str) -> Result<Vec<String>, crate::api_router::MappingError> {
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
        let access = runtime_book_access(Slot::ready(manager.clone()), Slot::ready(manager.clone()), Slot::ready(mapping));

        let result = access.get_book_and_ensure_viewer_access(1, &Claims::with_role("viewer"), "book").await;

        assert_eq!(result.unwrap_err().status_code, 403);
    }

    #[tokio::test]
    async fn runtime_book_access_allows_admin_without_loading_hidden_categories() {
        let manager: Arc<dyn BookManagerApi> = Arc::new(FixedBookManager(test_book("소설/판타지")));
        let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FixedHiddenCategories(vec!["소설".into()]));
        let access = runtime_book_access(Slot::ready(manager.clone()), Slot::ready(manager), Slot::ready(mapping));

        let book = access.get_book_and_ensure_viewer_access(1, &Claims::with_role("admin"), "book").await.unwrap();

        assert_eq!(book, BookStub { title: "title".into(), category: Some("소설/판타지".into()) });
    }

    #[tokio::test]
    async fn runtime_book_access_reports_an_unavailable_manager() {
        let unavailable: Slot<dyn BookManagerApi> = Slot::lazy(|| Err("만화 저장소 없음".to_string()));
        let manager: Arc<dyn BookManagerApi> = Arc::new(FixedBookManager(test_book("")));
        let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FixedHiddenCategories(Vec::new()));
        let access = runtime_book_access(Slot::ready(manager), unavailable, Slot::ready(mapping));

        let error = access.get_book_and_ensure_viewer_access(1, &Claims::with_role("admin"), "comic").await.unwrap_err();

        assert_eq!(error.status_code, 503);
        // 저장소는 있지만 책을 찾지 못하면 404다.
        let missing: Arc<dyn BookManagerApi> = Arc::new(crate::api_router::UnavailableBookManager);
        let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FixedHiddenCategories(Vec::new()));
        let access = runtime_book_access(Slot::ready(missing.clone()), Slot::ready(missing), Slot::ready(mapping));
        let error = access.get_book_and_ensure_viewer_access(1, &Claims::with_role("admin"), "book").await.unwrap_err();
        assert_eq!(error.status_code, 404);
    }

    // 프로세스 환경 변수를 읽으므로 경우를 한 테스트 안에서 차례로 본다. ES 설정이 없으면 연결 전에 실패한다.
    #[test]
    fn runtime_managers_report_missing_configuration_before_connecting() {
        let error = |result: Result<Arc<dyn BookManagerApi>, String>| result.err().unwrap_or_default();
        // SAFETY: 이 lib 단위 테스트 바이너리에서 아래 변수를 쓰는 테스트는 이것 하나다.
        unsafe {
            for key in ["TM_BOOK_DIR", "TM_COMICS_DIR", "TM_ES_COMICS_INDEX", "TM_ES_BOOK_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"] {
                std::env::remove_var(key);
            }
        }
        assert!(error(super::create_runtime_book_manager()).contains("TM_BOOK_DIR"));
        assert!(error(super::create_runtime_comics_manager()).contains("TM_COMICS_DIR"));
        unsafe {
            std::env::set_var("TM_BOOK_DIR", "/books");
            std::env::set_var("TM_COMICS_DIR", "/comics");
        }
        assert!(error(super::create_runtime_comics_manager()).contains("TM_ES_COMICS_INDEX"));
        assert!(!error(super::create_runtime_book_manager()).is_empty(), "ES 설정 없이 책 저장소를 만들었다");
        unsafe { std::env::set_var("TM_ES_COMICS_INDEX", "comics") };
        assert!(!error(super::create_runtime_comics_manager()).is_empty(), "ES 설정 없이 만화 저장소를 만들었다");
        unsafe {
            for key in ["TM_BOOK_DIR", "TM_COMICS_DIR", "TM_ES_COMICS_INDEX"] {
                std::env::remove_var(key);
            }
        }
    }

    #[tokio::test]
    async fn runtime_book_access_does_not_hide_an_empty_category() {
        let manager: Arc<dyn BookManagerApi> = Arc::new(FixedBookManager(test_book("")));
        let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FixedHiddenCategories(vec![String::new()]));
        let access = runtime_book_access(Slot::ready(manager.clone()), Slot::ready(manager), Slot::ready(mapping));

        let result = access.get_book_and_ensure_viewer_access(1, &Claims::with_role("viewer"), "book").await;

        assert!(result.is_ok());
    }
}
