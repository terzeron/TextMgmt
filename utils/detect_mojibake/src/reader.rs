//! 파일에서 본문 표본을 읽는다. backend/classifier/reader.py 중 탐지기가 쓰는 부분의 이식.
//!
//! 인코딩 판별은 네 번 고쳐 잡은 로직이다. 다시 쓰지 말 것:
//!   - "한글이 하나라도 나오면 그 인코딩 채택"은 정상 CP949 소설 1,856건을 오판했다
//!   - 한글 비율 하한이 없으면 영문 OCR 문서를 한글 잡음 60자로 판정한다
//!   - UTF-16을 빼면 97건을 못 읽는다
//!   - EPUB은 첫 문서가 목차인 경우가 있어 건너뛰어야 한다

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use zip::ZipArchive;
use zip::read::{ArchiveOffset, Config};

pub const COMMON_HANGUL_SYLLABLES: &str = "이다는을가지에그고하의아은어도로리나를한었기서들게니있라자사시만으해마보했수대것";

/// 이 수보다 음절이 적으면 비율이 흔들려 판정하지 않는다.
pub const MIN_SYLLABLES_FOR_LIKENESS: usize = 50;

pub const TEXT_HEAD_CHARS: usize = 20000;
pub const TEXT_TAIL_CHARS: usize = 10000;

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// 버퍼를 둔 ZIP 리더. 버퍼가 없으면 중앙 디렉토리를 읽느라 read/lseek가 수백만 번 난다.
pub type Zip = ZipArchive<BufReader<File>>;

fn buffered(path: &Path) -> std::io::Result<BufReader<File>> {
    Ok(BufReader::with_capacity(64 * 1024, File::open(path)?))
}

/// ZIP을 연다. 기본 설정(`Detect`)은 앞에 붙은 데이터를 찾으려고 파일 전체를 훑어 1.5MB EPUB 하나에
/// 읽기 1만 3천 번(48ms)이 든다. 앞에 데이터가 없는 보통의 경우를 먼저 `Known(0)`으로 열고
/// (0.2ms), 실패하면 `Detect`로 다시 연다. 앞에 데이터가 붙은 ZIP도 그대로 읽힌다.
pub fn open_zip(path: &Path) -> Option<Zip> {
    let fast = Config { archive_offset: ArchiveOffset::Known(0) };
    if let Ok(zip) = ZipArchive::with_config(fast, buffered(path).ok()?) {
        return Some(zip);
    }
    ZipArchive::new(buffered(path).ok()?).ok()
}

pub fn is_hangul_syllable(c: char) -> bool {
    ('가'..='힣').contains(&c)
}

/// 이 글이 정상 한국어로 읽히는 정도. 잴 음절이 모자라면 None.
pub fn korean_likeness(text: &str) -> Option<f64> {
    let (mut total, mut hits) = (0usize, 0usize);
    for c in text.chars().filter(|&c| is_hangul_syllable(c)) {
        total += 1;
        if COMMON_HANGUL_SYLLABLES.contains(c) {
            hits += 1;
        }
    }
    (total >= MIN_SYLLABLES_FOR_LIKENESS).then(|| hits as f64 / total as f64)
}

/// Python의 `decode(errors="ignore")`: 잘못된 바이트는 버린다.
pub fn decode_utf8_ignore(raw: &[u8]) -> String {
    raw.utf8_chunks().map(|c| c.valid()).collect()
}

/// UTF-16을 `errors="ignore"`로 푼다. 짝이 안 맞는 서로게이트와 홀수 바이트 꼬리는 버린다.
fn decode_utf16_ignore(raw: &[u8], big_endian: bool) -> String {
    let units = raw.chunks_exact(2).map(|b| if big_endian { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) });
    char::decode_utf16(units).filter_map(Result::ok).collect()
}

/// CP949(= WHATWG euc-kr, UHC 포함)를 엄격하게 푼다.
fn decode_cp949_strict(raw: &[u8]) -> Option<String> {
    encoding_rs::EUC_KR.decode_without_bom_handling_and_without_replacement(raw).map(|c| c.into_owned())
}

