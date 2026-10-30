//! 표준 에러로 기록하는 최소 로거.
//!
//! `log` 크레이트는 로거를 등록하지 않으면 모든 `log::error!`·`log::warn!` 출력을 버린다.
//! 그러면 `POST /logs/client-error`가 받은 프런트엔드 에러도 pod 로그에 남지 않는다.
//! 새 의존성 없이 경고 이상만 stderr에 한 줄씩 남긴다. 시각은 `kubectl logs --timestamps`가 붙인다.

use std::io::Write;

use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

/// 이 수준(포함) 이상의 심각도만 기록한다. 정보·디버그는 양이 많아 남기지 않는다.
const MAX_LEVEL: Level = Level::Warn;

struct StderrLogger;

static LOGGER: StderrLogger = StderrLogger;

/// 기록할 한 줄의 모양. 예: `[ERROR] backend::main::log_routes: [CLIENT_ERROR] ...`
pub fn format_line(record: &Record) -> String {
    format!("[{}] {}: {}", record.level(), record.target(), record.args())
}

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= MAX_LEVEL
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            // stderr가 닫혀 쓰기에 실패해도 요청 처리를 막지 않는다.
            let _ = writeln!(std::io::stderr().lock(), "{}", format_line(record));
        }
    }

    fn flush(&self) {}
}

/// 전역 로거로 등록한다. 이미 다른 로거가 있으면 오류를 돌려준다.
pub fn init() -> Result<(), SetLoggerError> {
    log::set_logger(&LOGGER)?;
    log::set_max_level(LevelFilter::Warn);
    Ok(())
}
