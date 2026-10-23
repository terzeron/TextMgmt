use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    // utils/quarantine_broken -> 저장소 루트. 기본 리포트와 매니페스트는 <repo>/tmp 에 둔다.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo_root = repo_root.canonicalize().unwrap_or(repo_root);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let log = |msg: &str| eprintln!("{} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"));
    let mut out = std::io::stdout().lock();
    let code = quarantine_broken::cli::run(&args, &repo_root, &mut out, &log);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
