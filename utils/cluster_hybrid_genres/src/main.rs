use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    let code = cluster_hybrid_genres::cli::run(&args, &mut out);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
