//! 디렉토리·파일 이름을 읽어 제목, 저자, 권수 범위를 뽑는 순수 함수들.
//! Python 원본의 정규식을 그대로 옮겼다. 기대값은 원본이 만든 `tests/rust/normalize_comic_directories/cases.json`이다.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use fancy_regex::Regex;
use unicode_normalization::UnicodeNormalization;

pub const TARGET_CATEGORIES: [&str; 3] = ["3_미정리", "4_대기", "9_temp"];
const BOOK_EXTENSIONS: [&str; 2] = ["pdf", "epub"];
const AUTHOR_TAGS: [&str; 12] = ["만화", "comic", "코믹", "webtoon", "scan", "스캔", "스캔본", "한글", "영문", "일문", "정발", "번역"];
const STRIP_SET: &str = " _-.()[]";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("잘못된 정규식 {pattern}: {e}"))
}

static VOLUME_SUFFIX: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\s*(?:[-_ ]+)?(?:\d{1,4}(?:\s*[-~]\s*\d{1,4})?\s*(?:권|화|편|장|부)|(?:vol(?:ume)?|ch(?:apter)?)\.?\s*\d+).*$"));
static CHAPTER_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:제\s*)?\d{1,5}\s*(?:화|권|장|편)\s*[-_. ]*"));
static NUMBER_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"^\[\d{2,}\]\s*"));
static DIRECTORY_METADATA: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?:\s+|\s*\()(?=",
        r"\d{1,4}\s*(?:[-~]\s*\d{1,4})\s*(?:(?:권|화|편|장)?(?:완결|완|미완|외전|외)?)|",
        r"\d{1,4}\s*(?:권|화|편|장)(?:완결|완|외전|외)?|",
        r"(?:완결|미완|연재\s*중?|누락|잘림|미수록)(?=$|[\s(\[~,-])",
        r")"
    ))
});
static VOLUME_NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:\bvol(?:ume)?\.?\s*(\d{1,4})|(?<!\d)(\d{1,4})\s*권)"));
static CHAPTER_NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:\bch(?:apter)?\.?\s*(\d{1,5})|(?<!\d)(\d{1,5})\s*화)"));
static TRAILING_SEQUENCE_NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"(?<!\d)(\d{1,4})(?:\s*[-_.]?\s*(?:전|후|상|중|하))?\s*$"));
static EXTRA_MATERIAL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:외전|번외|특전|특별편|보너스|\bextra\b|\bspecial\b|\bside[ _-]*story\b)"));
static INCOMPLETE_STATUS: LazyLock<Regex> = LazyLock::new(|| re(r"(?:미완|연재\s*중?|중단|누락|잘림|미수록)"));
static COMPLETE_STATUS: LazyLock<Regex> = LazyLock::new(|| re(r"(?:완결|(?<!미)완(?=외?(?:[\s,)\]]|$)))"));
static EXTRA_STATUS: LazyLock<Regex> = LazyLock::new(|| re(r"(?:외전|번외|특전|특별편|보너스|외(?=[\s,)\]]|$))"));
static AUTHOR_ROLE: LazyLock<Regex> = LazyLock::new(|| re(r"(?:글|그림|저|원작|스토리|만화)(?=$|[\s,/&·])"));
static NON_KOREAN_SCRIPT: LazyLock<Regex> = LazyLock::new(|| re(r"[A-Za-zぁ-ゟ゠-ヿ一-龯]"));
static FILE_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"^\[([^\]]+)\]\s*(.+)$"));
static DIR_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"^\[([^\]]+)\]\s*"));
static LEADING_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"^\[[^\]]+\]\s*"));
static TAG_HAS_LETTER: LazyLock<Regex> = LazyLock::new(|| re(r"[A-Za-z가-힣]"));
static HANGUL: LazyLock<Regex> = LazyLock::new(|| re(r"[가-힣]"));
static TRAILING_BRACKET: LazyLock<Regex> = LazyLock::new(|| re(r"\s*\[[^\]]*\]\s*$"));
static TRAILING_PAREN: LazyLock<Regex> = LazyLock::new(|| re(r"\s*\([^)]*(?:\d|완|미완|연재|권|화|부)[^)]*\)\s*$"));
static TRAILING_RANGE: LazyLock<Regex> = LazyLock::new(|| re(r"\s+\d{1,4}(?:\s*[-~]\s*\d{1,4})?\s*(?:권|화|편|장|부).*$"));
static TRAILING_STATUS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\s+(?:완결|완|미완|연재중?).*$"));
static BOOKSTORE_NUMBER_PAREN: LazyLock<Regex> = LazyLock::new(|| re(r"\s*(?:\(\s*|\[\s*)\d{1,4}(?:\s*[-~]\s*\d{1,4})?\s*(?:권|화|편|장)(?:\s*\)|\s*\])\s*$"));
static BOOKSTORE_TRAILING_NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"\s+\d{1,4}\s*(?:권|화|편|장)?\s*$"));
static ENGLISH_SUBTITLE: LazyLock<Regex> = LazyLock::new(|| re(r"\s+[-:：]\s*[A-Za-z][A-Za-z0-9\s'.,!?&-]*$"));
static EDITION_PAREN: LazyLock<Regex> = LazyLock::new(|| re(r"\s*\([^()]*(?:판|편|본|판본|[A-Za-z가-힣])[^()]*\)\s*$"));
static CONTROL_CHARS: LazyLock<Regex> = LazyLock::new(|| re(r"[\x00-\x1f\x7f]"));
static TRAILING_SEPARATORS: LazyLock<Regex> = LazyLock::new(|| re(r"\s*[,/&·]+\s*$"));
static WHITESPACE_RUN: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));

