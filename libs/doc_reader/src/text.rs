//! TXT 읽기. 원본: `Loader.read_from_text`, `Loader.reencode_text_file_to_utf8`.
//!
//! 원본은 chardet으로 후보 순서를 정한다. chardet은 옮기지 않았고, 같은 목적의 간단한 판별
//! ([`detect_hint`])으로 대신한다. 엄격 UTF-8로 읽히는 파일(코퍼스 표본 거의 전부)에서는 같은 길을 탄다.

use std::path::Path;
use std::{fs, io};

use encoding_rs::{EUC_KR, WINDOWS_1252};

use crate::pytext::{collapse_spaces, head_chars, is_all_space, keep_ascii_hangul, universal_newlines};
use crate::{TEXT_SIZE, mojibake};

/// txt 디코딩 시도 순서(원본 `TEXT_ENCODINGS`).
pub const TEXT_ENCODINGS: [Enc; 7] = [Enc::Utf8, Enc::Cp949, Enc::EucKr, Enc::Utf16, Enc::Utf16Le, Enc::Utf16Be, Enc::Utf8Sig];
/// 상위바이트가 고립되어 나타나면 서양어 단일바이트로 본다. 인접 비율 기준.
pub const HIGH_BYTE_PAIRED_MIN: f64 = 0.5;
/// 판별에 쓰는 앞부분 바이트 수.
pub const DETECT_PREFIX: usize = 8 * 1024;

/// Python 코덱 이름에 대응하는 인코딩.
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

    fn is_checked(self) -> bool {
        matches!(self, Enc::Cp949 | Enc::EucKr | Enc::Utf16 | Enc::Utf16Le | Enc::Utf16Be)
    }
}

fn utf16(raw: &[u8], big_endian: bool, replace: bool) -> Option<String> {
    let enc = if big_endian { encoding_rs::UTF_16BE } else { encoding_rs::UTF_16LE };
    if replace { Some(enc.decode_without_bom_handling(raw).0.into_owned()) } else { enc.decode_without_bom_handling_and_without_replacement(raw).map(|c| c.into_owned()) }
}

fn decode_with(enc: &'static encoding_rs::Encoding, raw: &[u8], replace: bool) -> Option<String> {
    if replace { Some(enc.decode_without_bom_handling(raw).0.into_owned()) } else { enc.decode_without_bom_handling_and_without_replacement(raw).map(|c| c.into_owned()) }
}

