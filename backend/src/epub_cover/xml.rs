//! 고칠 수 있는 작은 XML 트리. Python `lxml.etree`로 OPF와 XHTML을 고치던 부분을 옮기는 데 필요한 만큼만 둔다.
//!
//! 읽기는 `roxmltree`에 맡기고 결과를 이 트리로 옮긴다. 외부 엔티티는 풀지 않는다(CWE-611).
//! lxml과 같게 본문 텍스트는 별도 노드이고, 요소를 지우면 바로 뒤의 텍스트(lxml의 tail)도 함께 지운다.

use std::collections::BTreeMap;

use regex::Regex;
use std::sync::LazyLock;

pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
pub const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
pub const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub ns: Option<String>,
    pub prefix: Option<String>,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub ns: Option<String>,
    pub prefix: Option<String>,
    pub name: String,
    /// 이 요소에서 선언한 `xmlns`. `None`은 기본 네임스페이스다.
    pub decls: Vec<(Option<String>, String)>,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
    Comment(String),
    Pi(String, String),
}

impl Element {
    pub fn new(ns: Option<&str>, prefix: Option<&str>, name: &str) -> Self {
        Element { ns: ns.map(str::to_string), prefix: prefix.map(str::to_string), name: name.to_string(), decls: Vec::new(), attrs: Vec::new(), children: Vec::new() }
    }

    /// 같은 네임스페이스와 접두사로 새 요소를 만든다(`etree.SubElement(parent, "{ns}tag")`).
    pub fn sibling(&self, name: &str) -> Self {
        Element { ns: self.ns.clone(), prefix: self.prefix.clone(), name: name.to_string(), decls: Vec::new(), attrs: Vec::new(), children: Vec::new() }
    }

    pub fn with_attr(mut self, name: &str, value: &str) -> Self {
        self.set_attr(name, value);
        self
    }

    pub fn is(&self, ns: &str, name: &str) -> bool {
        self.name == name && self.ns.as_deref() == Some(ns)
    }

