//! 도서 자동 분류의 운영 계층. Python `backend/book_classifier.py` 대응.
//!
//! 판정 자체는 `crate::classifier` 의 모델이 한다. 이 모듈은 그 둘레를 맡는다: 캐시, 서점 조회와 다수결, 파일명 파싱,
//! 카테고리 매핑, 파일 이동, 빈 디렉토리 정리.
//!
//! 입력은 신뢰할 수 없는 파일 이름과 EPUB·캐시 파일이다. EPUB 은 DTD 를 읽지 않고 크기를 읽기 전에 재며, 캐시는 크기 상한을
//! 둔다. 옮길 목적지 카테고리는 디렉토리 이름 하나여야 한다. 로그에는 제목·저자·파일명을 남기지 않는다.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use serde_json::{Map, Value, json};
use sysdeps::{FsOps, RealFs};

use crate::bookstore::{AladinBookstore, BookstoreSearch, KyoboBookstore, Yes24Bookstore};
use crate::bookstore_cache::BookstoreResponseCache;
use crate::classifier::bookstore_policy::map_bookstore_candidates;
use crate::classifier::model::{CategoryModelApi, Prediction, get_default_model};
use crate::classifier::softmax::fixed;
use crate::classifier::tokenize::{is_py_space, is_py_word};
use crate::classifier::{BookCategoryClassifier, BookClassifierApi, ClassifierOptions, Classify, resolve_lenient};

mod map_category;
mod tables;

pub use crate::classifier::bookstore_policy::BookstorePolicy;
pub use map_category::map_category;
pub use tables::STANDARD_CATEGORIES;

/// 정상 EPUB 의 OPF 는 수십 KB 다. 압축을 풀었을 때의 크기로 재며, 작게 압축된 거대 파일(zip bomb)을 읽기 전에 끊는다.
pub const MAX_OPF_BYTES: usize = 8 * 1024 * 1024;
/// 분류 캐시 파일 최대 크기.
pub const MAX_CLASSIFICATION_CACHE_BYTES: u64 = 512 << 20;
/// 서점 다수결에 필요한 최소 표 수.
pub const MIN_STORE_VOTES: i64 = 2;
/// 이 확신도 미만이면 모델 답을 '확실'로 치지 않는다. 화면의 등급 판정 기준(`BookManager::MODEL_CERTAIN_MIN_CONFIDENCE`)과 같은 값이다.
pub const MODEL_TRUST_MIN_CONFIDENCE: f64 = 0.10;

macro_rules! rx {
    ($p:expr) => {{
        static R: LazyLock<Regex> = LazyLock::new(|| Regex::new($p).expect("정규식"));
        &*R
    }};
}

fn sub(re: &Regex, text: &str) -> String {
    re.replace_all(text, "").into_owned()
}

/// Python `str.strip()`.
fn strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

// ---------------------------------------------------------------------------
// 파일명·메타데이터
// ---------------------------------------------------------------------------

static EXPLICIT_TAGS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| tables::EXPLICIT_TAGS.iter().map(|(p, c)| (Regex::new(&format!("(?i){p}")).expect("정규식"), *c)).collect());

/// 파일명(URL 디코딩 후)에 명시된 장르 태그나 접두어를 표준 카테고리로 옮긴다.
pub fn extract_explicit_genre(filename: &str) -> Option<String> {
    let name = corpus_common::util::unquote(filename);
    EXPLICIT_TAGS.iter().find(|(re, _)| re.is_match(&name)).map(|(_, c)| (*c).to_string())
}

/// EPUB 의 OPF 에서 읽은 메타데이터.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpubMetadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub description: String,
}

impl PartialEq<Value> for EpubMetadata {
    fn eq(&self, other: &Value) -> bool {
        *other == json!({"title": self.title, "author": self.author, "subject": self.subject, "description": self.description})
    }
}

