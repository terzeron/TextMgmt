//! 책 파일(TXT·EPUB)에서 본문 앞뒤 표본을 인코딩에 맞게 떼어 낸다. 인코딩 손상 판정(`mojibake_detector`)이 쓴다.
//!
//! - 본문 발췌: [`extract_txt_text`], [`extract_epub_text`], [`extract_epub_text_limited`], [`extract_document_text`]
//! - 인코딩 추정(`decode_best`)과 한국어 판정(`korean_likeness`)은 `text_decoder::encoding_guesser`에 있다.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

use common::zip_reader::{MAX_MEMBER_BYTES, Zip, open_zip};
use regex::Regex;
use text_decoder::decode_utf8_ignore;
use text_decoder::encoding_guesser::decode_best;

pub const TEXT_HEAD_CHARS: usize = 20000;
pub const TEXT_TAIL_CHARS: usize = 10000;

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// 첫 `n`개 코드포인트.
pub(crate) fn head_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
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
    let mut parts = vec![self::head_chars(&decode_best(&buf), head_chars).to_owned()];
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
    head_chars(&out.join(" "), chars).to_owned()
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
