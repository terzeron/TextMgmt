//! pytest `caplog`에 대응하는 RAII 로그 수집기. 설계: docs/rust-test-seams.md 3.10.
//! 스레드마다 따로 쌓아서 병렬 테스트끼리 섞이지 않는다. 이전의 자유 함수 `capture_at_level`/`records`/`text`를 대체한다.
//!
//! 가정한 API: 프로덕션 코드는 `log` crate만 쓴다(`log::error!`, `log::warn!`). `backend::log_capture` 모듈은 없다.
//!
//! 사용: `let logs = LogCapture::start(log::Level::Warn); ...; assert!(logs.text().contains("실패"));`
//! 한계: 스레드 로컬이라 `spawn_blocking`이나 워커 스레드에서 남긴 로그는 잡히지 않는다.
//! `#[tokio::test]` 기본 런타임(current_thread)에서는 핸들러의 로그도 테스트 스레드에 쌓인다.
//! 같은 스레드에서 `LogCapture`를 겹쳐 만들면 안쪽이 기록을 비우고 수준을 바꾼다. drop하면 비운다.

#![allow(dead_code)]

use std::cell::RefCell;
use std::sync::Once;

use log::{Level, LevelFilter, Log, Metadata, Record};

#[derive(Clone, Debug)]
pub struct CapturedRecord {
    pub level: Level,
    pub target: String,
    pub message: String,
}

thread_local! {
    static RECORDS: RefCell<Vec<CapturedRecord>> = const { RefCell::new(Vec::new()) };
    static MIN_LEVEL: RefCell<Level> = const { RefCell::new(Level::Trace) };
}

struct Collector;

impl Log for Collector {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if record.level() > MIN_LEVEL.with(|l| *l.borrow()) {
            return;
        }
        RECORDS.with(|r| r.borrow_mut().push(CapturedRecord { level: record.level(), target: record.target().to_string(), message: record.args().to_string() }));
    }

    fn flush(&self) {}
}

static INIT: Once = Once::new();
static COLLECTOR: Collector = Collector;

/// 이 스레드의 로그를 `level` 이상만 모으는 가드. drop하면 기록을 비운다.
pub struct LogCapture {
    // 다른 스레드로 옮기면 스레드 로컬 버퍼가 달라지므로 `Send`가 되지 않게 막는다.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl LogCapture {
    /// `caplog.at_level(level)`. 이 스레드의 기록을 비우고 `level` 이상만 모은다.
    pub fn start(level: Level) -> Self {
        INIT.call_once(|| {
            // 다른 로거가 이미 있으면 set_logger가 실패한다. 그 경우 수집은 되지 않는다.
            let _ = log::set_logger(&COLLECTOR);
            log::set_max_level(LevelFilter::Trace);
        });
        RECORDS.with(|r| r.borrow_mut().clear());
        MIN_LEVEL.with(|l| *l.borrow_mut() = level);
        Self { _not_send: std::marker::PhantomData }
    }

    /// `caplog.records`
    pub fn records(&self) -> Vec<CapturedRecord> {
        RECORDS.with(|r| r.borrow().clone())
    }

    /// 기록된 메시지만. `caplog.messages`
    pub fn messages(&self) -> Vec<String> {
        RECORDS.with(|r| r.borrow().iter().map(|rec| rec.message.clone()).collect())
    }

    /// `caplog.text`
    pub fn text(&self) -> String {
        self.records().iter().map(|r| format!("{} {}: {}", r.level, r.target, r.message)).collect::<Vec<_>>().join("\n")
    }

    /// 지금까지 쌓은 기록을 비운다. `caplog.clear()`
    pub fn clear(&self) {
        RECORDS.with(|r| r.borrow_mut().clear());
    }

    /// `level` 이상 기록이 하나라도 있는가.
    pub fn has_level(&self, level: Level) -> bool {
        RECORDS.with(|r| r.borrow().iter().any(|rec| rec.level <= level))
    }
}

impl Drop for LogCapture {
    fn drop(&mut self) {
        RECORDS.with(|r| r.borrow_mut().clear());
        MIN_LEVEL.with(|l| *l.borrow_mut() = Level::Trace);
    }
}
