use std::io::Write;
use std::process::ExitCode;

use es_client::EsClient;
use es_client::indexing::IndexStore;
use loader::cli::{ChildRunner, ISBN_PDF_STAGE_COMMAND, PDF_STAGE_COMMAND, World, run, run_isbn_pdf_stage_command, run_pdf_stage_command};

fn connect(index: &str) -> Result<Box<dyn IndexStore>, String> {
    let client = EsClient::from_env_with_index(index).map_err(|error| error.to_string())?;
    client
        .connect_with_retry(5, &|secs| {
            eprintln!("WARNING ES 연결 실패. {secs}초 후 재시도...");
            std::thread::sleep(std::time::Duration::from_secs(secs));
        })
        .map_err(|e| format!("Elasticsearch 접속 실패: {e}"))?;
    Ok(Box::new(client))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    if args.first().map(String::as_str) == Some(PDF_STAGE_COMMAND) {
        let code = run_pdf_stage_command(&args[1..], &mut out);
        let _ = out.flush();
        return ExitCode::from(code as u8);
    }
    if args.first().map(String::as_str) == Some(ISBN_PDF_STAGE_COMMAND) {
        let code = run_isbn_pdf_stage_command(&args[1..], &mut out);
        let _ = out.flush();
        return ExitCode::from(code as u8);
    }
    let program = std::env::args().next().and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "loader".into());
    let exe = std::env::current_exe().unwrap_or_else(|_| "loader".into());
    let runner = ChildRunner::new(exe);
    let env = |k: &str| std::env::var(k).ok();
    let world = World { env: &env, connect: &connect, runner: &runner };
    let code = run(&program, &args, &world, &mut out);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