/// 잘린 태그를 닫고 lxml 복구처럼 서로 맞지 않는 종료 태그를 앞선 열린 태그에 맞춘다.
fn recover_xml_structure(xml: &str) -> String {
    let mut output = String::with_capacity(xml.len() + 64);
    let mut open = Vec::<String>::new();
    let mut cursor = 0;
    while cursor < xml.len() {
        let Some(relative) = xml[cursor..].find('<') else { break };
        let start = cursor + relative;
        output.push_str(&xml[cursor..start]);

        let special_end = if xml[start..].starts_with("<!--") {
            Some(("-->", 4))
        } else if xml[start..].starts_with("<![CDATA[") {
            Some(("]]>", 9))
        } else if xml[start..].starts_with("<?") {
            Some(("?>", 2))
        } else {
            None
        };
        if let Some((terminator, prefix_len)) = special_end {
            if let Some(end) = xml[start + prefix_len..].find(terminator) {
                let end = start + prefix_len + end + terminator.len();
                output.push_str(&xml[start..end]);
                cursor = end;
                continue;
            }
            cursor = xml.len();
            break;
        }

        let bytes = xml.as_bytes();
        let mut quote = None;
        let mut end = None;
        for (index, byte) in bytes.iter().enumerate().skip(start + 1) {
            match (*byte, quote) {
                (b'\'' | b'"', None) => quote = Some(*byte),
                (b'\'' | b'"', Some(current)) if *byte == current => quote = None,
                (b'>', None) => {
                    end = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            cursor = xml.len();
            break;
        };
        let token = &xml[start..=end];
        let body = token[1..token.len() - 1].trim();
        if let Some(closing) = body.strip_prefix('/') {
            let name = closing.split_whitespace().next().unwrap_or("");
            if let Some(position) = open.iter().rposition(|tag| tag == name) {
                while open.len() > position + 1 {
                    if let Some(tag) = open.pop() {
                        output.push_str(&format!("</{tag}>"));
                    }
                }
                open.pop();
                output.push_str(token);
            }
        } else if body.starts_with('!') {
            // DTD는 읽지 않는다. 호출자가 제거하지 못한 선언도 파서로 넘기지 않는다.
        } else {
            output.push_str(token);
            if !body.ends_with('/') {
                let name = body.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
                if !name.is_empty() {
                    open.push(name.to_owned());
                }
            }
        }
        cursor = end + 1;
    }
    output.push_str(&xml[cursor..]);
    while let Some(tag) = open.pop() {
        output.push_str(&format!("</{tag}>"));
    }
    output
}

fn parse_opf(xml: &str, meta: &mut EpubMetadata) -> bool {
    // DTD 는 읽지 않는다. 엔티티 폭탄(CWE-776)과 외부 엔티티(CWE-611)를 둘 다 이 한 줄로 막는다.
    let strict = roxmltree::Document::parse(xml);
    let cleaned;
    let recovered;
    let doc = match strict {
        Ok(d) => d,
        Err(_) => {
            // 복구: DOCTYPE 과 알 수 없는 엔티티 참조를 걷어내고 한 번 더 읽는다. 엔티티 내용은 읽지 않는다.
            let doctype = rx!(r"(?s)<!DOCTYPE[^>\[]*(\[.*?\])?\s*>");
            let entity = rx!(r"&[A-Za-z_][\w.-]*;");
            let no_doctype = doctype.replace_all(xml, "");
            cleaned =
                entity.replace_all(&no_doctype, |m: &regex::Captures| if matches!(&m[0], "&amp;" | "&lt;" | "&gt;" | "&quot;" | "&apos;") { m[0].to_string() } else { String::new() }).into_owned();
            match roxmltree::Document::parse(&cleaned) {
                Ok(d) => d,
                Err(_) => {
                    recovered = recover_xml_structure(&cleaned);
                    match roxmltree::Document::parse(&recovered) {
                        Ok(d) => d,
                        Err(_) => return false,
                    }
                }
            }
        }
    };
    for node in doc.descendants().filter(|n| n.is_element()) {
        let tag = node.tag_name().name().to_lowercase();
        let text = node.first_child().filter(|c| c.is_text()).and_then(|c| c.text()).filter(|t| !t.is_empty());
        let Some(text) = text else { continue };
        let text = strip(text);
        match tag.as_str() {
            "title" if meta.title.is_empty() => meta.title = text.to_string(),
            "creator" | "author" if meta.author.is_empty() => meta.author = text.to_string(),
            "subject" => {
                if meta.subject.is_empty() {
                    meta.subject = text.to_string();
                } else {
                    meta.subject = format!("{} > {text}", meta.subject);
                }
            }
            "description" if meta.description.is_empty() => meta.description = text.to_string(),
            _ => {}
        }
    }
    true
}

/// EPUB 파일 내부의 `.opf` 메타데이터(title, author, subject, description)를 읽는다. 못 읽으면 빈 값이다.
pub fn inspect_epub_metadata(path: &Path) -> EpubMetadata {
    let meta = EpubMetadata::default();
    let Some(mut zip) = corpus_common::reader::open_zip(path) else { return meta };
    if zip.len() > crate::classifier::reader::MAX_ZIP_ENTRIES {
        return meta;
    }
    let Some(index) = zip.file_names().enumerate().find(|(_, n)| n.as_ref().is_ok_and(|n| n.ends_with(".opf"))).map(|(i, _)| i) else { return meta };
    let mut raw = Vec::new();
    match zip.by_index(index) {
        Ok(entry) => {
            if entry.size() > MAX_OPF_BYTES as u64 {
                log::warn!("OPF too large, skipping metadata ({} bytes)", entry.size());
                return meta;
            }
            // 선언 크기를 속인 항목도 상한 이상은 읽지 않는다.
            if entry.take(MAX_OPF_BYTES as u64 + 1).read_to_end(&mut raw).is_err() || raw.len() > MAX_OPF_BYTES {
                return meta;
            }
        }
        Err(_) => return meta,
    }
    let text = String::from_utf8_lossy(&raw);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut found = EpubMetadata::default();
    if parse_opf(text, &mut found) { found } else { meta }
}

/// TXT 앞부분에서 읽은 해시태그, 헤더 장르, 소개글 조각.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TxtContent {
    pub hashtags: Vec<String>,
    pub snippet: String,
    pub header_genre: Option<String>,
}

/// 앞쪽 40줄(개행 포함)만 돌려준다. 줄바꿈은 `\n`, `\r\n`, `\r` 모두 한 줄이다(Python 텍스트 모드).
fn first_lines(text: &str, n: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                cur.push('\n');
                lines.push(std::mem::take(&mut cur));
            }
            '\n' => {
                cur.push('\n');
                lines.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
        if lines.len() >= n {
            return lines;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// TXT 앞부분 줄(최대 40줄)에서 해시태그, 헤더 장르, 소개글 조각을 뽑는다(UTF-8 실패 시 CP949 로 되돌린다).
pub fn inspect_txt_content(path: &Path) -> TxtContent {
    const HEAD_BYTES: u64 = 1 << 20;
    let mut info = TxtContent::default();
    let Ok(file) = fs::File::open(path) else { return info };
    let mut raw = Vec::new();
    if file.take(HEAD_BYTES).read_to_end(&mut raw).is_err() {
        return info;
    }
    let cp949 = |bytes: &[u8]| corpus_common::reader::decode_korean_ignore(bytes, false);
    // 앞 40줄이 들어가는 만큼만 본다. 잘린 끝 바이트 때문에 엄격 디코딩이 헛되이 실패하지 않게 꼬리를 다듬는다.
    let mut lines = match (0..4).find_map(|trim| std::str::from_utf8(&raw[..raw.len().saturating_sub(trim)]).ok()) {
        Some(text) => first_lines(text, 40),
        None => first_lines(&cp949(&raw), 40),
    };
    let raw_text: String = lines.concat();
    if !rx!(r"[가-힣]{2,}").is_match(&raw_text) {
        lines = first_lines(&cp949(&raw), 40);
    }
    let lines: Vec<&str> = lines.iter().map(|l| strip(l)).collect();
    let joined = lines.iter().filter(|l| !l.is_empty()).copied().collect::<Vec<_>>().join(" ");
    info.snippet = joined.chars().take(2000).collect();
    info.hashtags = rx!(r"#([가-힣a-zA-Z0-9_]{2,15})").captures_iter(&info.snippet).map(|c| c[1].to_string()).collect();
    if let Some(m) = rx!(r"(?:장르|분류)\s*[:：]\s*([가-힣a-zA-Z]+)|(?:^|\s)\[([가-힣]{2,6})\]|(?:^|\s)【([가-힣]{2,6})】").captures(&info.snippet) {
        info.header_genre = (1..=3).find_map(|i| m.get(i)).map(|g| g.as_str().to_string());
    }
    info
}

// ---------------------------------------------------------------------------
// 제목 비교
// ---------------------------------------------------------------------------

fn keep_word_chars(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| is_py_word(*c) || ('가'..='힣').contains(c)).collect()
}

/// 두 도서 제목 간의 글자 bi-gram 유사도(Dice coefficient).
pub fn title_similarity(t1: &str, t2: &str) -> f64 {
    let (s1, s2) = (keep_word_chars(t1), keep_word_chars(t2));
    if s1.is_empty() || s2.is_empty() {
        return 0.0;
    }
    let (l1, l2) = (s1.chars().count(), s2.chars().count());
    if s1.contains(&s2) || s2.contains(&s1) {
        return l1.min(l2) as f64 / l1.max(l2) as f64;
    }
    let bigrams = |s: &str| -> std::collections::HashSet<(char, char)> {
        let c: Vec<char> = s.chars().collect();
        c.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let (b1, b2) = (bigrams(&s1), bigrams(&s2));
    if b1.is_empty() || b2.is_empty() {
        return 0.0;
    }
    2.0 * b1.intersection(&b2).count() as f64 / (b1.len() + b2.len()) as f64
}

/// 문장부호를 지우고(공백은 남기고) 소문자로 바꾼다.
fn clean_for_words(s: &str) -> String {
    strip(&s.to_lowercase().chars().filter(|c| is_py_word(*c) || ('가'..='힣').contains(c) || is_py_space(*c)).collect::<String>()).to_string()
}

fn words_of(clean: &str) -> Vec<String> {
    clean.split(is_py_space).filter(|w| w.chars().count() >= 2).map(str::to_string).collect()
}

/// 단일 서점 매칭 시 검색된 제목이 원본 검색어와 유효하게 일치하는지 검증한다(추천도서 오매칭 방지).
pub fn is_single_match_valid(search_title: &str, found_title: &str) -> bool {
    if found_title.is_empty() || search_title.is_empty() {
        return false;
    }
    if title_similarity(search_title, found_title) >= 0.35 {
        return true;
    }
    let st_words = words_of(&clean_for_words(search_title));
    let ft_words = words_of(&clean_for_words(found_title));
    let matching = st_words.iter().filter(|w| ft_words.contains(w)).count();
    if matching >= 2 {
        return true;
    }
    st_words.len() == 1 && ft_words.len() <= 3 && ft_words.contains(&st_words[0])
}

/// 서점 간 충돌 해소 및 엄격한 단일 매칭 판정을 위한 고신뢰도 제목 일치 검증.
pub fn is_title_match_reliable(search_title: &str, found_title: &str) -> bool {
    if found_title.is_empty() || search_title.is_empty() {
        return false;
    }
    let (st_clean, ft_clean) = (clean_for_words(search_title), clean_for_words(found_title));
    let (st_words, ft_words) = (words_of(&st_clean), words_of(&ft_clean));
    if st_words.is_empty() {
        return false;
    }
    // 음반/OST 오매칭 필터링 (도서 파일인데 음반/사운드트랙으로 매칭되는 경우 방지)
    let upper = found_title.to_uppercase();
    if ["O.S.T", "OST", "(CD", "CD-ROM", "음반", "사운드트랙"].iter().any(|k| upper.contains(k)) {
        return false;
    }
    let squash = |s: &str| s.chars().filter(|c| !is_py_space(*c)).collect::<String>();
    let (s1, s2) = (squash(&st_clean), squash(&ft_clean));
    if s1 == s2 || (s2.contains(&s1) && s1.chars().count() >= 4) {
        return true;
    }
    let matching = st_words.iter().filter(|w| ft_words.iter().any(|fw| fw.contains(w.as_str()) || w.contains(fw.as_str()))).count();
    let ratio = matching as f64 / st_words.len() as f64;
    ratio >= 0.7 && title_similarity(search_title, found_title) >= 0.5
}

/// `os.path.splitext(filename)[0]`.
fn splitext_stem(name: &str) -> &str {
    let sep = name.rfind('/').map_or(0, |i| i + 1);
    let Some(dot) = name.rfind('.').filter(|d| *d >= sep) else { return name };
    // 맨 앞의 점들은 확장자 구분이 아니다.
    if name[sep..dot].chars().all(|c| c == '.') { name } else { &name[..dot] }
}

/// 영숫자·한글 사이의 점을 지운다(검열 우회용 점: `신.화.급` -> `신화급`). 앞뒤 판단은 원문 기준이다.
fn remove_inner_dots(s: &str) -> String {
    let word = |c: char| ('가'..='힣').contains(&c) || c.is_ascii_alphanumeric();
    let chars: Vec<char> = s.chars().collect();
    chars.iter().enumerate().filter(|(i, c)| !(**c == '.' && *i > 0 && *i + 1 < chars.len() && word(chars[*i - 1]) && word(chars[*i + 1]))).map(|(_, c)| *c).collect()
}

/// 파일명에서 (저자, 원제목, 검색용 제목)을 뽑는다.
pub fn clean_filename_to_author_title(filename: &str) -> (String, String, String) {
    // URL 인코딩 해제 (예: %5B로맨스판타지%5D -> [로맨스판타지])
    let filename = corpus_common::util::unquote(filename);
    let mut stem = splitext_stem(&filename).to_string();
    stem = sub(rx!(r"(?i)\s*\([^)]*z-lib[^)]*\)"), &stem);
    stem = sub(rx!(r"(?i)\s*\([^)]*1lib[^)]*\)"), &stem);
    stem = remove_inner_dots(&stem);
    stem = strip(&stem.replace('_', " ")).to_string();

    // 해시태그 및 특수 기호 접두어 제거 (#나무, #공금, ★공금, (019), (19) 등)
    stem = sub(rx!(r"^[#★■◆●▲\s]*(?:나무|공금|숲|텍본|완결|스캔|TXT|txt|EPUB|epub)[\s]*"), &stem);
    stem = sub(rx!(r"^#\S+\s*"), &stem);
    stem = sub(rx!(r"^[\s\(\[【]*(?:019|19|19금|성인)[\s\)\]】]*"), &stem);
    stem = sub(rx!(r"^[\s\(\[【]*(?:完|완|완결|외전|텍본|스캔|TXT|txt)[\s\)\]】]*"), &stem);
    stem = sub(rx!(r"^[\s\(\[【]*(?:현대\s*판타지|판타지|무협|로판|로맨스|현판|퓨판|SF)[\s\)\]】]*"), &stem);

    // 1. @author 패턴 처리
    let mut author_from_at = String::new();
    if let Some(m) = rx!(r"@([^\s\(\)\[\]\-]+)").captures(&stem) {
        author_from_at = strip(&m[1]).to_string();
        stem = strip(&sub(rx!(r"@[^\s\(\)\[\]\-]+(?:\([^\)]*\))?"), &stem)).to_string();
    }

    // 2. 뒤쪽에 붙은 저자 패턴 (예: 제목 1-300 완[]설봉, 제목 [저자], 제목-저자, 제목 (저자))
    if author_from_at.is_empty()
        && let Some(m) = rx!(r"\[(?:\s*|저자:?|글:?)\]?\s*([가-힣a-zA-Z]{2,10})\s*\]?\s*$").captures(&stem)
    {
        author_from_at = strip(&m[1]).to_string();
        stem = strip(&sub(rx!(r"\[(?:\s*|저자:?|글:?)\]?\s*[가-힣a-zA-Z]{2,10}\s*\]?\s*$"), &stem)).to_string();
    }
    if author_from_at.is_empty()
        && let Some(m) = rx!(r"\((?:\s*|저자:?|글:?)?\s*([가-힣a-zA-Z]{2,10})\s*\)\s*$").captures(&stem)
    {
        let cand = strip(&m[1]).to_string();
        if !rx!(r"(완결|완|개정판|외전|단편|1부|2부|판타지|무협|로판|스릴러)").is_match(&cand) {
            author_from_at = cand;
            stem = strip(&sub(rx!(r"\((?:\s*|저자:?|글:?)?\s*[가-힣a-zA-Z]{2,10}\s*\)\s*$"), &stem)).to_string();
        }
    }
    if author_from_at.is_empty()
        && let Some(m) = rx!(r"[-－]\s*([가-힣a-zA-Z]{2,4})\s*$").captures(&stem)
    {
        let cand = strip(&m[1]).to_string();
        if !rx!(r"(완결|완|개정판|외전|단편|1부|2부|판타지|무협|로판|스릴러|텍본)").is_match(&cand) {
            author_from_at = cand;
            stem = strip(&sub(rx!(r"[-－]\s*[가-힣a-zA-Z]{2,4}\s*$"), &stem)).to_string();
        }
    }

    // 3. [저자] 제목 패턴
    let (raw_author, raw_title) = if let Some(m) = rx!(r"^\[([^\]]+)\]\s*(.+)$").captures(&stem) {
        (strip(&m[1]).to_string(), strip(&m[2]).to_string())
    } else if let Some(m) = rx!(r"^([가-힣]{2,4})\s*[-－]\s*(.+)$").captures(&stem).filter(|_| author_from_at.is_empty()) {
        // 저자 - 제목 패턴 확인
        (strip(&m[1]).to_string(), strip(&m[2]).to_string())
    } else {
        (author_from_at, strip(&stem).to_string())
    };

    // 저자 정제
    let mut author = sub(rx!(r"\s*(저|역|지음|옮김|펴냄|엮음|글|그림|원작)\b.*$"), &raw_author);
    author = sub(rx!(r"\s*외\s*\d*명?$"), &author);
    let author = strip(&author).to_string();

    // 제목 정제
    let mut t = sub(rx!(r"^\[[^\]]*\]\s*"), &raw_title);
    for sep in [" - ", "－", " : "] {
        if let Some((head, _)) = t.split_once(sep) {
            t = strip(head).to_string();
        }
    }
    // 접두/접미 완결 표기 및 권수 정제
    t = sub(rx!(r"^[#\s]+"), &t);
    t = sub(rx!(r"^[\s\(\[【]*(?:完|완|완결|외전|텍본|스캔|TXT|txt)[\s\)\]】]*"), &t);
    t = t.replace("(개정판)", "");
    t = sub(rx!(r"\([^\)]*완[^\)]*\)"), &t);
    t = sub(rx!(r"\([^\)]*로판[^\)]*\)"), &t);
    t = t.replace("(19)", "");
    t = sub(rx!(r"\[[0-9~]+부\]"), &t);
    t = sub(rx!(r"\s+\d+\s+(?:완|完|외전|특외|에필|후기|미완|완결|완외포|완외|붙음).*"), &t);
    t = sub(rx!(r"\s+\d+(?:화|권|부|완|完|외|외전|특외|에필|후기|미완|완결|완외포)\b.*"), &t);
    t = sub(rx!(r"\s+\d{1,4}\s+\d{1,4}.*"), &t);
    t = sub(
        rx!(
            r"[\s,]+(?:1[-~]\d+|\d+[-~]\d+.*|\d+[-~]\d+권.*|\d+화.*|\d+권.*|\d+부.*|\b完\b.*|\b완\b.*|\b외전\b.*|\b특외\b.*|\b에필\b.*|\b후기\b.*|\b완외포\b.*|\b완외\b.*|\b완결\b.*|\b단권\b.*|\b미완\b.*)$"
        ),
        &t,
    );
    t = sub(rx!(r"(?i)\s*\b(?:issue|vol|v)\.?\s*\d+\b.*"), &t);
    t = sub(rx!(r"\s*\b\d{1,3}\b$"), &t); // 끝의 1, 2, 01, 02 등
    t = strip(&sub(rx!(r"\s*\([^)]*\)"), &t)).to_string();
    t = strip(&sub(rx!(r"\s*\[[^\]]*\]"), &t)).to_string();
    t = strip(&sub(rx!(r"^\s*[-~=,.]+\s*"), &t)).to_string();
    let search_title = strip(&sub(rx!(r"\s*[-~=,.]+\s*$"), &t)).to_string();

    (author, raw_title, search_title)
}

/// 서브디렉토리 안의 파일명이 단순 번호나 권수(예: 01.txt, 1권.epub)면 부모 디렉토리명(작품명)을 붙이고, 부모에만 `[저자]`나
/// `@저자`가 있으면 그것을 물려받는다.
pub fn get_effective_filename(fpath: &Path, target_dir: &Path) -> String {
    let fname = fpath.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if fpath.parent() == Some(target_dir) {
        return fname;
    }
    let parent_name = fpath.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = fpath.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = strip(&stem);
    let is_short = rx!(r"(?i)^(\d+|[0-9~]+권|[0-9~]+화|[0-9~]+부|\bvol\.?\s*\d+|\bv\d+)$").is_match(stem) || stem.chars().count() <= 3;
    if is_short {
        return format!("{parent_name} {fname}");
    }
    let tag = rx!(r"(\[[^\]]+\]|@[^\s]+)");
    if let Some(parent_m) = tag.captures(&parent_name)
        && !tag.is_match(&fname)
    {
        return format!("{} {fname}", &parent_m[1]);
    }
    fname
}

/// 파일 이동·삭제 뒤 비어 있는 상위 서브디렉토리를 재귀로 지운다. `stop_dir` 자신과 그 바깥은 지우지 않는다.
pub fn clean_empty_parent_dirs(parent_dir: &Path, stop_dir: &Path) {
    clean_empty_parent_dirs_with(parent_dir, stop_dir, &RealFs);
}

/// `clean_empty_parent_dirs` 의 파일 시스템 주입 판. 경로를 해석하지 못하면 아무것도 지우지 않는다.
///
/// 두 경로를 정규화해서 비교한다. 정규화 없이 `!=` 로만 보면 `..` 가 낀 표기로는 `stop_dir` 를 알아보지 못해 조상까지 올라간다.
/// 비었는지는 `rmdir` 과 같은 기준(숨김 파일 포함)으로 본다. 지우기는 항상 비재귀(`remove_dir`)다.
pub fn clean_empty_parent_dirs_with(parent_dir: &Path, stop_dir: &Path, fs: &dyn FsOps) {
    let resolve = |p: &Path| match fs.canonicalize(p) {
        Ok(real) => Some(real),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(resolve_lenient(p)),
        Err(_) => None,
    };
    let (Some(mut curr), Some(stop)) = (resolve(parent_dir), resolve(stop_dir)) else { return };
    while curr != stop && curr.starts_with(&stop) && fs.metadata(&curr).is_ok_and(|metadata| metadata.is_dir()) {
        match fs.read_dir(&curr) {
            Ok(entries) if entries.is_empty() => {}
            _ => break,
        }
        if fs.remove_dir(&curr).is_err() {
            break;
        }
        let Some(parent) = curr.parent() else { break };
        curr = parent.to_path_buf();
    }
}

// ---------------------------------------------------------------------------
// 서비스
// ---------------------------------------------------------------------------

/// `_decide` 옵션.
#[derive(Debug, Clone, Copy)]
pub struct DecideOptions {
    pub trust_single_match: bool,
}

impl Default for DecideOptions {
    fn default() -> Self {
        Self { trust_single_match: true }
    }
}

/// `classify_file` 옵션.
#[derive(Debug, Clone, Copy)]
pub struct ClassifyOptions {
    pub trust_single_match: bool,
    pub use_bookstore: bool,
    pub use_content_meta: bool,
    pub cache_only: bool,
}

impl Default for ClassifyOptions {
    fn default() -> Self {
        Self { trust_single_match: true, use_bookstore: true, use_content_meta: true, cache_only: false }
    }
}

/// `process_file` 옵션.
#[derive(Debug, Clone, Copy)]
pub struct ProcessOptions {
    pub auto_move: bool,
    pub clean_existing: bool,
    pub dry_run: bool,
    pub trust_single_match: bool,
    pub use_bookstore: bool,
    pub use_content_meta: bool,
    pub cache_only: bool,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        Self { auto_move: true, clean_existing: true, dry_run: false, trust_single_match: true, use_bookstore: true, use_content_meta: true, cache_only: false }
    }
}

/// 판정 결과. 모델 카테고리와 확신도는 등급 판정과 화면 설명에 쓴다.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub category: Option<String>,
    pub method: String,
    pub reason: String,
    pub model_category: Option<String>,
    pub confidence: Option<f64>,
}

type StoreVote = (Option<String>, String, String);

/// 서점 득표 단계를 감싸는 훅. 호출 횟수를 세거나 바꿔치기할 때 쓴다(구 `wrap_decide_by_bookstore`).
/// 기본 구현은 `original` 을 그대로 부른다.
pub trait BookstoreDecider: Send + Sync {
    fn decide(&self, entry: &mut Value, trust_single_match: bool, original: &dyn Fn(&mut Value, bool) -> StoreVote) -> StoreVote {
        original(entry, trust_single_match)
    }
}

/// 도서 자동 분류 통합 서비스. 서점 3곳 조회와 다수결, 파일명 파싱, 캐시, 안전한 이동을 맡는다.
pub struct BookClassifierService {
    library_root: PathBuf,
    classifier_root: PathBuf,
    delay: f64,
    classifier: Option<Arc<dyn Classify>>,
    bookstore_policy: BookstorePolicy,
    cache_file: PathBuf,
    cache: Map<String, Value>,
    title_cache: HashMap<String, Value>,
    bookstore_cache: BookstoreResponseCache,
    decider: Option<Arc<dyn BookstoreDecider>>,
    fs: Arc<dyn FsOps>,
    pub yes24: Box<dyn BookstoreSearch>,
    pub aladin: Box<dyn BookstoreSearch>,
    pub kyobo: Box<dyn BookstoreSearch>,
}

impl BookClassifierService {
    /// `classifier` 가 없으면 기본 경로의 모델을 읽어 `library_root` 기준 분류기를 만든다.
    /// 서점 규칙 파일(`bookstore_policy.json`)이 없으면 예전 동작(모델 우선)이다.
    pub fn new(library_root: &Path, cache_file: Option<&Path>, classifier: Option<Arc<dyn Classify>>) -> Self {
        let classifier = classifier.unwrap_or_else(|| {
            let model = get_default_model().map(|model| model as Arc<dyn CategoryModelApi>);
            Arc::new(BookCategoryClassifier::new(model, ClassifierOptions { library_root: Some(library_root.to_path_buf()), ..Default::default() }))
        });
        let cache_file = cache_file.map(Path::to_path_buf).unwrap_or_else(|| library_root.join("classification_cache.json"));
        let bookstore_cache = BookstoreResponseCache::new(cache_file.with_file_name("bookstore_cache.jsonl"));
        let mut service = Self {
            library_root: library_root.to_path_buf(),
            classifier_root: library_root.to_path_buf(),
            delay: 1.2,
            classifier: Some(classifier),
            bookstore_policy: BookstorePolicy::load(None),
            cache_file,
            cache: Map::new(),
            title_cache: HashMap::new(),
            bookstore_cache,
            decider: None,
            fs: Arc::new(RealFs),
            yes24: Box::new(Yes24Bookstore::new(false)),
            aladin: Box::new(AladinBookstore::new(false)),
            kyobo: Box::new(KyoboBookstore::new(false)),
        };
        service.load_cache();
        service
    }

    pub fn with_delay(mut self, delay: f64) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_bookstore_cache_file(mut self, path: &Path) -> Self {
        self.bookstore_cache = BookstoreResponseCache::new(path);
        self
    }

    pub fn with_bookstore_policy(mut self, policy: BookstorePolicy) -> Self {
        self.bookstore_policy = policy;
        self
    }

    pub fn with_bookstore_decider(mut self, decider: Arc<dyn BookstoreDecider>) -> Self {
        self.decider = Some(decider);
        self
    }

    pub fn with_fs(mut self, fs: Arc<dyn FsOps>) -> Self {
        self.fs = fs;
        self
    }

    pub fn library_root(&self) -> &Path {
        &self.library_root
    }

    /// 분류기가 ES 경로를 만들 때 쓰는 라이브러리 루트.
    pub fn classifier_library_root(&self) -> PathBuf {
        self.classifier_root.clone()
    }

    pub fn cache_file(&self) -> &Path {
        &self.cache_file
    }

    pub fn cache(&self) -> &Map<String, Value> {
        &self.cache
    }

    pub fn set_cache(&mut self, cache: Value) {
        self.cache = cache.as_object().cloned().unwrap_or_default();
    }

    pub fn title_cache(&self) -> &HashMap<String, Value> {
        &self.title_cache
    }

    pub fn bookstore_policy(&self) -> &BookstorePolicy {
        &self.bookstore_policy
    }

    /// 분류 캐시를 읽는다. 없거나 깨졌거나 너무 크면 빈 캐시다.
    pub fn load_cache(&mut self) {
        if self.cache_file.exists() {
            match self.read_cache_file() {
                Ok(cache) => {
                    log::info!("Loaded {} cached entries", cache.len());
                    self.cache = cache;
                }
                Err(e) => {
                    log::warn!("Failed to load cache: {e}");
                    self.cache = Map::new();
                }
            }
        }
        self.build_title_index();
    }

    fn read_cache_file(&self) -> Result<Map<String, Value>, String> {
        if fs::symlink_metadata(&self.cache_file).map_err(|e| e.to_string())?.file_type().is_symlink() {
            return Err("심볼릭 링크는 따르지 않는다".into());
        }
        let file = fs::File::open(&self.cache_file).map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > MAX_CLASSIFICATION_CACHE_BYTES {
            return Err("일반 파일이 아니거나 너무 크다".into());
        }
        let mut buf = Vec::with_capacity(meta.len() as usize);
        file.take(MAX_CLASSIFICATION_CACHE_BYTES + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        match serde_json::from_slice::<Value>(&buf).map_err(|e| e.to_string())? {
            Value::Object(m) => Ok(m),
            _ => Err("JSON 객체가 아니다".into()),
        }
    }

    /// 분류 캐시를 쓴다. 같은 디렉토리의 임시 파일(`O_EXCL`)에 쓴 뒤 이름을 바꿔 원자적으로 교체한다.
    pub fn save_cache(&self) {
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let parent = self.cache_file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
            let name = self.cache_file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "cache".into());
            let tmp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
            let body = crate::classifier::pyjson::dumps_indent2(&Value::Object(self.cache.clone()));
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            if let Err(e) = file.write_all(body.as_bytes()).and_then(|()| file.sync_all()).and_then(|()| self.fs.rename(&tmp, &self.cache_file)) {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
            Ok(())
        };
        if let Err(e) = write() {
            log::error!("Failed to save cache: {e}");
        }
    }

    pub fn build_title_index(&mut self) {
        self.title_cache.clear();
        let answered = |v: &Value, store: &str| v.get(store).and_then(|s| s.get("cat")).and_then(Value::as_str).is_some_and(|c| !c.is_empty());
        for v in self.cache.values() {
            let Some(st) = v.get("search_title").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue };
            if !(answered(v, "yes24") || answered(v, "aladin") || answered(v, "kyobo")) {
                continue;
            }
            let preferred = matches!(v.get("status").and_then(Value::as_str), Some("moved" | "already_exists_cleaned" | "matched"));
            if !self.title_cache.contains_key(st) || preferred {
                self.title_cache.insert(st.to_string(), v.clone());
            }
        }
    }

    fn query_store(store: &dyn BookstoreSearch, isbn: &str, search_title: &str, raw_author: &str, raw_title: &str) -> Value {
        let mut res = json!({"title": "", "author": "", "cat": "", "mapped": null, "url": "", "isbn": ""});
        // 서점 클라이언트 하나가 죽어도(패닉) 나머지 조회와 판정은 계속돼야 한다.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.search(isbn, search_title, raw_author)));
        let Ok((results, _, _)) = outcome else {
            log::debug!("Store query failed");
            return res;
        };
        if let Some((found_title, found_author, cat_str, detail_url, _, found_isbn)) = results.into_iter().next() {
            // DB(category_keywords)가 먼저다. 한 키워드가 여러 카테고리에 붙을 수 있어 후보를 그대로 들고 가 다수결로 가른다.
            let candidates = map_bookstore_candidates(&cat_str, raw_title, raw_author, None);
            res["mapped"] = if candidates.len() == 1 { json!(candidates[0]) } else { Value::Null };
            res["candidates"] = json!(candidates);
            res["title"] = json!(found_title);
            res["author"] = json!(found_author);
            res["cat"] = json!(cat_str);
            res["url"] = json!(detail_url);
            // 캐시를 ISBN 으로도 찾으려면 서점이 준 ISBN 을 버리지 않아야 한다.
            res["isbn"] = json!(found_isbn);
        }
        res
    }

    fn pause(&self) {
        if self.delay > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(self.delay));
        }
    }

    /// 서점 3곳에 묻는다. 같은 책을 이미 물었으면 캐시에서 꺼내 조회 3회와 간격 3회를 건너뛴다.
    pub fn query_bookstores(&mut self, search_title: &str, raw_author: &str, raw_title: &str, isbn: &str) -> (Value, Value, Value) {
        if let Some(cached) = self.bookstore_cache.get(isbn, search_title, raw_author) {
            let part = |name: &str| cached.get(name).cloned().unwrap_or_else(|| json!({}));
            return (part("yes24"), part("aladin"), part("kyobo"));
        }
        let y = Self::query_store(self.yes24.as_ref(), isbn, search_title, raw_author, raw_title);
        self.pause();
        let a = Self::query_store(self.aladin.as_ref(), isbn, search_title, raw_author, raw_title);
        self.pause();
        let k = Self::query_store(self.kyobo.as_ref(), isbn, search_title, raw_author, raw_title);
        self.pause();
        self.bookstore_cache.put(&json!({"yes24": y, "aladin": a, "kyobo": k}), search_title, raw_author, isbn);
        (y, a, k)
    }

    /// 파일 하나를 분류한다. 반환: (목적지, 방법, 사유, 캐시 항목).
    pub fn classify_file(&mut self, fpath: &Path, source_dir: &Path, opts: ClassifyOptions) -> (Option<String>, String, String, Option<Value>) {
        let (cat, method, reason, entry, _) = self.classify_file_keyed(fpath, source_dir, opts);
        (cat, method, reason, entry)
    }

    fn classify_file_keyed(&mut self, fpath: &Path, source_dir: &Path, opts: ClassifyOptions) -> (Option<String>, String, String, Option<Value>, String) {
        let fname = fpath.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let effective = get_effective_filename(fpath, source_dir);
        let (raw_author, raw_title, search_title) = clean_filename_to_author_title(&effective);
        let rel_path = fpath.strip_prefix(source_dir).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| fname.clone());

        let cache_key = if self.cache.contains_key(&rel_path) {
            rel_path.clone()
        } else if self.cache.contains_key(&effective) {
            effective.clone()
        } else if self.cache.contains_key(&fname) {
            fname.clone()
        } else {
            rel_path.clone()
        };
        let mut entry = self.cache.get(&cache_key).cloned().filter(truthy);

        if entry.is_none()
            && let Some(ref_entry) = self.title_cache.get(&search_title)
        {
            let ref_author = ref_entry.get("author").and_then(Value::as_str).unwrap_or("");
            let conflicting = !raw_author.is_empty() && !ref_author.is_empty() && raw_author != ref_author && !ref_author.contains(&raw_author) && !raw_author.contains(ref_author);
            if !conflicting {
                let store = |n: &str| ref_entry.get(n).cloned().unwrap_or_else(|| json!({}));
                let author = if raw_author.is_empty() { ref_author.to_string() } else { raw_author.clone() };
                let made = json!({"filename": effective, "rel_path": rel_path, "author": author, "title": raw_title, "search_title": search_title,
                    "yes24": store("yes24"), "aladin": store("aladin"), "kyobo": store("kyobo"), "status": "pending", "target_category": null});
                self.cache.insert(cache_key.clone(), made.clone());
                entry = Some(made);
            }
        }

        let mut entry = match entry {
            Some(e) => e,
            None => {
                if opts.cache_only {
                    return (None, "cache_only_skip".into(), "Not in cache".into(), Some(json!({})), cache_key);
                }
                let empty = json!({"title": "", "author": "", "cat": "", "mapped": null, "url": ""});
                let (y, a, k) = if opts.use_bookstore { self.query_bookstores(&search_title, &raw_author, &raw_title, "") } else { (empty.clone(), empty.clone(), empty) };
                let made = json!({"filename": effective, "rel_path": rel_path, "author": raw_author, "title": raw_title, "search_title": search_title,
                    "yes24": y, "aladin": a, "kyobo": k, "status": "pending", "target_category": null});
                self.cache.insert(cache_key.clone(), made.clone());
                made
            }
        };

        let d = self._decide(opts.use_content_meta.then_some(fpath), &effective, &mut entry, DecideOptions { trust_single_match: opts.trust_single_match });

        // 등급 판정에 쓰려면 모델 점수가 entry 에 남아 있어야 한다
        entry["confidence"] = json!(d.confidence.unwrap_or(0.0));
        entry["model_category"] = json!(d.model_category);
        entry["target_category"] = json!(d.category);
        entry["status"] = json!(if d.category.is_some() {
            d.method.clone()
        } else if d.method.contains("conflict") {
            "conflict".to_string()
        } else {
            "not_found".to_string()
        });
        self.cache.insert(cache_key.clone(), entry.clone());
        (d.category, d.method, d.reason, Some(entry), cache_key)
    }

    /// 모델로 판정하고, 확신이 모자라면 서점 다수결로 되돌아간다.
    ///
    /// 모델이 먼저다(홀드아웃 판정률 91.5% / 정답률 90.8%, 서점은 50.8% / 85.8%). 다만 확신도가 서점 경계(`bookstore-policy`
    /// 가 잰 값) 미만이거나 신뢰 기준(0.10) 미만이면 서점을 세어 본다. 서점 조회는 한 건에 3초 이상 걸려 한 번만 부른다.
    pub fn _decide(&self, fpath: Option<&Path>, effective_fname: &str, entry: &mut Value, opts: DecideOptions) -> Decision {
        let (model_cat, model_reason, confidence) = self._decide_by_model(fpath, effective_fname, Some(entry));
        let mut store: Option<StoreVote> = None;
        let mut bookstore = |entry: &mut Value| -> StoreVote { store.get_or_insert_with(|| self.vote(entry, opts.trust_single_match)).clone() };
        let decided = |category: String, method: String, reason: String| Decision { category: Some(category), method, reason, model_category: model_cat.clone(), confidence: Some(confidence) };

        if self.bookstore_policy.prefers_bookstore(confidence) {
            let (cat, method, reason) = bookstore(entry);
            if let Some(cat) = cat {
                let why = format!("{reason} (확신도 {} < 경계 {} 이라 서점 우선; 모델: {model_reason})", fixed(confidence, 3), fixed(self.bookstore_policy.override_below, 3));
                return decided(cat, method, why);
            }
        }
        if model_cat.is_some() && confidence < MODEL_TRUST_MIN_CONFIDENCE {
            // 모델이 답은 했지만 화면이 '확실'로 인정할 만큼은 아니다. 서점을 세어 본다. 다수결이 안 서도 득표는 entry 에 남는다.
            let (cat, method, reason) = bookstore(entry);
            if let Some(cat) = cat.filter(|_| method == "bookstore_majority") {
                let why = format!("{reason} (확신도 {} < 신뢰 기준 {} 이라 서점 다수결 우선; 모델: {model_reason})", fixed(confidence, 3), fixed(MODEL_TRUST_MIN_CONFIDENCE, 2));
                return decided(cat, method, why);
            }
        }
        if let Some(cat) = model_cat.clone() {
            return decided(cat, "model".into(), model_reason);
        }
        let (cat, method, reason) = bookstore(entry);
        match cat {
            Some(cat) => decided(cat, method, format!("{reason} (모델: {model_reason})")),
            None => Decision { category: None, method, reason: format!("{model_reason}; {reason}"), model_category: None, confidence: Some(confidence) },
        }
    }

    fn vote(&self, entry: &mut Value, trust: bool) -> StoreVote {
        match &self.decider {
            Some(d) => d.decide(entry, trust, &|e, t| self._decide_by_bookstore_with(e, t)),
            None => self._decide_by_bookstore_with(entry, trust),
        }
    }

    /// 판정 결과와 확신도를 돌려준다. 상위 후보는 `entry["model_candidates"]` 에 `[카테고리, 점수]` 로 남긴다.
    pub fn _decide_by_model(&self, fpath: Option<&Path>, effective_fname: &str, entry: Option<&mut Value>) -> (Option<String>, String, f64) {
        let mut entry = entry;
        if let Some(e) = entry.as_deref_mut()
            && e.get("model_candidates").is_none()
        {
            e["model_candidates"] = json!([]);
        }
        let Some(classifier) = self.classifier.as_ref().filter(|c| c.is_available()) else {
            return (None, "모델 파일이 없어 판정하지 않음".into(), 0.0);
        };
        let result: Result<Prediction, String> = match fpath {
            Some(p) => classifier.classify_path(p, None),
            // 본문을 안 보기로 한 호출이다. 파일명만으로 판정한다.
            None => {
                let ext = Path::new(effective_fname).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                classifier.classify_document(&json!({"name": effective_fname, "text": "", "title": "", "author": "", "publisher": "", "type": ext}), None)
            }
        };
        let pred = match result {
            Ok(p) => p,
            Err(e) => {
                log::warn!("모델 판정에 실패했다: {e}");
                return (None, format!("모델 판정 실패: {e}"), 0.0);
            }
        };
        if let Some(e) = entry {
            e["model_candidates"] = Value::Array(pred.ranked.iter().take(3).map(|(c, s)| json!([c, s])).collect());
        }
        (pred.category, pred.reason, pred.confidence)
    }

    /// 서점 3곳의 매핑 결과를 다수결로 모은다. 2곳 이상 같으면 채택한다.
    pub fn _decide_by_bookstore(&self, entry: &mut Value) -> StoreVote {
        self._decide_by_bookstore_with(entry, true)
    }

    pub fn decide_by_bookstore(&self, entry: &mut Value) -> StoreVote {
        self._decide_by_bookstore_with(entry, true)
    }

    /// 한 서점이 후보를 여럿 낼 수 있다(DB 에서 한 키워드가 여러 카테고리에 붙는 것이 정상). 그때는 후보를 모두 세고, 다른 서점과
    /// 겹치는 카테고리가 이긴다. 득표는 갈리거나 못 찾았을 때도 `entry["bookstore_candidates"]` 에 남긴다.
    pub fn _decide_by_bookstore_with(&self, entry: &mut Value, trust_single_match: bool) -> StoreVote {
        let search_title = entry.get("search_title").and_then(Value::as_str).unwrap_or("").to_string();
        let mut votes: Vec<(String, i64)> = Vec::new();
        let mut ambiguous = 0;
        for name in ["yes24", "aladin", "kyobo"] {
            let store = entry.get(name).cloned().unwrap_or_else(|| json!({}));
            if !is_title_match_reliable(&search_title, store.get("title").and_then(Value::as_str).unwrap_or("")) {
                continue;
            }
            let candidates: Vec<String> = match store.get("candidates").and_then(Value::as_array).filter(|a| !a.is_empty()) {
                Some(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                None => store.get("mapped").and_then(Value::as_str).filter(|m| !m.is_empty()).map(|m| vec![m.to_string()]).unwrap_or_default(),
            };
            if candidates.is_empty() {
                continue;
            }
            if candidates.len() > 1 {
                ambiguous += 1;
            }
            for cat in candidates {
                match votes.iter_mut().find(|(c, _)| *c == cat) {
                    Some((_, n)) => *n += 1,
                    None => votes.push((cat, 1)),
                }
            }
        }
        // Counter.most_common: 득표 내림차순, 같으면 처음 나온 순서.
        votes.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        entry["bookstore_candidates"] = Value::Array(votes.iter().take(2).map(|(c, n)| json!([c, n])).collect());

        let Some((top, n)) = votes.first().cloned() else { return (None, "not_found".into(), "서점에서 못 찾음".into()) };
        if n >= MIN_STORE_VOTES {
            return (Some(top.clone()), "bookstore_majority".into(), format!("서점 {n}곳이 {top} 로 일치"));
        }
        // 한 곳만 답했는데 그 한 곳이 후보를 여럿 냈다면 고를 근거가 없다.
        if votes.len() == 1 && ambiguous == 0 && trust_single_match {
            return (Some(top.clone()), "bookstore_single".into(), format!("서점 한 곳만 찾음: {top}"));
        }
        let mut names: Vec<&str> = votes.iter().map(|(c, _)| c.as_str()).collect();
        names.sort();
        (None, "conflict".into(), format!("서점 판정이 갈림: [{}]", names.iter().map(|c| format!("'{c}'")).collect::<Vec<_>>().join(", ")))
    }

    /// 파일을 분류하고, 목적지가 정해지면 옮기거나(이미 있으면 정리하거나) 건너뛴다.
    pub fn process_file(&mut self, fpath: &Path, source_dir: &Path, opts: ProcessOptions) -> Value {
        let (target_cat, method, reason, entry, key) = self.classify_file_keyed(
            fpath,
            source_dir,
            ClassifyOptions { trust_single_match: opts.trust_single_match, use_bookstore: opts.use_bookstore, use_content_meta: opts.use_content_meta, cache_only: opts.cache_only },
        );
        let mut result = json!({"file": fpath.to_string_lossy(), "target_category": target_cat, "method": method, "reason": reason, "action": "none"});
        let Some(target_cat) = target_cat else { return result };
        if source_dir == self.library_root.join(&target_cat) {
            result["action"] = json!("skipped_same_category");
            return result;
        }
        // 목적지는 라이브러리 바로 아래 디렉토리 하나여야 한다. `..` 나 경로 구분자가 든 값은 거부한다.
        if !is_single_component(&target_cat) {
            result["action"] = json!("none");
            result["reason"] = json!("목적지 카테고리 이름이 안전하지 않아 옮기지 않음");
            return result;
        }
        let Some(name) = fpath.file_name() else { return result };
        let dest_dir = self.library_root.join(&target_cat);
        let dest_path = dest_dir.join(name);
        let has_entry = entry.as_ref().is_some_and(truthy);
        let set_status = |svc: &mut Self, status: &str| {
            if has_entry && let Some(e) = svc.cache.get_mut(&key) {
                e["status"] = json!(status);
            }
        };

        if dest_path.exists() {
            if same_file(fpath, &dest_path) {
                result["action"] = json!("skipped_same_file");
                return result;
            }
            if opts.clean_existing {
                if !opts.dry_run {
                    let _ = fs::remove_file(fpath);
                    clean_empty_parent_dirs_with(fpath.parent().unwrap_or(Path::new("")), source_dir, self.fs.as_ref());
                    set_status(self, "already_exists_cleaned");
                }
                result["action"] = json!("cleaned_duplicate");
            } else {
                result["action"] = json!("skipped_already_exists");
            }
        } else {
            if opts.auto_move && !opts.dry_run {
                if fs::create_dir_all(&dest_dir).is_ok() && move_file(self.fs.as_ref(), fpath, &dest_path).is_ok() {
                    clean_empty_parent_dirs_with(fpath.parent().unwrap_or(Path::new("")), source_dir, self.fs.as_ref());
                    set_status(self, "moved");
                } else {
                    log::error!("파일을 옮기지 못했다");
                    result["action"] = json!("move_failed");
                    return result;
                }
            }
            result["action"] = json!(if opts.auto_move && !opts.dry_run { "moved" } else { "classified" });
        }
        result
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Object(o) => !o.is_empty(),
        Value::Array(a) => !a.is_empty(),
        _ => true,
    }
}

