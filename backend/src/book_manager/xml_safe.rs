//! XXE를 막은 XML 읽기(Python `_safe_xml_parser`, `_declare_unbound_prefixes`, `_prune_dead_ncx_navpoints`)의 모듈 수준 이름.
//!
//! `roxmltree`는 외부 엔티티를 읽지 않는다(내부 엔티티만 펼친다). lxml의 `recover=True`처럼 깨진 문서를 고쳐 읽지는 못하므로,
//! 복구 모드는 DOCTYPE과 알 수 없는 엔티티 참조를 걷어낸 뒤 한 번 더 읽는 데 그친다.

use std::borrow::Cow;
use std::sync::OnceLock;

use regex::Regex;
use roxmltree::Node;
use serde_json::Value;

use super::epub::{declare_unbound_prefixes, parse_xml, xml_text};

/// 읽어 낸 XML 요소. 이름은 네임스페이스를 뺀 로컬 이름이다.
#[derive(Debug, Clone)]
pub struct XmlElement {
    name: String,
    text: Option<String>,
    attributes: Vec<(String, String)>,
    children: Vec<XmlElement>,
}

impl XmlElement {
    fn from_node(node: Node) -> Self {
        let text: String = node.children().filter(Node::is_text).filter_map(|t| t.text()).collect();
        XmlElement {
            name: node.tag_name().name().to_string(),
            text: (!text.is_empty()).then_some(text),
            attributes: node.attributes().map(|a| (a.name().to_string(), a.value().to_string())).collect(),
            children: node.children().filter(Node::is_element).map(XmlElement::from_node).collect(),
        }
    }

    /// 이름이 같은 첫 직계 자식. lxml `find("a")`.
    pub fn find(&self, name: &str) -> Option<&XmlElement> {
        self.children.iter().find(|c| c.name == name)
    }

    /// 요소 안의 텍스트. 없으면 `None`.
    pub fn text(&self) -> Option<String> {
        self.text.clone()
    }

    /// 문자열로 다시 쓴다.
    pub fn tostring(&self) -> String {
        let attrs: String = self.attributes.iter().map(|(k, v)| format!(" {k}=\"{}\"", escape(v))).collect();
        let body: String = self.text.as_deref().map(escape).unwrap_or_default() + &self.children.iter().map(XmlElement::tostring).collect::<String>();
        format!("<{}{attrs}>{body}</{}>", self.name, self.name)
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 외부 엔티티, DTD, 네트워크 접근을 모두 끈 파서.
#[derive(Debug, Clone, Copy)]
pub struct SafeXmlParser {
    recover: bool,
}

/// Python `_safe_xml_parser(recover)`.
pub fn _safe_xml_parser(recover: bool) -> SafeXmlParser {
    SafeXmlParser { recover }
}

fn doctype_and_entities() -> &'static (Regex, Regex) {
    static RE: OnceLock<(Regex, Regex)> = OnceLock::new();
    RE.get_or_init(|| (Regex::new(r"(?s)<!DOCTYPE[^>\[]*(\[.*?\])?\s*>").expect("정규식"), Regex::new(r"&[A-Za-z_][\w.-]*;").expect("정규식")))
}

impl SafeXmlParser {
    pub fn fromstring(&self, data: &[u8]) -> Result<XmlElement, String> {
        let text = xml_text(data);
        match parse_xml(&text) {
            Ok(doc) => return Ok(XmlElement::from_node(doc.root_element())),
            Err(e) if !self.recover => return Err(e.to_string()),
            Err(_) => {}
        }
        // 복구: DOCTYPE과 알 수 없는 엔티티 참조(외부 엔티티 포함)를 걷어내고 다시 읽는다. 엔티티 내용은 읽지 않는다.
        let (doctype, entity) = doctype_and_entities();
        let cleaned = doctype.replace_all(&text, "");
        let cleaned = entity.replace_all(&cleaned, |m: &regex::Captures| if matches!(&m[0], "&amp;" | "&lt;" | "&gt;" | "&quot;" | "&apos;") { m[0].to_string() } else { String::new() }).into_owned();
        let doc = parse_xml(&cleaned).map_err(|e| e.to_string())?;
        Ok(XmlElement::from_node(doc.root_element()))
    }
}

/// 선언 없이 쓰인 접두사를 루트 요소에 선언한다. 빠진 선언이 없으면 입력을 그대로(`Borrowed`) 돌려준다.
pub fn _declare_unbound_prefixes(xml_bytes: &[u8]) -> Cow<'_, [u8]> {
    let text = String::from_utf8_lossy(xml_bytes);
    let fixed = declare_unbound_prefixes(&text);
    if fixed == text { Cow::Borrowed(xml_bytes) } else { Cow::Owned(fixed.into_bytes()) }
}

pub use super::epub::prune_dead_ncx_navpoints as _prune_dead_ncx_navpoints;

/// 정렬 값 배열(JSON)을 클라이언트에 내보낼 커서 문자열로 만든다.
pub fn encode_category_cursor(sort_values: &Value) -> String {
    crate::es_manager::encode_category_cursor(sort_values.as_array().map(Vec::as_slice).unwrap_or(&[]))
}

/// 커서를 정렬 값 배열(JSON)로 되돌린다. 형식이 잘못되면 `None`이다.
pub fn decode_category_cursor(cursor: &str) -> Option<Value> {
    crate::es_manager::decode_category_cursor(cursor).map(Value::Array)
}
