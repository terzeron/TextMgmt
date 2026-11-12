use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use es_client::transport::EsTransport;
use search::cli_parser::{World, run};
use search_manager::{ConnectOptions, EsConnector, RealEsConnector};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let program = std::env::args().next().and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "search".into());
    let env = |key: &str| std::env::var(key).ok();
    let connect = |options: &ConnectOptions| -> Arc<dyn EsTransport> { RealEsConnector.connect(options) };
    let world = World { env: &env, connect: &connect };
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    let code = run(&program, &args, &world, &mut out, &mut err);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
