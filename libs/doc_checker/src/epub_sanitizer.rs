//! `--sanitize`: EPUB을 허용 목록으로 재구성한다. 텍스트와 이미지를 보여 주는 데 필요 없는 요소(스크립트, 실행 파일,
//! 원격 로드, 외부 개체)를 지우고, 지운 멤버를 가리키는 manifest 항목도 함께 지운다. 인터랙티브 책은 깨지는 것을 감수한다.
//!
//! 순서: 검사 → 보안 지표(HIGH 이상)가 있는 파일만 대상 → `rebuild` → `verify`(본문 글자와 구조) → 재검사 → 적용.
//! 적용은 원본을 휴지통 디렉터리에 두고, 같은 inode에 덮어쓰고 mtime을 되돌린다(ES 문서 id가 inode라서 `mv` 교체를 쓰지 않는다).
//!
//! 문서 한 개의 글 정리는 `markup_sanitizer`, 멤버 읽기·분류·재구성은 `package_rebuilder`가 맡는다.
//! 이 모듈에는 둘이 함께 쓰는 URL 판정과 검증·적용·진입점이 있다.

use common::xml_parser::parse_xml;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::LazyLock;

use crate::Severity;
use common::url_parser::unquote;
use common::zip_reader::zip_entry_name;
use container_parser::magic_sniffer::{Sniffed, sniff};
use htmlkit::{decode_entities, text_of};
use regex::Regex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(crate) const HTML_EXTS: &[&str] = &["xhtml", "html", "htm"];
const SAFE_SCHEMES: &[&str] = &["http", "https", "mailto", "tel", "sms", "epub", "cid", "urn", "tag"];

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("잘못된 정규식 {p}: {e}"))
}

static SELF_CLOSING_RAW: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<(script|style)\b([^>]*?)/>"));

/// 바꾼 내용을 종류별로 센다.
#[derive(Default, Debug)]
pub struct Changes(BTreeMap<&'static str, usize>);

impl Changes {
    fn add(&mut self, what: &'static str) {
        *self.0.entry(what).or_default() += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn summary(&self) -> String {
        self.0.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
    }

    fn merge(&mut self, other: Changes) {
        for (k, n) in other.0 {
            *self.0.entry(k).or_default() += n;
        }
    }
}

// ---------------------------------------------------------------- URL 판정

/// 브라우저가 URL을 읽는 방식으로 정규화한다: 엔티티를 풀고, 공백·제어문자를 지우고, 소문자로 만들고, `\`를 `/`로 본다
/// (`/\evil`과 `\/evil`은 프로토콜 상대 주소 `//evil`이다).
fn normalize_url(v: &str) -> String {
    decode_entities(v).chars().filter(|c| !c.is_control() && !c.is_whitespace()).map(|c| if c == '\\' { '/' } else { c }).collect::<String>().to_lowercase()
}

fn scheme_of(n: &str) -> Option<&str> {
    let (s, _) = n.split_once(':')?;
    (!s.is_empty() && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))).then_some(s)
}

/// 안전한 `data:` 형식(이미지·글꼴)인가. SVG와 문서는 스크립트를 품을 수 있어 아니다.
fn safe_data_url(n: &str) -> bool {
    let mime = n.strip_prefix("data:").unwrap_or("").split([';', ',']).next().unwrap_or("");
    matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp") || mime.starts_with("font/") || mime.starts_with("application/font-") || mime.starts_with("application/x-font-")
}

/// URL을 남겨도 되는가. `resource`는 클릭 없이 자동으로 불러오는 자리다: 원격 주소도 남기지 않는다.
fn keep_url(value: &str, resource: bool) -> bool {
    if decode_entities(value).trim().starts_with("\\\\") {
        return false;
    }
    let n = normalize_url(value);
    if n.is_empty() {
        return true;
    }
    if n.starts_with("//") {
        return !resource;
    }
    match scheme_of(&n) {
        None => true,
        Some("http" | "https") => !resource,
        Some("data") => safe_data_url(&n),
        Some(s) if s.len() == 1 => true,
        Some(s) => SAFE_SCHEMES.contains(&s),
    }
}

fn is_resource_attr(tag: &str, attr: &str) -> bool {
    matches!(
        (tag, attr),
        ("img", "src" | "srcset")
            | ("link", "href")
            | ("video" | "audio" | "source" | "track" | "embed" | "input" | "frame" | "iframe", "src")
            | ("object", "data")
            | ("video", "poster")
            | ("image" | "use" | "feimage" | "script", "href")
            | (_, "background")
    )
}

