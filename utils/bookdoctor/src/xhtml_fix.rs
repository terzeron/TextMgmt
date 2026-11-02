//! `--fix-xhtml`: 뷰어가 거부하는 본문 XHTML을 고친다.
//!
//! 대상은 `XHTML_PARSE_FAILED`나 `HEAD_MISSING`이 나온 EPUB이다. 본문 멤버마다 브라우저와 같은 규칙(html5ever)으로 읽어
//! 닫는 태그와 중첩을 바로잡고 XHTML로 다시 쓴다. 빈 멤버는 뼈대로 채우고, 읽을 수 없거나 마크업이 아닌 데이터인 멤버는
//! 지운다(manifest·spine·guide·목차·링크도 함께 정리). 많은 멤버가 깨진 파일은 고치지 않고 삭제 후보로만 알린다.
//!
//! 순서: 검사 → 결함이 있는 파일만 대상 → `rebuild` → `verify`(본문 글자·구조) → 재검사 → 적용.
//! 적용은 `--sanitize`와 같다: 원본을 휴지통 디렉터리에 두고 같은 inode에 덮어쓰고 mtime을 되돌린다.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::LazyLock;

use corpus_common::util::{posix_dirname, posix_join, posix_normpath, unquote};
use regex::Regex;
use scan_common::files::collect_files;
use scraper::{Html, Node};

use crate::cli::Options;
use crate::epub::render::xhtml_defect;
use crate::sanitize::{HTML_EXTS, Member, apply, create_private_temp, dangling, decode, ext_of, has_serious_security, is_subsequence, package_names, package_text, read_members, read_members_lenient, show, visible_text, write_members};
use crate::scan::scan_file;

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
const HTML_VOID: &[&str] = &["area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input", "keygen", "link", "meta", "param", "source", "track", "wbr"];
const SKELETON: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<head>\n  <title></title>\n</head>\n<body>\n  <p></p>\n</body>\n</html>\n";

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("잘못된 정규식 {p}: {e}"))
}

static STRAY_PAIR: LazyLock<Regex> = LazyLock::new(|| re(r"<([^A-Za-z/!?<>\n][^<>\n]{0,80}?)>"));
static START_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<[A-Za-z][^<>]*>"));
/// `</...>`처럼 영문자로 시작하지 않는 닫는 태그. HTML 파서는 이것을 주석으로 버리므로 글자가 아니다.
static STRAY_CLOSE: LazyLock<Regex> = LazyLock::new(|| re(r"</(?:[^A-Za-z<>/\n][^<>\n]{0,40})?>"));
/// 세미콜론 없는 `&nbsp` 같은 이름 있는 문자. HTML 파서는 글자로 풀지만 다른 도구는 글자 그대로 읽는다.
static LEGACY_ENTITY: LazyLock<Regex> = LazyLock::new(|| re(r"&(nbsp|amp|lt|gt|quot)([^;A-Za-z0-9]|$)"));
static STYLE_OPEN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)<style\b"));
static STYLE_CLOSE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)</style\s*>"));
static BODY_CLOSE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)</body\s*>"));
static NUL_REF: LazyLock<Regex> = LazyLock::new(|| re(r"&#(?:0+|[xX]0+);"));
static OPF_ITEM: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)[ \t]*<(?:\w+:)?item\b[^>]*?(?:/>|>\s*</(?:\w+:)?item>)[ \t]*\r?\n?"));
static OPF_ITEMREF: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)[ \t]*<(?:\w+:)?itemref\b[^>]*?(?:/>|>\s*</(?:\w+:)?itemref>)[ \t]*\r?\n?"));
static OPF_REFERENCE: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)[ \t]*<(?:\w+:)?reference\b[^>]*?(?:/>|>\s*</(?:\w+:)?reference>)[ \t]*\r?\n?"));
static ATTR_HREF: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
static ATTR_LINK: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)\s+href\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
static ATTR_ID: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)\bid\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
static ATTR_IDREF: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)\bidref\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
static NAV_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)</?(?:\w+:)?navPoint\b[^>]*>"));
static NAV_SRC: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)<(?:\w+:)?content\b[^>]*?\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)')"#));

