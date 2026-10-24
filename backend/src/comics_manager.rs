//! 만화 관리자. Python `backend/comics_manager.py`에 대응한다.
//!
//! 만화 루트와 ES 색인을 따로 보관하고 도서 처리 기능은 `BookManager`에 위임한다.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

use serde_json::Value;
use sysdeps::EnvFn;

use crate::book_manager::{BookManager, BookManagerDeps};
use crate::es_manager::EsManagerApi;
use crate::main::BookManagerApi;

mod api;

/// Python `RuntimeError`에 대응한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError(pub String);

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RuntimeError {}

const CREATED_TIME_BACKFILL_ENV: &str = "TM_BACKFILL_CREATED_TIME_ON_STARTUP";

fn env_flag_enabled(env: EnvFn, name: &str) -> bool {
    matches!(env(name).unwrap_or_default().trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

/// Python `time.monotonic()`처럼 단조 증가하는 초. 0.0은 "없음"으로 쓰므로 프로세스 시작 시점에서 1초를 더한다.
fn monotonic_seconds() -> f64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() + 1.0
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

    pub fn from_env(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>) -> Result<ComicsManager, RuntimeError> {
        let Some(dir) = env("TM_COMICS_DIR") else {
            return Err(RuntimeError("The environment variable TM_COMICS_DIR is not set.".to_string()));
        };
        let Some(index_name) = env("TM_ES_COMICS_INDEX") else {
            return Err(RuntimeError("The environment variable TM_ES_COMICS_INDEX is not set.".to_string()));
        };
        let path_prefix = PathBuf::from(dir);
        log::debug!("{}", path_prefix.display());
        let es_manager = es_factory(&index_name);
        es_manager.create_index().map_err(|e| RuntimeError(format!("create_index failed: {e}")))?;
        let mut manager = Self::with_deps(BookManagerDeps::new(path_prefix, es_manager));
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
        self.mismatch_cache_time = monotonic_seconds();
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
