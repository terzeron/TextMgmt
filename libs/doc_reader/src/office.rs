//! 오피스 문서와 이미지 읽기. DOCX는 직접 읽고, DOC·HWP·이미지는 LibreOffice로 변환해 읽는다.
//! 원본: `Loader.read_from_docx/doc/hwp/image`, `_convert_with_libreoffice`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use sysdeps::timeout_runner::run_with_hard_timeout;
use text_extractor::open_zip;

use crate::TEXT_SIZE;
use crate::py_str_compat::{head_chars, is_space, keep_word_chars};

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

/// HWP 파일: HWP 3.x 이하는 네이티브 HWP3 파서를 먼저 쓴다(LibreOffice는 한자를 `?`로 깨뜨린다).
/// 그 결과가 비거나 HWP 5.x면 LibreOffice로 변환한다. 네이티브 파서는 이미 실패했으므로 다시 부르지 않는다.
pub fn read_from_hwp(path: &Path, binary: &Path) -> (String, usize, Vec<String>) {
    if let Some(done) = read_hwp3_natively(path, &|path| Ok(hwp3_parser::extract_text_from_hwp3(path))) {
        return done;
    }
    let mut errors = Vec::new();
    let raw = convert_with_libreoffice(path, "txt:Text", binary, &mut errors);
    let (summary, lines) = finish(&raw);
    (summary, lines, errors)
}

pub fn read_from_hwp_with(path: &Path, _binary: &Path, convert: &dyn Fn(&Path) -> Result<String, String>) -> (String, usize, Vec<String>) {
    read_from_hwp_with_fallback(path, _binary, convert, &|path| Ok(hwp3_parser::extract_text_from_hwp3(path)))
}

/// HWP 3.x 이하이고 네이티브 추출 결과가 있으면 그것으로 끝낸다.
fn read_hwp3_natively(path: &Path, extract: &dyn Fn(&Path) -> Result<String, String>) -> Option<(String, usize, Vec<String>)> {
    if !hwp3_parser::is_hwp3(path) {
        return None;
    }
    let raw = extract(path).ok().filter(|text| !text.trim_matches(is_space).is_empty())?;
    let (summary, lines) = finish(&raw);
    Some((summary, lines, Vec::new()))
}

/// [`read_from_hwp`]와 같은 순서이되 LibreOffice 변환(`convert`)과 HWP3 추출(`extract_native`)을 주입한다.
pub fn read_from_hwp_with_fallback(
    path: &Path,
    _binary: &Path,
    convert: &dyn Fn(&Path) -> Result<String, String>,
    extract_native: &dyn Fn(&Path) -> Result<String, String>,
) -> (String, usize, Vec<String>) {
    if let Some(done) = read_hwp3_natively(path, extract_native) {
        return done;
    }
    let mut errors = Vec::new();
    let converted = match convert(path) {
        Ok(text) => text,
        Err(error) => {
            errors.push(error);
            let (summary, lines) = finish("");
            return (summary, lines, errors);
        }
    };
    let (summary, lines) = finish(&converted);
    (summary, lines, errors)
}

/// 이미지에는 본문·줄·쪽이 없다.
pub fn read_from_image() -> (String, usize, usize) {
    (String::new(), 0, 0)
}

// ---- DOCX 읽기. 원본: `Loader.read_from_docx`(python-docx `Document(path).paragraphs`). ----
//
// 본문(`w:body`)의 바로 아래 문단만 센다(표 안 문단은 제외). 문단 글은 `w:r`과 `w:hyperlink`의 런 글을 이은 것이다.
const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn is_w(node: roxmltree::Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(W_NS)
}

/// 런 하나의 글: `w:t`, `w:tab`, `w:br`(줄바꿈이면 `\n`), `w:cr`, `w:noBreakHyphen`, `w:ptab`.
fn run_text(run: roxmltree::Node) -> String {
    let mut out = String::new();
    for e in run.children().filter(|c| c.is_element() && c.tag_name().namespace() == Some(W_NS)) {
        match e.tag_name().name() {
            "t" => out.push_str(e.text().unwrap_or("")),
            "tab" | "ptab" => out.push('\t'),
            "br" => {
                let kind = e.attribute((W_NS, "type")).unwrap_or("textWrapping");
                if kind == "textWrapping" {
                    out.push('\n');
                }
            }
            "cr" => out.push('\n'),
            "noBreakHyphen" => out.push('-'),
            _ => {}
        }
    }
    out
}

/// 문단 글: 직계 `w:r`과 `w:hyperlink`(그 안의 `w:r`)를 문서 순서대로 잇는다.
pub fn paragraph_text(p: roxmltree::Node) -> String {
    let mut out = String::new();
    for e in p.children().filter(|c| c.is_element()) {
        if is_w(e, "r") {
            out.push_str(&run_text(e));
        } else if is_w(e, "hyperlink") {
            for r in e.children().filter(|c| is_w(*c, "r")) {
                out.push_str(&run_text(r));
            }
        }
    }
    out
}

fn read_member(zip: &mut text_extractor::Zip, name: &str) -> Result<String, String> {
    let file = zip.by_name(name).map_err(|_| format!("KeyError: {name}"))?;
    let mut buf = Vec::new();
    file.take(text_extractor::MAX_MEMBER_BYTES).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.starts_with(b"\xef\xbb\xbf") {
        buf.drain(..3);
    }
    String::from_utf8(buf).map_err(|e| e.to_string())
}

/// 주 문서 파트 이름. `_rels/.rels`의 officeDocument 관계를 따르고, 없으면 `word/document.xml`.
fn main_part(zip: &mut text_extractor::Zip) -> Result<String, String> {
    let rels = read_member(zip, "_rels/.rels")?;
    let doc = roxmltree::Document::parse(&rels).map_err(|e| format!("XML 해석 실패: {e}"))?;
    let target = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Relationship" && n.tag_name().namespace() == Some(REL_NS) && n.attribute("Type").is_some_and(|t| t.ends_with("/officeDocument")))
        .and_then(|n| n.attribute("Target"));
    Ok(target.map_or_else(|| "word/document.xml".to_owned(), |t| t.trim_start_matches('/').to_owned()))
}

/// DOCX 파일: (요약, 줄 수). 패키지를 열 수 없으면 `Err`.
pub fn read_from_docx(path: &Path) -> Result<(String, usize), String> {
    let mut zip = open_zip(path).ok_or("Package not found / bad zip file")?;
    let part = main_part(&mut zip)?;
    let xml = read_member(&mut zip, &part)?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| format!("XML 해석 실패: {e}"))?;
    let body = doc.root_element().children().find(|c| is_w(*c, "body")).ok_or("문서 본문(w:body)이 없다")?;

    let mut result = String::new();
    let mut total_text = String::new();
    for p in body.children().filter(|c| is_w(*c, "p")) {
        let text = paragraph_text(p);
        total_text.push_str(&text);
        total_text.push('\n');
        if result.chars().count() < TEXT_SIZE {
            result.push_str(&text);
        }
    }
    let line_count = if total_text.is_empty() { 0 } else { total_text.matches('\n').count() };
    let cleaned = keep_word_chars(&result);
    Ok((head_chars(&cleaned, TEXT_SIZE).to_owned(), line_count))
}
