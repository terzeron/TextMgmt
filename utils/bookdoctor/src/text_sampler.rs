//! 책 파일(TXT·EPUB)에서 본문 앞뒤 표본을 인코딩에 맞게 떼어 내고, 정상 한국어로 읽히는 정도를 잰다. 인코딩 손상 판정(`mojibake_detector`)이 쓴다.
//!
//! - 본문 발췌: [`extract_txt_text`], [`extract_epub_text`], [`extract_epub_text_limited`], [`extract_document_text`]
//! - 인코딩 추정: [`decode_best`]. 텍스트 인코딩을 추정하는 휴리스틱 로직이다
//! - 한국어 판정: [`korean_likeness`]
//!
//! 인코딩 판별은 실제 코퍼스의 오판 사례로 네 번 고쳐 잡은 휴리스틱이다. 다시 쓰지 말 것:
//!   - "한글이 하나라도 나오면 그 인코딩 채택"은 정상 CP949 소설 1,856건을 오판했다
//!   - 한글 비율 하한이 없으면 영문 OCR 문서를 한글 잡음 60자로 판정한다
//!   - UTF-16을 빼면 97건을 못 읽는다
//!   - EPUB은 첫 문서가 목차인 경우가 있어 건너뛰어야 한다

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use sysdeps::zip_opener::{MAX_MEMBER_BYTES, Zip, open_zip};
use text_decoder::{decode_cp949_strict, decode_korean_ignore, decode_utf8_ignore, is_hangul_syllable};

pub const COMMON_HANGUL_SYLLABLES: &str = "이다는을가지에그고하의아은어도로리나를한었기서들게니있라자사시만으해마보했수대것";

/// 이 수보다 음절이 적으면 비율이 흔들려 판정하지 않는다.
pub const MIN_SYLLABLES_FOR_LIKENESS: usize = 50;

pub const TEXT_HEAD_CHARS: usize = 20000;
pub const TEXT_TAIL_CHARS: usize = 10000;

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

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

/// UTF-16을 `errors="ignore"`로 푼다. 짝이 안 맞는 서로게이트와 홀수 바이트 꼬리는 버린다.
fn decode_utf16_ignore(raw: &[u8], big_endian: bool) -> String {
    let units = raw.chunks_exact(2).map(|b| if big_endian { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) });
    char::decode_utf16(units).filter_map(Result::ok).collect()
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
pub fn extract_txt_text(path: &Path, head_chars: usize, tail_chars: usize) -> String {
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

/// `extract_epub_text_limited`가 두 패스(앞·뒤)에 걸쳐 함께 쓰는 압축 해제 예산.
struct Budget {
    bytes_left: u64,
    members_left: usize,
    /// 예산 때문에 읽지 못했거나 잘린 멤버가 있었다.
    cut: bool,
}

/// 예산 안에서 멤버 하나를 읽는다. 읽은 바이트와 멤버 수를 예산에서 뺀다. 예산이 바닥났으면 None이고 `cut`을 켠다.
fn read_entry(zip: &mut Zip, index: usize, budget: &mut Budget) -> Option<Vec<u8>> {
    if budget.bytes_left == 0 || budget.members_left == 0 {
        budget.cut = true;
        return None;
    }
    budget.members_left -= 1;
    let cap = MAX_MEMBER_BYTES.min(budget.bytes_left);
    let entry = zip.by_index(index).ok()?;
    let mut data = Vec::new();
    let res = entry.take(cap).read_to_end(&mut data);
    // 읽다가 실패해도(CRC 불일치, 잘린 deflate) 이미 푼 바이트는 예산에서 뺀다. 안 빼면 풀고 실패하는 멤버로 예산을 우회한다.
    budget.bytes_left = budget.bytes_left.saturating_sub(data.len() as u64);
    if res.is_err() {
        budget.cut = true;
        return None;
    }
    // 예산 상한에 걸려 잘렸다면(멤버 상한보다 예산이 작았다) 표본이 불완전하다.
    if data.len() as u64 == cap && cap < MAX_MEMBER_BYTES {
        budget.cut = true;
    }
    Some(data)
}

/// 문서 목록 순서대로 읽어 `budget`자 안에서 모은다. 읽을 수 없거나 50자 미만인 문서는 건너뛴다.
fn gather(zip: &mut Zip, order: &[usize], chars: usize, budget: &mut Budget) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut total = 0usize;
    for &i in order {
        if total >= chars {
            break;
        }
        let Some(raw) = read_entry(zip, i, budget) else {
            if budget.cut && (budget.bytes_left == 0 || budget.members_left == 0) {
                break;
            }
            continue;
        };
        let stripped = TAG_RE.replace_all(&decode_utf8_ignore(&raw), " ").into_owned();
        let clean = WS_RE.replace_all(&stripped, " ").trim().to_owned();
        let len = clean.chars().count();
        if len < 50 {
            continue;
        }
        total += len;
        out.push(clean);
    }
    take_chars(&out.join(" "), chars)
}

/// EPUB 본문 앞쪽/뒤쪽 챕터의 태그를 걷어낸 텍스트. 총 압축 해제량과 읽는 멤버 수에 상한이 없다(멤버당 상한만 있다).
/// 신뢰할 수 없는 파일에는 `extract_epub_text_limited`를 쓴다.
pub fn extract_epub_text(path: &Path, head_chars: usize, tail_chars: usize) -> String {
    extract_epub_text_limited(path, head_chars, tail_chars, u64::MAX, usize::MAX).0
}

/// `extract_epub_text`에 압축 해제 총량(`max_total` 바이트)과 읽는 멤버 수(`max_members`) 상한을 둔다.
/// 앞·뒤 두 패스가 같은 예산을 나눠 쓴다. 예산이 바닥나 표본이 잘렸으면 두 번째 값이 true다.
/// 작게 압축된 거대 멤버가 많은 EPUB(zip bomb)이 표본 추출만으로 분 단위 시간을 쓰지 못하게 한다.
pub fn extract_epub_text_limited(path: &Path, head_chars: usize, tail_chars: usize, max_total: u64, max_members: usize) -> (String, bool) {
    let Some(mut zip) = open_zip(path) else { return (String::new(), false) };

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
        return (String::new(), false);
    }
    const FRONT: [&str; 6] = ["cover", "nav", "toc", "titlepage", "index", "contents"];
    let body: Vec<usize> = names.iter().filter(|(_, n)| !FRONT.iter().any(|k| n.to_lowercase().contains(k))).map(|(i, _)| *i).collect();
    let mut order = if body.is_empty() { names.iter().map(|(i, _)| *i).collect() } else { body };

    // 앞쪽 문서는 표지·목차·판권지다. 거기서 표본을 뽑으면 본문을 못 본다.
    if order.len() > 4 {
        let mid = order.len() / 3;
        order.rotate_left(mid);
    }

    let mut budget = Budget { bytes_left: max_total, members_left: max_members, cut: false };
    let head = gather(&mut zip, &order, head_chars, &mut budget);
    let tail = if tail_chars > 0 {
        order.reverse();
        gather(&mut zip, &order, tail_chars, &mut budget)
    } else {
        String::new()
    };
    (format!("{head} {tail}").trim().to_owned(), budget.cut)
}

/// 확장자에 맞춰 본문 표본을 읽는다.
pub fn extract_document_text(path: &Path, head_chars: usize, tail_chars: usize) -> String {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("txt") => extract_txt_text(path, head_chars, tail_chars),
        Some("epub") => extract_epub_text(path, head_chars, tail_chars),
        _ => String::new(),
    }
}
