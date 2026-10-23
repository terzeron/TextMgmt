//! 앱 의존성 묶음 `AppDeps`와 인증 방식 `AuthMode`. 설계: docs/rust-test-seams.md 3.2.
//!
//! `AppDeps::new(auth)`는 실제 구현을 기본값으로 채우고, 소비형 `with_xxx`로 하나씩 대역으로 바꾼다.
//! 무거운 의존성(DB 연결, 매니저)은 처음 쓸 때 만든다(Python `_LazyProxy`). 만들기에 실패하면 다음 호출에서 다시 시도한다.
//! 라우트는 `Arc<AppDeps>`를 state로 받아 `deps.refresh_store()` 같은 접근자로 꺼내 쓴다.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use sysdeps::{Clock, FsOps, RealFs, SystemClock};

use super::traits::{
    BookAccess, BookManagerApi, CategoryMappingApi, GoogleVerifier, TaskRunner, ThreadRunner, UnavailableBookAccess, UnavailableBookManager, UnavailableGoogleVerifier, ViewHistoryApi,
};
use crate::auth::{EnvRoleResolver, JwtIssuer, RoleResolver, TokenIssuer};
use crate::category_mapping::CategoryMapping;
use crate::cover_management::CoverHooks;
use crate::refresh_token_store::{RefreshTokenStoreApi, create_refresh_token_store};
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

type Factory<T> = Box<dyn Fn() -> Result<Arc<T>, String> + Send + Sync>;

/// 주입된 값 또는 처음 쓸 때 만드는 값. 만들기에 실패하면 캐시하지 않고 다음 호출에서 다시 시도한다.
pub struct Slot<T: ?Sized> {
    value: Mutex<Option<Arc<T>>>,
    factory: Option<Factory<T>>,
}

impl<T: ?Sized> Slot<T> {
    pub fn ready(value: Arc<T>) -> Self {
        Self { value: Mutex::new(Some(value)), factory: None }
    }

    pub fn lazy(factory: impl Fn() -> Result<Arc<T>, String> + Send + Sync + 'static) -> Self {
        Self { value: Mutex::new(None), factory: Some(Box::new(factory)) }
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
    cover_hooks: CoverHooks,
    reload_progress_flush_interval: Duration,
}

/// 재적재 진행 카운트를 DB에 기록하는 기본 주기(Python `RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS`).
pub const RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS: u64 = 30;

impl AppDeps {
    /// 나머지는 실제 구현(또는 환경 변수 기본값)으로 채운다.
    ///
    /// 책/만화 관리자 기본값은 아직 "구성되지 않음"이다. 실제 `BookManager`가 생기면 여기서 `Slot::lazy`로 연결한다.
    pub fn new(auth: AuthMode) -> Self {
        Self {
            auth,
            role_resolver: Arc::new(EnvRoleResolver),
            tokens: Arc::new(JwtIssuer),
            google: Arc::new(UnavailableGoogleVerifier),
            refresh_store: Slot::lazy(|| create_refresh_token_store().map(|s| Arc::new(s) as Arc<dyn RefreshTokenStoreApi>).map_err(|e| e.to_string())),
            view_history: Slot::lazy(|| create_view_history_store().map(|s| Arc::new(s) as Arc<dyn ViewHistoryApi>).map_err(|e| e.to_string())),
            category_mapping: Slot::lazy(|| CategoryMapping::new().map(|s| Arc::new(s) as Arc<dyn CategoryMappingApi>).map_err(|e| e.to_string())),
            book_manager: Slot::ready(Arc::new(UnavailableBookManager)),
            comics_manager: Slot::ready(Arc::new(UnavailableBookManager)),
            book_access: Arc::new(UnavailableBookAccess),
            fs: Arc::new(RealFs),
            clock: Arc::new(SystemClock),
            task_runner: Arc::new(ThreadRunner),
            cover_hooks: CoverHooks::default(),
            reload_progress_flush_interval: Duration::from_secs(RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS),
        }
    }

    /// 운영용 기본 조립. 환경 변수는 각 구현이 쓸 때 읽는다.
    pub fn from_env() -> Self {
        Self::new(AuthMode::Cookies)
    }

    pub fn with_book_manager(mut self, m: Arc<dyn BookManagerApi>) -> Self {
        self.book_manager = Slot::ready(m);
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

    /// 표지 관리 라우트가 쓰는 검색·내려받기·시계 등 바깥 의존성.
    pub fn with_cover_hooks(mut self, hooks: CoverHooks) -> Self {
        self.cover_hooks = hooks;
        self
    }

    pub fn with_reload_progress_flush_interval(mut self, d: Duration) -> Self {
        self.reload_progress_flush_interval = d;
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