fn nfkc(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut assigned = String::new();
    for character in value.chars() {
        if pyfmt::py_is_assigned(character) {
            assigned.push(character);
        } else {
            normalized.extend(assigned.nfkc());
            assigned.clear();
            normalized.push(character);
        }
    }
    normalized.extend(assigned.nfkc());
    normalized
}

#[cfg(test)]
mod nfkc_oracle_tests {
    use std::collections::HashMap;

    use super::nfkc;

    #[test]
    fn nfkc_matches_python_313_unicode_151_for_every_single_codepoint() {
        let mappings: HashMap<u32, String> = include_str!("../../../tests/fixtures/python_nfkc_mappings_15_1.txt")
            .lines()
            .map(|line| {
                let (codepoint, normalized) = line.split_once(';').unwrap();
                let chars = normalized.split(',').map(|cp| char::from_u32(u32::from_str_radix(cp, 16).unwrap()).unwrap()).collect();
                (u32::from_str_radix(codepoint, 16).unwrap(), chars)
            })
            .collect();
        for codepoint in 0..=0x10ffff {
            let Some(character) = char::from_u32(codepoint) else { continue };
            let input = character.to_string();
            let expected = mappings.get(&codepoint).map_or(input.as_str(), String::as_str);
            assert_eq!(nfkc(&input), expected, "U+{codepoint:04X}");
        }
    }

    #[test]
    fn unicode_151_unassigned_codepoints_are_normalization_boundaries() {
        for codepoint in 0..=0x10ffff {
            let Some(character) = char::from_u32(codepoint) else { continue };
            if pyfmt::py_is_assigned(character) {
                continue;
            }
            let input = format!("A{character}\u{030A}");
            assert_eq!(nfkc(&input), input, "U+{codepoint:04X}");
        }
    }
}

fn strip_set(value: &str) -> &str {
    value.trim_matches(|c| STRIP_SET.contains(c))
}

fn is_book_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| BOOK_EXTENSIONS.iter().any(|b| e.to_string_lossy().eq_ignore_ascii_case(b)))
}

