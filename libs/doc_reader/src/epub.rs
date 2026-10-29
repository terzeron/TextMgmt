//! EPUB 읽기. 원본: `Loader.read_from_epub`(ebooklib)와 `read_from_epub_with_extracting_zip`(정규식 대체 경로).
//!
//! ebooklib 경로가 예외를 내면(필수 요소 누락, 매니페스트 항목이 ZIP에 없음, NCX·nav 해석 실패 등)
//! 정규식으로 챕터만 읽는 대체 경로로 간다. 두 경로는 결과가 달라서 어느 쪽으로 갔는지가 중요하다.
//!
//! 본문은 ebooklib처럼 `<body>`를 `etree.tostring(pretty_print=True)`로 직렬화했다가 다시 파싱한 글이다.
//! libxml2의 들여쓰기 출력은 자식이 요소뿐인 요소에서 줄바꿈을 더한다. 그 줄바꿈을 [`pretty_text`]가 흉내 낸다.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use corpus_common::reader::{Zip, open_zip};
use corpus_common::util::{posix_dirname, posix_join, posix_normpath, unquote};
use ego_tree::NodeRef;
use regex::Regex;
use scraper::{Html, Node};

use crate::TEXT_SIZE;
use crate::markup::{count_lines, get_body_text, is_blank_char_text};
use crate::pytext::{head_chars, keep_word_chars};

const CONTAINER_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
const OPF_NS: &str = "http://www.idpf.org/2007/opf";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
const DAISY_NS: &str = "http://www.daisy.org/z3986/2005/ncx/";
const OEBPS_MEDIA_TYPE: &str = "application/oebps-package+xml";
const XHTML_MEDIA_TYPE: &str = "application/xhtml+xml";

/// 어떤 경로로 읽혔나. 통계(`normal_epub_count` / `zipped_epub_count`)에 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpubPath {
    /// ebooklib 경로
    Normal,
    /// 정규식 대체 경로
    Zipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpubRead {
    pub summary: String,
    pub line_count: usize,
    pub path: EpubPath,
    /// 원본이 `LOGGER.error`로 남기는 메시지들
    pub errors: Vec<String>,
}

/// EPUB reader 주입 경계. `read_epub` 오류는 정규식 ZIP fallback을 실행한다.
pub type EpubReader<'a> = dyn Fn(&Path) -> Result<(String, usize), String> + 'a;

#[derive(Clone, Copy, Default)]
pub struct EpubHooks<'a> {
    pub read_epub: Option<&'a EpubReader<'a>>,
}

/// XML 바이트를 문자열로. BOM과 선언된 인코딩을 따른다. 읽을 수 없으면 `None`.
fn xml_text(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(b"\xef\xbb\xbf") {
        return String::from_utf8(rest.to_vec()).ok();
    }
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let (enc, _) = encoding_rs::Encoding::for_bom(bytes)?;
        return enc.decode_without_bom_handling_and_without_replacement(&bytes[2..]).map(|c| c.into_owned());
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some(s.to_owned());
    }
    static DECL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^<\?xml[^>]*encoding=["']([A-Za-z0-9._-]+)["']"#).unwrap());
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).into_owned();
    let label = DECL.captures(&head)?.get(1)?.as_str().to_owned();
    encoding_rs::Encoding::for_label(label.as_bytes())?.decode_without_bom_handling_and_without_replacement(bytes).map(|c| c.into_owned())
}

fn parse_xml(bytes: &[u8]) -> Result<(String, ()), String> {
    let text = xml_text(bytes).ok_or("XML 인코딩을 읽을 수 없다")?;
    Ok((text, ()))
}

fn xml_doc(text: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).map_err(|e| format!("XML 해석 실패: {e}"))
}

fn read_member(zip: &mut Zip, name: &str) -> Result<Vec<u8>, String> {
    let file = zip.by_name(name).map_err(|_| format!("KeyError: {name}"))?;
    // 헤더의 선언 크기는 믿지 않는다(zip bomb). 미리 할당하지 않고 상한까지만 푼다.
    let mut buf = Vec::new();
    file.take(corpus_common::reader::MAX_MEMBER_BYTES).read_to_end(&mut buf).map_err(|e| format!("{name}: {e}"))?;
    Ok(buf)
}