/// Python `bytes.decode(enc)`(엄격) 또는 `errors="replace"`. 텍스트 모드 줄바꿈 번역은 하지 않는다.
pub fn decode(enc: Enc, raw: &[u8], replace: bool) -> Option<String> {
    match enc {
        Enc::Utf8 => decode_with(encoding_rs::UTF_8, raw, replace),
        Enc::Utf8Sig => decode_with(encoding_rs::UTF_8, raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(raw), replace),
        // 엄격 디코딩은 WHATWG euc-kr(=cp949)로 충분하다. replace는 Python 코덱처럼 안 읽히는 바이트 하나씩 바꾼다.
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

fn valid_korean_bytes(raw: &[u8], euc_kr: bool) -> bool {
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

pub(crate) fn decode_markup_bytes(raw: &[u8]) -> Option<String> {
    decode_text_with(Path::new(""), raw, &TextHooks::default()).ok()
}

/// 상위바이트(0x80~0xFF) 연속 구간으로 (고립 아닌 바이트 수 / 전체 상위바이트 수)를 센다. 상위바이트가 없으면 `None`.
pub fn high_byte_paired_ratio(raw: &[u8]) -> Option<f64> {
    let (mut total, mut paired, mut run) = (0usize, 0usize, 0usize);
    for &b in raw.iter().chain(std::iter::once(&0u8)) {
        if b >= 0x80 {
            run += 1;
        } else if run > 0 {
            total += run;
            if run >= 2 {
                paired += run;
            }
            run = 0;
        }
    }
    (total > 0).then(|| paired as f64 / total as f64)
}

/// 상위바이트가 모두 KS X 1001 영역(두 바이트 모두 0xA1~0xFE)의 쌍인가. 잘린 끝 바이트는 무시한다.
fn is_ksx1001_only(raw: &[u8]) -> bool {
    let mut i = 0;
    while i < raw.len() {
        if raw[i] < 0x80 {
            i += 1;
        } else if let Some(&d) = raw.get(i + 1) {
            if !(0xa1..=0xfe).contains(&raw[i]) || !(0xa1..=0xfe).contains(&d) {
                return false;
            }
            i += 2;
        } else {
            break;
        }
    }
    true
}

/// chardet 대신 쓰는 후보 힌트. BOM, 상위바이트 인접 비율로 가장 그럴듯한 인코딩 하나를 고른다.
pub fn detect_hint(prefix: &[u8]) -> Option<Enc> {
    if prefix.starts_with(b"\xef\xbb\xbf") {
        return Some(Enc::Utf8Sig);
    }
    if prefix.starts_with(&[0xff, 0xfe]) || prefix.starts_with(&[0xfe, 0xff]) {
        return Some(Enc::Utf16);
    }
    match high_byte_paired_ratio(prefix) {
        None => None,
        // chardet은 KS X 1001 안에 머무르면 EUC-KR, 확장 완성형(UHC)이 섞이면 CP949라고 답한다.
        Some(r) if r >= HIGH_BYTE_PAIRED_MIN => {
            if valid_korean_bytes(prefix, true) && is_ksx1001_only(prefix) {
                Some(Enc::EucKr)
            } else if valid_korean_bytes(prefix, false) {
                Some(Enc::Cp949)
            } else {
                None
            }
        }
        Some(_) => Some(Enc::Cp1252),
    }
}

fn has_text_char(s: &str) -> bool {
    s.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c) || c.is_ascii_alphanumeric())
}

/// 읽기 결과: 요약, 줄 수, 쪽 수(TXT는 0), 전체 본문.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextRead {
    pub summary: String,
    pub line_count: usize,
    pub page_count: usize,
    pub raw_content: String,
}

pub type TextDetector<'a> = dyn Fn(&[u8]) -> Option<Enc> + 'a;
pub type BackupWriter<'a> = dyn Fn(&Path, &[u8]) -> io::Result<()> + 'a;
pub type TextReader<'a> = dyn Fn(&Path) -> io::Result<Vec<u8>> + 'a;
pub type ReplaceReader<'a> = dyn Fn(&Path) -> io::Result<String> + 'a;

#[derive(Clone)]
pub enum Candidate<'a> {
    Builtin(Enc),
    Custom { name: &'a str, decode: fn(&[u8]) -> String, encode: fn(&str) -> Option<Vec<u8>> },
}

/// TXT 판별과 재인코딩 I/O의 주입 경계.
#[derive(Clone, Default)]
pub struct TextHooks<'a> {
    pub detect: Option<&'a TextDetector<'a>>,
    pub write_backup: Option<&'a BackupWriter<'a>>,
    pub read_bytes: Option<&'a TextReader<'a>>,
    pub encodings: Option<Vec<Candidate<'a>>>,
    pub read_replace: Option<&'a ReplaceReader<'a>>,
}

