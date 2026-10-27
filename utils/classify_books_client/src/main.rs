use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let program = std::env::args().next().and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "classify_books_client".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    let code = classify_books_client::run(&program, &args, std::env::var("TM_API_TOKEN").ok(), &mut out);
    let _ = out.flush();
    ExitCode::from(code as u8)
}
