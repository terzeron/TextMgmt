use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let program = std::env::args().next().and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "move_semi_clusters".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    let code = move_semi_clusters::run(&program, &args, &mut out);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
