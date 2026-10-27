//! LibreOffice로 변환해 읽는 형식(DOC, HWP)과 이미지. 원본: `Loader.read_from_doc/hwp/image`, `_convert_with_libreoffice`.

use std::path::{Path, PathBuf};
use std::process::Command;

use parser_timeout::run_with_hard_timeout;

use crate::TEXT_SIZE;
use crate::pytext::{head_chars, is_space, keep_word_chars};

/// 변환 한 번에 허용하는 시간(초).
pub const LIBREOFFICE_TIMEOUT: f64 = 60.0;

/// LibreOffice 실행 파일 경로. PATH의 `libreoffice`, `soffice`, macOS 앱 순서, 없으면 `libreoffice`.
pub fn find_libreoffice() -> PathBuf {
    find_libreoffice_with(&which, &|path| path.is_file())
}

/// PATH 조회와 후보 존재 검사를 주입할 수 있는 LibreOffice 실행 파일 탐색기.
pub fn find_libreoffice_with(which: &dyn Fn(&str) -> Option<PathBuf>, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    for name in ["libreoffice", "soffice"] {
        if let Some(path) = which(name) {
            return path;
        }
    }
    let mac = Path::new("/Applications/LibreOffice.app/Contents/MacOS/soffice");
    if exists(mac) {
        return mac.to_path_buf();
    }
    PathBuf::from("libreoffice")
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

/// 파일을 `output_format`(예: `txt:Text`)으로 변환하고 결과 텍스트를 돌려준다. 결과 파일이 없으면 오류 메시지와 빈 문자열.
pub fn convert_with_libreoffice(path: &Path, output_format: &str, binary: &Path, errors: &mut Vec<String>) -> String {
    let Ok(tmp) = tempfile::tempdir() else { return String::new() };
    let mut cmd = Command::new(binary);
    cmd.args(["--headless", "--convert-to", output_format, "--outdir"]).arg(tmp.path()).arg(path);
    let output = run_with_hard_timeout(cmd, LIBREOFFICE_TIMEOUT);
    let ext = output_format.split(':').next().unwrap_or("txt");
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let read = |p: &Path| String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned();
    let expected = tmp.path().join(format!("{stem}.{ext}"));
    if expected.exists() {
        return read(&expected);
    }
    // 이름이 달라졌으면 확장자로 찾는다.
    let mut found: Vec<PathBuf> = std::fs::read_dir(tmp.path()).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == ext)).collect()).unwrap_or_default();
    found.sort();
    if let Some(first) = found.first() {
        return read(first);
    }
    let files: Vec<String> = std::fs::read_dir(tmp.path()).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    let (code, stderr) = match &output {
        Ok(o) => (o.status.code().unwrap_or(-1).to_string(), head_chars(&String::from_utf8_lossy(&o.stderr), 500).to_owned()),
        Err(e) => ("timeout".to_owned(), e.to_string()),
    };
    errors.push(format!("LibreOffice produced no output: file='{}', format='{output_format}', returncode={code}, stderr={stderr}, tmpdir_files={files:?}", path.display()));
    String::new()
}

fn finish(raw_text: &str) -> (String, usize) {
    let line_count = if raw_text.is_empty() { 0 } else { raw_text.matches('\n').count() + 1 };
    (keep_word_chars(head_chars(raw_text, TEXT_SIZE)), line_count)
}

/// DOC 파일: (요약, 줄 수, 오류 메시지들).
pub fn read_from_doc(path: &Path, binary: &Path) -> (String, usize, Vec<String>) {
    let mut errors = Vec::new();
    let raw = convert_with_libreoffice(path, "txt:Text", binary, &mut errors);
    let (summary, lines) = finish(&raw);
    (summary, lines, errors)
}

/// HWP 파일: LibreOffice 변환이 비면 HWP3 파서로 대신한다.
pub fn read_from_hwp(path: &Path, binary: &Path) -> (String, usize, Vec<String>) {
    let mut errors = Vec::new();
    let mut raw = convert_with_libreoffice(path, "txt:Text", binary, &mut errors);
    if errors.is_empty() && raw.trim_matches(is_space).is_empty() {
        raw = hwp3_parser::extract_text_from_hwp3(path);
    }
    let (summary, lines) = finish(&raw);
    (summary, lines, errors)
}

pub fn read_from_hwp_with(path: &Path, _binary: &Path, convert: &dyn Fn(&Path) -> Result<String, String>) -> (String, usize, Vec<String>) {
    read_from_hwp_with_fallback(path, _binary, convert, &|path| Ok(hwp3_parser::extract_text_from_hwp3(path)))
}

pub fn read_from_hwp_with_fallback(
    path: &Path,
    _binary: &Path,
    convert: &dyn Fn(&Path) -> Result<String, String>,
    extract_fallback: &dyn Fn(&Path) -> Result<String, String>,
) -> (String, usize, Vec<String>) {
    let mut errors = Vec::new();
    let converted = match convert(path) {
        Ok(text) => text,
        Err(error) => {
            errors.push(error);
            let (summary, lines) = finish("");
            return (summary, lines, errors);
        }
    };
    let mut raw = converted;
    if raw.trim_matches(is_space).is_empty() {
        raw = match extract_fallback(path) {
            Ok(text) => text,
            Err(error) => {
                errors.push(error);
                String::new()
            }
        };
    }
    let (summary, lines) = finish(&raw);
    (summary, lines, errors)
}

/// 이미지에는 본문·줄·쪽이 없다.
pub fn read_from_image() -> (String, usize, usize) {
    (String::new(), 0, 0)
}
