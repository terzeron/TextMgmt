use std::io::Write;
use std::process::ExitCode;

fn launch(mut cmd: std::process::Command) -> i32 {
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("loader를 실행하지 못했다(uv가 PATH에 있어야 한다): {e}");
            1
        }
    }
}

fn main() -> ExitCode {
    let program = std::env::args().next().and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "reload_samgukji_epubs".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let mut out = std::io::stdout().lock();
    let code = reload_samgukji_epubs::run(&program, &args, &|k| std::env::var(k).ok(), &cwd, &mut out, &launch);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
