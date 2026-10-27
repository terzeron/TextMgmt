//! 백엔드 서버 진입점. 인자 없이 실행하면 용도와 사용법을 출력한다.

use std::process::ExitCode;

fn print_usage() {
    println!("backend: TextMgmt 백엔드 API 서버");
    println!();
    println!("사용법: backend serve [--addr HOST:PORT]");
    println!();
    println!("옵션:");
    println!("  --addr HOST:PORT  수신 주소 (기본 0.0.0.0:8000)");
    println!("  -h, --help        이 도움말");
    println!();
    println!("종료 코드: 0 정상, 2 잘못된 사용, 255 실행 오류");
}

fn validate_frontend_url(frontend_url: Option<&str>) -> Result<(), &'static str> {
    frontend_url.ok_or("The environment variable TM_FRONTEND_URL is not set.").map(|_| ())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(loader::cli::PDF_STAGE_COMMAND) {
        let mut out = std::io::stdout().lock();
        let code = loader::cli::run_pdf_stage_command(&args[1..], &mut out);
        let _ = std::io::Write::flush(&mut out);
        return ExitCode::from(code as u8);
    }
    if args.first().map(String::as_str) == Some(loader::cli::ISBN_PDF_STAGE_COMMAND) {
        let mut out = std::io::stdout().lock();
        let code = loader::cli::run_isbn_pdf_stage_command(&args[1..], &mut out);
        let _ = std::io::Write::flush(&mut out);
        return ExitCode::from(code as u8);
    }
    if args.first().map(String::as_str) == Some(loader::cli::READ_FILE_COMMAND) {
        let mut out = std::io::stdout().lock();
        let code = loader::cli::run_read_file_command(&args[1..], &mut out);
        let _ = std::io::Write::flush(&mut out);
        return ExitCode::from(code as u8);
    }
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return ExitCode::SUCCESS;
    }
    let address = match args.as_slice() {
        [command] if command == "serve" => "0.0.0.0:8000",
        [command, flag, address] if command == "serve" && flag == "--addr" => address,
        _ => {
            eprintln!("사용법: backend serve [--addr HOST:PORT]");
            return ExitCode::from(2);
        }
    };
    if !address.rsplit_once(':').is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok()) {
        eprintln!("잘못된 수신 주소: {address}");
        return ExitCode::from(2);
    }
    if let Err(error) = validate_frontend_url(std::env::var("TM_FRONTEND_URL").ok().as_deref()) {
        eprintln!("{error}");
        return ExitCode::from(255);
    }
    match serve(address).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(255)
        }
    }
}

async fn serve(address: &str) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|e| format!("bind {address}: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    // 수신 주소를 알리기 전에 signal handler를 등록한다.
    #[cfg(unix)]
    let shutdown = {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).map_err(|e| e.to_string())?;
        let mut interrupt = signal(SignalKind::interrupt()).map_err(|e| e.to_string())?;
        async move {
            tokio::select! {_=terminate.recv()=>{},_=interrupt.recv()=>{}}
        }
    };
    #[cfg(not(unix))]
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let app = backend::main::app();
    println!("Listening on {address}");
    axum::serve(listener, app).with_graceful_shutdown(shutdown).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::validate_frontend_url;

    #[test]
    fn frontend_url_is_required_for_server_startup() {
        assert!(validate_frontend_url(Some("https://frontend.example")).is_ok());
        assert_eq!(validate_frontend_url(None), Err("The environment variable TM_FRONTEND_URL is not set."));
    }
}