/// CP949 / EUC-KR을 Python 코덱처럼 `errors="ignore"`로 푼다.
///
/// Python 코덱은 해석할 수 없는 선두 바이트를 1바이트만 버리고 다음 바이트부터 다시 읽는다.
/// encoding_rs는 비ASCII 후행 바이트까지 함께 소비해서 어긋나는 위치가 달라지고, 중문·일문
/// 텍스트를 이 경로로 읽을 때 판정이 바뀐다(실측: 표본 400건 중 7건).
/// `ksx1001_only`가 참이면 확장 완성형(UHC)을 빼고 KS X 1001 영역만 받아들인다(Python `euc-kr`).
pub fn decode_korean_ignore(raw: &[u8], ksx1001_only: bool) -> String {
    // 흔한 경우: 전부 유효하면 한 번에 푼다. 잘못된 바이트가 있을 때만 아래 느린 경로를 탄다.
    if !ksx1001_only && let Some(all) = decode_cp949_strict(raw) {
        return all;
    }
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c < 0x80 {
            out.push(c as char);
            i += 1;
            continue;
        }
        let pair_ok = match raw.get(i + 1) {
            Some(&d) if c != 0x80 && c != 0xff && (!ksx1001_only || ((0xa1..=0xfe).contains(&c) && (0xa1..=0xfe).contains(&d))) => {
                let before = out.len();
                let mut dec = encoding_rs::EUC_KR.new_decoder_without_bom_handling();
                let (res, _) = dec.decode_to_string_without_replacement(&[c, d], &mut out, true);
                if res == encoding_rs::DecoderResult::InputEmpty && out.len() > before {
                    true
                } else {
                    out.truncate(before);
                    false
                }
            }
            _ => false,
        };
        i += if pair_ok { 2 } else { 1 };
    }
    out
}

/// 바이트 열을 올바른 인코딩으로 디코딩한다.
///
/// UTF-8은 자기검증 인코딩이다. CP949로 저장된 한국어는 유효한 UTF-8이 되는 일이 거의 없어
/// 엄격 디코딩이 실패한다. 그래서 "엄격 UTF-8 성공 여부"가 가장 믿을 만한 판별이다.
/// "likeness가 가장 높은 인코딩 채택"은 이미 손상된 파일을 더 한국어처럼 보이는 쓰레기로
/// 되살려 손상을 놓치므로 마지막 수단으로만 쓴다.
pub fn decode_best(raw: &[u8]) -> String {
    if raw.is_empty() {
        return String::new();
    }

    // UTF-16은 BOM이나 널 바이트 밀도로 먼저 알아본다.
    if raw.starts_with(&[0xff, 0xfe]) {
        return decode_utf16_ignore(&raw[2..], false);
    }
    if raw.starts_with(&[0xfe, 0xff]) {
        return decode_utf16_ignore(&raw[2..], true);
    }
    let nulls = raw.iter().filter(|&&b| b == 0).count();
    if raw.len() >= 200 && nulls * 4 > raw.len() {
        let even_nulls = raw.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
        let odd_nulls = raw.iter().step_by(2).filter(|&&b| b == 0).count();
        return decode_utf16_ignore(raw, even_nulls <= odd_nulls);
    }

    // 잘린 멀티바이트 문자 때문에 엄격 디코딩이 헛되이 실패하지 않도록 꼬리를 다듬는다.
    for trim in 0..4 {
        if let Some(chunk) = raw.len().checked_sub(trim).map(|n| &raw[..n])
            && let Ok(text) = std::str::from_utf8(chunk)
        {
            return text.to_owned();
        }
    }

    // cp949는 euc-kr의 상위 집합이라 엄격 디코딩은 cp949 하나로 충분하다. 실패하는 입력은 둘 다 실패한다.
    for trim in 0..2 {
        if let Some(chunk) = raw.len().checked_sub(trim).map(|n| &raw[..n])
            && let Some(text) = decode_cp949_strict(chunk)
        {
            return text;
        }
    }

    // 어느 것으로도 깨끗이 안 읽히면 한국어로 가장 잘 읽히는 쪽을 쓴다. 동점이면 앞선 후보.
    let mut best: (String, f64) = (String::new(), -1.0);
    for text in [decode_utf8_ignore(raw), decode_korean_ignore(raw, false), decode_korean_ignore(raw, true)] {
        let score = korean_likeness(&text).unwrap_or(0.001);
        if score > best.1 {
            best = (text, score);
        }
    }
    best.0
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn last_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

/// TXT 앞부분과 뒷부분을 인코딩 추정하여 읽는다.
pub fn read_txt_text(path: &Path, head_chars: usize, tail_chars: usize) -> String {
    let Ok(size) = std::fs::metadata(path).map(|m| m.len()) else {
        return String::new();
    };
    read_txt_parts(path, size, head_chars, tail_chars).unwrap_or_default()
}

fn read_txt_parts(path: &Path, size: u64, head_chars: usize, tail_chars: usize) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let head_bytes = (head_chars * 3) as u64;
    let mut buf = Vec::new();
    (&mut f).take(head_bytes).read_to_end(&mut buf)?;
    let mut parts = vec![take_chars(&decode_best(&buf), head_chars)];
    if tail_chars > 0 && size > head_bytes {
        f.seek(SeekFrom::Start(size.saturating_sub((tail_chars * 3) as u64)))?;
        buf.clear();
        f.read_to_end(&mut buf)?;
        parts.push(last_chars(&decode_best(&buf), tail_chars));
    }
    Ok(parts.join(" "))
}

