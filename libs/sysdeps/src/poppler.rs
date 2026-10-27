//! Poppler command adapters. Tool binaries are resolved through `PATH`.

use std::path::Path;
use std::time::Duration;

use crate::{CommandError, CommandRunner};

const TIMEOUT: Duration = Duration::from_secs(30);

fn run(runner: &dyn CommandRunner, args: &[String]) -> Result<Vec<u8>, String> {
    let output = runner.run(args, Some(TIMEOUT)).map_err(|error| match error {
        CommandError::NotFound => format!("{} not found; install poppler-utils", args.first().map(String::as_str).unwrap_or("Poppler")),
        other => format!("{} failed: {other}", args.first().map(String::as_str).unwrap_or("Poppler")),
    })?;
    if output.returncode != 0 {
        return Err(format!("{} failed: {}", args[0], String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(output.stdout)
}

/// Read the number of pages reported by `pdfinfo`.
pub fn page_count(runner: &dyn CommandRunner, path: &Path) -> Result<usize, String> {
    let output = run(runner, &["pdfinfo".into(), path.to_string_lossy().into_owned()])?;
    String::from_utf8_lossy(&output)
        .lines()
        .find_map(|line| line.strip_prefix("Pages:").map(str::trim))
        .ok_or_else(|| "pdfinfo did not report a page count".to_owned())?
        .parse()
        .map_err(|_| "pdfinfo returned an invalid page count".to_owned())
}

/// Extract text from an inclusive page range. Output is UTF-8 and preserves page breaks.
pub fn extract_text(runner: &dyn CommandRunner, path: &Path, first: usize, last: usize) -> Result<String, String> {
    if first == 0 || last < first {
        return Err("invalid Poppler page range".to_owned());
    }
    let output =
        run(runner, &["pdftotext".into(), "-q".into(), "-enc".into(), "UTF-8".into(), "-f".into(), first.to_string(), "-l".into(), last.to_string(), path.to_string_lossy().into_owned(), "-".into()])?;
    String::from_utf8(output).map_err(|error| format!("pdftotext returned invalid UTF-8: {error}"))
}

/// Render one page to a bounded PPM image on stdout.
pub fn render_page(runner: &dyn CommandRunner, path: &Path, page: usize, max_dimension: u32) -> Result<Vec<u8>, String> {
    if page == 0 || max_dimension == 0 {
        return Err("invalid Poppler render dimensions".to_owned());
    }
    run(
        runner,
        &[
            "pdftoppm".into(),
            "-q".into(),
            "-f".into(),
            page.to_string(),
            "-l".into(),
            page.to_string(),
            "-scale-to".into(),
            max_dimension.to_string(),
            "-singlefile".into(),
            path.to_string_lossy().into_owned(),
        ],
    )
}