/// 제외할 만한 역할 태그(만화, 스캔 …)가 아니고 글자가 들어 있으면 저자 태그로 본다.
fn author_tag(tag: &str) -> Option<String> {
    let tag = tag.trim();
    if AUTHOR_TAGS.contains(&tag.to_lowercase().as_str()) || !TAG_HAS_LETTER.is_match(tag).unwrap_or(false) {
        return None;
    }
    Some(tag.to_owned())
}

/// 파일 이름에서 (제목, 저자)를 뽑는다. 제목이 비면 `None`.
pub fn parse_filename(path: &Path) -> Option<(String, Option<String>)> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut stem = nfkc(&stem).trim().to_owned();
    stem = NUMBER_TAG.replace(&stem, "").into_owned();
    let mut author = None;
    if let Ok(Some(caps)) = FILE_TAG.captures(&stem.clone())
        && let Some(tag) = author_tag(&caps[1])
    {
        author = Some(tag);
        stem = caps[2].trim().to_owned();
    }
    stem = CHAPTER_PREFIX.replace(&stem, "").into_owned();
    let before_volume = strip_set(&stem).to_owned();
    stem = strip_set(&VOLUME_SUFFIX.replace_all(&stem, "")).to_owned();
    if stem == before_volume {
        stem = strip_set(&TRAILING_SEQUENCE_NUMBER.replace_all(&stem, "")).to_owned();
    }
    if !stem.chars().any(|c| c == '_' || pyfmt::py_isalnum(c)) {
        return None;
    }
    Some((stem, author))
}

/// 비교용 키: NFKC로 풀고 소문자로 바꾼 뒤 글자·숫자만 남긴다.
pub fn title_key(value: &str) -> String {
    pyfmt::py_casefold(&nfkc(value)).chars().filter(|c| pyfmt::py_isalnum(*c)).collect()
}

/// 디렉토리 이름 앞의 `[저자]` 태그.
pub fn directory_author(name: &str) -> Option<String> {
    let caps = DIR_TAG.captures(name).ok().flatten()?;
    author_tag(&caps[1])
}

/// 한글 저자 이름만 남긴다. 역할어(글, 그림 …)를 떼고, 한글이 없거나 한자·가나·알파벳이 섞이면 `None`.
pub fn korean_author(value: Option<&str>) -> Option<String> {
    let value = value.filter(|v| !v.is_empty())?;
    let roles_removed = AUTHOR_ROLE.replace_all(&nfkc(value), "").into_owned();
    let cleaned = TRAILING_SEPARATORS.replace_all(&roles_removed, "").trim().to_owned();
    if !HANGUL.is_match(&cleaned).unwrap_or(false) || NON_KOREAN_SCRIPT.is_match(&cleaned).unwrap_or(false) {
        return None;
    }
    Some(cleaned)
}

/// 디렉토리 이름에서 저자 태그와 뒤쪽 권수·완결 표기를 떼어 낸 제목.
pub fn directory_title(name: &str) -> String {
    let mut title = LEADING_TAG.replace(name, "").into_owned();
    if let Ok(Some(m)) = DIRECTORY_METADATA.find(&title) {
        let mut start = m.start();
        let prefix = &title[..start];
        let open = prefix.rfind('(').map(|i| i as i64).unwrap_or(-1);
        let close = prefix.rfind(')').map(|i| i as i64).unwrap_or(-1);
        if open > close {
            start = open as usize;
        }
        title.truncate(start);
    }
    for pattern in [&*TRAILING_BRACKET, &*TRAILING_PAREN, &*TRAILING_RANGE, &*TRAILING_STATUS] {
        title = pattern.replace_all(&title, "").into_owned();
    }
    title.trim_matches(|c| " _-.".contains(c)).to_owned()
}