/// 멤버 하나에서 읽는 압축 해제 바이트 상한. 작게 압축된 거대 멤버(zip bomb)가 메모리를 먹지 못하게 한다.
/// 표본은 앞쪽 수만 자만 쓰므로 정상 문서는 영향이 없다.
pub const MAX_MEMBER_BYTES: u64 = 16 * 1024 * 1024;

fn read_entry(zip: &mut Zip, index: usize) -> Option<Vec<u8>> {
    let entry = zip.by_index(index).ok()?;
    let mut data = Vec::new();
    entry.take(MAX_MEMBER_BYTES).read_to_end(&mut data).ok()?;
    Some(data)
}

/// 문서 목록 순서대로 읽어 `budget`자 안에서 모은다. 읽을 수 없거나 50자 미만인 문서는 건너뛴다.
fn gather(zip: &mut Zip, order: &[usize], budget: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut total = 0usize;
    for &i in order {
        if total >= budget {
            break;
        }
        let Some(raw) = read_entry(zip, i) else { continue };
        let stripped = TAG_RE.replace_all(&decode_utf8_ignore(&raw), " ").into_owned();
        let clean = WS_RE.replace_all(&stripped, " ").trim().to_owned();
        let len = clean.chars().count();
        if len < 50 {
            continue;
        }
        total += len;
        out.push(clean);
    }
    take_chars(&out.join(" "), budget)
}

/// EPUB 본문 앞쪽/뒤쪽 챕터의 태그를 걷어낸 텍스트.
pub fn read_epub_text(path: &Path, head_chars: usize, tail_chars: usize) -> String {
    let Some(mut zip) = open_zip(path) else { return String::new() };

    let names: Vec<(usize, String)> = zip
        .file_names()
        .enumerate()
        .filter_map(|(i, n)| n.ok().map(|n| (i, n.into_owned())))
        .filter(|(_, n)| {
            let l = n.to_lowercase();
            l.ends_with(".html") || l.ends_with(".xhtml") || l.ends_with(".htm")
        })
        .collect();
    if names.is_empty() {
        return String::new();
    }
    const FRONT: [&str; 6] = ["cover", "nav", "toc", "titlepage", "index", "contents"];
    let body: Vec<usize> = names.iter().filter(|(_, n)| !FRONT.iter().any(|k| n.to_lowercase().contains(k))).map(|(i, _)| *i).collect();
    let mut order = if body.is_empty() { names.iter().map(|(i, _)| *i).collect() } else { body };

    // 앞쪽 문서는 표지·목차·판권지다. 거기서 표본을 뽑으면 본문을 못 본다.
    if order.len() > 4 {
        let mid = order.len() / 3;
        order.rotate_left(mid);
    }

    let head = gather(&mut zip, &order, head_chars);
    let tail = if tail_chars > 0 {
        order.reverse();
        gather(&mut zip, &order, tail_chars)
    } else {
        String::new()
    };
    format!("{head} {tail}").trim().to_owned()
}

/// 확장자에 맞춰 본문 표본을 읽는다.
pub fn read_document_text(path: &Path, head_chars: usize, tail_chars: usize) -> String {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("txt") => read_txt_text(path, head_chars, tail_chars),
        Some("epub") => read_epub_text(path, head_chars, tail_chars),
        _ => String::new(),
    }
}