    /// 네임스페이스 없는 속성.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.ns.is_none() && a.name == name).map(|a| a.value.as_str())
    }

    pub fn attr_or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.attr(name).unwrap_or(default)
    }

    pub fn attr_ns(&self, ns: &str, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.ns.as_deref() == Some(ns) && a.name == name).map(|a| a.value.as_str())
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        match self.attrs.iter_mut().find(|a| a.ns.is_none() && a.name == name) {
            Some(existing) => existing.value = value.to_string(),
            None => self.attrs.push(Attr { ns: None, prefix: None, name: name.to_string(), value: value.to_string() }),
        }
    }

    pub fn remove_attr(&mut self, name: &str) -> Option<String> {
        let at = self.attrs.iter().position(|a| a.ns.is_none() && a.name == name)?;
        Some(self.attrs.remove(at).value)
    }

    pub fn child_elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| if let Node::Element(e) = n { Some(e) } else { None })
    }

    pub fn child_elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| if let Node::Element(e) = n { Some(e) } else { None })
    }

    /// 직계 자식 중 `{ns}name`인 첫 요소(`find("{ns}name")`).
    pub fn find(&self, ns: &str, name: &str) -> Option<&Element> {
        self.child_elements().find(|e| e.is(ns, name))
    }

    pub fn find_mut(&mut self, ns: &str, name: &str) -> Option<&mut Element> {
        self.child_elements_mut().find(|e| e.is(ns, name))
    }

    /// 직계 자식 중 `{ns}name`인 요소 전부(`findall`).
    pub fn find_all<'a>(&'a self, ns: &'a str, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.child_elements().filter(move |e| e.is(ns, name))
    }

    /// 문서 순서로 자신과 모든 하위 요소(`iter()`에서 주석과 처리 명령은 뺀다).
    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
            out.push(e);
            for child in e.child_elements() {
                walk(child, out);
            }
        }
        walk(self, &mut out);
        out
    }

    /// 문서 순서로 자신과 모든 하위 요소를 고친다.
    pub fn for_each_mut(&mut self, f: &mut dyn FnMut(&mut Element)) {
        f(self);
        for child in self.child_elements_mut() {
            child.for_each_mut(f);
        }
    }

    /// 모든 하위 텍스트(`"".join(element.itertext())`). 주석 본문은 넣지 않는다.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        fn walk(e: &Element, out: &mut String) {
            for node in &e.children {
                match node {
                    Node::Text(t) => out.push_str(t),
                    Node::Element(child) => walk(child, out),
                    _ => {}
                }
            }
        }
        walk(self, &mut out);
        out
    }

    /// 자식 노드(요소·주석·처리 명령) 중 `index`번째 앞에 끼운다. lxml의 `insert(index, child)`와 같다.
    pub fn insert_child(&mut self, index: usize, child: Element) {
        let positions: Vec<usize> = self.children.iter().enumerate().filter(|(_, n)| !matches!(n, Node::Text(_))).map(|(i, _)| i).collect();
        let at = positions.get(index).copied().unwrap_or(self.children.len());
        self.children.insert(at, Node::Element(child));
    }

    pub fn push_child(&mut self, child: Element) {
        self.children.push(Node::Element(child));
    }

    /// `index`번째 직계 요소의 자식 노드 번호(lxml `index`가 세는 것과 같은 순서의 위치가 아니라 요소 순번)를 돌려준다.
    pub fn element_position(&self, predicate: impl Fn(&Element) -> bool) -> Option<usize> {
        self.children.iter().filter(|n| !matches!(n, Node::Text(_))).position(|n| matches!(n, Node::Element(e) if predicate(e)))
    }

    /// 조건에 맞는 직계 요소를 지운다. lxml처럼 바로 뒤의 텍스트도 함께 지운다.
    pub fn remove_children_where(&mut self, predicate: impl Fn(&Element) -> bool) -> usize {
        let mut removed = 0;
        let mut i = 0;
        while i < self.children.len() {
            if matches!(&self.children[i], Node::Element(e) if predicate(e)) {
                self.children.remove(i);
                if matches!(self.children.get(i), Some(Node::Text(_))) {
                    self.children.remove(i);
                }
                removed += 1;
            } else {
                i += 1;
            }
        }
        removed
    }
}

// ---------------------------------------------------------------------------
// 읽기
// ---------------------------------------------------------------------------

/// 읽기 오류. `UnknownNamespace`는 접두사를 고쳐서 다시 읽을 수 있어 따로 둔다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlError {
    UnknownNamespace(String),
    UnknownEntity(String),
    Syntax(String),
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XmlError::UnknownNamespace(p) => write!(f, "undefined namespace prefix: {p}"),
            XmlError::UnknownEntity(e) => write!(f, "undefined entity: {e}"),
            XmlError::Syntax(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for XmlError {}

/// BOM을 걷고 UTF-8로 읽는다. UTF-16 BOM은 변환한다. 그 밖의 인코딩 선언은 따르지 않는다.
pub fn decode(data: &[u8]) -> Result<String, XmlError> {
    if let Some(rest) = data.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).map_err(|e| XmlError::Syntax(e.to_string()));
    }
    let utf16 = |rest: &[u8], big: bool| -> Result<String, XmlError> {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        String::from_utf16(&units).map_err(|e| XmlError::Syntax(e.to_string()))
    };
    if let Some(rest) = data.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, false);
    }
    if let Some(rest) = data.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, true);
    }
    String::from_utf8(data.to_vec()).map_err(|e| XmlError::Syntax(e.to_string()))
}

fn map_error(err: roxmltree::Error) -> XmlError {
    match err {
        roxmltree::Error::UnknownNamespace(prefix, _) => XmlError::UnknownNamespace(prefix),
        roxmltree::Error::UnknownEntityReference(name, _) => XmlError::UnknownEntity(name),
        other => XmlError::Syntax(other.to_string()),
    }
}

