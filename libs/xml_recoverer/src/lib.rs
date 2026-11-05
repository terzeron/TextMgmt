//! 깨진 XML을 엄격한 파서(`roxmltree`)가 다시 읽을 수 있게 고치는 문자열 변환.
//!
//! Python 백엔드는 lxml `recover=True` 하나로 깨진 XML을 읽었다. 이 crate는 그중 코퍼스에서 실제로 만나는 경우만 대신한다:
//! 선언 없이 쓴 접두사, DTD에만 정의된 엔티티(`&nbsp;`), 잘린 태그. 파싱은 하지 않는다. 어떤 변환을 어떤 순서로
//! 적용할지는 호출자가 정한다. DTD는 지울 뿐 읽지 않는다(엔티티 폭탄 CWE-776, 외부 엔티티 CWE-611).

use std::sync::LazyLock;

use regex::Regex;

// ---------------------------------------------------------------------------
// 접두사 선언
// ---------------------------------------------------------------------------

/// OPF에서 선언 없이 자주 쓰는 접두사.
const KNOWN_PREFIXES: [(&str, &str); 6] = [
    ("opf", "http://www.idpf.org/2007/opf"),
    ("dc", "http://purl.org/dc/elements/1.1/"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
    ("xlink", "http://www.w3.org/1999/xlink"),
    ("epub", "http://www.idpf.org/2007/ops"),
];

/// 접두사의 네임스페이스. 모르는 접두사는 자리표시 URN이다(값은 읽지 않는다).
fn prefix_uri(prefix: &str) -> String {
    KNOWN_PREFIXES.iter().find(|(p, _)| *p == prefix).map(|(_, u)| (*u).to_string()).unwrap_or_else(|| format!("urn:unknown-prefix:{prefix}"))
}

static ROOT_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\A(?:\s|<\?.*?\?>|<!--.*?-->|<!DOCTYPE(?:[^\[>]|\[.*?\])*>)*<([^\s/>!?]+)").expect("정규식"));

/// 접두사 하나를 루트 요소에 선언한다. 파서가 알려 준 접두사를 하나씩 고칠 때 쓴다. 루트 요소가 없으면 `None`이다.
pub fn declare_prefix(xml: &str, prefix: &str) -> Option<String> {
    let caps = ROOT_TAG.captures(xml)?;
    let name = caps.get(1)?;
    let mut out = String::with_capacity(xml.len() + 64);
    out.push_str(&xml[..name.end()]);
    out.push_str(&format!(" xmlns:{prefix}=\"{}\"", prefix_uri(prefix)));
    out.push_str(&xml[name.end()..]);
    Some(out)
}

/// 주석·PI·DOCTYPE, 태그, 요소 접두사, 속성 접두사, 선언된 접두사.
static SKIP_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->|<\?.*?\?>|<!DOCTYPE[^>]*>").expect("정규식"));
static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<(/?)([^\s/>!?][^>]*)>").expect("정규식"));
static ELEMENT_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z_][\w.-]*):").expect("정규식"));
static ATTR_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s([A-Za-z_][\w.-]*):[\w.-]+\s*=").expect("정규식"));
static DECLARED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"xmlns:([A-Za-z_][\w.-]*)\s*=").expect("정규식"));

/// 요소·속성 이름에 쓰였지만 선언되지 않은 접두사를 모두 루트 요소에 선언한다. 빠진 선언이 없으면 입력을 그대로 돌려준다.
pub fn declare_unbound_prefixes(xml: &str) -> String {
    let masked = SKIP_RE.replace_all(xml, |m: &regex::Captures| " ".repeat(m[0].len())).into_owned();
    let Some(root) = TAG_RE.captures(&masked) else { return xml.to_string() };
    let declared: Vec<String> = DECLARED_RE.captures_iter(&masked).map(|c| c[1].to_string()).collect();
    let mut used: Vec<String> = Vec::new();
    for tag in TAG_RE.captures_iter(&masked) {
        let body = &tag[2];
        let mut names: Vec<String> = Vec::new();
        if let Some(c) = ELEMENT_PREFIX_RE.captures(body) {
            names.push(c[1].to_string());
        }
        names.extend(ATTR_PREFIX_RE.captures_iter(body).map(|c| c[1].to_string()));
        for name in names {
            if !declared.contains(&name) && name != "xml" && name != "xmlns" && !used.contains(&name) {
                used.push(name);
            }
        }
    }
    if used.is_empty() {
        return xml.to_string();
    }
    let declarations: String = used.iter().map(|n| format!(" xmlns:{n}=\"{}\"", prefix_uri(n))).collect();
    let body = root.get(2).expect("그룹 2");
    let name_len = body.as_str().find(|c: char| c.is_whitespace() || c == '/' || c == '>').unwrap_or(body.as_str().len());
    let name_end = body.start() + name_len;
    format!("{}{declarations}{}", &xml[..name_end], &xml[name_end..])
}

// ---------------------------------------------------------------------------
// DOCTYPE·엔티티
// ---------------------------------------------------------------------------

static DOCTYPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!DOCTYPE[^>\[]*(\[.*?\])?\s*>").expect("정규식"));
static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&([A-Za-z_][\w.-]*);").expect("정규식"));

/// DOCTYPE 선언을 내부 부분집합(`[...]`)까지 지운다.
pub fn strip_doctype(xml: &str) -> String {
    DOCTYPE_RE.replace_all(xml, "").into_owned()
}

/// XML이 미리 정의한 다섯 개(`amp`·`lt`·`gt`·`quot`·`apos`)를 뺀 이름 있는 엔티티 참조를 지운다. 문자 참조(`&#160;`)는 남긴다.
pub fn strip_unknown_entities(xml: &str) -> String {
    ENTITY_RE.replace_all(xml, |caps: &regex::Captures| if matches!(&caps[1], "amp" | "lt" | "gt" | "quot" | "apos") { caps[0].to_string() } else { String::new() }).into_owned()
}

// ---------------------------------------------------------------------------
// 태그 구조
// ---------------------------------------------------------------------------

/// 잘린 태그를 닫고 서로 맞지 않는 종료 태그를 앞선 열린 태그에 맞춘다. 짝 없는 종료 태그와 DOCTYPE 같은 `<!…>`는 버린다.
pub fn close_open_tags(xml: &str) -> String {
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
