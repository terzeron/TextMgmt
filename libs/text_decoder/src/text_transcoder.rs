//! 인코딩 이름([`Enc`])과 엄격·관대 디코딩, 같은 인코딩으로 다시 인코딩하기.
//! TXT 읽기(`doc_reader::text`)와 TXT 재인코딩(`doc_repairer::encoding_repairer`)이 함께 쓴다.

use encoding_rs::{EUC_KR, WINDOWS_1252};

/// 텍스트 인코딩.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enc {
    Utf8,
    Cp949,
    EucKr,
    Utf16,
    Utf16Le,
    Utf16Be,
    Utf8Sig,
    Cp1252,
}

impl Enc {
    pub fn name(self) -> &'static str {
        match self {
            Enc::Utf8 => "utf-8",
            Enc::Cp949 => "cp949",
            Enc::EucKr => "euc-kr",
            Enc::Utf16 => "utf-16",
            Enc::Utf16Le => "utf-16-le",
            Enc::Utf16Be => "utf-16-be",
            Enc::Utf8Sig => "utf-8-sig",
            Enc::Cp1252 => "cp1252",
        }
    }
}

fn utf16(raw: &[u8], big_endian: bool, replace: bool) -> Option<String> {
    let enc = if big_endian { encoding_rs::UTF_16BE } else { encoding_rs::UTF_16LE };
    if replace { Some(enc.decode_without_bom_handling(raw).0.into_owned()) } else { enc.decode_without_bom_handling_and_without_replacement(raw).map(|c| c.into_owned()) }
}

fn decode_with(enc: &'static encoding_rs::Encoding, raw: &[u8], replace: bool) -> Option<String> {
    if replace { Some(enc.decode_without_bom_handling(raw).0.into_owned()) } else { enc.decode_without_bom_handling_and_without_replacement(raw).map(|c| c.into_owned()) }
}

/// 엄격하게 풀거나, 못 읽는 바이트를 U+FFFD로 바꿔 푼다. 텍스트 모드 줄바꿈 번역은 하지 않는다.
pub fn decode(enc: Enc, raw: &[u8], replace: bool) -> Option<String> {
    match enc {
        Enc::Utf8 => decode_with(encoding_rs::UTF_8, raw, replace),
        Enc::Utf8Sig => decode_with(encoding_rs::UTF_8, raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(raw), replace),
        // 엄격 디코딩은 WHATWG euc-kr(=cp949)로 충분하다. replace는 안 읽히는 바이트를 하나씩 바꾼다.
        Enc::Cp949 | Enc::EucKr if replace => Some(decode_korean_with_replace(raw, enc == Enc::EucKr)),
        Enc::Cp949 | Enc::EucKr if valid_korean_bytes(raw, enc == Enc::EucKr) => decode_with(EUC_KR, raw, false),
        Enc::Cp949 | Enc::EucKr => None,
        Enc::Cp1252 => decode_with(WINDOWS_1252, raw, replace),
        Enc::Utf16Le => utf16(raw, false, replace),
        Enc::Utf16Be => utf16(raw, true, replace),
        // BOM이 있으면 그 엔디언을 따르고 BOM은 버린다. 없으면 리틀엔디언이다.
        Enc::Utf16 => match raw {
            [0xff, 0xfe, rest @ ..] => utf16(rest, false, replace),
            [0xfe, 0xff, rest @ ..] => utf16(rest, true, replace),
            _ => utf16(raw, false, replace),
        },
    }
}

fn decode_korean_with_replace(raw: &[u8], euc_kr: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] < 0x80 {
            out.push(raw[i] as char);
            i += 1;
            continue;
        }
        if raw.get(i + 1).is_some()
            && valid_korean_bytes(&raw[i..i + 2], euc_kr)
            && let Some(decoded) = decode_with(EUC_KR, &raw[i..i + 2], false)
        {
            out.push_str(&decoded);
            i += 2;
        } else {
            out.push('\u{fffd}');
            i += 1;
        }
    }
    out
}

pub(crate) fn valid_korean_bytes(raw: &[u8], euc_kr: bool) -> bool {
    let mut i = 0;
    while i < raw.len() {
        let lead = raw[i];
        if lead < 0x80 {
            i += 1;
            continue;
        }
        let Some(&trail) = raw.get(i + 1) else { return false };
        let valid = if euc_kr {
            (0xa1..=0xfe).contains(&lead) && (0xa1..=0xfe).contains(&trail)
        } else {
            (0x81..=0xfe).contains(&lead) && ((0x41..=0x5a).contains(&trail) || (0x61..=0x7a).contains(&trail) || (0x81..=0xfe).contains(&trail))
        };
        if !valid {
            return false;
        }
        i += 2;
    }
    true
}

/// 디코딩한 텍스트를 같은 인코딩으로 다시 인코딩한다(왕복 검증용). 인코딩할 수 없으면 `None`.
pub fn encode(enc: Enc, text: &str) -> Option<Vec<u8>> {
    match enc {
        Enc::Utf8 => Some(text.as_bytes().to_vec()),
        // `utf-8-sig`와 `utf-16`은 인코딩할 때 항상 BOM을 붙인다.
        Enc::Utf8Sig => Some([b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat()),
        Enc::Utf16 => Some(std::iter::once(0xfeffu16).chain(text.encode_utf16()).flat_map(u16::to_le_bytes).collect()),
        Enc::Utf16Le => Some(text.encode_utf16().flat_map(u16::to_le_bytes).collect()),
        Enc::Utf16Be => Some(text.encode_utf16().flat_map(u16::to_be_bytes).collect()),
        Enc::Cp949 | Enc::EucKr | Enc::Cp1252 => {
            let e = if enc == Enc::Cp1252 { WINDOWS_1252 } else { EUC_KR };
            let (bytes, _, had_errors) = e.encode(text);
            (!had_errors).then(|| bytes.into_owned())
        }
    }
}

/// 호출자가 정하는 인코딩 후보. `Custom`은 내장 [`Enc`] 밖의 코덱을 함수 한 쌍으로 넘긴다.
#[derive(Clone)]
pub enum Candidate<'a> {
    Builtin(Enc),
    Custom { name: &'a str, decode: fn(&[u8]) -> String, encode: fn(&str) -> Option<Vec<u8>> },
}