/// 엄격하게 읽는다(`etree.fromstring(data, _safe_xml_parser())`). DTD는 읽되 외부 엔티티는 풀지 않는다.
pub fn parse(data: &[u8]) -> Result<Element, XmlError> {
    let text = decode(data)?;
    parse_str(&text)
}

pub fn parse_str(text: &str) -> Result<Element, XmlError> {
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..roxmltree::ParsingOptions::default() };
    let doc = roxmltree::Document::parse_with_options(text, options).map_err(map_error)?;
    Ok(convert(doc.root_element(), text))
}

fn convert(node: roxmltree::Node, src: &str) -> Element {
    let ns = node.tag_name().namespace().map(str::to_string);
    let mut element = Element { prefix: element_prefix(node, src), ns, name: node.tag_name().name().to_string(), decls: Vec::new(), attrs: Vec::new(), children: Vec::new() };
    let parent_namespaces: Vec<(Option<&str>, &str)> = node.parent_element().map(|p| p.namespaces().map(|n| (n.name(), n.uri())).collect()).unwrap_or_default();
    let mut decls: Vec<(Option<String>, String)> =
        node.namespaces().filter(|n| n.name() != Some("xml") && !parent_namespaces.contains(&(n.name(), n.uri()))).map(|n| (n.name().map(str::to_string), n.uri().to_string())).collect();
    // roxmltree가 되돌려 주는 순서는 선언 순서와 다를 수 있어 접두사 이름으로 고정한다.
    decls.sort();
    element.decls = decls;
    for attr in node.attributes() {
        let prefix = attr.namespace().and_then(|uri| {
            if uri == XML_NS {
                Some("xml".to_string())
            } else {
                node.namespaces().find(|n| n.uri() == uri && n.name().is_some()).and_then(|n| n.name().map(str::to_string))
            }
        });
        element.attrs.push(Attr { ns: attr.namespace().map(str::to_string), prefix, name: attr.name().to_string(), value: attr.value().to_string() });
    }
    for child in node.children() {
        if child.is_element() {
            element.children.push(Node::Element(convert(child, src)));
        } else if child.is_text() {
            element.children.push(Node::Text(child.text().unwrap_or("").to_string()));
        } else if child.is_comment() {
            element.children.push(Node::Comment(child.text().unwrap_or("").to_string()));
        } else if let Some(pi) = child.pi() {
            element.children.push(Node::Pi(pi.target.to_string(), pi.value.unwrap_or("").to_string()));
        }
    }
    element
}

/// 시작 태그에 쓴 접두사. 원문에서 읽어 한 네임스페이스에 접두사가 여럿이어도 그대로 보존한다.
fn element_prefix(node: roxmltree::Node, src: &str) -> Option<String> {
    let range = node.range();
    let tag = src.get(range.start..)?;
    let tag = tag.strip_prefix('<')?;
    let end = tag.find(|c: char| c.is_whitespace() || c == '/' || c == '>')?;
    tag[..end].split_once(':').map(|(prefix, _)| prefix.to_string())
}

// ---------------------------------------------------------------------------
// 너그럽게 읽기
// ---------------------------------------------------------------------------

