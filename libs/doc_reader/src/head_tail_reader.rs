//! 파일의 앞뒤 일부만 읽는다. 문서 전체가 아니라 앞뒤에 모이는 정보(ISBN 등)를 찾는 호출자가 쓴다.
//!
//! TXT는 앞뒤 바이트, PDF·DJVU는 앞뒤 [`HEAD_TAIL_PAGES`]쪽, EPUB은 OPF와 spine의 첫·끝 챕터, HWP는 `strings` 출력을 읽는다.
//! 어느 부분을 어떤 순서로 볼지, 시간 상한과 통계 기록은 호출자가 정한다.
//!
//! 인코딩 손상 판정(`doc_checker::mojibake_detector`)용 본문 앞뒤 표본도 여기서 뗀다:
//! [`extract_txt_text`], [`extract_epub_text`], [`extract_epub_text_limited`], [`extract_document_text`].
//! 표본 규칙(문서 순서, 50자 미만 건너뛰기, 예산)은 손상 판정 문턱값의 기준이라 바꾸지 않는다.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use common::zip_reader::{MAX_MEMBER_BYTES, MemberError, Zip, open_zip, read_member_prefix};
use htmlkit::{text_of, tokenize};
use regex::Regex;
use sysdeps::{CommandError, CommandRunner, SystemRunner};
use text_decoder::decode_utf8_ignore;
use text_decoder::encoding_guesser::decode_best;
use text_decoder::head_chars;

/// PDF·DJVU에서 앞뒤로 읽을 쪽 수.
pub const HEAD_TAIL_PAGES: usize = 5;

// ---------------------------------------------------------------- 앞뒤 읽기

/// 파일의 앞/뒤 부분만 읽는다(바이트 기준). 작은 파일은 전체를 읽는다.
pub fn read_head_tail_from_file(path: &Path, size: usize) -> io::Result<String> {
    let mut f = File::open(path)?;
    let file_size = f.seek(SeekFrom::End(0))? as usize;
    f.seek(SeekFrom::Start(0))?;
    if file_size <= size * 2 {
        let mut all = Vec::new();
        f.read_to_end(&mut all)?;
        return Ok(decode_utf8_ignore(&all));
    }
    let mut head = vec![0u8; size];
    f.read_exact(&mut head)?;
    f.seek(SeekFrom::End(-(size as i64)))?;
    let mut tail = vec![0u8; size];
    f.read_exact(&mut tail)?;
    Ok(decode_utf8_ignore(&head) + &decode_utf8_ignore(&tail))
}

/// 문자열의 앞/뒤 부분만 추출한다(문자 수 기준).
pub fn read_head_tail_from_content(content: &str, size: usize) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= size * 2 {
        return content.to_owned();
    }
    chars[..size].iter().chain(&chars[chars.len() - size..]).collect()
}

// ---------------------------------------------------------------- 외부 명령