/// 인코딩 후보를 순서대로 시도해 본문을 읽는다. 읽을 수 없으면 `Err`(원본은 빈 본문과 오류 로그).
fn decode_text_with(path: &Path, raw: &[u8], hooks: &TextHooks<'_>) -> Result<String, String> {
    let mut order: Vec<Enc> =
        hooks.encodings.as_ref().map(|cs| cs.iter().filter_map(|c| if let Candidate::Builtin(e) = c { Some(*e) } else { None }).collect()).unwrap_or_else(|| TEXT_ENCODINGS.to_vec());
    let skip_detect = std::str::from_utf8(raw).is_ok() && !raw.starts_with(b"\xef\xbb\xbf");
    if !skip_detect
        && hooks.encodings.is_none()
        && let Some(hint) = hooks.detect.map_or_else(|| detect_hint(&raw[..raw.len().min(DETECT_PREFIX)]), |f| f(&raw[..raw.len().min(DETECT_PREFIX)]))
    {
        order.retain(|e| *e != hint);
        order.insert(0, hint);
    }

    let mut last_error = None;
    for enc in &order {
        match decode(*enc, raw, false) {
            Some(content) => {
                let content = universal_newlines(&content);
                if enc.is_checked() && !content.is_empty() && !has_text_char(&content) && !is_all_space(&content) {
                    continue;
                }
                return Ok(content);
            }
            None => last_error = Some(format!("'{}' codec can't decode bytes", enc.name())),
        }
    }
    if let Some(candidates) = &hooks.encodings {
        for candidate in candidates {
            if let Candidate::Custom { decode, .. } = candidate {
                let content = universal_newlines(&decode(raw));
                if !content.is_empty() {
                    return Ok(content);
                }
            }
        }
    }
    if let Some(reader) = hooks.read_replace
        && let Ok(content) = reader(path)
        && !content.is_empty()
    {
        return Ok(universal_newlines(&content));
    }
    for enc in &order {
        if let Some(content) = decode(*enc, raw, true) {
            let content = universal_newlines(&content);
            if enc.is_checked() && !content.is_empty() && !has_text_char(&content) && !is_all_space(&content) {
                continue;
            }
            if !content.is_empty() {
                return Ok(content);
            }
        }
    }
    Err(last_error.unwrap_or_default())
}

/// 요약(앞 `TEXT_SIZE`자를 영문·숫자·공백·한글만 남겨 정리)과 줄 수를 만든다.
pub fn summarize(raw_content: &str) -> (String, usize) {
    if raw_content.is_empty() {
        return (String::new(), 0);
    }
    let line_count = raw_content.matches('\n').count() + 1;
    let head = head_chars(raw_content, TEXT_SIZE).replace('\u{feff}', "");
    (collapse_spaces(&keep_ascii_hangul(&head)), line_count)
}

/// TXT 파일을 읽는다. 읽지 못하면 빈 결과와 오류 메시지를 함께 돌려준다.
pub fn read_from_text(path: &Path) -> (TextRead, Option<String>) {
    read_from_text_with(path, &TextHooks::default())
}

pub fn read_from_text_with(path: &Path, hooks: &TextHooks<'_>) -> (TextRead, Option<String>) {
    let raw = match hooks.read_bytes.map_or_else(|| fs::read(path), |read| read(path)) {
        Ok(raw) => raw,
        Err(e) => return (TextRead::default(), Some(format!("can't read unicode text from file '{}', {e}", path.display()))),
    };
    let (raw_content, error) = match decode_text_with(path, &raw, hooks) {
        Ok(content) => (content, None),
        Err(e) => (String::new(), Some(format!("can't read unicode text from file '{}', {e}", path.display()))),
    };
    let (summary, line_count) = summarize(&raw_content);
    (TextRead { summary, line_count, page_count: 0, raw_content }, error)
}

/// `reencode_text_file_to_utf8`이 쓰는 인코딩 순서. 근거 없는 UTF-16과, 고립 상위바이트일 때의 한글 코덱을 뺀다.
pub fn reencode_order(raw: &[u8]) -> Vec<Enc> {
    let mut order: Vec<Enc> = TEXT_ENCODINGS.to_vec();
    let has_bom = raw.starts_with(&[0xff, 0xfe]) || raw.starts_with(&[0xfe, 0xff]);
    let nul_ratio = raw.iter().filter(|&&b| b == 0).count() as f64 / raw.len().max(1) as f64;
    if !has_bom && nul_ratio < 0.02 {
        order.retain(|e| !matches!(e, Enc::Utf16 | Enc::Utf16Le | Enc::Utf16Be));
    }
    if let Some(ratio) = high_byte_paired_ratio(raw)
        && ratio < HIGH_BYTE_PAIRED_MIN
    {
        order.retain(|e| !matches!(e, Enc::Cp949 | Enc::EucKr));
        order.push(Enc::Cp1252);
    }
    order
}