/// OPF에서 선언 없이 자주 쓰는 접두사. 모르는 접두사는 자리표시 URN으로 선언한다(값은 읽지 않는다).
const KNOWN_PREFIXES: [(&str, &str); 6] = [
    ("opf", "http://www.idpf.org/2007/opf"),
    ("dc", "http://purl.org/dc/elements/1.1/"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
    ("xlink", XLINK_NS),
    ("epub", "http://www.idpf.org/2007/ops"),
];

static ROOT_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\A(?:\s|<\?.*?\?>|<!--.*?-->|<!DOCTYPE(?:[^\[>]|\[.*?\])*>)*<([^\s/>!?]+)").expect("정규식"));
static NAMED_ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&([A-Za-z_][A-Za-z0-9_.-]*);").expect("정규식"));

fn declare_prefix(text: &str, prefix: &str) -> Option<String> {
    let caps = ROOT_TAG.captures(text)?;
    let name = caps.get(1)?;
    let uri = KNOWN_PREFIXES.iter().find(|(p, _)| *p == prefix).map(|(_, u)| (*u).to_string()).unwrap_or_else(|| format!("urn:unknown-prefix:{prefix}"));
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str(&text[..name.end()]);
    out.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
    out.push_str(&text[name.end()..]);
    Some(out)
}

fn strip_unknown_entities(text: &str) -> String {
    NAMED_ENTITY.replace_all(text, |caps: &regex::Captures| if matches!(&caps[1], "amp" | "lt" | "gt" | "quot" | "apos") { caps[0].to_string() } else { String::new() }).into_owned()
}

/// `etree.fromstring(data, _safe_xml_parser(recover=True))`의 일부만 대신한다. 선언 없는 접두사와 모르는 엔티티는 고쳐서 다시 읽는다.
/// 그 밖의 구문 오류는 `None`이다(lxml은 깨진 부분을 건너뛰고 계속 읽는다).
pub fn parse_lenient(data: &[u8]) -> Option<Element> {
    let mut text = match decode(data) {
        Ok(text) => text,
        Err(_) => String::from_utf8_lossy(data).into_owned(),
    };
    let mut declared: Vec<String> = Vec::new();
    for _ in 0..16 {
        match parse_str(&text) {
            Ok(element) => return Some(element),
            Err(XmlError::UnknownNamespace(prefix)) if !declared.contains(&prefix) => {
                text = declare_prefix(&text, &prefix)?;
                declared.push(prefix);
            }
            Err(XmlError::UnknownEntity(_)) => {
                let stripped = strip_unknown_entities(&text);
                if stripped == text {
                    return None;
                }
                text = stripped;
            }
            Err(_) => return None,
        }
    }
    None
}

/// `opf:` 속성만 선언이 빠진 OPF를 읽는다. 구조 오류와 다른 접두사는 받지 않는다.
pub fn parse_package(data: &[u8]) -> Result<Element, XmlError> {
    let text = decode(data)?;
    match parse_str(&text) {
        Err(XmlError::UnknownNamespace(prefix)) if prefix == "opf" => {
            // 요소 이름에 쓴 `opf:`는 고치지 않는다. 속성에만 쓴 경우를 받는다.
            if text.contains("<opf:") || text.contains("</opf:") {
                return Err(XmlError::UnknownNamespace(prefix));
            }
            let fixed = declare_prefix(&text, "opf").ok_or(XmlError::UnknownNamespace(prefix))?;
            parse_str(&fixed)
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------
// 쓰기
// ---------------------------------------------------------------------------

fn escape_text(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
}

fn escape_attr(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
}

fn qualified(prefix: &Option<String>, name: &str) -> String {
    match prefix {
        Some(p) if !p.is_empty() => format!("{p}:{name}"),
        _ => name.to_string(),
    }
}

fn write_element(e: &Element, out: &mut String) {
    let tag = qualified(&e.prefix, &e.name);
    out.push('<');
    out.push_str(&tag);
    for (prefix, uri) in &e.decls {
        match prefix {
            Some(p) => out.push_str(&format!(" xmlns:{p}=\"")),
            None => out.push_str(" xmlns=\""),
        }
        escape_attr(uri, out);
        out.push('"');
    }
    for attr in &e.attrs {
        out.push(' ');
        out.push_str(&qualified(&attr.prefix, &attr.name));
        out.push_str("=\"");
        escape_attr(&attr.value, out);
        out.push('"');
    }
    if e.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for node in &e.children {
        match node {
            Node::Element(child) => write_element(child, out),
            Node::Text(t) => escape_text(t, out),
            Node::Comment(c) => {
                out.push_str("<!--");
                out.push_str(c);
                out.push_str("-->");
            }
            Node::Pi(target, data) => {
                out.push_str("<?");
                out.push_str(target);
                if !data.is_empty() {
                    out.push(' ');
                    out.push_str(data);
                }
                out.push_str("?>");
            }
        }
    }
    out.push_str("</");
    out.push_str(&tag);
    out.push('>');
}

/// XML 선언 없이 요소만 쓴다(`etree.tostring(element)`).
pub fn to_string(e: &Element) -> String {
    let mut out = String::new();
    write_element(e, &mut out);
    out
}

/// `etree.tostring(element, encoding="utf-8", xml_declaration=True)`.
pub fn to_bytes_with_declaration(e: &Element) -> Vec<u8> {
    let mut out = String::from("<?xml version='1.0' encoding='utf-8'?>\n");
    write_element(e, &mut out);
    out.into_bytes()
}

/// 접두사 → 네임스페이스 표. 진단과 테스트에서 쓴다.
pub fn declared_prefixes(e: &Element) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for el in e.descendants() {
        for (prefix, uri) in &el.decls {
            out.insert(prefix.clone().unwrap_or_default(), uri.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_prefixes_and_declarations() {
        let src = r#"<opf:package xmlns="urn:b" xmlns:opf="urn:a" x="1 &amp; 2"><item id="a"/><opf:meta>t&lt;</opf:meta></opf:package>"#;
        let root = parse_str(src).unwrap();
        assert_eq!(root.prefix.as_deref(), Some("opf"));
        let out = to_string(&root);
        assert_eq!(out, src);
        let again = parse_str(&out).unwrap();
        assert_eq!(again, root);
    }

    #[test]
    fn undeclared_opf_attribute_is_declared_on_root() {
        let root = parse_package(br#"<package xmlns="http://www.idpf.org/2007/opf"><metadata><creator opf:role="aut"/></metadata></package>"#).unwrap();
        let creator = root.find("http://www.idpf.org/2007/opf", "metadata").unwrap().child_elements().next().unwrap();
        assert_eq!(creator.attr_ns("http://www.idpf.org/2007/opf", "role"), Some("aut"));
        let again = parse_str(&to_string(&root)).unwrap();
        assert_eq!(again, root);
    }

    #[test]
    fn unknown_prefix_and_structure_errors_are_rejected() {
        assert!(parse_package(br#"<package xmlns="urn:a"><metadata><creator unknown:role="aut"/></metadata></package>"#).is_err());
        assert!(parse_package(br#"<package xmlns="urn:a"><metadata></package>"#).is_err());
        assert!(parse_package(b"").is_err());
    }

    #[test]
    fn external_entities_are_not_resolved() {
        assert!(parse_package(br#"<!DOCTYPE p [<!ENTITY e SYSTEM "file:///etc/passwd">]><p>&e;</p>"#).is_err());
        assert!(!parse_lenient(br#"<!DOCTYPE p [<!ENTITY e SYSTEM "file:///etc/passwd">]><p>&e;</p>"#).is_some_and(|p| p.text_content().contains("root:")));
    }

    #[test]
    fn lenient_parse_survives_html_entities_and_unknown_prefixes() {
        let root = parse_lenient("<html><body>a&nbsp;b<x:y/></body></html>".as_bytes()).unwrap();
        assert_eq!(root.text_content(), "ab");
    }

    #[test]
    fn removing_an_element_drops_its_tail_text() {
        let mut root = parse_str("<m>\n <a/>\n <b/>\n</m>").unwrap();
        root.remove_children_where(|e| e.name == "a");
        assert_eq!(to_string(&root), "<m>\n <b/>\n</m>");
    }

    #[test]
    fn insert_child_counts_nodes_like_lxml() {
        let mut root = parse_str("<s>\n <a/>\n <b/>\n</s>").unwrap();
        root.insert_child(0, Element::new(None, None, "n"));
        assert_eq!(to_string(&root), "<s>\n <n/><a/>\n <b/>\n</s>");
        root.insert_child(2, Element::new(None, None, "m"));
        assert_eq!(to_string(&root), "<s>\n <n/><a/>\n <m/><b/>\n</s>");
    }
}