/// 외부 도구를 `timeout` 안에서 실행하고 (종료 코드, 표준 출력)을 돌려준다. 넘기면 프로세스를 죽이고 `Err(Timeout)`이다.
fn run_tool(runner: &dyn CommandRunner, program: &str, args: &[String], timeout: Duration) -> Result<(i32, String), CommandError> {
    let argv: Vec<String> = std::iter::once(program.to_owned()).chain(args.iter().cloned()).collect();
    let out = runner.run(&argv, Some(timeout))?;
    Ok((out.returncode, decode_utf8_ignore(&out.stdout)))
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// DJVU 앞뒤 [`HEAD_TAIL_PAGES`]쪽의 텍스트. 도구가 없거나 실패하면 `None`이다.
pub fn read_djvu_head_tail(path: &Path, runner: &dyn CommandRunner, timeout: Duration) -> Option<String> {
    let file = path_arg(path);
    let (code, pages) = run_tool(runner, "djvused", &[file.clone(), "-e".into(), "n".into()], timeout).ok()?;
    if code != 0 {
        return None;
    }
    let total = pages.trim().parse::<usize>().ok()?;

    let head: Vec<String> = (1..=total.min(HEAD_TAIL_PAGES)).map(|i| i.to_string()).collect();
    let tail: Vec<String> = (total.saturating_sub(HEAD_TAIL_PAGES - 1).max(1)..=total).map(|i| i.to_string()).collect();
    let mut content = String::new();
    for range in [head.join(","), tail.join(",")] {
        match run_tool(runner, "djvutxt", &[format!("--page={range}"), file.clone()], timeout) {
            Ok((0, text)) => content.push_str(&text),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    Some(content)
}

/// HWP의 `strings` 출력 전체. 도구가 없거나 실패하면 `None`이다.
pub fn read_hwp_strings(path: &Path, runner: &dyn CommandRunner, timeout: Duration) -> Option<String> {
    let (code, out) = run_tool(runner, "strings", &[path_arg(path)], timeout).ok()?;
    (code == 0).then_some(out)
}

// ---------------------------------------------------------------- EPUB

fn read_member(zip: &mut Zip, name: &str) -> Result<Option<String>, ()> {
    match read_member_prefix(zip, name, MAX_MEMBER_BYTES) {
        Ok(data) => Ok(Some(decode_utf8_ignore(&data))),
        Err(MemberError::Open(_)) => Ok(None),
        Err(_) => Err(()),
    }
}

/// 연 EPUB과 OPF 본문. 챕터는 [`EpubPackage::edge_chapters_text`]를 부를 때만 읽는다.
pub struct EpubPackage {
    zip: Zip,
    opf_path: String,
    /// OPF 파일 본문.
    pub opf: String,
}

impl EpubPackage {
    /// EPUB을 열고 OPF를 읽는다. OPF가 없거나 읽다 깨지면 `None`이다.
    pub fn open(path: &Path) -> Option<Self> {
        let mut zip = open_zip(path)?;
        let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
        // 확장자 비교는 대소문자를 가리고 첫 번째 것만 쓴다.
        let opf_path = names.iter().find(|n| n.ends_with(".opf"))?.clone();
        let opf = read_member(&mut zip, &opf_path).ok()??;
        Some(EpubPackage { zip, opf_path, opf })
    }

    /// spine의 첫 챕터와 끝 챕터 본문 텍스트. spine이 비거나 챕터를 읽다 깨지면 `None`이다. 없는 챕터는 건너뛴다.
    pub fn edge_chapters_text(&mut self) -> Option<String> {
        let (tags, _) = tokenize(&self.opf);
        let mut manifest: Vec<(String, String)> = Vec::new();
        let mut spine: Vec<String> = Vec::new();
        for tag in tags.iter().filter(|t| !t.closing) {
            match tag.name.as_str() {
                "item" => {
                    let (id, href, media) = (tag.attr("id"), tag.attr("href"), tag.attr("media-type").unwrap_or(""));
                    if let (Some(id), Some(href)) = (id, href)
                        && !id.is_empty()
                        && !href.is_empty()
                        && media.contains("html")
                    {
                        manifest.retain(|(k, _)| k != id);
                        manifest.push((id.to_owned(), href.to_owned()));
                    }
                }
                "itemref" => spine.extend(tag.attr("idref").map(str::to_owned)),
                _ => {}
            }
        }
        let hrefs: Vec<&String> = spine.iter().filter_map(|id| manifest.iter().find(|(k, _)| k == id).map(|(_, h)| h)).collect();
        let (first, last) = (hrefs.first()?, hrefs.last()?);
        let opf_dir = self.opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

        let mut content = String::new();
        for href in [first, last] {
            let chapter = if opf_dir.is_empty() { (*href).clone() } else { format!("{opf_dir}/{href}") };
            // 챕터가 없으면 건너뛰고, 읽다가 깨진 경우는 전체를 포기한다.
            if let Some(html) = read_member(&mut self.zip, &chapter).ok()? {
                content.push_str(&text_of(&html));
                content.push('\n');
            }
        }
        Some(content)
    }
}

// ---------------------------------------------------------------- PDF

/// 앞 `window`쪽과 뒤 `window`쪽의 0부터 시작하는 번호. 겹치면 한 번만 넣는다.
pub fn head_tail_page_indices(total_pages: usize, window: usize) -> Vec<usize> {
    let mut seen = Vec::new();
    for idx in (0..window.min(total_pages)).chain(total_pages.saturating_sub(window)..total_pages) {
        if !seen.contains(&idx) {
            seen.push(idx);
        }
    }
    seen
}

/// PDF 앞뒤 쪽의 텍스트와 전체 쪽수. 문서를 열지 못하면 오류다.
pub fn pdf_text_primary(path: &Path) -> Result<(String, usize), String> {
    let doc = lopdf::Document::load(path).map_err(|e| e.to_string())?;
    let numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    let total = numbers.len();
    let mut text = String::new();
    for idx in head_tail_page_indices(total, HEAD_TAIL_PAGES) {
        // 한 쪽을 못 읽어도 나머지 쪽은 계속 읽는다.
        if let Ok(page) = doc.extract_text(&[numbers[idx]]) {
            text.push_str(&page);
        }
    }
    Ok((text, total))
}

/// Poppler로 앞뒤 쪽 텍스트와 전체 쪽수를 읽는다.
pub fn pdf_text_poppler(path: &Path) -> Result<(String, usize), String> {
    let runner = SystemRunner;
    let total = pdf_engine::pdf_worker_client::page_count(&runner, path)?;
    let mut text = String::new();
    for idx in head_tail_page_indices(total, HEAD_TAIL_PAGES) {
        text.push_str(&pdf_engine::pdf_worker_client::extract_text(&runner, path, idx + 1, idx + 1)?);
    }
    Ok((text, total))
}

/// Poppler 우선 추출기. 실행 파일이 없거나 PDF를 열지 못하면 순수 Rust(lopdf)를 쓴다.
pub fn pdf_text_auto(path: &Path) -> Result<(String, usize), String> {
    pdf_text_poppler(path).or_else(|_| pdf_text_primary(path))
}

/// 문서 구조를 못 여는 손상 PDF용 대비책: 스트림을 직접 훑어 글자열을 모은다.
pub fn pdf_text_fallback(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let blocks = stream_blocks(&bytes);
    let total = blocks.len();
    let mut text = String::new();
    for idx in head_tail_page_indices(total, HEAD_TAIL_PAGES) {
        let raw = blocks[idx];
        let mut inflated = Vec::new();
        // 압축 폭탄 스트림이 메모리를 다 쓰지 않도록 상한까지만 푼다.
        let content = match flate2::read::ZlibDecoder::new(raw).take(common::zip_reader::MAX_MEMBER_BYTES).read_to_end(&mut inflated) {
            Ok(_) => &inflated[..],
            Err(_) => raw,
        };
        text.push_str(&strings_of(content));
    }
    Ok(text)
}

fn stream_blocks(bytes: &[u8]) -> Vec<&[u8]> {
    let find = |needle: &[u8], from: usize| bytes[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from);
    let mut blocks = Vec::new();
    let mut at = 0;
    while let Some(s) = find(b"stream", at) {
        let mut start = s + 6;
        if bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if bytes.get(start) == Some(&b'\n') {
            start += 1;
        }
        match find(b"endstream", start) {
            Some(end) => {
                // "endstream" 안의 "stream"을 다시 시작점으로 잡지 않게 그 뒤부터 이어 찾는다.
                blocks.push(&bytes[start..end]);
                at = end + 9;
            }
            None => break,
        }
    }
    blocks
}

/// 콘텐츠 스트림의 `(...)`와 `<...>` 문자열을 모은다. `[...] TJ` 배열 안의 조각은 이어 붙이고 나머지는 공백으로 나눈다.
fn strings_of(content: &[u8]) -> String {
    let mut out = String::new();
    let (mut i, mut in_array) = (0, false);
    while i < content.len() {
        match content[i] {
            b'[' => in_array = true,
            b']' => {
                in_array = false;
                out.push(' ');
            }
            b'(' => {
                let (s, next) = literal_string(content, i);
                out.push_str(&s);
                if !in_array {
                    out.push(' ');
                }
                i = next;
                continue;
            }
            b'<' if content.get(i + 1) != Some(&b'<') => {
                if let Some(close) = content[i..].iter().position(|b| *b == b'>') {
                    let hex: Vec<u8> = content[i + 1..i + close].iter().copied().filter(u8::is_ascii_hexdigit).collect();
                    let decoded: String = hex.chunks(2).map(|p| u8::from_str_radix(&String::from_utf8_lossy(p), 16).unwrap_or(b'?') as char).collect();
                    out.push_str(&decoded);
                    if !in_array {
                        out.push(' ');
                    }
                    i += close;
                }
            }
            b'<' => i += 1, // 딕셔너리 `<<`
            _ => {}
        }
        i += 1;
    }
    out
}

/// PDF 리터럴 문자열을 읽는다. 중첩 괄호와 `\` 이스케이프(`\n`, `\(`, 8진수)를 푼다.
fn literal_string(content: &[u8], start: usize) -> (String, usize) {
    let (mut depth, mut i, mut out) = (0usize, start, String::new());
    while i < content.len() {
        match content[i] {
            b'(' => {
                depth += 1;
                if depth > 1 {
                    out.push('(');
                }
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return (out, i + 1);
                }
                out.push(')');
            }
            b'\\' => {
                i += 1;
                match content.get(i) {
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(d @ b'0'..=b'7') => {
                        let mut value = u32::from(d - b'0');
                        for _ in 0..2 {
                            match content.get(i + 1) {
                                Some(n @ b'0'..=b'7') => {
                                    value = value * 8 + u32::from(n - b'0');
                                    i += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push(char::from_u32(value & 0xff).unwrap_or('?'));
                    }
                    Some(other) => out.push(*other as char),
                    None => break,
                }
            }
            b => out.push(b as char),
        }
        i += 1;
    }
    (out, content.len())
}

// ---- 인코딩 손상 판정용 본문 표본 ----

pub const TEXT_HEAD_CHARS: usize = 20000;
pub const TEXT_TAIL_CHARS: usize = 10000;

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

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