/// 이미 정리된 다른 디렉토리에서 (제목 키, 한글 저자) 쌍을 모은다.
pub fn local_author_references(root: &Path) -> Vec<(String, String)> {
    let mut references = Vec::new();
    for category in sorted_subdirs(root) {
        for directory in sorted_subdirs(&category) {
            let name = directory.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let author = korean_author(directory_author(&name).as_deref());
            let title = title_key(&directory_title(&name));
            if let Some(author) = author
                && !title.is_empty()
            {
                references.push((title, author));
            }
        }
    }
    references
}

fn sorted_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
    dirs.sort();
    dirs
}

/// 제목 키가 같은 저자가 하나면 그 저자, 아니면 접두 일치 후보의 저자가 하나일 때 그 저자.
pub fn matching_local_author(title: &str, references: &[(String, String)]) -> Option<String> {
    let key = title_key(title);
    let exact: HashSet<&String> = references.iter().filter(|(candidate, _)| *candidate == key).map(|(_, a)| a).collect();
    if exact.len() == 1 {
        return exact.into_iter().next().cloned();
    }
    let variants: HashSet<&String> =
        references.iter().filter(|(candidate, _)| key.chars().count() >= 2 && (candidate.starts_with(&key) || key.starts_with(candidate.as_str()))).map(|(_, a)| a).collect();
    if variants.len() == 1 { variants.into_iter().next().cloned() } else { None }
}

/// 디렉토리 이름의 첫 권수·완결 표기부터 끝까지.
pub fn existing_metadata(name: &str) -> String {
    match DIRECTORY_METADATA.find(name) {
        Ok(Some(m)) => name[m.start()..].trim().to_owned(),
        _ => String::new(),
    }
}

fn matched_numbers(pattern: &Regex, value: &str) -> Vec<u32> {
    let mut numbers = Vec::new();
    for caps in pattern.captures_iter(value).flatten() {
        if let Some(group) = caps.iter().skip(1).flatten().next()
            && let Ok(n) = group.as_str().parse::<u32>()
            && n > 0
        {
            numbers.push(n);
        }
    }
    numbers
}

/// 디렉토리 안 파일·하위 폴더 이름에서 `(1-10권 완외)` 같은 범위 표기를 만든다. 범위를 못 구하면 `None`.
pub fn normalized_directory_metadata(directory: &Path) -> Option<String> {
    let mut volumes = Vec::new();
    let mut chapters = Vec::new();
    let mut fallback_volumes = Vec::new();
    let mut main_names: Vec<String> = Vec::new();
    let mut has_extra_entry = false;

    for entry in fs::read_dir(directory).ok()?.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        let name = if kind.is_file() {
            if !is_book_file(&path) {
                continue;
            }
            path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        } else if kind.is_dir() {
            entry.file_name().to_string_lossy().into_owned()
        } else {
            continue;
        };
        let normalized = nfkc(&name);
        if EXTRA_MATERIAL.is_match(&normalized).unwrap_or(false) {
            has_extra_entry = true;
            continue;
        }
        let found_volumes = matched_numbers(&VOLUME_NUMBER, &normalized);
        let found_chapters = matched_numbers(&CHAPTER_NUMBER, &normalized);
        let none_found = found_volumes.is_empty() && found_chapters.is_empty();
        volumes.extend(found_volumes);
        chapters.extend(found_chapters);
        if none_found
            && let Ok(Some(caps)) = TRAILING_SEQUENCE_NUMBER.captures(&normalized)
            && let Ok(n) = caps[1].parse::<u32>()
            && n > 0
        {
            fallback_volumes.push(n);
        }
        main_names.push(normalized);
    }

    let min_max = |v: &[u32]| (*v.iter().min().unwrap_or(&0), *v.iter().max().unwrap_or(&0));
    let ((first, last), unit) = if !chapters.is_empty() {
        (min_max(&chapters), "화")
    } else if !volumes.is_empty() {
        (min_max(&volumes), "권")
    } else if fallback_volumes.iter().collect::<HashSet<_>>().len() >= 2 {
        (min_max(&fallback_volumes), "권")
    } else {
        return None;
    };

    let name = directory.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let metadata = existing_metadata(&name);
    let completion = if INCOMPLETE_STATUS.is_match(&metadata).unwrap_or(false) {
        "미완"
    } else if COMPLETE_STATUS.is_match(&metadata).unwrap_or(false) || main_names.iter().any(|n| n.contains("완결")) {
        "완"
    } else {
        ""
    };
    let has_extras = EXTRA_STATUS.is_match(&metadata).unwrap_or(false) || (has_extra_entry && !main_names.is_empty());
    let flags = format!("{completion}{}", if has_extras { "외" } else { "" });
    let flag_suffix = if flags.is_empty() { String::new() } else { format!(" {flags}") };
    Some(format!("({first}-{last}{unit}{flag_suffix})"))
}

