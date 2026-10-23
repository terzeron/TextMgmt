//! created_time 백필 스레드 기다리기. 시작은 `seams.rs`의 `start_created_time_backfill`이 한다.

use std::time::Duration;

use super::BookManager;

impl BookManager {
    /// 백필 스레드가 끝나기를 `timeout`까지 기다린다. 테스트와 종료 처리에 쓴다. 시간이 지나면 기다림을 포기한다.
    pub fn join_created_time_backfill_thread(&self, timeout: Duration) {
        if let Some(thread) = self.created_time_backfill_thread() {
            thread.join(timeout);
        }
    }
}
