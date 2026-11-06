//! MuPDF Rust worker adapter. Native parsers keep the existing process limits.
use std::path::{Path, PathBuf};
use std::time::Duration;
use sysdeps::{CommandError, CommandRunner};
const TIMEOUT: Duration = Duration::from_secs(30);
pub const WORKER_NAME: &str = "tm-pdf-worker";
fn worker_path() -> PathBuf {
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let p = dir.join(WORKER_NAME);
        if p.is_file() {
            return p;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.parent().into_iter().chain(exe.parent().and_then(Path::parent)) {
            let p = dir.join(WORKER_NAME);
            if p.is_file() {
                return p;
            }
        }
    }
    PathBuf::from(WORKER_NAME)
}
fn run(runner: &dyn CommandRunner, operation: &str, path: &Path, args: &[String]) -> Result<Vec<u8>, String> {
    let mut command =
        vec!["prlimit".into(), "--as=536870912".into(), "--cpu=25".into(), "--".into(), worker_path().to_string_lossy().into_owned(), operation.into(), path.to_string_lossy().into_owned()];
    command.extend_from_slice(args);
    let output = runner.run(&command, Some(TIMEOUT)).map_err(|error| match error {
        CommandError::NotFound => "PDF worker unavailable; build tm-pdf-worker".into(),
        other => format!("PDF worker failed: {other}"),
    })?;
    if output.returncode != 0 {
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        let diagnostic = diagnostic.trim();
        if output.returncode == 127 {
            return Err("PDF worker unavailable; build tm-pdf-worker".into());
        }
        if diagnostic.is_empty() {
            return Err("PDF worker terminated without a diagnostic".into());
        }
        let lower = diagnostic.to_ascii_lowercase();
        if ["out of memory", "cannot allocate", "memory allocation", "malloc of"].iter().any(|s| lower.contains(s)) {
            return Err(format!("PDF worker resource limit: {diagnostic}"));
        }
        return Err(format!("PDF worker failed: {diagnostic}"));
    }
    Ok(output.stdout)
}
pub fn page_count(runner: &dyn CommandRunner, path: &Path) -> Result<usize, String> {
    let bytes = run(runner, "info", path, &[])?;
    String::from_utf8_lossy(&bytes)
        .lines()
        .find_map(|line| line.strip_prefix("Pages:").map(str::trim))
        .ok_or_else(|| "PDF worker did not report a page count".to_string())?
        .parse()
        .map_err(|_| "PDF worker returned an invalid page count".into())
}
pub fn extract_text(runner: &dyn CommandRunner, path: &Path, first: usize, last: usize) -> Result<String, String> {
    if first == 0 || last < first {
        return Err("invalid PDF page range".into());
    }
    String::from_utf8(run(runner, "text", path, &[first.to_string(), last.to_string()])?).map_err(|e| format!("PDF worker returned invalid UTF-8: {e}"))
}
/// P6 RGB pixels; the native renderer bounds both dimensions before allocation.
pub fn render_page(runner: &dyn CommandRunner, path: &Path, page: usize, max_dimension: u32) -> Result<Vec<u8>, String> {
    if page == 0 || max_dimension == 0 || max_dimension > 1200 {
        return Err("invalid PDF render dimensions".into());
    }
    run(runner, "render", path, &[page.to_string(), max_dimension.to_string()])
}
pub fn extract_pages(runner: &dyn CommandRunner, path: &Path, first: usize, last: usize) -> Result<Vec<u8>, String> {
    if first == 0 || last < first {
        return Err("invalid PDF page range".into());
    }
    run(runner, "extract", path, &[first.to_string(), last.to_string()])
}
pub fn first_page_jpeg(runner: &dyn CommandRunner, path: &Path) -> Result<Vec<u8>, String> {
    run(runner, "jpeg", path, &[])
}
/// Entry point shared by the project's native PDF worker binary.
pub fn run_worker(args: &[String], out: &mut dyn std::io::Write) -> Result<(), String> {
    crate::native_pdf::run(args, out)
}
