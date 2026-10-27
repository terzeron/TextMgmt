//! HTML·XHTML 본문 텍스트 추출과 줄 수 세기. 원본: `BeautifulSoup(markup, "lxml")`의 `get_text()`와 `Loader._count_epub_lines`.
//!
//! lxml(libxml2) 대신 html5ever(`scraper`)로 파싱한다. 정상 문서에서는 같은 글이 나오고, 심하게 깨진
//! 문서에서는 트리가 다를 수 있다. libxml2 2.14처럼 공백뿐인 텍스트 노드는 한 글자로 줄인다.

use std::path::Path;

use ego_tree::NodeRef;
use scraper::{Html, Node};

use crate::TEXT_SIZE;
use crate::pytext::{head_chars, keep_word_chars, universal_newlines};

/// EPUB 줄 수를 셀 때 앞뒤에 줄바꿈을 두는 블록 태그.
const BLOCK_TAGS: [&str; 35] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "caption",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
];
const SKIPPED_TAGS: [&str; 4] = ["head", "script", "style", "template"];
/// bs4가 `get_text()`에서 일반 문자열로 치지 않는 컨테이너.
const NON_TEXT_CONTAINERS: [&str; 3] = ["script", "style", "template"];

/// HTML 빈 요소. 이 요소의 `/>`는 그대로 둔다.
const VOID_ELEMENTS: [&str; 18] = ["area", "base", "basefont", "bgsound", "br", "col", "command", "embed", "frame", "hr", "img", "input", "isindex", "keygen", "link", "meta", "param", "source"];

fn is_void(name: &str) -> bool {
    VOID_ELEMENTS.contains(&name) || matches!(name, "track" | "wbr")
}

/// libxml2는 HTML에서도 `<div/>`를 빈 요소로 닫지만 html5ever는 `<div>`로 읽어 뒤따르는 글을 안에 넣고,
/// `<title/>`은 문서 나머지를 삼킨다. XHTML 문서가 흔해서, 빈 요소가 아닌 태그의 `/>`를 `></name>`로 바꿔 준다.
/// 주석, CDATA, 처리 지시, script·style 본문 안은 건드리지 않는다.
pub fn expand_self_closing(markup: &str) -> std::borrow::Cow<'_, str> {
    if !markup.contains("/>") {
        return std::borrow::Cow::Borrowed(markup);
    }
    let bytes = markup.as_bytes();
    let mut out = String::with_capacity(markup.len() + 16);
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &markup[i..];
        // 주석·CDATA·처리 지시·선언은 끝까지 건너뛴다.
        let skip_to = if rest.starts_with("<!--") {
            rest.find("-->").map(|e| e + 3)
        } else if rest.starts_with("<![CDATA[") {
            rest.find("]]>").map(|e| e + 3)
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            rest.find('>').map(|e| e + 1)
        } else {
            None
        };
        if let Some(n) = skip_to {
            i += n;
            continue;
        }
        if rest.starts_with("</") || !rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        // 시작 태그 끝 찾기(따옴표 안의 `>`는 무시)
        let tag_bytes = rest.as_bytes();
        let (mut j, mut quote) = (1, 0u8);
        while j < tag_bytes.len() {
            let b = tag_bytes[j];
            if quote != 0 {
                if b == quote {
                    quote = 0;
                }
            } else if b == b'"' || b == b'\'' {
                quote = b;
            } else if b == b'>' {
                break;
            }
            j += 1;
        }
        if j >= tag_bytes.len() {
            break;
        }
        let tag = &rest[..=j];
        let name_end = tag[1..].find(|c: char| c.is_whitespace() || c == '/' || c == '>').map_or(tag.len(), |e| e + 1);
        let name = tag[1..name_end].to_ascii_lowercase();
        let raw_text = matches!(name.as_str(), "script" | "style") && !tag.ends_with("/>");
        if tag.ends_with("/>") && !is_void(&name) {
            out.push_str(&markup[copied..i]);
            out.push_str(&tag[..tag.len() - 2]);
            out.push_str("></");
            out.push_str(&tag[1..name_end]);
            out.push('>');
            copied = i + tag.len();
        }
        i += tag.len();
        if raw_text {
            let close = format!("</{name}");
            let lower_rest = markup[i..].to_ascii_lowercase();
            i += lower_rest.find(&close).unwrap_or(lower_rest.len());
        }
    }
    out.push_str(&markup[copied..]);
    std::borrow::Cow::Owned(out)
}

/// 마크업을 파싱한다. libxml2처럼 `<name/>`을 빈 요소로 본다.
pub fn parse(markup: &str) -> Html {
    Html::parse_document(&expand_self_closing(markup))
}