/// 매니페스트 항목.
struct Item {
    id: Option<String>,
    /// 압축을 푼 href(`unquote`)
    file_name: String,
    /// ZIP 안 멤버 이름
    member: String,
    is_document: bool,
    is_nav: bool,
}

fn child<'a, 'b>(node: roxmltree::Node<'a, 'b>, ns: &str, name: &str) -> Option<roxmltree::Node<'a, 'b>> {
    node.children().find(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(ns))
}

/// ebooklib 경로. 예외가 날 상황이면 `Err`.
fn read_with_ebooklib(path: &Path) -> Result<(String, usize), String> {
    let mut zip = open_zip(path).ok_or("Bad Zip file")?;
    let names: HashSet<String> = zip.file_names().filter_map(|n| n.ok()).map(|n| n.into_owned()).collect();

    // 컨테이너
    let container_bytes = read_member(&mut zip, "META-INF/container.xml")?;
    let (container_text, ()) = parse_xml(&container_bytes)?;
    let container = xml_doc(&container_text)?;
    let mut opf_file = String::new();
    for rootfile in container.descendants().filter(|n| n.is_element() && n.tag_name().name() == "rootfile" && n.tag_name().namespace() == Some(CONTAINER_NS)) {
        if rootfile.attribute("media-type") == Some(OEBPS_MEDIA_TYPE) {
            opf_file = rootfile.attribute("full-path").ok_or("TypeError: full-path 없음")?.to_owned();
        }
    }
    let opf_dir = posix_dirname(&opf_file);

    // OPF
    let opf_bytes = read_member(&mut zip, &posix_normpath(&opf_file)).map_err(|_| "Can not find container file".to_owned())?;
    let (opf_text, ()) = parse_xml(&opf_bytes)?;
    let opf = xml_doc(&opf_text)?;
    let root = opf.root_element();

    // 메타데이터: 요소마다 네임스페이스가 있어야 한다(없으면 KeyError).
    let metadata = child(root, OPF_NS, "metadata").ok_or("AttributeError: metadata 없음")?;
    let mut titles: Vec<Option<String>> = Vec::new();
    let mut creators: Vec<Option<String>> = Vec::new();
    for t in metadata.children().filter(|c| c.is_element()) {
        let default_ns = metadata.lookup_namespace_uri(None).unwrap_or("");
        let is_meta = t.tag_name().name() == "meta" && t.tag_name().namespace().unwrap_or("") == default_ns;
        if is_meta {
            continue;
        }
        let ns = t.tag_name().namespace().ok_or("KeyError: 네임스페이스 없는 메타데이터 요소")?;
        if ns == DC_NS {
            let text = t.first_child().filter(|c| c.is_text()).and_then(|c| c.text()).map(str::to_owned);
            match t.tag_name().name() {
                "title" => titles.push(text),
                "creator" => creators.push(text),
                _ => {}
            }
        }
    }

    // 매니페스트: 모든 항목의 멤버가 ZIP에 있어야 한다.
    let manifest = child(root, OPF_NS, "manifest").ok_or("TypeError: manifest 없음")?;
    let mut items: Vec<Item> = Vec::new();
    for r in manifest.children().filter(|c| c.is_element() && c.tag_name().name() == "item" && c.tag_name().namespace() == Some(OPF_NS)) {
        let media_type = r.attribute("media-type");
        let href = r.attribute("href").ok_or("TypeError: href 없음")?;
        let file_name = unquote(href);
        let properties: Vec<&str> = r.attribute("properties").map(|p| p.split(' ').collect()).unwrap_or_default();
        let is_xhtml = media_type == Some(XHTML_MEDIA_TYPE);
        let is_nav = is_xhtml && properties.contains(&"nav");
        // nav 항목만 href를 풀지 않은 채로 멤버를 찾는다(ebooklib의 동작).
        let member_name = if is_nav { posix_normpath(&posix_join(&opf_dir, href)) } else { posix_normpath(&posix_join(&opf_dir, &file_name)) };
        if !names.contains(&member_name) {
            return Err(format!("KeyError: {member_name}"));
        }
        items.push(Item { id: r.attribute("id").map(str::to_owned), file_name, member: member_name, is_document: is_xhtml, is_nav });
    }

    // 스파인과 목차
    let spine = child(root, OPF_NS, "spine").ok_or("TypeError: spine 없음")?;
    let nav_item = items.iter().find(|i| i.is_nav);
    if let Some(toc) = spine.attribute("toc").filter(|t| !t.is_empty()) {
        let item = items.iter().find(|i| i.id.as_deref() == Some(toc)).ok_or("AttributeError: ncx 항목 없음")?;
        let ncx_name = posix_normpath(&posix_join(&opf_dir, &item.file_name));
        let ncx_bytes = read_member(&mut zip, &ncx_name).map_err(|_| "Can not find ncx file.".to_owned())?;
        validate_ncx(&ncx_bytes)?;
    }
    if let Some(nav) = nav_item {
        let nav_bytes = read_member(&mut zip, &nav.member)?;
        let toc_parsed = spine.attribute("toc").is_some_and(|t| !t.is_empty());
        let nav_text = String::from_utf8_lossy(&nav_bytes).into_owned();
        let base = posix_dirname(&nav.file_name);
        if !toc_parsed {
            validate_nav(&nav_text, &base, "toc")?;
        }
        validate_nav(&nav_text, &base, "page-list")?;
    }

    // 본문
    let mut result = String::new();
    for title in titles.iter().flatten().filter(|t| !t.is_empty()) {
        result.push(' ');
        result.push_str(title);
    }
    for creator in creators.iter().flatten().filter(|t| !t.is_empty()) {
        result.push(' ');
        result.push_str(creator);
    }
    let mut line_count = 0;
    for item in items.iter().filter(|i| i.is_document) {
        let content = read_member(&mut zip, &item.member)?;
        let (text, lines) = document_text_and_lines(&content)?;
        if result.chars().count() < TEXT_SIZE {
            result.push_str(&text);
        }
        line_count += lines;
    }
    Ok((result, line_count))
}