mod markup_sanitizer;
mod package_rebuilder;

pub use markup_sanitizer::{strip_css, strip_markup};
pub(crate) use package_rebuilder::{Member, decode, ext_of, package_names, package_text, read_members, read_members_lenient, write_members};
pub use package_rebuilder::{Plan, rebuild};

// ---------------------------------------------------------------- 검증

/// 눈에 보이는 글자만 공백 없이 모은다. `text_of`는 HTML 규칙으로 읽어서 XHTML의 자기 닫힘 `<script src=".."/>`를 문서 끝까지 이어지는
/// 스크립트로 보고 본문을 버리므로, 자기 닫힘 `script`·`style`을 먼저 펼친다. 스크립트나 주석을 지우면 이어 붙는 텍스트 노드의
/// 경계 공백도 달라지므로 공백은 글자로 세지 않는다.
pub(crate) fn visible_text(markup: &str) -> String {
    let expanded = SELF_CLOSING_RAW.replace_all(markup, "<$1$2></$1>");
    text_of(&expanded).chars().filter(|c| !c.is_whitespace()).collect()
}

pub(crate) fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// OPF manifest 항목 중 대상이 없는 수와 spine에서 manifest에 없는 idref 수.
pub(crate) fn dangling(members: &[Member]) -> Result<(usize, usize), String> {
    let names: BTreeSet<&str> = members.iter().map(|m| m.name.as_str()).collect();
    let (mut missing, mut bad_spine) = (0, 0);
    let packages = package_names(members)?;
    for m in members.iter().filter(|m| packages.contains(&m.name)) {
        let text = package_text(m)?;
        let doc = parse_xml(&text).map_err(|e| format!("{}: OPF를 해석할 수 없다: {e}", m.name))?;
        let base = Path::new(&m.name).parent().and_then(Path::to_str).unwrap_or_default().to_owned();
        let mut ids = BTreeSet::new();
        for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "item") {
            ids.insert(item.attribute("id").unwrap_or("").to_owned());
            let href = item.attribute("href").unwrap_or("");
            if keep_url(href, true) && scheme_of(&normalize_url(href)).is_none() {
                let resolved = zip_entry_name(Path::new(&base).join(unquote(href).split('#').next().unwrap_or(""))).unwrap_or_default();
                if !names.contains(resolved.as_str()) {
                    missing += 1;
                }
            }
        }
        bad_spine += doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "itemref").filter(|n| !ids.contains(n.attribute("idref").unwrap_or(""))).count();
    }
    Ok((missing, bad_spine))
}

/// 재구성본이 원본의 읽을 수 있는 내용을 지키는지 본다: 열리는지, 본문 글자가 줄기만 했는지, manifest·spine이 더 깨지지 않았는지.
pub fn verify(orig: &[u8], new: &[u8]) -> Result<(), String> {
    let (old_members, new_members) = (read_members(orig)?, read_members(new)?);
    if let Some(first) = ZipArchive::new(Cursor::new(new)).map_err(|e| e.to_string())?.by_index(0).ok().map(|f| f.name().map(|n| n.into_owned()).unwrap_or_default())
        && old_members.iter().any(|m| m.name == "mimetype")
        && first != "mimetype"
    {
        return Err("mimetype이 첫 멤버가 아니다".into());
    }
    let old_by_name: BTreeMap<&str, &Member> = old_members.iter().map(|m| (m.name.as_str(), m)).collect();
    for m in new_members.iter().filter(|m| HTML_EXTS.contains(&ext_of(&m.name).as_str())) {
        let Some(old) = old_by_name.get(m.name.as_str()) else { return Err(format!("{}: 원본에 없는 본문 멤버", m.name)) };
        let (o, n) = (visible_text(&decode(&old.name, &old.data)?), visible_text(&decode(&m.name, &m.data)?));
        let lost = o.chars().count().saturating_sub(n.chars().count());
        if !is_subsequence(&n, &o) || lost > (o.chars().count() / 200).max(100) {
            return Err(format!("{}: 본문 텍스트가 달라졌다(잃은 글자 {lost})", m.name));
        }
    }
    let (old_dangling, new_dangling) = (dangling(&old_members)?, dangling(&new_members)?);
    if new_dangling.0 > old_dangling.0 || new_dangling.1 > old_dangling.1 {
        return Err(format!("manifest·spine 정합성이 나빠졌다: {old_dangling:?} → {new_dangling:?}"));
    }
    Ok(())
}

// ---------------------------------------------------------------- 실행