fn element_name<'a>(node: &NodeRef<'a, Node>) -> Option<&'a str> {
    match node.value() {
        Node::Element(e) => Some(e.name()),
        _ => None,
    }
}

fn is_blank(text: &str) -> bool {
    text.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

/// 비어 있지 않고 libxml2가 공백으로 보는 글자(`' '`, `\t`, `\n`, `\r`)뿐인가.
pub fn is_blank_char_text(text: &str) -> bool {
    !text.is_empty() && is_blank(text)
}

/// 공백뿐인 텍스트를 libxml2 2.14처럼 한 글자로 줄인다: 줄바꿈이 있으면 `\n`, 없으면 공백 하나.
pub fn collapse_blank(text: &str) -> &'static str {
    if text.contains('\n') { "\n" } else { " " }
}

fn inside_pre(node: &NodeRef<Node>) -> bool {
    node.ancestors().any(|a| element_name(&a) == Some("pre"))
}

/// lxml(libxml2 2.14)은 공백뿐인 텍스트를 한 글자로 줄인다: 줄바꿈이 있으면 `\n`, 없으면 공백 하나.
/// `pre` 안에서는 그대로 둔다.
fn normalized_text<'a>(node: &NodeRef<Node>, text: &'a str) -> &'a str {
    if text.is_empty() || !is_blank(text) || inside_pre(node) {
        return text;
    }
    collapse_blank(text)
}

/// html5ever는 `</body>`와 `</html>` 뒤의 공백을 body 안으로 옮기지만 libxml2는 body 밖에 둔다.
/// 소스에서 그 공백(줄바꿈은 `\n`으로 정규화)을 돌려준다. 해당 구조가 아니면 빈 문자열이다.
pub fn relocated_tail(markup: &str) -> String {
    let lower = markup.to_ascii_lowercase();
    let Some(body_end) = lower.rfind("</body>") else { return String::new() };
    let after = &markup[body_end + "</body>".len()..];
    let lower_after = &lower[body_end + "</body>".len()..];
    let blanks = |t: &str| t.chars().filter(|c| matches!(c, ' ' | '\t' | '\n' | '\r')).collect::<String>();
    let all_blank = |t: &str| t.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'));
    let relocated = match lower_after.find("</html>") {
        Some(h) => {
            if !all_blank(&after[..h]) || !all_blank(&after[h + "</html>".len()..]) {
                return String::new();
            }
            format!("{}{}", &after[..h], &after[h + "</html>".len()..])
        }
        None if all_blank(after) => after.to_owned(),
        None => return String::new(),
    };
    blanks(&relocated).replace("\r\n", "\n").replace('\r', "\n")
}

/// 마지막 자식 텍스트에서 옮겨진 꼬리 공백을 뗀 글. 뗀 뒤 비면 `None`.
pub fn trim_relocated<'a>(text: &'a str, tail: &str) -> Option<&'a str> {
    let trimmed = if tail.is_empty() { text } else { text.strip_suffix(tail).unwrap_or(text) };
    (!trimmed.is_empty()).then_some(trimmed)
}

/// 문서 전체의 `get_text()`: 스크립트·스타일·템플릿 안의 글자와 주석은 뺀다.
pub fn get_text(markup: &str) -> String {
    let doc = parse(markup);
    // html5ever는 `<html>`과 `<head>` 사이 공백을 버리지만 libxml2는 한 글자로 남긴다.
    let lead = html_head_gap(markup).unwrap_or("");
    let text = collect_text(doc.tree.root(), "");
    format!("{lead}{}", fix_tail_blanks(markup, text))
}

/// 공백 덩어리를 libxml2처럼 한 글자로 줄인다(`\n`이 있으면 `\n`, 없으면 공백).
fn collapse_blank_run(run: &str) -> &'static str {
    if run.is_empty() {
        ""
    } else if run.contains('\n') {
        "\n"
    } else {
        " "
    }
}

/// `</body>`와 `</html>` 둘레의 공백. html5ever는 이 세 덩어리를 한 텍스트 노드로 합치지만
/// libxml2는 덩어리마다 한 글자씩 남긴다. 합쳐진 한 글자를 덩어리별 글자로 되돌린다.
fn fix_tail_blanks(markup: &str, text: String) -> String {
    let lower = markup.to_ascii_lowercase();
    let Some(html_end) = lower.rfind("</html>") else { return text };
    let tail3 = &markup[html_end + "</html>".len()..];
    if !is_blank(tail3) {
        return text;
    }
    let before_html = &markup[..html_end];
    let Some(body_end) = before_html.to_ascii_lowercase().rfind("</body>") else { return text };
    let between = &before_html[body_end + "</body>".len()..];
    if !is_blank(between) {
        return text;
    }
    let inside = &before_html[..body_end];
    let g1 = &inside[inside.trim_end_matches([' ', '\t', '\n', '\r']).len()..];
    // html5ever가 한 노드로 합쳐 남긴 글자와 libxml2가 남기는 글자
    let merged_source = format!("{g1}{between}{tail3}");
    let merged = collapse_blank_run(&merged_source);
    let wanted = format!("{}{}{}", collapse_blank_run(g1), collapse_blank_run(between), collapse_blank_run(tail3));
    match text.strip_suffix(merged) {
        Some(head) if !merged.is_empty() => format!("{head}{wanted}"),
        _ => text,
    }
}