/// NCX를 읽다 예외가 날 상황인지 본다(`_parse_ncx`).
fn validate_ncx(bytes: &[u8]) -> Result<(), String> {
    let (text, ()) = parse_xml(bytes)?;
    let doc = xml_doc(&text)?;
    let nav_map = child(doc.root_element(), DAISY_NS, "navMap").ok_or("AttributeError: navMap 없음")?;
    fn walk(node: roxmltree::Node) -> Result<(), String> {
        for a in node.children().filter(|c| c.is_element()) {
            if a.tag_name().namespace() != Some(DAISY_NS) {
                continue;
            }
            match a.tag_name().name() {
                "navLabel" => {
                    // `a.getchildren()[0].text`: 자식 요소가 하나도 없으면 IndexError
                    a.children().find(|c| c.is_element()).ok_or("IndexError: navLabel이 비었다")?;
                }
                "navPoint" => walk(a)?,
                _ => {}
            }
        }
        Ok(())
    }
    walk(nav_map)
}

fn element_children<'a>(node: NodeRef<'a, Node>, name: &str) -> Vec<NodeRef<'a, Node>> {
    node.children().filter(|c| matches!(c.value(), Node::Element(e) if e.name() == name)).collect()
}

/// nav 문서를 읽다 예외가 날 상황인지 본다(`_parse_nav`). `kind`는 `toc` 또는 `page-list`.
fn validate_nav(markup: &str, _base: &str, kind: &str) -> Result<(), String> {
    if markup.trim().is_empty() {
        return Err("ParserError: Document is empty".into());
    }
    let doc = crate::markup::parse(markup);
    let nav = doc.tree.root().descendants().find(|n| matches!(n.value(), Node::Element(e) if e.name() == "nav" && e.attrs().any(|(_, v)| v == kind)));
    let Some(nav) = nav else {
        return if kind == "toc" { Err("IndexError: nav toc 없음".into()) } else { Ok(()) };
    };
    let ol = element_children(nav, "ol").into_iter().next().ok_or("AttributeError: nav에 ol 없음")?;
    fn parse_list(list: NodeRef<Node>) -> Result<(), String> {
        for li in element_children(list, "li") {
            let sublist = element_children(li, "ol").into_iter().next();
            let link = element_children(li, "a").into_iter().next();
            let href_ok = |link: NodeRef<Node>| matches!(link.value(), Node::Element(e) if e.attr("href").is_some());
            if let Some(sub) = sublist {
                parse_list(sub)?;
                if let Some(link) = link
                    && !href_ok(link)
                {
                    return Err("TypeError: href 없음".into());
                }
            } else if let Some(link) = link
                && !href_ok(link)
            {
                return Err("TypeError: href 없음".into());
            }
        }
        Ok(())
    }
    parse_list(ol)
}

