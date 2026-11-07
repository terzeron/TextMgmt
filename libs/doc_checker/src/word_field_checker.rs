//! Word 필드 지시문(DDE, INCLUDETEXT·INCLUDEPICTURE·LINK·IMPORT)의 위험 판정. 구형 DOC와 OOXML 검사기가 함께 쓴다.

use std::sync::LazyLock;

use common::text_sanitizer::sanitize_display;
use regex::Regex;

use crate::{Category, FileReport, Severity};

/// 지시문 하나에서 분석하는 최대 바이트. 정상 문서의 긴 지시문(참고문헌 관리자의 `ADDIN ... CSL_CITATION {...}`은 수십 KB까지 간다)을 담을 만큼 크게 잡는다.
pub(crate) const MAX_FIELD_TEXT: usize = 64 * 1024;
/// 지표에 보이는 글의 최대 글자 수.
const DETAIL_CHARS: usize = 160;

fn preview(text: &str) -> String {
    sanitize_display(text, DETAIL_CHARS, Some('…'))
}

/// `file:` 뒤의 문자열(역슬래시는 `/`로, 소문자로 바꾼 것)이 원격을 가리키는가. 호스트가 있으면(`file://host/...`) 원격이고,
/// 호스트가 비었거나 `localhost`여도 경로가 `//`로 시작하면(`file:////host/share`, `file://localhost//host/share`) UNC다.
/// 드라이브 경로(`file:///C:/...`, `file:/C:/...`)와 POSIX 절대 경로만 로컬이다.
/// `file:` 뒤에서 보는 최대 바이트 수.
const TAIL_LOOKAHEAD: usize = 512;

pub(crate) fn file_tail_is_remote(tail: &str) -> bool {
    // 앞 `TAIL_LOOKAHEAD`바이트만 본다. 경로의 시작(`//`, 호스트)만 필요하고, `file:`이 수십만 번 되풀이되는 입력에서 일치마다 끝까지 훑으면 제곱 시간이다.
    let mut end = tail.len().min(TAIL_LOOKAHEAD);
    while !tail.is_char_boundary(end) {
        end -= 1;
    }
    let tail = &tail[..end];
    let tail = tail.split(|c: char| c.is_whitespace() || c == '"' || c == '\'').next().unwrap_or("");
    let (authority, path) = match tail.strip_prefix("//") {
        Some(after) => {
            let end = after.find('/').unwrap_or(after.len());
            (&after[..end], &after[end..])
        }
        None => ("", tail),
    };
    (!authority.is_empty() && authority != "localhost") || path.starts_with("//")
}

