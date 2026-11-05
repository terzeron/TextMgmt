//! 만화 관리자.
//!
//! 만화 루트와 ES 색인을 따로 보관하고 도서 처리 기능은 `BookManager`에 위임한다.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde_json::Value;
use sysdeps::EnvFn;

use crate::BookManagerApi;
use crate::{BookManager, BookManagerDeps};
use search_manager::EsManagerApi;

mod api_adapter;

pub use crate::BookManagerError;

const CREATED_TIME_BACKFILL_ENV: &str = "TM_BACKFILL_CREATED_TIME_ON_STARTUP";

fn env_flag_enabled(env: EnvFn, name: &str) -> bool {
    matches!(env(name).unwrap_or_default().trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

pub struct ComicsManager {
    inner: BookManager,
    path_prefix: PathBuf,
    es_index_name: String,
    es_manager: Arc<dyn EsManagerApi>,
    backfill_thread: Mutex<Option<JoinHandle<()>>>,
    mismatch_cache: Option<Value>,
    mismatch_cache_time: f64,
    mismatch_state_lock: Mutex<()>,
    mismatch_scan_lock: Mutex<()>,
}

impl ComicsManager {
    /// 책과 분리된 경로·ES·테스트 대역으로 만화 관리자를 만든다.
    pub fn with_deps(deps: BookManagerDeps) -> Self {
        let inner = BookManager::with_deps(deps);
        Self {
            path_prefix: inner.path_prefix(),
            es_manager: BookManagerApi::es_manager(&inner),
            inner,
            es_index_name: String::new(),
            backfill_thread: Mutex::new(None),
            mismatch_cache: None,
            mismatch_cache_time: 0.0,
            mismatch_state_lock: Mutex::new(()),
            mismatch_scan_lock: Mutex::new(()),
        }
    }

    pub fn from_env(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>) -> Result<ComicsManager, BookManagerError> {
        Self::from_env_with(env, es_factory, |deps| deps)
    }

    /// `from_env`와 같고, 운영용 reader 등 manager 의존성을 구성할 때 꽂는다.
    /// `TM_LOADER_BIN`도 `from_env_with_es`와 같이 읽는다.
    pub fn from_env_with(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>, customize: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Result<ComicsManager, BookManagerError> {
        // 디렉터리·색인 설정이 없으면 ES에 연결하기 전에 멈춘다.
        let (_, index_name) = Self::comics_env(env)?;
        Self::from_env_with_es(env, es_factory(&index_name), customize)
    }

    fn comics_env(env: EnvFn) -> Result<(PathBuf, String), BookManagerError> {
        let missing = |name: &str| BookManagerError(format!("The environment variable {name} is not set."));
        let dir = env("TM_COMICS_DIR").ok_or_else(|| missing("TM_COMICS_DIR"))?;
        let index_name = env("TM_ES_COMICS_INDEX").ok_or_else(|| missing("TM_ES_COMICS_INDEX"))?;
        Ok((PathBuf::from(dir), index_name))
    }

    /// 이미 연결한 ES manager로 운영 인스턴스를 만든다.
    pub fn from_env_with_es(env: EnvFn, es_manager: Arc<dyn EsManagerApi>, customize: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Result<ComicsManager, BookManagerError> {
        let (path_prefix, index_name) = Self::comics_env(env)?;
        log::debug!("{}", path_prefix.display());
        es_manager.create_index().map_err(|e| BookManagerError(format!("create_index failed: {e}")))?;
        let mut deps = BookManagerDeps::new(path_prefix, es_manager);
        if let Some(program) = env("TM_LOADER_BIN").filter(|p| !p.is_empty()) {
            deps = deps.with_loader_program(&program);
        }
        let mut manager = Self::with_deps(customize(deps));
        manager.es_index_name = index_name;
        manager.backfill_created_time_if_enabled(env);
        Ok(manager)
    }

    fn backfill_created_time_if_enabled(&self, env: EnvFn) {
        if !env_flag_enabled(env, CREATED_TIME_BACKFILL_ENV) {
            return;
        }
        let mut slot = self.backfill_thread.lock().unwrap();
        if slot.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let es = self.es_manager.clone();
        let prefix = self.path_prefix.clone();
        let spawned = std::thread::Builder::new().name(format!("created-time-backfill-{}", self.es_index_name)).spawn(move || {
            if let Err(e) = es.backfill_created_time(&prefix) {
                log::warn!("created_time backfill failed: {e}");
            }
        });
        match spawned {
            Ok(handle) => *slot = Some(handle),
            Err(e) => log::warn!("created_time backfill failed to start: {e}"),
        }
    }

    /// 백필 스레드가 끝나기를 기다린다. 테스트와 종료 처리에 쓴다.
    pub fn join_backfill(&self) {
        let handle = self.backfill_thread.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    pub fn path_prefix(&self) -> &Path {
        &self.path_prefix
    }

    pub fn es_index_name(&self) -> &str {
        &self.es_index_name
    }

    pub fn es_manager(&self) -> &Arc<dyn EsManagerApi> {
        &self.es_manager
    }

    pub fn mismatch_cache(&self) -> Option<&Value> {
        self.mismatch_cache.as_ref()
    }

    pub fn mismatch_cache_time(&self) -> f64 {
        self.mismatch_cache_time
    }

    pub fn set_mismatch_cache(&mut self, cache: Value) {
        self.mismatch_cache = Some(cache);
        // 0.0은 "없음"으로 쓰므로 단조 시각에 1초를 더한다.
        self.mismatch_cache_time = sysdeps::Clock::monotonic(&sysdeps::SystemClock) + 1.0;
    }

    pub fn clear_mismatch_cache(&mut self) {
        self.mismatch_cache = None;
        self.mismatch_cache_time = 0.0;
    }

    pub fn mismatch_state_lock(&self) -> &Mutex<()> {
        &self.mismatch_state_lock
    }

    pub fn mismatch_scan_lock(&self) -> &Mutex<()> {
        &self.mismatch_scan_lock
    }
}