fn decode_markup(content: &[u8]) -> String {
    let content = content.strip_prefix(b"\xef\xbb\xbf").unwrap_or(content);
    match std::str::from_utf8(content) {
        Ok(s) => s.to_owned(),
        Err(_) => crate::text::decode_markup_bytes(content).unwrap_or_else(|| String::from_utf8_lossy(content).into_owned()),
    }
}

fn child_count(body: NodeRef<Node>) -> usize {
    body.children().filter(|c| matches!(c.value(), Node::Element(_) | Node::Comment(_) | Node::ProcessingInstruction(_))).count()
}

fn is_hidden(node: &NodeRef<Node>) -> bool {
    node.ancestors().any(|a| matches!(a.value(), Node::Element(e) if matches!(e.name(), "script" | "style" | "template")))
}

fn in_pre(node: &NodeRef<Node>) -> bool {
    node.ancestors().any(|a| matches!(a.value(), Node::Element(e) if e.name() == "pre"))
}

/// libxml2 들여쓰기 출력을 다시 파싱한 글. 자식이 요소·주석뿐인 요소는 여는 태그 뒤, 자식 사이, 마지막 자식 뒤에
/// 줄바꿈 하나씩을 얻고, 텍스트 자식이 하나라도 있으면 그 안쪽은 들여쓰기하지 않는다.
///
/// `skip_leading`이 참이면(속성 없는 `<body>`를 떼어 낸 경우) 직렬화한 글의 맨 앞 공백 한 덩어리를 버린다.
/// 다시 파싱할 때 문서 맨 앞 공백은 건너뛰기 때문이다. 태그 뒤의 공백은 해당하지 않는다.
fn emit_pretty(node: NodeRef<Node>, format: bool, skip_leading: bool, tail: &str, out: &mut String) {
    let last_id = node.last_child().map(|c| c.id());
    // body의 마지막 텍스트에서 `</body>` 뒤로 옮겨진 공백을 떼고 나면 비는 노드는 없는 셈이다.
    let children: Vec<NodeRef<Node>> = node.children().filter(|c| !(Some(c.id()) == last_id && matches!(c.value(), Node::Text(t) if crate::markup::trim_relocated(t, tail).is_none()))).collect();
    let has_text = children.iter().any(|c| matches!(c.value(), Node::Text(_)));
    let format = format && !has_text;
    if children.is_empty() {
        return;
    }
    // 문서 앞부분(첫 요소 전)의 공백·주석 사이 공백은 다시 파싱할 때 건너뛰므로, 첫 요소나 글이 나올 때까지 공백을 버린다.
    let mut skipping = skip_leading;
    if format && !skipping {
        out.push('\n');
    }
    for c in children {
        match c.value() {
            Node::Text(text) => {
                let text: &str = if Some(c.id()) == last_id { crate::markup::trim_relocated(text, tail).unwrap_or("") } else { text };
                if !is_hidden(&c) {
                    let text = if skipping { text.trim_start_matches([' ', '\t', '\n', '\r']) } else { text };
                    if !text.is_empty() {
                        skipping = false;
                        if is_blank_char_text(text) && !in_pre(&c) {
                            out.push_str(crate::markup::collapse_blank(text));
                        } else {
                            out.push_str(text);
                        }
                    }
                }
            }
            Node::Element(_) => {
                skipping = false;
                emit_pretty(c, format, false, "", out);
            }
            _ => {}
        }
        if format && !skipping {
            out.push('\n');
        }
    }
}

