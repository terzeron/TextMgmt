use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    // 손상 파일에서 난 panic은 파일별 incomplete로 기록되므로 기본 panic 메시지는 끈다.
    std::panic::set_hook(Box::new(|_| {}));

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    let code = bookdoctor::cli::run(&args, &mut out);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