/// 이름 정리 결과. `failure_status`가 있으면 건너뛴다.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentityResult {
    pub title: Option<String>,
    pub author: Option<String>,
    pub confidence: f64,
    pub file_count: usize,
    pub failure_status: Option<String>,
    pub failure_reason: String,
}

fn failure(status: &str, reason: String, confidence: f64, file_count: usize) -> IdentityResult {
    IdentityResult { title: None, author: None, confidence, file_count, failure_status: Some(status.to_owned()), failure_reason: reason }
}

/// 처음 나온 순서가 앞선 쪽을 이기는 최빈값. Python `Counter.most_common(1)`과 같은 동률 처리다.
pub fn most_common<T: Eq + std::hash::Hash + Clone>(items: impl IntoIterator<Item = T>) -> Option<(T, usize)> {
    let mut order: Vec<T> = Vec::new();
    let mut counts: HashMap<T, usize> = HashMap::new();
    for item in items {
        let c = counts.entry(item.clone()).or_insert(0);
        if *c == 0 {
            order.push(item);
        }
        *c += 1;
    }
    let mut best: Option<(T, usize)> = None;
    for item in order {
        let n = counts[&item];
        if best.as_ref().is_none_or(|(_, b)| n > *b) {
            best = Some((item, n));
        }
    }
    best
}

/// 디렉토리 안 책 파일 이름들로 작품 제목과 저자를 추정한다.
pub fn infer_identity(directory: &Path) -> IdentityResult {
    let mut book_files: Vec<PathBuf> =
        fs::read_dir(directory).map(|it| it.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && is_book_file(&e.path())).map(|e| e.path()).collect()).unwrap_or_default();
    book_files.sort();
    let parsed: Vec<(String, Option<String>)> = book_files.iter().filter_map(|p| parse_filename(p)).collect();
    let dir_name = directory.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    if parsed.len() < 2 {
        let fallback_title = directory_title(&dir_name);
        let matches_directory = parsed.iter().all(|(t, _)| titles_equivalent(t, &fallback_title));
        if book_files.len() >= 2 && normalized_directory_metadata(directory).is_some() && !fallback_title.is_empty() && matches_directory {
            let author = most_common(parsed.iter().filter_map(|(_, a)| a.clone())).map(|(a, _)| a).or_else(|| directory_author(&dir_name));
            return IdentityResult { title: Some(fallback_title), author, confidence: 0.8, file_count: book_files.len(), failure_status: None, failure_reason: String::new() };
        }
        let status = if book_files.len() < 2 { "SKIP_INSUFFICIENT_FILES" } else { "SKIP_UNPARSEABLE_FILENAMES" };
        return failure(status, format!("book files: {}; parsed filenames: {}", book_files.len(), parsed.len()), 0.0, book_files.len());
    }

    // 키별 묶음. 처음 나온 키가 동률에서 이긴다.
    let mut keys: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&(String, Option<String>)>> = HashMap::new();
    for item in &parsed {
        let key = title_key(&item.0);
        if !groups.contains_key(&key) {
            keys.push(key.clone());
        }
        groups.entry(key).or_default().push(item);
    }
    let mut best_key = keys[0].clone();
    for key in &keys {
        if groups[key].len() > groups[&best_key].len() {
            best_key = key.clone();
        }
    }
    let matches = &groups[&best_key];
    let confidence = matches.len() as f64 / parsed.len() as f64;
    if best_key.is_empty() {
        return failure("SKIP_UNPARSEABLE_FILENAMES", format!("book files: {}; parsed filenames: {}", book_files.len(), parsed.len()), confidence, book_files.len());
    }
    if confidence < 0.6 {
        return failure("SKIP_AMBIGUOUS_FILE_TITLES", format!("dominant filename title: {}/{}", matches.len(), parsed.len()), confidence, book_files.len());
    }
    let title = most_common(matches.iter().map(|m| m.0.clone())).map(|(t, _)| t).unwrap_or_default();
    let current_title = directory_title(&dir_name);
    if !titles_equivalent(&title, &current_title) {
        return failure("SKIP_TITLE_MISMATCH", format!("filename title: {title}; directory title: {current_title}"), confidence, book_files.len());
    }
    let mut author = None;
    if let Some((a, count)) = most_common(matches.iter().filter_map(|m| m.1.clone()))
        && count as f64 / matches.len() as f64 >= 0.6
    {
        author = Some(a);
    }
    IdentityResult { title: Some(title), author, confidence, file_count: book_files.len(), failure_status: None, failure_reason: String::new() }
}