fn same_file(left: &Path, right: &Path) -> bool {
    let (Ok(left_metadata), Ok(right_metadata)) = (fs::metadata(left), fs::metadata(right)) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left_metadata.dev() == right_metadata.dev() && left_metadata.ino() == right_metadata.ino()
    }
    #[cfg(not(unix))]
    {
        match (fs::canonicalize(left), fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    }
}

/// 구분자와 `.`/`..` 가 없는 이름 하나인가.
fn is_single_component(name: &str) -> bool {
    let mut comps = Path::new(name).components();
    matches!((comps.next(), comps.next()), (Some(Component::Normal(_)), None)) && !name.contains(['/', '\\', '\0'])
}

/// `shutil.move`: 이름 바꾸기를 먼저 하고, 다른 파일 시스템이면 복사 뒤 지운다.
fn move_file(fs_ops: &dyn FsOps, from: &Path, to: &Path) -> std::io::Result<()> {
    match fs_ops.rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            fs::copy(from, to)?;
            fs::remove_file(from)
        }
        Err(e) => Err(e),
    }
}

/// `BookClassifierApi` 로 쓰기 위한 공유 래퍼. 서비스는 캐시를 바꾸므로 잠금 안에서 호출한다.
pub struct SharedBookClassifier {
    inner: Mutex<BookClassifierService>,
    override_below: f64,
    min_confidence: Option<f64>,
    classifier: Option<Arc<dyn Classify>>,
    policy: BookstorePolicy,
}