// ---------------------------------------------------------------- 마크업 수리

/// 태그가 될 수 없는 `<`(뒤가 영문자·`/`·`!`·`?`가 아님)를 `&lt;`로 바꾼다. 같은 줄에서 짝이 되는 `>`는 `&gt;`로 바꾼다.
/// `<불주먹>`이나 `<...>` 같은 글이 브라우저에서 보이지 않거나 XML을 깨는 것을 막는다.
pub fn escape_stray_angle(s: &str) -> String {
    let paired = STRAY_PAIR.replace_all(s, "&lt;$1&gt;");
    let chars: Vec<char> = paired.chars().collect();
    let mut out = String::with_capacity(paired.len());
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        // `</`는 뒤가 영문자일 때만 닫는 태그다. `</브레먀`는 HTML 파서가 주석으로 읽어 다음 `>`까지 본문을 삼키므로 글자로 둔다.
        let tag_start = match next {
            Some('/') => chars.get(i + 2).is_some_and(char::is_ascii_alphabetic),
            Some(n) => n.is_ascii_alphabetic() || matches!(n, '!' | '?'),
            None => false,
        };
        if c == '<' && !tag_start {
            out.push_str("&lt;");
        } else {
            out.push(c);
        }
    }
    out
}

/// 속성 따옴표가 홀수로 열린 시작 태그는 `>` 앞에서 닫는다. 닫지 않으면 뒤따르는 본문이 속성값으로 삼켜진다.
fn close_open_quotes(s: &str) -> String {
    START_TAG
        .replace_all(s, |c: &regex::Captures| {
            let tag = &c[0];
            if tag.matches('"').count().is_multiple_of(2) {
                return tag.to_owned();
            }
            match tag.strip_suffix("/>") {
                Some(head) => format!("{head}\"/>"),
                None => format!("{}\">", &tag[..tag.len() - 1]),
            }
        })
        .into_owned()
}

/// 닫히지 않은 `<style>`은 첫 `</body>` 앞(없으면 끝)에서 닫는다. 닫지 않으면 본문이 문서 끝까지 스타일로 읽힌다.
fn close_style(s: &str) -> String {
    if STYLE_OPEN.find_iter(s).count() <= STYLE_CLOSE.find_iter(s).count() {
        return s.to_owned();
    }
    match BODY_CLOSE.find(s) {
        Some(m) => format!("{}</style>{}", &s[..m.start()], &s[m.start()..]),
        None => format!("{s}</style>"),
    }
}

/// 파서에 넣기 전의 정리. 검증도 같은 정리를 거친 원본과 비교한다.
fn prepare(src: &str) -> String {
    let s = NUL_REF.replace_all(src.trim_start_matches('\u{feff}'), "");
    let s = LEGACY_ENTITY.replace_all(&s, "&$1;$2");
    let s = STRAY_CLOSE.replace_all(&s, "");
    close_style(&close_open_quotes(&escape_stray_angle(&s)))
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// 속성 이름. `xml:`·`xlink:`·`epub:` 접두사만 허용하고, `xmlns`는 직접 관리하므로 버린다.
fn valid_attr(name: &str) -> bool {
    match name.split_once(':') {
        Some((prefix, rest)) => matches!(prefix, "xml" | "xlink" | "epub") && valid_name(rest),
        None => valid_name(name) && name != "xmlns",
    }
}

fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}')
}

fn escape_into(out: &mut String, text: &str, quote: bool) {
    for c in text.chars().filter(|c| xml_char(*c)) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if quote => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// 요소의 출력 이름과 자기 닫힘 여부. XML 이름이 될 수 없는 태그 이름은 `span`으로 바꾼다.
fn element_shape<'a>(node: &scraper::ElementRef<'a>) -> (&'a str, bool, bool) {
    let el = node.value();
    let local = &*el.name.local;
    let name = if valid_name(local) { local } else { "span" };
    let is_html = &*el.name.ns == XHTML_NS;
    let empty = node.first_child().is_none();
    let self_closed = if is_html { HTML_VOID.contains(&name) } else { empty };
    (name, self_closed, name != local)
}

