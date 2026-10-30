//! ZIP 멤버 이름 규칙. EPUB 보안 검사(`epub::security`)와 ZIP·DOCX·HWPX 검사가 같은 규칙을 쓰도록 순수 함수로 둔다.
//! 이름이 가리키는 위험은 압축을 푸는 쪽(추출기)이 입는다: 경로 탈출(Zip Slip), 절대 경로, Windows 장치 이름, NTFS 스트림.

use corpus_common::util::unquote;
use scan_common::Severity;

/// Windows 예약 장치 이름. 확장자를 떼고 소문자로 비교한다.
pub const WIN_RESERVED: &[&str] = &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];

/// 이름 하나에서 나온 지표. 위치(멤버 이름)는 호출자가 붙인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameFinding {
    pub severity: Severity,
    pub code: &'static str,
    pub detail: &'static str,
}

fn f(severity: Severity, code: &'static str, detail: &'static str) -> NameFinding {
    NameFinding { severity, code, detail }
}

/// 멤버 이름 `name`의 위험을 찾는다. 같은 코드가 두 번 나올 수 있다(긴 구성요소와 긴 전체 이름). 호출자가 (코드, 위치)로 합친다.
pub fn check(name: &str) -> Vec<NameFinding> {
    let mut out = Vec::new();
    let decoded = unquote(name).replace('\\', "/");
    let drive = decoded.len() >= 2 && decoded.as_bytes()[0].is_ascii_alphabetic() && decoded.as_bytes()[1] == b':';
    if decoded.split('/').any(|c| c == "..") {
        out.push(f(Severity::Critical, "ZIP_PATH_TRAVERSAL", "Entry name climbs out of the archive"));
    }
    if decoded.starts_with('/') || drive {
        out.push(f(Severity::High, "ZIP_ABSOLUTE_PATH", "Absolute path or drive letter"));
    }
    if name.contains('\\') {
        out.push(f(Severity::Medium, "ZIP_BACKSLASH_PATH", "Backslash in entry name"));
    }
    if name.chars().any(char::is_control) {
        out.push(f(Severity::Medium, "ZIP_CONTROL_CHAR_NAME", "Control character in entry name"));
    }
    let comps: Vec<&str> = decoded.split('/').collect();
    for (i, c) in comps.iter().enumerate() {
        let stem = c.trim_end_matches(['.', ' ']).split('.').next().unwrap_or("").to_lowercase();
        if WIN_RESERVED.contains(&stem.as_str()) {
            out.push(f(Severity::Medium, "WINDOWS_RESERVED_NAME", "Windows device name"));
        }
        if c.contains(':') && !(drive && i == 0) {
            out.push(f(Severity::Medium, "ZIP_ALT_DATA_STREAM", "Colon in name (NTFS stream)"));
        }
        if c.chars().count() > 255 {
            out.push(f(Severity::Low, "ZIP_NAME_TOO_LONG", "Path component over 255 characters"));
        }
    }
    if name.chars().count() > 1024 {
        out.push(f(Severity::Low, "ZIP_NAME_TOO_LONG", "Entry name over 1024 characters"));
    }
    out
}

/// 로컬 파일 헤더의 문제. 위치(멤버 이름)는 호출자가 붙인다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderIssue {
    pub severity: Severity,
    pub code: &'static str,
    pub detail: String,
}

/// 멤버의 로컬 헤더가 있는지, 이름이 중앙 디렉터리와 같은지 본다. 문제가 없으면 `None`이다.
/// 중앙 디렉터리만 읽는 추출기와 로컬 헤더를 따라가는 추출기가 서로 다른 파일을 보게 하는 위조를 잡는다.
pub fn local_header_issue(file: &mut std::fs::File, header_start: u64, central_name: &[u8]) -> Option<HeaderIssue> {
    use std::io::{Read, Seek, SeekFrom};
    let mut h = [0u8; 30];
    if file.seek(SeekFrom::Start(header_start)).is_err() || file.read_exact(&mut h).is_err() || &h[..4] != b"PK\x03\x04" {
        return Some(HeaderIssue { severity: Severity::High, code: "BAD_LOCAL_HEADER", detail: "ZIP local header absent or invalid".into() });
    }
    let name_len = usize::from(u16::from_le_bytes([h[26], h[27]]));
    let mut local = vec![0u8; name_len];
    if file.read_exact(&mut local).is_err() {
        return Some(HeaderIssue { severity: Severity::High, code: "TRUNCATED_LOCAL_HEADER", detail: "Incomplete local filename".into() });
    }
    if local != central_name {
        return Some(HeaderIssue { severity: Severity::High, code: "ZIP_HEADER_NAME_MISMATCH", detail: format!("Local filename differs: {}", String::from_utf8_lossy(&local)) });
    }
    None
}
