use std::io::Write;
use std::process::ExitCode;

use es_client::EsClient;
use es_client::indexing::IndexStore;
use file_indexer::child_process_runner::{ChildRunner, index_worker_path};
use loader::cli_parser::{World, run};

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
    let program = std::env::args().next().and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "loader".into());
    let runner = ChildRunner::new(index_worker_path());
    let env = |k: &str| std::env::var(k).ok();
    let world = World { env: &env, connect: &connect, runner: &runner };
    let code = run(&program, &args, &world, &mut out);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
