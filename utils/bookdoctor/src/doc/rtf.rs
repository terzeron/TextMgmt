//! RTF 검사. RTF는 텍스트 형식이라 Word가 `.doc`·`.docx`라는 이름으로도 연다. 내장 OLE 객체를 심는 공격의 통로다.
//! 구조를 해석하지 않고 바이트를 선형으로 훑는다(중첩 깊이를 세지 않으므로 깊이 폭탄이 통하지 않는다).

use scan_common::sniff::{Sniffed, sniff};
use scan_common::text::sanitize_display;
use scan_common::{Category, FileReport, Severity};

/// 객체를 뜻하는 제어어. 하나라도 있으면 내장 객체가 있다.
/// `\datastore`는 뺀다: Word가 저장한 RTF는 모두 customXml용 `\datastore`를 가지므로 이것만으로는 신호가 못 된다.
const OBJECT_WORDS: &[&str] = &["objdata", "objclass", "objupdate", "objautlink"];
/// 제어어 이름으로 읽는 최대 글자 수. RTF 규격은 32자다.
const MAX_WORD_LEN: usize = 32;
/// `\objdata` 하나에서 해독하는 앞부분(바이트). OLE1 머리말 뒤의 본문 매직까지 닿는다.
const PAYLOAD_PREFIX: usize = 4096;
/// 페이로드를 확인하는 `\objdata` 수의 상한.
const MAX_OBJDATA: usize = 256;
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// `\'xx`로 쓴 영문자·숫자를 그 문자로 되돌린다. RTF 파서는 이 이스케이프를 합쳐 읽으므로 `\obj\'64ata`도 `\objdata`다.
/// 그 밖의 바이트(구분자나 제어 문자가 될 수 있는 값)는 건드리지 않는다.
fn normalize(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && bytes.get(i + 1) == Some(&b'\'')
            && let (Some(&h), Some(&l)) = (bytes.get(i + 2), bytes.get(i + 3))
            && let (Some(h), Some(l)) = ((h as char).to_digit(16), (l as char).to_digit(16))
        {
            let decoded = (h * 16 + l) as u8;
            if decoded.is_ascii_alphanumeric() {
                out.push(decoded);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// `pos`(`\` 다음 바이트)부터 제어어 이름을 읽는다. 이름과 이름이 끝난 위치를 돌려준다.
fn control_word(data: &[u8], pos: usize) -> (&[u8], usize) {
    let end = data[pos..].iter().take(MAX_WORD_LEN + 1).position(|b| !b.is_ascii_alphabetic()).map_or(data.len().min(pos + MAX_WORD_LEN + 1), |n| pos + n);
    (&data[pos..end], end)
}

/// 이름 뒤의 숫자 인자와 구분 공백 하나를 건너뛴다.
fn skip_parameter(data: &[u8], mut pos: usize) -> usize {
    if data.get(pos) == Some(&b'-') {
        pos += 1;
    }
    while data.get(pos).is_some_and(u8::is_ascii_digit) {
        pos += 1;
    }
    if data.get(pos) == Some(&b' ') {
        pos += 1;
    }
    pos
}

/// `\objdata` 본문의 16진 앞부분을 해독한다. 공백과 줄바꿈은 건너뛰고 16진이 아닌 문자에서 멈춘다.
fn decode_hex_prefix(data: &[u8], mut pos: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pending: Option<u32> = None;
    while let Some(&b) = data.get(pos) {
        pos += 1;
        match (b as char).to_digit(16) {
            Some(d) => match pending.take() {
                Some(h) => {
                    out.push((h * 16 + d) as u8);
                    if out.len() >= PAYLOAD_PREFIX {
                        break;
                    }
                }
                None => pending = Some(d),
            },
            None if matches!(b, b' ' | b'\r' | b'\n' | b'\t') => {}
            None => break,
        }
    }
    out
}

#[derive(PartialEq)]
enum Payload {
    None,
    Ole,
    Executable,
}

/// 해독한 앞부분에서 실행 파일이나 OLE 복합 파일을 찾는다. OLE1 머리말(`01050000`)은 클래스 이름 뒤에 본문이 오므로 앞부분 어디든 본다.
fn payload_kind(decoded: &[u8]) -> Payload {
    let mut kind = Payload::None;
    for i in 0..decoded.len() {
        let rest = &decoded[i..];
        if rest.starts_with(&OLE_MAGIC) {
            kind = Payload::Ole;
        } else if rest.starts_with(b"MZ") && sniff(rest) == Sniffed::Pe || rest.starts_with(b"\x7fELF") {
            return Payload::Executable;
        }
    }
    kind
}

fn push_once(report: &mut FileReport, severity: Severity, code: &str, location: &str, detail: &str) {
    if !report.has(code) {
        report.push(Category::Security, severity, code, location, detail);
    }
}

/// RTF 본문(`bytes`)을 검사한다. 지표는 코드마다 한 번만 낸다.
pub fn inspect(bytes: &[u8], report: &mut FileReport) {
    let data = normalize(bytes);
    let mut i = 0;
    let mut objdata_seen = 0;
    while let Some(off) = data[i..].iter().position(|&b| b == b'\\') {
        let start = i + off + 1;
        let (name, end) = control_word(&data, start);
        i = start.max(end);
        let Ok(name) = std::str::from_utf8(name) else { continue };
        let lower = name.to_ascii_lowercase();
        let datastore = lower == "datastore";
        if !datastore && !OBJECT_WORDS.contains(&lower.as_str()) {
            continue;
        }
        if datastore {
            push_once(report, Severity::Medium, "RTF_DATASTORE", "\\datastore", "RTF has a data store (Word writes one for customXml); payload checked");
        } else {
            push_once(report, Severity::High, "RTF_EMBEDDED_OBJECT", &sanitize_display(&format!("\\{lower}"), 40, Some('…')), "RTF embeds an OLE object");
        }
        // 16진 본문을 갖는 제어어는 앞부분에서 실행 파일·OLE 매직을 찾는다.
        if (lower == "objdata" || datastore) && objdata_seen < MAX_OBJDATA {
            objdata_seen += 1;
            match payload_kind(&decode_hex_prefix(&data, skip_parameter(&data, end))) {
                Payload::Executable => push_once(report, Severity::Critical, "RTF_EMBEDDED_EXECUTABLE", &format!("\\{lower}"), "Embedded object payload is an executable (PE/ELF)"),
                Payload::Ole => push_once(report, Severity::High, "RTF_EMBEDDED_OLE", &format!("\\{lower}"), "Embedded object payload is an OLE compound file"),
                Payload::None => {}
            }
        }
    }
}
