//! `Scripts` 스토리지 검사(명세 3.2.9, 표 8·9). 스크립트는 텍스트로만 본다.

use ole_reader::OleFile;
use scan_common::{Category, FileReport, Severity};

use scan_common::text::sanitize_display;

use super::header::FileHeader;
use super::inflate::{InflateError, decompress};

/// 스크립트 스트림에서 읽는 최대 바이트.
const MAX_SCRIPT: usize = 4 * 1024 * 1024;
/// 실행·내려받기·환경 접근을 뜻하는 키워드(소문자). 단어 경계가 필요한 `eval(`·`new function`은 따로 본다.
const KEYWORDS: &[&str] = &["activexobject", "wscript", "shell.", "shellexecute", "xmlhttp", "adodb.stream", "powershell", "cmd.exe", "urldownloadtofile"];
/// 검사하는 `Scripts` 아래 스트림 수의 상한.
const MAX_STREAMS: usize = 32;
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

/// 원시 바이트를 UTF-16과 8비트로 모두 읽은 텍스트. 구조를 모를 때 키워드만 찾는 데 쓴다.
fn raw_text(data: &[u8]) -> String {
    let latin1: String = data.iter().map(|&b| b as char).collect();
    format!("{}\n{latin1}", utf16_lossy(data))
}

/// 스트림을 읽어 검사할 바이트 후보를 돌려준다: 원본, 그리고 압축 비트가 서 있으면 raw deflate나 zlib으로 푼 내용.
/// 표 2는 `Scripts`를 압축 대상으로 표시하지 않으므로 압축하지 않은 스트림도 받는다. 읽는 쪽이 어느 쪽으로 읽는지 알 수 없어 둘 다 본다.
/// 상한이나 풀기 문제로 덜 본 사유도 함께 돌려준다.
fn read_script(file: &mut OleFile, entry: &ole_reader::Entry, compressed: bool) -> Result<(Vec<Vec<u8>>, Option<String>), String> {
    let data = file.read_stream_prefix(entry, MAX_SCRIPT).map_err(|e| format!("{} could not be read: {e:?}", entry.name))?;
    let mut note = (entry.size > MAX_SCRIPT as u64).then(|| format!("{} ({} bytes) exceeds the scan limit; only the first {} MiB were scanned", entry.name, entry.size, MAX_SCRIPT / (1024 * 1024)));
    let mut candidates = Vec::new();
    // 8바이트 이하(`JScriptVersion` 등)는 풀 필요가 없다.
    if compressed && data.len() > 8 {
        match decompress(&data, MAX_SCRIPT) {
            Ok(inflated) => candidates.push(inflated),
            Err(InflateError::TooLarge) => note = Some(format!("{} decompresses beyond the {} MiB scan limit", entry.name, MAX_SCRIPT / (1024 * 1024))),
            // 압축된 스트림이 아니었다. 원본만 본다.
            Err(InflateError::Invalid) => {}
        }
    }
    candidates.push(data);
    Ok((candidates, note))
}

/// `Scripts` 스토리지를 검사한다. 한글은 스크립트가 없어도 빈 `DefaultJScript`가 든 `Scripts` 저장소를 항상 쓰므로(실제 HWP5로 확인했다)
/// 저장소가 있다는 것만으로는 지표가 아니다. `FileHeader` 비트 3(스크립트 저장)이 서 있거나, 소스가 비어 있지 않거나, 비어 있음을 증명하지 못할 때만 `HWP_SCRIPT`다.
/// 비트만 서 있고 저장소가 없으면 결함이다. `DefaultJScript`는 표 9 구조로, 그 밖의 `Scripts` 아래 스트림은 원시 텍스트로 키워드를 찾는다.
pub fn inspect(file: &mut OleFile, header: &FileHeader, report: &mut FileReport) {
    let has_storage = file.find("Scripts").is_some();
    if header.has_scripts() && !has_storage {
        report.push(Category::Defect, Severity::Medium, "HWP_SCRIPT_FLAG_MISMATCH", "Scripts", "FileHeader bit3 (script saved) is set but there is no Scripts storage");
    }
    if !has_storage {
        return;
    }
    // 작성자가 스크립트 저장을 선언했으면 소스가 비었어도 스크립트가 있는 문서다.
    let mut present = header.has_scripts();
    let mut streams: Vec<(String, ole_reader::Entry)> = file
        .entries()
        .iter()
        .filter(|e| e.kind == ole_reader::EntryKind::Stream)
        .map(|e| (file.path_of(e), e.clone()))
        .filter(|(path, _)| path.to_lowercase().starts_with("scripts/"))
        .collect();
    // `DefaultJScript`를 먼저 보고, 상한 때문에 건너뛴 스트림이 있으면 깨끗하다고 하지 않는다.
    streams.sort_by_key(|(_, e)| !e.name.eq_ignore_ascii_case("DefaultJScript"));
    if streams.len() > MAX_STREAMS {
        report.mark_incomplete(&format!("{} streams under Scripts exceed the scan limit of {MAX_STREAMS}; the rest were not scanned", streams.len() - MAX_STREAMS));
        streams.truncate(MAX_STREAMS);
    }
    for (path, entry) in streams {
        let (candidates, note) = match read_script(file, &entry, header.compressed()) {
            Ok(r) => r,
            Err(reason) => {
                // 읽지 못했으면 비어 있음을 증명하지 못한다.
                present = true;
                report.mark_incomplete(&reason);
                continue;
            }
        };
        if let Some(note) = note {
            present = true;
            report.mark_incomplete(&note);
        }
        let default = entry.name.eq_ignore_ascii_case("DefaultJScript");
        let within_limit = entry.size <= MAX_SCRIPT as u64;
        // 표 9 구조로 읽히는 후보는 그 텍스트를, 그렇지 않은 후보는 원시 바이트를 본다. 원본의 원시 바이트는 항상 함께 본다(표 뒤에 붙은 바이트 포함).
        let parsed: Vec<String> = if default && within_limit { candidates.iter().filter_map(|c| parse(c)).collect() } else { Vec::new() };
        if default && within_limit && parsed.is_empty() {
            report.mark_incomplete("Scripts/DefaultJScript does not follow the documented layout; scanned as raw bytes");
        }
        // `DefaultJScript`는 소스가 비어 있지 않거나 표로 읽히지 않을 때, 그 밖의 알 수 없는 스트림은 있다는 것만으로 스크립트가 있다고 본다(`JScriptVersion`은 버전 숫자뿐이다).
        if default {
            present |= parsed.is_empty() || parsed.iter().any(|t| !t.trim().is_empty());
        } else if !entry.name.eq_ignore_ascii_case("JScriptVersion") {
            present = true;
        }
        let original = candidates.last().map(|d| raw_text(d)).unwrap_or_default();
        let inflated: Vec<String> = candidates.iter().rev().skip(1).map(|d| raw_text(d)).collect();
        let text = [parsed, inflated, vec![original]].concat().join("\n");
        if let Some(keyword) = suspicious(&text) {
            present = true;
            report.push(Category::Security, Severity::Critical, "HWP_SCRIPT_SUSPICIOUS", &sanitize_display(&path, 120, Some('…')), &format!("script uses {keyword}"));
        }
    }
    if present {
        report.push(Category::Security, Severity::High, "HWP_SCRIPT", "Scripts", "HWP document stores script code");
    }
}