impl SharedBookClassifier {
    pub fn new(service: BookClassifierService) -> Self {
        Self {
            override_below: service.bookstore_policy.override_below,
            policy: service.bookstore_policy.clone(),
            classifier: service.classifier.clone(),
            min_confidence: service.classifier.as_ref().and_then(|c| c.min_confidence()),
            inner: Mutex::new(service),
        }
    }
}

impl SharedBookClassifier {
    /// 요청 하나를 시작한다. 디스크의 분류 캐시를 다시 읽어 이전 요청의 메모리 상태가 새지 않게 한다(Python 은 요청마다 서비스를 새로 만든다).
    pub fn begin_request(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).load_cache();
    }

    /// 분류 캐시를 파일에 쓴다. 같은 잠금 안에서 쓰므로 동시 요청이 같은 파일을 겹쳐 쓰지 않는다.
    pub fn save_cache(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).save_cache();
    }

    /// 요청 하나의 호출 조건(원본 디렉토리, 서점·본문 사용 여부)을 묶어 `BookClassifierApi` 로 내놓는다.
    pub fn call(&self, source_dir: &Path, use_bookstore: bool, use_content_meta: bool) -> ServiceCall<'_> {
        self.call_with_delay(source_dir, use_bookstore, use_content_meta, None)
    }

    pub fn call_with_delay(&self, source_dir: &Path, use_bookstore: bool, use_content_meta: bool, delay: Option<f64>) -> ServiceCall<'_> {
        ServiceCall { shared: self, source_dir: source_dir.to_path_buf(), opts: ClassifyOptions { use_bookstore, use_content_meta, ..Default::default() }, delay }
    }
}

