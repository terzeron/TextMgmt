use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    // 손상된 PDF에서 난 panic은 파일별 메모로 기록되므로 기본 panic 메시지는 끈다.
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    let code = pdfscan::run(&args, &mut out);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