/// html5ever 트리를 XHTML로 쓴다. 글자 노드는 이스케이프하고, 주석·처리 명령은 버리고, 빈 비-void 요소는 `<p></p>`로 쓴다.
fn serialize(html: &Html) -> String {
    let mut out = String::new();
    let (mut epub_ns, mut xlink_ns) = (false, false);
    let root = html.tree.root();
    let mut node = root;
    loop {
        match node.value() {
            Node::Text(t) => escape_into(&mut out, t, false),
            Node::Element(el) => {
                let element = scraper::ElementRef::wrap(node).expect("요소 노드");
                let (name, self_closed, renamed) = element_shape(&element);
                out.push('<');
                out.push_str(name);
                let ns = &*el.name.ns;
                let parent_ns = node.parent().and_then(|p| if let Node::Element(pe) = p.value() { Some(&*pe.name.ns) } else { None }).unwrap_or("");
                if ns != parent_ns && !ns.is_empty() {
                    out.push_str(&format!(" xmlns=\"{ns}\""));
                }
                if !renamed {
                    for (attr, value) in &el.attrs {
                        let local = &*attr.local;
                        let full = match &attr.prefix {
                            Some(p) if !p.is_empty() => format!("{}:{local}", &**p),
                            _ => local.to_owned(),
                        };
                        if !valid_attr(&full) {
                            continue;
                        }
                        epub_ns |= full.starts_with("epub:");
                        xlink_ns |= full.starts_with("xlink:");
                        out.push(' ');
                        out.push_str(&full);
                        out.push_str("=\"");
                        escape_into(&mut out, value, true);
                        out.push('"');
                    }
                }
                out.push_str(if self_closed { " />" } else { ">" });
            }
            _ => {}
        }
        if let Some(child) = node.first_child() {
            node = child;
            continue;
        }
        loop {
            if let Node::Element(_) = node.value() {
                let element = scraper::ElementRef::wrap(node).expect("요소 노드");
                let (name, self_closed, _) = element_shape(&element);
                if !self_closed {
                    out.push_str(&format!("</{name}>"));
                }
            }
            if node.id() == root.id() {
                return finish(out, epub_ns, xlink_ns);
            }
            if let Some(next) = node.next_sibling() {
                node = next;
                break;
            }
            match node.parent() {
                Some(parent) => node = parent,
                None => return finish(out, epub_ns, xlink_ns),
            }
        }
    }
}

fn finish(body: String, epub_ns: bool, xlink_ns: bool) -> String {
    let mut declared = format!("<html xmlns=\"{XHTML_NS}\"");
    if epub_ns {
        declared.push_str(" xmlns:epub=\"http://www.idpf.org/2007/ops\"");
    }
    if xlink_ns {
        declared.push_str(" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
    }
    let body = body.replacen(&format!("<html xmlns=\"{XHTML_NS}\""), &declared, 1);
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n{body}")
}

/// 마크업 텍스트를 XHTML로 고쳐 쓴다. 닫는 태그와 중첩은 브라우저의 HTML 파싱 규칙으로 바로잡고, `head`·`body`가 없으면 만든다.
pub fn repair_markup(src: &str) -> String {
    serialize(&Html::parse_document(&prepare(src)))
}

// ---------------------------------------------------------------- 참조 정리

fn resolve(base: &str, href: &str) -> String {
    posix_normpath(&posix_join(base, unquote(href).split('#').next().unwrap_or("")))
}

fn attr_value<'a>(caps: &regex::Captures<'a>) -> &'a str {
    caps.get(1).or_else(|| caps.get(2)).map_or("", |m| m.as_str())
}

