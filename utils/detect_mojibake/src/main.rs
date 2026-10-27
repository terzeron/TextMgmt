use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    // 손상 ZIP에서 난 panic은 파일별 scan_error로 기록되므로 기본 panic 메시지는 끈다.
    std::panic::set_hook(Box::new(|_| {}));

    // utils/detect_mojibake -> 저장소 루트. 기본 리포트는 <repo>/tmp 에 쓴다.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo_root = repo_root.canonicalize().unwrap_or(repo_root);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let log = |msg: &str| eprintln!("{} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"));
    let mut out = std::io::stdout().lock();
    let code = detect_mojibake::cli::run(&args, &repo_root, &mut out, &log);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
