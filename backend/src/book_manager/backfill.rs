//! 시작 때 created_time 백필. Python `_backfill_created_time_if_enabled`에 대응한다.

use std::time::{Duration, Instant};

use sysdeps::EnvFn;

use super::{BookManager, CREATED_TIME_BACKFILL_ENV};

fn env_flag_enabled(env: EnvFn, name: &str) -> bool {
    matches!(env(name).unwrap_or_default().trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

impl BookManager {
    /// 환경 변수가 켜져 있으면 별도 스레드에서 `backfill_created_time`을 한 번 돌린다. 생성자는 기다리지 않는다.
    pub(crate) fn backfill_created_time_if_enabled(&self, env: EnvFn, index_name: &str) {
        if !env_flag_enabled(env, CREATED_TIME_BACKFILL_ENV) {
            return;
        }
        let mut slot = self.backfill_thread.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let es = self.es.clone();
        let prefix = self.path_prefix.clone();
        let name = format!("created-time-backfill-{}", if index_name.is_empty() { "book" } else { index_name });
        match std::thread::Builder::new().name(name).spawn(move || {
            if let Err(e) = es.backfill_created_time(&prefix) {
                log::warn!("created_time backfill failed: {e}");
            }
        }) {
            Ok(handle) => *slot = Some(handle),
            Err(e) => log::warn!("created_time backfill failed to start: {e}"),
        }
    }

    /// 백필 스레드가 끝나기를 `timeout`까지 기다린다. 테스트와 종료 처리에 쓴다. 시간이 지나면 기다림을 포기한다.
    pub fn join_created_time_backfill_thread(&self, timeout: Duration) {
        let handle = self.backfill_thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(handle) = handle else { return };
        let deadline = Instant::now() + timeout;
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if handle.is_finished() {
            let _ = handle.join();
        }
    }
}