/// 디코딩한 텍스트를 같은 인코딩으로 다시 인코딩한다(왕복 검증용). 인코딩할 수 없으면 `None`.
fn encode(enc: Enc, text: &str) -> Option<Vec<u8>> {
    match enc {
        Enc::Utf8 => Some(text.as_bytes().to_vec()),
        // Python `utf-8-sig`와 `utf-16`은 인코딩할 때 항상 BOM을 붙인다.
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

/// 한 인코딩으로 읽은 결과가 원본을 무손실로 되살리고 타당한지 검사한다. 통과하면 텍스트를 돌려준다.
fn validated_decode(enc: Enc, raw: &[u8]) -> Option<String> {
    let text = decode(enc, raw, false)?;
    if text.contains('\u{fffd}') {
        return None;
    }
    // 1. 바이트 왕복: 무손실·가역을 확정한다.
    let round = encode(enc, &text)?;
    if round != raw {
        return None;
    }
    // 2. 타당성: 실제 글자가 있어야 하고 mojibake로 읽힌 결과여서는 안 된다.
    if !text.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c) || c.is_ascii_alphabetic() || c.is_ascii_digit() || ('\u{3400}'..='\u{9fff}').contains(&c)) {
        return None;
    }
    if mojibake::count_mojibake_runs(&text) >= mojibake::PDF_MOJIBAKE_RUN_LIMIT {
        return None;
    }
    // 텍스트에는 줄바꿈이 있다. 엔디언을 잘못 고른 UTF-16은 줄바꿈이 사라진다.
    if text.chars().count() >= 200 && !text.contains('\n') && !text.contains('\r') {
        return None;
    }
    let sample = head_chars(&text, TEXT_SIZE);
    let total = sample.chars().count();
    if total > 0 && sample.chars().filter(|c| ('\u{80}'..='\u{ff}').contains(c)).count() as f64 / total as f64 > 0.1 {
        return None;
    }
    Some(text)
}

fn validated_custom_decode(decode: fn(&[u8]) -> String, encode: fn(&str) -> Option<Vec<u8>>, raw: &[u8]) -> Option<String> {
    let text = decode(raw);
    if text.contains('\u{fffd}') || encode(&text)?.as_slice() != raw {
        return None;
    }
    if !text.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c) || c.is_ascii_alphanumeric() || ('\u{3400}'..='\u{9fff}').contains(&c)) {
        return None;
    }
    (mojibake::count_mojibake_runs(&text) < mojibake::PDF_MOJIBAKE_RUN_LIMIT).then_some(text)
}

/// 재인코딩 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reencode {
    pub changed: bool,
    pub reason: String,
}

fn no(reason: impl Into<String>) -> Reencode {
    Reencode { changed: false, reason: reason.into() }
}

/// txt 파일을 UTF-8로 재인코딩해 같은 inode에 덮어쓴다.
///
/// 검증을 모두 통과하지 못하면 파일에 손대지 않는다. 바이트 왕복, 타당성, 저장 후 재읽기를 확인한다.
/// ES 문서 `_id`가 inode이므로 임시 파일 교체를 쓰지 않는다. `backup_dir`이 있으면 덮어쓰기 전에
/// 원본을 그 아래 `relative`로 복사·검증하고, 실패하면 변환하지 않는다.
pub fn reencode_text_file_to_utf8(path: &Path, dry_run: bool, backup_dir: Option<&Path>, relative: &Path) -> Reencode {
    reencode_text_file_to_utf8_with(path, dry_run, backup_dir, relative, &TextHooks::default())
}