/// `<html ...>`와 `<head>` 또는 암묵적 `<body>` 사이 공백을 libxml2처럼 남긴다.
fn html_head_gap(markup: &str) -> Option<&'static str> {
    let lower = markup.to_ascii_lowercase();
    let start = lower.find("<html")?;
    let open_end = start + lower[start..].find('>')? + 1;
    let rest = &markup[open_end..];
    let gap_len = rest.len() - rest.trim_start_matches([' ', '\t', '\n', '\r']).len();
    let next = &lower[open_end + gap_len..];
    if gap_len == 0 || !(next.starts_with("<head") || next.starts_with("<body")) {
        return None;
    }
    Some(if rest[..gap_len].contains('\n') { "\n" } else { " " })
}

fn collect_text(start: NodeRef<Node>, tail: &str) -> String {
    let mut out = String::new();
    let last_child_id = start.last_child().map(|c| c.id());
    for node in start.descendants() {
        if let Node::Text(text) = node.value() {
            let text: &str = if Some(node.id()) == last_child_id {
                match trim_relocated(text, tail) {
                    Some(t) => t,
                    None => continue,
                }
            } else {
                text
            };
            // 스크립트·스타일·템플릿 안쪽 글자는 안쪽 태그가 무엇이든 일반 문자열이 아니다.
            let hidden = node.ancestors().any(|a| element_name(&a).is_some_and(|n| NON_TEXT_CONTAINERS.contains(&n)));
            if !hidden {
                out.push_str(normalized_text(&node, text));
            }
        }
    }
    out
}

/// 본문(`body`가 있으면 body)의 `get_text()`.
pub fn get_body_text(markup: &str) -> String {
    let doc = parse(markup);
    let body = doc.tree.root().descendants().find(|n| element_name(n) == Some("body")).unwrap_or_else(|| doc.tree.root());
    collect_text(body, &relocated_tail(markup))
}

/// 문서 전체의 텍스트를 모은다. EPUB ZIP 대체 경로의 BeautifulSoup `get_text()` 대응.
pub fn get_document_text(markup: &str) -> String {
    let doc = parse(markup);
    collect_text(doc.tree.root(), &relocated_tail(markup))
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if crate::pytext::is_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

fn walk_lines(node: NodeRef<Node>, in_pre: bool, parts: &mut Vec<String>) {
    for child in node.children() {
        match child.value() {
            Node::Element(e) => {
                let name = e.name();
                if SKIPPED_TAGS.contains(&name) {
                    continue;
                }
                let block = BLOCK_TAGS.contains(&name);
                if block {
                    parts.push("\n".into());
                }
                walk_lines(child, in_pre || name == "pre", parts);
                if block {
                    parts.push("\n".into());
                }
            }
            Node::Text(text) => parts.push(if in_pre { text.to_string() } else { collapse_ws(text) }),
            _ => {}
        }
    }
}

/// 문단과 명시적 줄바꿈으로 행을 센다. 소스 포맷상의 줄바꿈과 빈 행은 세지 않는다. `pre` 안 줄바꿈은 본문이다.
pub fn count_lines(markup: &str) -> usize {
    let doc = parse(markup);
    let body = doc.tree.root().descendants().find(|n| element_name(n) == Some("body")).unwrap_or_else(|| doc.tree.root());
    let mut parts = Vec::new();
    walk_lines(body, false, &mut parts);
    parts.concat().lines().filter(|l| !l.trim_matches(crate::pytext::is_space).is_empty()).count()
}

/// HTML 파일: (요약, 줄 수, 쪽 수=0). UTF-8로 읽을 수 없으면 `Err`.
pub fn read_from_html(path: &Path) -> Result<(String, usize, usize), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let content = String::from_utf8(bytes).map_err(|e| format!("'utf-8' codec can't decode '{}': {e}", path.display()))?;
    let content = universal_newlines(&content);
    let line_count = content.matches('\n').count() + 1;
    let text = keep_word_chars(&get_text(&content));
    Ok((head_chars(&text, TEXT_SIZE).to_owned(), line_count, 0))
}
