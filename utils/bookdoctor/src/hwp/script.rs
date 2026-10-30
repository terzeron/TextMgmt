//! `Scripts` 스토리지 검사(명세 3.2.9, 표 8·9). 스크립트는 텍스트로만 본다.

use ole_reader::OleFile;
use scan_common::{Category, FileReport, Severity};

use super::header::FileHeader;

/// 스크립트 스트림에서 읽는 최대 바이트.
const MAX_SCRIPT: usize = 4 * 1024 * 1024;
/// 실행·내려받기·환경 접근을 뜻하는 키워드(소문자). 단어 경계가 필요한 `eval(`·`new function`은 따로 본다.
const KEYWORDS: &[&str] = &["activexobject", "wscript", "shell.", "shellexecute", "xmlhttp", "adodb.stream", "powershell", "cmd.exe", "urldownloadtofile"];
/// `DefaultJScript`의 끝 표시(DWORD `-1`).
const END_FLAG: u32 = u32::MAX;

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn utf16_lossy(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// 표 9 구조(헤더·소스·Pre·Post 각각 DWORD 글자 수 + WCHAR 배열, 끝에 `-1`)로 읽어 네 부분을 이은 텍스트를 돌려준다.
/// 길이가 스트림을 넘거나 끝 표시가 다르면 `None`이다.
fn parse(data: &[u8]) -> Option<String> {
    let mut pos = 0usize;
    let mut text = String::new();
    for _ in 0..4 {
        let chars = u32_at(data, pos)? as usize;
        pos += 4;
        let end = pos.checked_add(chars.checked_mul(2)?)?;
        text.push_str(&utf16_lossy(data.get(pos..end)?));
        text.push('\n');
        pos = end;
    }
    (u32_at(data, pos)? == END_FLAG).then_some(text)
}

/// `word`가 단어 경계에서 시작하고 그 뒤에 (공백을 건너뛰고) `(`가 오는가. `retrieval (`은 해당하지 않는다.
fn has_call(lower: &str, word: &str) -> bool {
    lower.match_indices(word).any(|(i, _)| {
        let before_ok = lower[..i].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '$'));
        before_ok && lower[i + word.len()..].trim_start().starts_with('(')
    })
}

fn suspicious(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    if let Some(k) = KEYWORDS.iter().find(|k| lower.contains(*k)) {
        return Some(k);
    }
    if has_call(&lower, "eval") {
        return Some("eval(");
    }
    // `new Function(...)`은 문자열을 코드로 실행한다. 사이 공백은 여럿일 수 있다.
    let squeezed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    if has_call(&squeezed, "new function") {
        return Some("new Function(");
    }
    None
}

/// `Scripts` 스토리지를 검사한다. 스토리지와 `FileHeader` 비트 3이 어긋나면 결함이다.
pub fn inspect(file: &mut OleFile, header: &FileHeader, report: &mut FileReport) {
    let has_storage = file.find("Scripts").is_some();
    if has_storage {
        report.push(Category::Security, Severity::High, "HWP_SCRIPT", "Scripts", "HWP document stores script code");
    }
    if header.has_scripts() != has_storage {
        let detail = if has_storage { "Scripts storage exists but FileHeader bit3 (script saved) is clear" } else { "FileHeader bit3 (script saved) is set but there is no Scripts storage" };
        report.push(Category::Defect, Severity::Medium, "HWP_SCRIPT_FLAG_MISMATCH", "Scripts", detail);
    }
    let Some(entry) = file.find("Scripts/DefaultJScript").cloned() else { return };
    let data = match file.read_stream_prefix(&entry, MAX_SCRIPT) {
        Ok(d) => d,
        Err(e) => {
            report.mark_incomplete(&format!("Scripts/DefaultJScript could not be read: {e:?}"));
            return;
        }
    };
    if entry.size > MAX_SCRIPT as u64 {
        report.mark_incomplete(&format!("Scripts/DefaultJScript ({} bytes) exceeds the scan limit; only the first {} MiB were scanned", entry.size, MAX_SCRIPT / (1024 * 1024)));
    }
    let parsed = if entry.size > MAX_SCRIPT as u64 { None } else { parse(&data) };
    let text = parsed.clone().unwrap_or_else(|| {
        // 구조가 맞지 않으면 원시 바이트를 UTF-16과 8비트로 모두 읽어 키워드만 찾는다.
        let latin1: String = data.iter().map(|&b| b as char).collect();
        format!("{}\n{latin1}", utf16_lossy(&data))
    });
    if parsed.is_none() && entry.size <= MAX_SCRIPT as u64 {
        report.mark_incomplete("Scripts/DefaultJScript does not follow the documented layout; scanned as raw bytes");
    }
    if let Some(keyword) = suspicious(&text) {
        report.push(Category::Security, Severity::Critical, "HWP_SCRIPT_SUSPICIOUS", "Scripts/DefaultJScript", &format!("script uses {keyword}"));
    }
}