pub fn reencode_text_file_to_utf8_with(path: &Path, dry_run: bool, backup_dir: Option<&Path>, relative: &Path, hooks: &TextHooks<'_>) -> Reencode {
    let read_bytes = |path: &Path| hooks.read_bytes.map_or_else(|| fs::read(path), |f| f(path));
    let raw = match read_bytes(path) {
        Ok(raw) => raw,
        Err(e) => return no(format!("읽기 실패: {e}")),
    };
    if raw.is_empty() {
        return no("빈 파일");
    }
    if std::str::from_utf8(&raw).is_ok() {
        return no("이미 UTF-8");
    }
    // 판별 힌트가 가리키는 인코딩을 앞으로 보낸다(허용 목록 안에서만).
    let mut order = reencode_order(&raw);
    if let Some(hint) = hooks.detect.map_or_else(|| detect_hint(&raw[..raw.len().min(DETECT_PREFIX)]), |f| f(&raw[..raw.len().min(DETECT_PREFIX)]))
        && let Some(pos) = order.iter().position(|e| *e == hint)
    {
        let e = order.remove(pos);
        order.insert(0, e);
    }
    let standard = || order.iter().find_map(|e| validated_decode(*e, &raw).map(|t| (*e, t)));
    let selected = hooks
        .encodings
        .as_ref()
        .and_then(|candidates| {
            candidates.iter().find_map(|candidate| match candidate {
                Candidate::Builtin(enc) => validated_decode(*enc, &raw).map(|text| (*enc, text)),
                Candidate::Custom { decode, encode, name: _ } => validated_custom_decode(*decode, *encode, &raw).map(|text| (Enc::Utf8, text)),
            })
        })
        .or_else(|| if hooks.encodings.is_none() { standard() } else { None });
    let Some((enc, text)) = selected else {
        return no("무손실 디코딩 가능한 인코딩을 찾지 못함");
    };
    let utf8 = text.as_bytes();
    let summary = format!("{} → utf-8 ({}자, {}B → {}B)", enc.name(), text.chars().count(), raw.len(), utf8.len());
    if dry_run {
        return Reencode { changed: true, reason: format!("{summary} [dry-run]") };
    }
    if let Some(dir) = backup_dir {
        let backup = dir.join(relative);
        let result = (|| -> std::io::Result<bool> {
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Some(write_backup) = hooks.write_backup {
                write_backup(&backup, &raw)?;
            } else {
                fs::write(&backup, &raw)?;
            }
            Ok(read_bytes(&backup)? == raw)
        })();
        match result {
            Ok(true) => {}
            Ok(false) => return no(format!("백업 검증 실패: {}", backup.display())),
            Err(e) => return no(format!("백업 실패: {e}")),
        }
    }
    // 검증된 내용을 먼저 임시 파일에 보존한 뒤 같은 inode에 덮어쓴다.
    let tmp = path.with_file_name(format!("{}.utf8.tmp", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        fs::write(&tmp, utf8)?;
        let mut file = fs::OpenOptions::new().write(true).open(path)?;
        file.write_all(utf8)?;
        file.set_len(utf8.len() as u64)?;
        file.flush()?;
        file.sync_all()
    })();
    let _ = fs::remove_file(&tmp);
    if let Err(e) = written {
        return no(format!("쓰기 실패: {e}"));
    }
    match read_bytes(path).map_err(|e| e.to_string()).and_then(|b| String::from_utf8(b).map_err(|e| e.to_string())) {
        Err(e) => no(format!("저장 후 재읽기 실패: {e}")),
        Ok(saved) if saved != text => no(format!("저장 후 불일치: {}자 → {}자", text.chars().count(), saved.chars().count())),
        Ok(_) => Reencode { changed: true, reason: summary },
    }
}
