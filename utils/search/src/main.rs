use std::io::Write;
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use es_client::{BookSearch, EsClient, EsError};

fn log(level: &str, msg: &str) {
    eprintln!("{} {level} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"));
}

fn connect() -> Result<&'static dyn BookSearch, EsError> {
    let client = EsClient::from_env()?;
    client.connect_with_retry(5, &|secs| {
        log("WARNING", &format!("ES 연결 실패. {secs}초 후 재시도..."));
        sleep(Duration::from_secs(secs));
    })?;
    log("INFO", "Elasticsearch 연결 성공");
    // 프로세스가 끝날 때까지 쓰는 단일 클라이언트다.
    Ok(Box::leak(Box::new(client)))
}

fn main() -> ExitCode {
    let program = std::env::args().next().unwrap_or_else(|| "search".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    let code = search::run(&program, &args, &mut out, &connect);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
