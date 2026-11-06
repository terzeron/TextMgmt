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