/// `SharedBookClassifier` 를 한 요청의 조건으로 부르는 `BookClassifierApi`. Python `classify_file(path, source_dir, use_bookstore=..., use_content_meta=...)` 과 같다.
pub struct ServiceCall<'a> {
    shared: &'a SharedBookClassifier,
    source_dir: PathBuf,
    opts: ClassifyOptions,
    delay: Option<f64>,
}

impl BookClassifierApi for ServiceCall<'_> {
    fn classify_file(&self, path: &Path) -> (Option<String>, String, String, Value) {
        let mut svc = self.shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        let original_delay = svc.delay;
        if let Some(delay) = self.delay {
            svc.delay = delay.max(0.0);
        }
        let (cat, method, reason, entry) = svc.classify_file(path, &self.source_dir, self.opts);
        svc.delay = original_delay;
        (cat, method, reason, entry.unwrap_or_else(|| json!({})))
    }

    fn override_below(&self) -> f64 {
        self.shared.override_below()
    }

    fn prefers_bookstore(&self, confidence: f64) -> bool {
        self.shared.prefers_bookstore(confidence)
    }

    fn min_confidence(&self) -> Option<f64> {
        self.shared.min_confidence()
    }

    fn expected_accuracy(&self, confidence: Option<f64>) -> Option<Option<f64>> {
        self.shared.expected_accuracy(confidence)
    }
}

impl BookClassifierApi for SharedBookClassifier {
    fn classify_file(&self, path: &Path) -> (Option<String>, String, String, Value) {
        let mut svc = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let root = svc.library_root.clone();
        let source = if path.starts_with(&root) { root } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
        let (cat, method, reason, entry) = svc.classify_file(path, &source, ClassifyOptions::default());
        (cat, method, reason, entry.unwrap_or_else(|| json!({})))
    }

    fn override_below(&self) -> f64 {
        self.override_below
    }

    fn prefers_bookstore(&self, confidence: f64) -> bool {
        self.policy.prefers_bookstore(confidence)
    }

    fn min_confidence(&self) -> Option<f64> {
        self.min_confidence
    }

    fn expected_accuracy(&self, confidence: Option<f64>) -> Option<Option<f64>> {
        let classifier = self.classifier.as_ref()?;
        Some(confidence.and_then(|c| classifier.expected_accuracy(c)))
    }
}
