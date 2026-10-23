//! 환경 변수 가드. auth, main, main_mock, refresh_token_store, book_manager_2, classifier가 따로 만든 6개 `EnvGuard`를 하나로 합쳤다.
//! 설계: docs/rust-test-seams.md 3.9.
//!
//! - 값이 test binary당 고정이면 `common::setup_env()`로 충분하다.
//! - 테스트마다 값이 바뀌면 `EnvGuard`를 쓴다. 쓰기 잠금을 쥔 채 값을 바꾸고 drop에서 되돌린다.
//! - 환경을 읽기만 하는 테스트가 바뀌는 중인 값을 보지 않게 하려면 `EnvGuard::read()`로 읽기 잠금을 쥔다.
//! - 프로덕션이 `EnvFn`을 받는 경우에는 프로세스 환경을 건드리지 말고 `map_env`를 넘긴다.
//!
//! 사용: `let mut env = EnvGuard::new(); env.set("TM_JWT_SECRET", "x"); env.remove("TM_ADMIN_EMAIL");`

#![allow(dead_code)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// 환경을 바꾸는 테스트끼리 직렬화하고, 읽는 테스트와도 겹치지 않게 막는다.
static ENV_LOCK: RwLock<()> = RwLock::new(());

/// `monkeypatch.setenv`/`delenv`에 대응한다. drop되면 처음 값으로 되돌린다.
pub struct EnvGuard {
    saved: Vec<(String, Option<OsString>)>,
    // 필드 drop 순서상 `saved` 복원이 끝난 뒤에 잠금이 풀린다.
    _lock: RwLockWriteGuard<'static, ()>,
}

/// 환경을 읽기만 하는 테스트가 쥐는 공유 잠금.
pub struct EnvReadGuard {
    _lock: RwLockReadGuard<'static, ()>,
}

impl EnvGuard {
    /// 쓰기 잠금을 쥐고 `setup_env()`로 고정 값을 먼저 맞춘다.
    pub fn new() -> Self {
        crate::common::setup_env();
        Self { saved: Vec::new(), _lock: ENV_LOCK.write().unwrap_or_else(|e| e.into_inner()) }
    }

    /// `(키, Some(값))`은 설정, `(키, None)`은 삭제한다. main.rs의 `EnvGuard::apply`.
    pub fn apply(vars: &[(&str, Option<&str>)]) -> Self {
        let mut guard = Self::new();
        for (key, value) in vars {
            match value {
                Some(v) => guard.set(key, v),
                None => guard.remove(key),
            }
        }
        guard
    }

    /// 변수 하나만 설정하는 단축형. book_manager_2의 `EnvGuard::set(key, value)`.
    pub fn single(key: &str, value: &str) -> Self {
        let mut guard = Self::new();
        guard.set(key, value);
        guard
    }

    /// 읽기만 하는 테스트용. `EnvGuard`의 쓰기 잠금이 풀릴 때까지 기다린다.
    pub fn read() -> EnvReadGuard {
        crate::common::setup_env();
        EnvReadGuard { _lock: ENV_LOCK.read().unwrap_or_else(|e| e.into_inner()) }
    }

    fn remember(&mut self, key: &str) {
        self.saved.push((key.to_string(), std::env::var_os(key)));
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.remember(key);
        // SAFETY: ENV_LOCK 쓰기 잠금을 쥔 동안에만 환경 변수를 바꾼다.
        unsafe { std::env::set_var(key, value) };
    }

    pub fn remove(&mut self, key: &str) {
        self.remember(key);
        // SAFETY: ENV_LOCK 쓰기 잠금을 쥔 동안에만 환경 변수를 바꾼다.
        unsafe { std::env::remove_var(key) };
    }

    /// 여러 변수를 한 번에 설정한다.
    pub fn set_all(&mut self, pairs: &[(&str, &str)]) {
        for (key, value) in pairs {
            self.set(key, value);
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, old) in self.saved.drain(..).rev() {
            // SAFETY: 아직 쓰기 잠금을 쥐고 있다(필드 drop은 이 함수가 끝난 뒤다).
            unsafe {
                match old {
                    Some(v) => std::env::set_var(&key, v),
                    None => std::env::remove_var(&key),
                }
            }
        }
    }
}

/// `EnvFn`(`&dyn Fn(&str) -> Option<String>`)에 넘길 가짜 환경. 프로세스 환경을 건드리지 않으므로 잠금이 필요 없다.
/// `from_env(&map_env(&[("TM_ES_URL", "http://localhost:9200")]))`
pub fn map_env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + Send + Sync + 'static {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |key| map.get(key).cloned()
}

/// 값이 하나도 없는 환경.
pub fn empty_env() -> impl Fn(&str) -> Option<String> + Send + Sync + 'static {
    |_| None
}