/// UNC 경로가 토큰 맨 앞에 오는가. Word 지시문은 역슬래시를 두 배로 쓰므로 `"\\\\host`와 `"\\host`를 모두 본다.
/// 로컬 경로의 `c:\\dir`는 `:` 뒤에 오므로 맞지 않는다.
static FIELD_UNC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?:^|["'\s])\\{2,4}[A-Za-z0-9_.$-]"#).unwrap());
static FIELD_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\b([a-z][a-z0-9+.-]*)://([^\s"'/]*)"#).unwrap());

/// 지시문이 가리키는 원격 대상. 원격이 아니면 `None`이다. 값은 UNC·file 호스트(NTLM 해시가 새는 대상)인지다.
/// `file:`은 역슬래시를 `/`로 바꿔 본다. 그 변환을 URL 검사에 쓰면 두 배 역슬래시의 로컬 경로(`c:\\dir`)가 `c://dir`이 되어 URL로 오인된다.
fn field_remote(instr: &str) -> Option<bool> {
    if FIELD_UNC.is_match(instr) {
        return Some(true);
    }
    let normalized = instr.replace('\\', "/").to_ascii_lowercase();
    if normalized.match_indices("file:").any(|(i, _)| file_tail_is_remote(&normalized[i + 5..])) {
        return Some(true);
    }
    FIELD_URL.captures_iter(instr).any(|c| !c[1].eq_ignore_ascii_case("file")).then_some(false)
}

/// `s`를 `MAX_FIELD_TEXT`바이트까지만 남긴다(글자 경계에서 자른다). 잘렸으면 `true`다.
pub(crate) fn cap_text(s: &str) -> (&str, bool) {
    if s.len() <= MAX_FIELD_TEXT {
        return (s, false);
    }
    let mut end = MAX_FIELD_TEXT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

pub(crate) fn classify_field(instr: &str, name: &str, report: &mut FileReport) {
    // 어떤 경로로 들어와도 분석하는 길이를 상한으로 막는다(`fldSimple`의 `instr` 속성은 `push_capped`를 거치지 않는다).
    let (instr, _) = cap_text(instr);
    let word = instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    let shown = preview(name);
    match word.as_str() {
        "DDEAUTO" | "DDE" => report.push_once(Category::Security, Severity::Critical, "OOXML_DDE_FIELD", &shown, &preview(instr.trim())),
        "INCLUDETEXT" | "INCLUDEPICTURE" | "LINK" | "IMPORT" => {
            let Some(unc) = field_remote(instr) else { return };
            // 브라우저에서 붙여 넣은 그림은 Word가 `INCLUDEPICTURE "https://..."`로 쓴다. http(s)는 MEDIUM이고 UNC·file 호스트와 텍스트·링크 포함은 HIGH다.
            let severity = if word == "INCLUDEPICTURE" && !unc { Severity::Medium } else { Severity::High };
            report.push_once(Category::Security, severity, "OOXML_DANGEROUS_FIELD", &shown, &preview(instr.trim()));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn classify(instr: &str) -> FileReport {
        let mut report = FileReport::new(Path::new("a.docx"), None);
        classify_field(instr, "word/document.xml", &mut report);
        report
    }

    fn only(report: &FileReport) -> (&str, Severity) {
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        (&report.findings[0].code, report.findings[0].severity)
    }

    #[test]
    fn file_tail_with_a_host_or_a_unc_path_is_remote() {
        assert!(file_tail_is_remote("//host/share/x.doc"));
        assert!(file_tail_is_remote("////host/share"), "호스트가 비어도 경로가 //로 시작하면 UNC다");
        assert!(file_tail_is_remote("//localhost//host/share"));
    }

    #[test]
    fn file_tail_with_a_drive_or_posix_path_is_local() {
        assert!(!file_tail_is_remote("///c:/dir/x.doc"));
        assert!(!file_tail_is_remote("//localhost/c:/x"));
        assert!(!file_tail_is_remote("/c:/x"));
        assert!(!file_tail_is_remote(""));
    }

    #[test]
    fn file_tail_stops_at_whitespace_and_quotes() {
        assert!(!file_tail_is_remote("///c:/x.doc \"//evil/share\""));
        assert!(!file_tail_is_remote("///c:/x.doc'//evil/share"));
    }

    #[test]
    fn file_tail_lookahead_cuts_on_a_char_boundary() {
        // 앞 512바이트에서 자르는 위치가 가 글자 한가운데여도 panic하지 않는다.
        let tail = format!("/{}", "가".repeat(300));
        assert!(!file_tail_is_remote(&tail));
    }

    #[test]
    fn cap_text_keeps_text_within_the_limit_untouched() {
        assert_eq!(cap_text("abc"), ("abc", false));
        let exact = "a".repeat(MAX_FIELD_TEXT);
        assert_eq!(cap_text(&exact), (exact.as_str(), false));
    }

    #[test]
    fn cap_text_truncates_long_text_and_reports_it() {
        let long = "a".repeat(MAX_FIELD_TEXT + 10);
        let (kept, truncated) = cap_text(&long);
        assert_eq!(kept.len(), MAX_FIELD_TEXT);
        assert!(truncated);
    }

    #[test]
    fn cap_text_cuts_on_a_char_boundary() {
        let long = "가".repeat(MAX_FIELD_TEXT / 3 + 5);
        let (kept, truncated) = cap_text(&long);
        assert!(truncated);
        assert!(kept.len() <= MAX_FIELD_TEXT && kept.chars().all(|c| c == '가'));
    }

    #[test]
    fn dde_fields_are_critical_in_any_case() {
        for instr in ["DDEAUTO c:\\\\cmd.exe", "dde cmd", "  DdeAuto x"] {
            let report = classify(instr);
            assert_eq!(only(&report), ("OOXML_DDE_FIELD", Severity::Critical), "{instr}");
            assert_eq!(report.findings[0].location, "word/document.xml");
        }
    }

    #[test]
    fn remote_include_fields_are_high() {
        for instr in [r#"INCLUDETEXT "\\host\share\x.doc""#, r#"LINK Excel.Sheet.8 "file://host/share/x.xls""#, r#"IMPORT "http://evil.example/x.png""#, r#"INCLUDETEXT "https://evil.example/x.doc""#]
        {
            assert_eq!(only(&classify(instr)), ("OOXML_DANGEROUS_FIELD", Severity::High), "{instr}");
        }
    }

    #[test]
    fn include_picture_over_http_is_medium_but_over_unc_is_high() {
        assert_eq!(only(&classify(r#"INCLUDEPICTURE "https://cdn.example/a.png""#)), ("OOXML_DANGEROUS_FIELD", Severity::Medium));
        assert_eq!(only(&classify(r#"INCLUDEPICTURE "\\host\share\a.png""#)), ("OOXML_DANGEROUS_FIELD", Severity::High));
        assert_eq!(only(&classify(r#"INCLUDEPICTURE "file://host/share/a.png""#)), ("OOXML_DANGEROUS_FIELD", Severity::High));
    }

    #[test]
    fn local_and_unrelated_fields_are_ignored() {
        for instr in [r#"INCLUDETEXT "c:\\dir\\x.doc""#, r#"INCLUDEPICTURE "file:///c:/a.png""#, "PAGE", "", "   "] {
            assert!(classify(instr).findings.is_empty(), "{instr}");
        }
    }

    #[test]
    fn the_same_location_and_code_is_reported_once() {
        let mut report = FileReport::new(Path::new("a.docx"), None);
        classify_field("DDE a", "word/document.xml", &mut report);
        classify_field("DDE b", "word/document.xml", &mut report);
        classify_field("DDE c", "word/footer1.xml", &mut report);
        assert_eq!(report.findings.len(), 2);
    }

    #[test]
    fn analysis_stops_at_the_length_cap() {
        let hidden = format!("INCLUDETEXT {} http://evil.example/x", "a".repeat(MAX_FIELD_TEXT));
        assert!(classify(&hidden).findings.is_empty(), "상한 뒤의 URL은 보지 않는다");
    }
}
