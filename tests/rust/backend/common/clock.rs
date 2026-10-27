//! 가짜 시계와 `threading.Event` 대응. 설계: docs/rust-test-seams.md 3.8.
//!
//! 가정한 API (아직 구현 전이다): `sysdeps::Clock { fn now(&self) -> f64; fn monotonic(&self) -> f64; fn sleep(&self, seconds: f64); }`
//!
//! 사용: `use common::clock::FakeClock;` `let clock = FakeClock::new(1000.0); ... .with_clock(clock.clone())`

#![allow(dead_code)]

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use sysdeps::Clock;

/// `now`와 `monotonic`이 같은 시간선을 쓰는 가짜 시계. `sleep`은 실제로 자지 않고 기록한 뒤 시간을 그만큼 보낸다.
/// cover_fixtures의 `FakeClock`과 ESManager 재시도 테스트의 `sleeps` 기록을 합쳤다.
pub struct FakeClock {
    now: Mutex<f64>,
    slept: Mutex<Vec<f64>>,
}

impl FakeClock {
    pub fn new(start: f64) -> Arc<Self> {
        Arc::new(Self { now: Mutex::new(start), slept: Mutex::new(Vec::new()) })
    }

    /// 시간을 `seconds`만큼 보낸다. `sleep` 기록은 남기지 않는다.
    pub fn advance(&self, seconds: f64) {
        *self.now.lock().unwrap() += seconds;
    }

    /// `sleep`에 넘어온 초를 호출 순서대로 돌려준다. ESManager 재시도 대기(2, 4초) 검증에 쓴다.
    pub fn slept(&self) -> Vec<f64> {
        self.slept.lock().unwrap().clone()
    }

    pub fn clear_slept(&self) {
        self.slept.lock().unwrap().clear();
    }

    /// `sleep` 호출 횟수.
    pub fn sleep_count(&self) -> usize {
        self.slept.lock().unwrap().len()
    }

    /// 지금 시각을 직접 정한다.
    pub fn set(&self, now: f64) {
        *self.now.lock().unwrap() = now;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> f64 {
        *self.now.lock().unwrap()
    }

    fn monotonic(&self) -> f64 {
        *self.now.lock().unwrap()
    }

    fn sleep(&self, seconds: f64) {
        self.slept.lock().unwrap().push(seconds);
        *self.now.lock().unwrap() += seconds;
    }
}

/// Python `threading.Event`. 스레드 사이 순서를 맞출 때 쓴다.
pub struct Event {
    flag: Mutex<bool>,
    condvar: Condvar,
}

impl Event {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { flag: Mutex::new(false), condvar: Condvar::new() })
    }

    pub fn set(&self) {
        *self.flag.lock().unwrap() = true;
        self.condvar.notify_all();
    }

    pub fn clear(&self) {
        *self.flag.lock().unwrap() = false;
    }

    pub fn is_set(&self) -> bool {
        *self.flag.lock().unwrap()
    }

    /// 켜졌으면 true, 제한 시간 안에 안 켜졌으면 false.
    pub fn wait(&self, seconds: u64) -> bool {
        let guard = self.flag.lock().unwrap();
        let (guard, _) = self.condvar.wait_timeout_while(guard, Duration::from_secs(seconds), |set| !*set).unwrap();
        *guard
    }
}