/// ebooklib `get_body_content`를 거친 뒤 `(soup.body or soup).get_text()`와 같은 글.
pub fn pretty_text(markup: &str) -> Option<String> {
    pretty_text_in(&crate::markup::parse(markup), markup)
}

/// 이미 파싱한 문서로 `pretty_text`를 만든다. `markup`은 원문 꼬리를 옮길 때 쓴다.
fn pretty_text_in(doc: &Html, markup: &str) -> Option<String> {
    let body = doc.tree.root().descendants().find(|n| matches!(n.value(), Node::Element(e) if e.name() == "body"))?;
    if child_count(body) == 0 {
        return None;
    }
    let has_attrs = matches!(body.value(), Node::Element(e) if e.attrs().next().is_some());
    let mut out = String::new();
    // `<body>` 태그가 속성 없이 `<body>`로 시작하면 ebooklib가 태그를 떼어 낸다.
    emit_pretty(body, true, !has_attrs, &crate::markup::relocated_tail(markup), &mut out);
    Some(out)
}

/// 문서 하나의 (본문 글, 줄 수). `get_body_content()`가 비면 원본 내용을 쓴다.
pub fn document_text_and_lines(content: &[u8]) -> Result<(String, usize), String> {
    let markup = decode_markup(content);
    if markup.trim().is_empty() {
        return Ok((String::new(), 0));
    }
    let doc = crate::markup::parse(&markup);
    let body = doc.tree.root().descendants().find(|n| matches!(n.value(), Node::Element(e) if e.name() == "body"));
    // lxml은 본문이 없는 문서(머리만 있는 문서)에서 body를 만들지 않아 `len(None)`에서 예외가 난다.
    let body_children = body.map_or(0, child_count);
    let has_body_text = body.is_some_and(|b| b.children().any(|c| matches!(c.value(), Node::Text(t) if !t.trim().is_empty())));
    if body_children == 0 && !has_body_text && !markup.to_ascii_lowercase().contains("<body") {
        return Err("TypeError: object of type 'NoneType' has no len()".into());
    }
    let lines = crate::markup::count_lines_in(&doc);
    let text = pretty_text_in(&doc, &markup).unwrap_or_else(|| get_body_text(&markup));
    Ok((text, lines))
}

fn zip_document_text_and_lines(content: &[u8]) -> Result<(String, usize), String> {
    let markup = decode_markup(content);
    if markup.trim().is_empty() {
        return Ok((String::new(), 0));
    }
    let lines = count_lines(&markup);
    Ok((crate::markup::get_document_text(&markup), lines))
}

/// 정규식 대체 경로. 챕터 전체 문서의 `get_text()`를 모은다.
fn read_with_zip_regex(path: &Path, errors: &mut Vec<String>) -> Result<(String, usize), String> {
    read_with_zip_regex_using(path, errors, &zip_document_text_and_lines)
}

type ChapterParser = dyn Fn(&[u8]) -> Result<(String, usize), String>;

fn read_with_zip_regex_using(path: &Path, errors: &mut Vec<String>, parse: &ChapterParser) -> Result<(String, usize), String> {
    static ROOTFILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<rootfile[^>]*full-path="([^"]+)""#).unwrap());
    static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<(?:opf:)?item\s[^>]*href="([^"]*\.x?html)"[^>]*media-type="application/xhtml\+xml""#).unwrap());
    let Some(mut zip) = open_zip(path) else {
        errors.push(path.display().to_string());
        errors.push("File is not a zip file".into());
        return Ok((String::new(), 0));
    };
    let container = String::from_utf8_lossy(&read_member(&mut zip, "META-INF/container.xml")?).into_owned();
    let Some(caps) = ROOTFILE.captures(&container) else { return Ok((String::new(), 0)) };
    let root_file = caps[1].to_owned();
    let root_dir = root_file.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_owned());
    let opf = decode_utf8_ignore(&read_member(&mut zip, &root_file)?);

    let mut result = String::new();
    let mut line_count = 0;
    for caps in ITEM.captures_iter(&opf) {
        let chapter = &caps[1];
        let chapter_path = if root_dir.is_empty() { chapter.to_owned() } else { format!("{root_dir}/{chapter}") };
        let Ok(bytes) = read_member(&mut zip, &chapter_path) else { continue };
        let (text, chapter_lines) = match parse(&bytes) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if result.chars().count() < TEXT_SIZE {
            result.push_str(&text);
        }
        line_count += chapter_lines;
    }
    Ok((head_chars(&result, TEXT_SIZE).to_owned(), line_count))
}