/// OPF에서 지운 멤버의 manifest 항목, 그 id를 가리키는 spine 항목, 지운 멤버를 가리키는 guide 참조를 지운다.
fn drop_from_opf(text: &str, opf_name: &str, dropped: &BTreeSet<String>) -> String {
    let base = posix_dirname(opf_name);
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let pruned = OPF_ITEM.replace_all(text, |c: &regex::Captures| {
        let item = &c[0];
        let Some(h) = ATTR_HREF.captures(item) else { return item.to_owned() };
        if !dropped.contains(&resolve(&base, attr_value(&h))) {
            return item.to_owned();
        }
        if let Some(id) = ATTR_ID.captures(item) {
            ids.insert(attr_value(&id).to_owned());
        }
        String::new()
    });
    let pruned = OPF_ITEMREF.replace_all(&pruned, |c: &regex::Captures| match ATTR_IDREF.captures(&c[0]) {
        Some(r) if ids.contains(attr_value(&r)) => String::new(),
        _ => c[0].to_owned(),
    });
    OPF_REFERENCE
        .replace_all(&pruned, |c: &regex::Captures| match ATTR_HREF.captures(&c[0]) {
            Some(h) if dropped.contains(&resolve(&base, attr_value(&h))) => String::new(),
            _ => c[0].to_owned(),
        })
        .into_owned()
}

