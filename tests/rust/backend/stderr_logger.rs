use backend::stderr_logger::{format_line, init};
use log::{Level, LevelFilter, Record};

fn line(level: Level) -> String {
    format_line(&Record::builder().level(level).target("backend::sample").args(format_args!("메시지 {}", 7)).build())
}

#[test]
fn line_has_level_target_and_message() {
    assert_eq!(line(Level::Error), "[ERROR] backend::sample: 메시지 7");
    assert_eq!(line(Level::Warn), "[WARN] backend::sample: 메시지 7");
}

#[test]
fn init_registers_logger_that_keeps_only_warn_and_above() {
    assert!(init().is_ok());
    // 이미 등록된 뒤에는 다시 등록할 수 없다.
    assert!(init().is_err());
    assert_eq!(log::max_level(), LevelFilter::Warn);

    let logger = log::logger();
    let meta = |level| log::Metadata::builder().level(level).target("t").build();
    assert!(logger.enabled(&meta(Level::Error)));
    assert!(logger.enabled(&meta(Level::Warn)));
    assert!(!logger.enabled(&meta(Level::Info)));
    assert!(!logger.enabled(&meta(Level::Debug)));

    // 기록 경로(enabled 통과·차단)가 패닉 없이 끝난다.
    log::error!("로거 테스트 오류");
    log::info!("기록되지 않는 정보");
    logger.flush();
}