fn decode_utf8_ignore(bytes: &[u8]) -> String {
    corpus_common::reader::decode_utf8_ignore(bytes)
}

/// EPUB 파일: (요약, 줄 수). ebooklib 경로가 실패하면 정규식 경로로 간다.
pub fn read_from_epub(path: &Path) -> EpubRead {
    read_from_epub_with(path, &EpubHooks::default())
}

/// EPUB을 읽는다. 테스트와 호출자가 normal reader 실패를 재현할 수 있다.
pub fn read_from_epub_with(path: &Path, hooks: &EpubHooks<'_>) -> EpubRead {
    let mut errors = Vec::new();
    let normal = hooks.read_epub.map_or_else(|| read_with_ebooklib(path), |reader| reader(path));
    let (result, line_count, used) = match normal {
        Ok((result, lines)) => (result, lines, EpubPath::Normal),
        Err(first) => match read_with_zip_regex(path, &mut errors) {
            Ok((result, lines)) => {
                if errors.is_empty() {
                    // 대체 경로가 성공하면 첫 오류는 기록하지 않는다.
                }
                (result, lines, EpubPath::Zipped)
            }
            Err(second) => {
                errors.push(path.display().to_string());
                errors.push(second);
                errors.push(path.display().to_string());
                errors.push(first);
                (String::new(), 0, EpubPath::Zipped)
            }
        },
    };
    let cleaned = keep_word_chars(&result);
    EpubRead { summary: head_chars(&cleaned, TEXT_SIZE).to_owned(), line_count, path: used, errors }
}

/// 정규식 ZIP 경로만 직접 실행한다. 일반 reader의 결과와 비교할 때 사용한다.
pub fn read_from_epub_with_extracting_zip(path: &Path) -> EpubRead {
    let mut errors = Vec::new();
    let (result, line_count) = match read_with_zip_regex(path, &mut errors) {
        Ok(result) => result,
        Err(error) => {
            errors.extend([path.display().to_string(), error]);
            (String::new(), 0)
        }
    };
    let cleaned = keep_word_chars(&result);
    EpubRead { summary: head_chars(&cleaned, TEXT_SIZE).to_owned(), line_count, path: EpubPath::Zipped, errors }
}

/// ZIP fallback의 챕터 파서를 주입한다. 챕터별 parse 오류는 건너뛴다.
pub fn read_from_epub_with_extracting_zip_using(path: &Path, parse: &ChapterParser) -> (String, usize) {
    let mut errors = Vec::new();
    match read_with_zip_regex_using(path, &mut errors, parse) {
        Ok((summary, lines)) => (summary, lines),
        Err(_) => (String::new(), 0),
    }
}

/// EPUB의 `dc:publisher`. OPF 하나만 풀고 본문은 읽지 않는다. 읽을 수 없으면 빈 문자열이다.
/// 원본: `backend/classifier/reader.py`의 `read_epub_publisher`.
pub fn read_epub_publisher(path: &Path) -> String {
    static DC_PUBLISHER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<dc:publisher[^>]*>(.*?)</dc:publisher>").unwrap());
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
    let Some(mut zip) = open_zip(path) else { return String::new() };
    let opf = zip.file_names().filter_map(|n| n.ok()).map(|n| n.into_owned()).find(|n| n.to_lowercase().ends_with(".opf"));
    let Some(opf) = opf else { return String::new() };
    let Ok(bytes) = read_member(&mut zip, &opf) else { return String::new() };
    let xml = String::from_utf8_lossy(&bytes).into_owned();
    DC_PUBLISHER.captures_iter(&xml).map(|c| TAG.replace_all(&c[1], "").trim().to_owned()).filter(|v| !v.is_empty()).collect::<Vec<_>>().join(" ")
}