/// 서점 제목에서 권수 표기와 끝의 마침표를 뗀 기본 제목.
pub fn bookstore_title_base(found_title: &str) -> String {
    let title = nfkc(found_title).trim().to_owned();
    let title = BOOKSTORE_NUMBER_PAREN.replace_all(&title, "").into_owned();
    let title = BOOKSTORE_TRAILING_NUMBER.replace_all(&title, "").into_owned();
    title.trim().trim_end_matches(['.', '．', '。']).trim_end().to_owned()
}

/// 영문 부제·괄호 판본 표기를 뗀 변형들의 키 집합.
pub fn title_variants(value: &str) -> HashSet<String> {
    let normalized = nfkc(value).trim().to_owned();
    let candidates = [normalized.clone(), ENGLISH_SUBTITLE.replace_all(&normalized, "").trim().to_owned(), EDITION_PAREN.replace_all(&normalized, "").trim().to_owned()];
    candidates.iter().map(|c| title_key(c)).filter(|k| !k.is_empty()).collect()
}

pub fn titles_equivalent(left: &str, right: &str) -> bool {
    let l = title_variants(left);
    !l.is_disjoint(&title_variants(right))
}

pub fn bookstore_title_matches(found_title: &str, candidate_title: &str) -> bool {
    titles_equivalent(&bookstore_title_base(found_title), candidate_title)
}

/// 파일 시스템에 쓸 수 있는 이름. 금지 문자는 전각으로 바꾸고, 비거나 255바이트를 넘으면 `None`.
pub fn filesystem_safe_name(value: &str) -> Option<String> {
    let replaced: String = nfkc(value)
        .chars()
        .map(|c| match c {
            '/' => '／',
            '\\' => '＼',
            ':' => '：',
            '*' => '＊',
            '?' => '？',
            '"' => '＂',
            '<' => '＜',
            '>' => '＞',
            '|' => '｜',
            other => other,
        })
        .collect();
    let no_control = CONTROL_CHARS.replace_all(&replaced, "").into_owned();
    let squeezed = WHITESPACE_RUN.replace_all(&no_control, " ");
    let value = squeezed.trim().trim_end_matches([' ', '.']).to_owned();
    if value.is_empty() || value == "." || value == ".." || value.len() > 255 {
        return None;
    }
    Some(value)
}