/// NCX에서 자식이 없고 지운 멤버를 가리키는 `navPoint`를 지운다.
fn drop_from_ncx(text: &str, ncx_name: &str, dropped: &BTreeSet<String>) -> String {
    let base = posix_dirname(ncx_name);
    let mut stack: Vec<(usize, bool)> = Vec::new();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    for m in NAV_TAG.find_iter(text) {
        let tag = m.as_str();
        if tag.starts_with("</") {
            let Some((start, has_child)) = stack.pop() else { continue };
            let points_at_dropped = NAV_SRC.captures(&text[start..m.end()]).is_some_and(|c| dropped.contains(&resolve(&base, attr_value(&c))));
            if !has_child && points_at_dropped {
                let from = text[..start].trim_end_matches([' ', '\t']).len();
                let rest = &text[m.end()..];
                let tail = if rest.starts_with("\r\n") { 2 } else { usize::from(rest.starts_with('\n')) };
                cuts.push((from, m.end() + tail));
            }
        } else if !tag.ends_with("/>") {
            if let Some(top) = stack.last_mut() {
                top.1 = true;
            }
            stack.push((m.start(), false));
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (from, to) in cuts {
        out.push_str(&text[last..from]);
        last = to;
    }
    out.push_str(&text[last..]);
    out
}

/// 본문 링크 중 지운 멤버를 가리키는 `href`를 지운다(글자는 남긴다).
fn drop_links(text: &str, member: &str, dropped: &BTreeSet<String>) -> String {
    let base = posix_dirname(member);
    ATTR_LINK
        .replace_all(text, |c: &regex::Captures| if dropped.contains(&resolve(&base, attr_value(c))) { String::new() } else { c[0].to_owned() })
        .into_owned()
}

fn is_utf16(data: &[u8]) -> bool {
    data.starts_with(b"\xFF\xFE") || data.starts_with(b"\xFE\xFF")
}

fn drop_references(members: &mut [Member], dropped: &BTreeSet<String>, edited: &mut Vec<(String, String)>) -> Result<(), String> {
    let packages = package_names(members)?;
    for m in members.iter_mut() {
        let ext = ext_of(&m.name);
        let is_opf = packages.contains(&m.name);
        let is_ncx = ext == "ncx";
        let is_html = HTML_EXTS.contains(&ext.as_str());
        if !(is_opf || is_ncx || is_html) {
            continue;
        }
        if is_utf16(&m.data) && (is_opf || is_ncx) {
            return Err(format!("{}: UTF-16이라 참조를 안전하게 정리할 수 없다", m.name));
        }
        let Ok(text) = (if is_opf { package_text(m) } else { decode(&m.name, &m.data) }) else { continue };
        let (new, what) = if is_opf {
            (drop_from_opf(&text, &m.name, dropped), "지운 멤버의 manifest·spine·guide 항목 제거")
        } else if is_ncx {
            (drop_from_ncx(&text, &m.name, dropped), "지운 멤버의 목차 항목 제거")
        } else {
            (drop_links(&text, &m.name, dropped), "지운 멤버로 가는 링크 href 제거")
        };
        if new != text {
            m.data = new.into_bytes();
            edited.push((m.name.clone(), what.to_owned()));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 재구성

/// 고친 결과.
pub struct Plan {
    pub bytes: Vec<u8>,
    /// 내용을 고친 멤버와 고친 종류.
    pub edited: Vec<(String, String)>,
    /// 지운 멤버와 이유.
    pub dropped: Vec<(String, String)>,
}

pub enum Verdict {
    Fix(Plan),
    /// 깨진 멤버가 많아 고치지 않는다. 파일을 지울지는 사람이 정한다.
    Delete(String),
}

/// 마크업으로 시작하는가: 앞 공백과 BOM을 건너뛴 첫 글자가 `<`이다(UTF-16이면 사이에 NUL이 낀다).
/// 코드페이지(EUC-KR 등)로 저장한 진짜 마크업은 통과시키고, 난수처럼 보이는 데이터는 거른다.
fn looks_like_markup_bytes(data: &[u8]) -> bool {
    let head: Vec<u8> = data.iter().take(64).copied().filter(|b| *b != 0).collect();
    let head = head.strip_prefix(b"\xEF\xBB\xBF").or_else(|| head.strip_prefix(b"\xFF\xFE")).or_else(|| head.strip_prefix(b"\xFE\xFF")).unwrap_or(&head);
    head.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<')
}

/// 깨진 멤버가 이만큼 많으면 삭제 후보다: 둘 이상이면서 마크업 멤버의 20%를 넘거나, 다섯 개를 넘는다.
fn too_broken(dropped: usize, markup_total: usize) -> bool {
    (dropped >= 2 && dropped * 5 > markup_total) || dropped > 5
}

/// EPUB 바이트의 본문 XHTML 결함을 고친다.
pub fn rebuild(src: &[u8]) -> Result<Verdict, String> {
    let (mut members, broken) = read_members_lenient(src)?;
    if let Some(m) = members.iter().find(|m| m.name.starts_with('\0')) {
        return Err(format!("{}: 이름이 UTF-8이 아니라 다시 쓸 수 없다", m.name.trim_start_matches('\0')));
    }
    let mut dropped: Vec<(String, String)> = Vec::new();
    for (name, message) in broken {
        if !HTML_EXTS.contains(&ext_of(&name).as_str()) {
            return Err(format!("{name}: 마크업이 아닌 멤버를 읽을 수 없다: {message}"));
        }
        dropped.push((name, "읽을 수 없음".to_owned()));
    }
    let mut edited: Vec<(String, String)> = Vec::new();
    let mut markup_total = dropped.len();
    for m in &mut members {
        if !HTML_EXTS.contains(&ext_of(&m.name).as_str()) {
            continue;
        }
        markup_total += 1;
        let Some(defect) = xhtml_defect(&m.data) else { continue };
        match decode(&m.name, &m.data) {
            Ok(text) if text.trim().is_empty() => {
                m.data = SKELETON.as_bytes().to_vec();
                edited.push((m.name.clone(), "빈 멤버를 뼈대로 채움".to_owned()));
            }
            Ok(text) => {
                let fixed = repair_markup(&text);
                if let Some(left) = xhtml_defect(fixed.as_bytes()) {
                    return Err(format!("{}: 수리한 뒤에도 {left}", m.name));
                }
                m.data = fixed.into_bytes();
                edited.push((m.name.clone(), format!("{defect} 수리")));
            }
            Err(e) if looks_like_markup_bytes(&m.data) => return Err(e),
            Err(_) => dropped.push((m.name.clone(), "마크업이 아닌 데이터".to_owned())),
        }
    }
    if too_broken(dropped.len(), markup_total) {
        return Ok(Verdict::Delete(format!("본문 멤버 {markup_total}개 중 {}개가 깨져 있다", dropped.len())));
    }
    let dropped_names: BTreeSet<String> = dropped.iter().map(|(n, _)| n.clone()).collect();
    members.retain(|m| !dropped_names.contains(&m.name));
    if !dropped_names.is_empty() {
        drop_references(&mut members, &dropped_names, &mut edited)?;
    }
    let bytes = if edited.is_empty() && dropped.is_empty() { src.to_vec() } else { write_members(&mut members)? };
    Ok(Verdict::Fix(Plan { bytes, edited, dropped }))
}

// ---------------------------------------------------------------- 검증

/// 비교에서 제외하는 글자: 공백(`visible_text`가 이미 뺀다), 제어문자, 대체 문자.
fn comparable(text: &str) -> Vec<char> {
    text.chars().filter(|c| !c.is_control() && *c != '\u{fffd}').collect()
}

/// 원본 글자 중 사라진 것: 순서대로 남았거나, 파서가 순서를 바꿨어도 글자별 개수가 줄지 않았으면 비어 있다. 줄었으면 줄어든 글자 몇 개를 돌려준다.
fn lost_chars(old: &str, new: &str) -> String {
    let (o, n) = (comparable(old).into_iter().collect::<String>(), comparable(new).into_iter().collect::<String>());
    if is_subsequence(&o, &n) {
        return String::new();
    }
    let count = |s: &str| {
        let mut m = std::collections::BTreeMap::new();
        for c in s.chars() {
            *m.entry(c).or_insert(0usize) += 1;
        }
        m
    };
    let have = count(&n);
    count(&o).into_iter().filter(|(c, k)| have.get(c).copied().unwrap_or(0) < *k).map(|(c, _)| c).take(8).collect()
}

/// 고친 결과가 원본의 읽을 수 있는 내용을 지키는지 본다: 본문 결함이 없는지, 글자를 잃지 않았는지, manifest·spine이 더 깨지지 않았는지.
pub fn verify(orig: &[u8], new: &[u8]) -> Result<(), String> {
    let (old_members, _) = read_members_lenient(orig)?;
    let new_members = read_members(new)?;
    if let Some(first) = zip::ZipArchive::new(io::Cursor::new(new)).map_err(|e| e.to_string())?.by_index(0).ok().map(|f| f.name().map(|n| n.into_owned()).unwrap_or_default())
        && old_members.iter().any(|m| m.name == "mimetype")
        && first != "mimetype"
    {
        return Err("mimetype이 첫 멤버가 아니다".into());
    }
    let old_by_name: std::collections::BTreeMap<&str, &Member> = old_members.iter().map(|m| (m.name.as_str(), m)).collect();
    for m in new_members.iter().filter(|m| HTML_EXTS.contains(&ext_of(&m.name).as_str())) {
        if let Some(left) = xhtml_defect(&m.data) {
            return Err(format!("{}: 수리한 뒤에도 {left}", m.name));
        }
        let Some(old) = old_by_name.get(m.name.as_str()) else { return Err(format!("{}: 원본에 없는 본문 멤버", m.name)) };
        if old.data == m.data {
            continue;
        }
        let (o, n) = (visible_text(&prepare(&decode(&old.name, &old.data)?)), visible_text(&decode(&m.name, &m.data)?));
        let lost = lost_chars(&o, &n);
        if !lost.is_empty() {
            return Err(format!("{}: 본문 텍스트가 달라졌다(줄어든 글자: {lost})", m.name));
        }
    }
    let (old_dangling, new_dangling) = (dangling(&old_members)?, dangling(&new_members)?);
    if new_dangling.0 > old_dangling.0 || new_dangling.1 > old_dangling.1 {
        return Err(format!("manifest·spine 정합성이 나빠졌다: {old_dangling:?} → {new_dangling:?}"));
    }
    Ok(())
}

// ---------------------------------------------------------------- 실행

fn has_defect(report: &scan_common::FileReport) -> bool {
    report.findings.iter().any(|f| matches!(f.code.to_string().as_str(), "XHTML_PARSE_FAILED" | "HEAD_MISSING"))
}

/// 고친 결과를 임시 파일에 써서 다시 검사한다. 본문 결함이 남거나 보안 지표(HIGH 이상)가 있으면 오류다.
/// 수리는 마크업을 읽는 방식을 바꾼다. 닫히지 않은 따옴표에 가려져 있던 `<script>` 같은 것이 살아날 수 있으므로 결과를 다시 검사한다.
fn rescan(bytes: &[u8], opts: &Options) -> Result<(), String> {
    let tmp = create_private_temp(bytes).map_err(|e| e.to_string())?;
    let report = scan_file(&tmp, opts);
    let _ = fs::remove_file(&tmp);
    if has_serious_security(&report) {
        return Err("수리한 결과에 보안 지표(HIGH 이상)가 있다. 먼저 --sanitize로 정리한다".to_owned());
    }
    if has_defect(&report) { Err("재검사에서 본문 결함이 남았다".to_owned()) } else { Ok(()) }
}

/// `--fix-xhtml` 진입점. 종료 코드를 돌려준다(건너뜀·삭제 후보가 있어도 0, 쓰기 실패가 있으면 1).
pub fn run(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let paths = collect_files(&opts.paths, opts.recursive, |p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("epub")));
    let scan_opts = Options { only: Vec::new(), ..opts.clone() };
    let (mut done, mut untouched, mut candidates, mut skipped, mut failed) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for path in &paths {
        let shown = show(&path.to_string_lossy());
        let report = scan_file(path, &scan_opts);
        if !has_defect(&report) {
            untouched += 1;
            continue;
        }
        if has_serious_security(&report) {
            writeln!(out, "SKIP {shown}: 보안 지표(HIGH 이상)가 있다. 먼저 --sanitize로 정리한다")?;
            skipped += 1;
            continue;
        }
        let planned = fs::read(path).map_err(|e| e.to_string()).and_then(|src| rebuild(&src).map(|verdict| (src, verdict)));
        let (src, plan) = match planned {
            Ok((_, Verdict::Delete(why))) => {
                writeln!(out, "DELETE_CANDIDATE {shown}: {}", show(&why))?;
                candidates += 1;
                continue;
            }
            Ok((src, Verdict::Fix(plan))) => (src, plan),
            Err(e) => {
                writeln!(out, "SKIP {shown}: {}", show(&e))?;
                skipped += 1;
                continue;
            }
        };
        if plan.edited.is_empty() && plan.dropped.is_empty() {
            writeln!(out, "UNCHANGED {shown}: 고칠 멤버를 찾지 못했다")?;
            untouched += 1;
            continue;
        }
        if let Err(e) = verify(&src, &plan.bytes).and_then(|()| rescan(&plan.bytes, &scan_opts)) {
            writeln!(out, "SKIP {shown}: 검증 실패: {}", show(&e))?;
            skipped += 1;
            continue;
        }
        let head = if opts.dry_run { "WOULD" } else { "FIXED" };
        writeln!(out, "{head} {shown}: 멤버 {}개 수정, {}개 제거", plan.edited.len(), plan.dropped.len())?;
        for (name, what) in &plan.edited {
            writeln!(out, "  - 수정 {} ({})", show(name), show(what))?;
        }
        for (name, why) in &plan.dropped {
            writeln!(out, "  - 제거 {} ({})", show(name), show(why))?;
        }
        if !opts.dry_run {
            let trash = opts.trash_dir.as_deref().unwrap_or_else(|| Path::new("0_trash"));
            match apply(path, &plan.bytes, trash) {
                Ok(saved) => writeln!(out, "  - 원본 보관 {}", show(&saved.to_string_lossy()))?,
                Err(e) => {
                    writeln!(out, "FAIL {shown}: {}", show(&e.to_string()))?;
                    failed += 1;
                    continue;
                }
            }
        }
        done += 1;
    }
    writeln!(out, "검사 {}건, {} {done}건, 대상 아님·변경 없음 {untouched}건, 삭제 후보 {candidates}건, 건너뜀 {skipped}건, 실패 {failed}건", paths.len(), if opts.dry_run { "수리 예정" } else { "수리" })?;
    Ok(i32::from(failed > 0))
}
