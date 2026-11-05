//! 문서 한 개의 글을 정리한다: HTML·XHTML·SVG·XML 마크업에서 비렌더링 요소와 위험 속성을, CSS에서 원격 로드와 스크립트 훅을 지운다.

use super::*;

/// 시작 태그째로 지우고 끝 태그도 지우되 안쪽 내용은 남기는 요소.
const DROP_TAGS: &[&str] = &["base", "form", "input", "textarea", "select", "button"];
/// 내용까지 통째로 지우는 요소.
/// raw text처럼 파싱되는 요소(`noscript`, `xmp`, `noframes`, `plaintext`)도 같이 지운다. 브라우저와 스캐너가 내용을 다르게 끊을 수 있는 자리다.
const REMOVE_ELEMENTS: &[&str] = &["script", "object", "embed", "applet", "iframe", "frame", "frameset", "noembed", "foreignobject", "noscript", "xmp", "noframes", "plaintext"];
const URL_ATTRS: &[&str] = &["href", "src", "action", "formaction", "background", "poster", "data", "codebase", "manifest", "ping", "srcset"];

static CSS_IMPORT: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)@import\s+[^;{}]*;?"));
static CSS_URL: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)(?:url|src)\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s]*))\s*\)"#));
static CSS_IMAGE_SET: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)(?:-webkit-)?image-set\((?:[^()"']|"[^"]*"|'[^']*'|\([^()]*\))*\)"#));
static CSS_STRING: LazyLock<Regex> = LazyLock::new(|| re(r#""([^"]*)"|'([^']*)'"#));
static CSS_EXPRESSION: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)expression\s*\((?:[^()]|\([^()]*\))*\)"));
static CSS_HOOKS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:-moz-binding|behavior)\s*:[^;}]*"));
static XML_DECL: LazyLock<Regex> = LazyLock::new(|| re(r"^<\?xml\s[^<>?]*\?>"));

/// CSS 이스케이프(`\75`, `\75 `, `\u`)를 푼다. 줄바꿈 앞의 `\`는 이어 붙임이라 지운다.
fn css_unescape(css: &str) -> String {
    // CSS 규격의 입력 전처리: CRLF·CR·FF는 LF 하나로, NUL은 U+FFFD로 바꾼다. 이스케이프 뒤의 CRLF가 공백 하나로 먹히게 한다.
    let normalized = css.replace("\r\n", "\n").replace(['\r', '\x0c'], "\n").replace('\0', "\u{FFFD}");
    let chars: Vec<char> = normalized.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if chars[j] == '\n' {
            i = j + 1;
            continue;
        }
        let start = j;
        while j < chars.len() && j - start < 6 && chars[j].is_ascii_hexdigit() {
            j += 1;
        }
        if j > start {
            let code = u32::from_str_radix(&chars[start..j].iter().collect::<String>(), 16).unwrap_or(0xFFFD);
            out.push(char::from_u32(code).filter(|c| *c != '\0').unwrap_or('\u{FFFD}'));
            if j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
        } else {
            out.push(chars[j]);
            j += 1;
        }
        i = j;
    }
    out
}

/// CSS에서 원격 로드, 스크립트 훅(`expression()`, `-moz-binding`, `behavior`)을 지운다.
/// 이스케이프로 감춘 위험(`\75rl(...)`)은 이스케이프를 푼 글로 찾는다. 푼 글에서 지울 것이 나오면 푼 글을 정리해 쓰고,
/// 아니면 원문을 그대로 둔다(한자 글꼴 이름의 `\5B8B` 같은 정상 이스케이프를 보존한다).
pub fn strip_css(css: &str) -> String {
    if css.contains('\\') {
        let unescaped = css_unescape(css);
        let cleaned = strip_css_plain(&unescaped);
        if cleaned != unescaped {
            return cleaned;
        }
    }
    strip_css_plain(css)
}

fn strip_css_plain(css: &str) -> String {
    let no_import = CSS_IMPORT.replace_all(css, |c: &regex::Captures| {
        let m = &c[0];
        let n = normalize_url(m);
        if n.contains("//") || n.contains("javascript:") || n.contains("vbscript:") || n.contains("data:") { String::new() } else { m.to_owned() }
    });
    let no_url = CSS_URL.replace_all(&no_import, |c: &regex::Captures| {
        let target = c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)).map_or("", |m| m.as_str());
        if keep_url(target, true) { c[0].to_owned() } else { "url()".to_owned() }
    });
    // `image-set()`의 문자열 인자와 `url()`은 모두 URL이다. 하나라도 남기면 안 되는 주소면 함수째 `none`으로 바꾼다.
    let no_set = CSS_IMAGE_SET.replace_all(&no_url, |c: &regex::Captures| {
        let all_local = CSS_STRING.captures_iter(&c[0]).all(|s| keep_url(s.get(1).or_else(|| s.get(2)).map_or("", |m| m.as_str()), true)) && !c[0].to_ascii_lowercase().contains("//");
        if all_local { c[0].to_owned() } else { "none".to_owned() }
    });
    let no_expr = CSS_EXPRESSION.replace_all(&no_set, "");
    CSS_HOOKS.replace_all(&no_expr, "").into_owned()
}

// ---------------------------------------------------------------- 마크업

struct Attr<'a> {
    name: &'a str,
    value: Option<&'a str>,
    start: usize,
    end: usize,
    vstart: usize,
    vend: usize,
}

struct StartTag<'a> {
    text: &'a str,
    name: &'a str,
    attrs: Vec<Attr<'a>>,
    self_closing: bool,
    terminated: bool,
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

/// 시작 태그의 길이와 `>`로 닫혔는지. 따옴표는 `=` 바로 뒤에서 열릴 때만 따옴표로 본다(`it's` 같은 글이 태그를 삼키지 않게).
fn tag_len(rest: &str) -> (usize, bool) {
    let b = rest.as_bytes();
    let (mut quote, mut prev) = (0u8, 0u8);
    for (i, &c) in b.iter().enumerate().skip(1) {
        if quote != 0 {
            if c == quote {
                quote = 0;
                prev = c;
            }
            continue;
        }
        if (c == b'"' || c == b'\'') && prev == b'=' {
            quote = c;
        } else if c == b'>' {
            return (i + 1, true);
        }
        if !c.is_ascii_whitespace() {
            prev = c;
        }
    }
    (b.len(), false)
}

fn parse_start_tag(rest: &str) -> Option<StartTag<'_>> {
    if !rest.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let (len, terminated) = tag_len(rest);
    let text = &rest[..len];
    let b = text.as_bytes();
    let mut i = 1;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'/' && b[i] != b'>' {
        i += 1;
    }
    let name = &text[1..i];
    let body_end = if terminated { b.len() - 1 } else { b.len() };
    let trimmed = text[..body_end].trim_end();
    let self_closing = terminated && trimmed.ends_with('/');
    let limit = if self_closing { trimmed.len() - 1 } else { body_end };
    let mut attrs = Vec::new();
    while i < limit {
        while i < limit && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= limit {
            break;
        }
        let start = i;
        while i < limit && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let attr_name = &text[start..i];
        let mut j = i;
        while j < limit && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < limit && b[j] == b'=' {
            j += 1;
            while j < limit && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let (vstart, vend, end) = if j < limit && (b[j] == b'"' || b[j] == b'\'') {
                let q = b[j];
                let close = text[j + 1..limit].find(q as char).map_or(limit, |k| j + 1 + k);
                (j + 1, close, (close + 1).min(limit))
            } else {
                let mut k = j;
                while k < limit && !b[k].is_ascii_whitespace() {
                    k += 1;
                }
                (j, k, k)
            };
            attrs.push(Attr { name: attr_name, value: Some(&text[vstart..vend]), start, end, vstart, vend });
            i = end;
        } else {
            attrs.push(Attr { name: attr_name, value: None, start, end: i, vstart: i, vend: i });
        }
    }
    Some(StartTag { text, name, attrs, self_closing, terminated })
}

/// `from`부터 같은 이름의 요소가 닫힐 때까지의 끝 위치. 같은 이름이 안에 중첩되면 깊이를 센다. 닫히지 않으면 문서 끝이다.
fn skip_element(src: &str, from: usize, full_name: &str) -> usize {
    let lower = src[from..].to_ascii_lowercase();
    let (open, close) = (format!("<{}", full_name.to_ascii_lowercase()), format!("</{}", full_name.to_ascii_lowercase()));
    let name_char = |b: Option<&u8>| b.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':'));
    let (mut depth, mut pos) = (1usize, 0usize);
    loop {
        let (o, c) = (lower[pos..].find(&open), lower[pos..].find(&close));
        match (o, c) {
            (_, None) => return src.len(),
            (Some(o), Some(c)) if o < c => {
                let at = pos + o + open.len();
                if !name_char(lower.as_bytes().get(at)) {
                    let (len, _) = tag_len(&lower[pos + o..]);
                    if !lower[pos + o..pos + o + len].trim_end_matches('>').trim_end().ends_with('/') {
                        depth += 1;
                    }
                }
                pos = at;
            }
            (_, Some(c)) => {
                let at = pos + c + close.len();
                if name_char(lower.as_bytes().get(at)) {
                    pos = at;
                    continue;
                }
                depth -= 1;
                pos = at;
                if depth == 0 {
                    return from + lower[pos..].find('>').map_or(lower.len(), |g| pos + g + 1);
                }
            }
        }
    }
}

/// 선언(`<!DOCTYPE …>`)의 끝. 내부 부분집합(`[ … ]`) 안의 `>`는 끝이 아니다.
fn decl_end(rest: &str) -> usize {
    let (mut depth, mut quote) = (0i32, 0u8);
    for (i, &c) in rest.as_bytes().iter().enumerate() {
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => quote = c,
            b'[' => depth += 1,
            b']' => depth -= 1,
            b'>' if depth <= 0 => return i + 1,
            _ => {}
        }
    }
    rest.len()
}

fn simplified_doctype(decl: &str) -> String {
    let name: String = decl["<!doctype".len()..].trim_start().chars().take_while(|c| !c.is_whitespace() && !matches!(c, '[' | '>' | '/')).collect();
    format!("<!DOCTYPE {}>", if name.is_empty() { "html" } else { &name })
}

fn rewrite_tag(tag: &StartTag<'_>, lname: &str, changes: &mut Changes) -> String {
    // (시작, 끝, 대체) 편집. 지우는 속성은 앞의 공백도 함께 지운다.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let text = tag.text;
    let drop_attr = |a: &Attr<'_>| {
        let mut s = a.start;
        while s > 0 && text.as_bytes()[s - 1].is_ascii_whitespace() {
            s -= 1;
        }
        (s, a.end, String::new())
    };
    let quoted = |a: &Attr<'_>| a.vstart > 0 && matches!(text.as_bytes()[a.vstart - 1], b'"' | b'\'');
    for a in &tag.attrs {
        let an = local(a.name);
        let value = a.value.unwrap_or("");
        let edits_before = edits.len();
        if a.name.contains(['<', '"', '\'']) {
            // 이름에 `<`나 따옴표가 든 속성은 파서마다 다르게 읽는다.
            edits.push(drop_attr(a));
            changes.add("비정상 속성 이름");
        } else if an.len() >= 4 && an.starts_with("on") && an[2..].bytes().all(|b| b.is_ascii_lowercase()) {
            edits.push(drop_attr(a));
            changes.add("이벤트 핸들러 속성");
        } else if matches!(an.as_str(), "srcdoc" | "formaction") {
            edits.push(drop_attr(a));
            changes.add("위험 속성");
        } else if an == "style" && a.value.is_some() {
            let decoded = decode_entities(value);
            let cleaned = strip_css(&decoded);
            if cleaned != decoded {
                // 원래 따옴표 종류(또는 따옴표 없음)와 무관하게 큰따옴표로 감싼 값으로 통째로 바꾼다.
                let (from, to) = if quoted(a) { (a.vstart - 1, a.end.max(a.vend)) } else { (a.vstart, a.vend) };
                edits.push((from, to, format!("\"{}\"", htmlkit::escape_html(&cleaned))));
                changes.add("style 속성 정리");
            }
        } else if URL_ATTRS.contains(&an.as_str()) && a.value.is_some() {
            let resource = is_resource_attr(lname, &an);
            let candidates: Vec<&str> = if an == "srcset" { value.split(',').filter_map(|c| c.split_whitespace().next()).collect() } else { vec![value] };
            if candidates.iter().any(|c| !keep_url(c, resource)) {
                edits.push(drop_attr(a));
                changes.add(if resource { "원격·위험 자원 속성" } else { "위험 링크 속성" });
            }
        }
        // 남기는 속성값의 `<`·`>`는 항상 이스케이프한다. `</noscript>`처럼 raw text 문맥을 닫는 글이 값 안에 숨지 못하게 한다.
        if edits.len() == edits_before && a.value.is_some() && value.contains(['<', '>']) {
            let escaped = value.replace('<', "&lt;").replace('>', "&gt;");
            if quoted(a) {
                edits.push((a.vstart, a.vend, escaped));
            } else {
                edits.push((a.vstart, a.vend, format!("\"{escaped}\"")));
            }
            changes.add("속성값 꺾쇠 이스케이프");
        }
    }
    if edits.is_empty() {
        return text.to_owned();
    }
    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (s, e, rep) in edits {
        if s < at {
            continue;
        }
        out.push_str(&text[at..s]);
        out.push_str(&rep);
        at = e;
    }
    out.push_str(&text[at..]);
    out
}

/// HTML·XHTML·SVG·XML 텍스트에서 비렌더링 요소를 지운다. 같은 입력이면 같은 결과다.
pub fn strip_markup(src: &str, svg: bool) -> (String, Changes) {
    // 한 번 정리한 결과를 다시 정리해도 같아질 때까지 반복한다(정리 결과가 새 위험을 만들지 못하게 한다).
    let (mut text, mut changes) = strip_pass(src, svg);
    for _ in 0..4 {
        let (next, more) = strip_pass(&text, svg);
        if next == text {
            break;
        }
        text = next;
        changes.merge(more);
    }
    (text, changes)
}

fn strip_pass(src: &str, _svg: bool) -> (String, Changes) {
    let mut out = String::with_capacity(src.len());
    let mut ch = Changes::default();
    let mut i = 0;
    while i < src.len() {
        if src.as_bytes()[i] != b'<' {
            let j = src[i..].find('<').map_or(src.len(), |k| i + k);
            out.push_str(&src[i..j]);
            i = j;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            // 주석은 통째로 지운다. `<!-->`와 `<!--->`도 완전한 주석이다(`-->`를 `<!--`의 대시부터 찾는다).
            // 브라우저와 끝나는 곳이 달라도 지우는 쪽으로만 어긋나므로 안에 숨긴 마크업이 남지 않는다.
            let end = rest.find("-->").map_or(rest.len(), |k| k + 3);
            ch.add("주석");
            i += end;
        } else if rest.starts_with("<![CDATA[") {
            // CDATA는 마크업이 아니라 이스케이프한 글자로 남긴다.
            let close = rest.find("]]>");
            out.push_str(&htmlkit::escape_text(&rest[9..close.unwrap_or(rest.len())]));
            ch.add("CDATA 섹션");
            i += close.map_or(rest.len(), |k| k + 3);
        } else if rest.starts_with("<?") {
            // XML 선언만 문서 맨 앞에서 남긴다. 그 밖의 처리 명령은 브라우저처럼 첫 `>`에서 끝난 것으로 보고 지운다.
            let at_start = out.is_empty() || out == "\u{feff}";
            if at_start && let Some(m) = XML_DECL.find(rest) {
                out.push_str(m.as_str());
                i += m.end();
            } else {
                let end = rest.find('>').map_or(rest.len(), |k| k + 1);
                ch.add(if rest[..end].to_ascii_lowercase().starts_with("<?xml-stylesheet") { "xml-stylesheet" } else { "처리 명령" });
                i += end;
            }
        } else if rest.starts_with("<!") {
            // 브라우저는 모든 선언을 첫 `>`에서 끝낸다. DOCTYPE의 내부 부분집합(`[ … ]`)만 XML 규칙으로 통째로 지운다.
            let first_gt = rest.find('>').map_or(rest.len(), |k| k + 1);
            if rest[..first_gt].to_ascii_lowercase().starts_with("<!doctype") {
                if rest[..first_gt].contains('[') {
                    let end = decl_end(rest);
                    out.push_str(&simplified_doctype(&rest[..end]));
                    ch.add("DOCTYPE 내부 부분집합");
                    i += end;
                } else {
                    out.push_str(&rest[..first_gt]);
                    i += first_gt;
                }
            } else {
                ch.add("알 수 없는 선언");
                i += first_gt;
            }
        } else if rest.starts_with("</") {
            let end = rest.find('>').map_or(rest.len(), |k| k + 1);
            let name = local(rest[2..end].trim_end_matches('>').trim());
            if !(DROP_TAGS.contains(&name.as_str()) || REMOVE_ELEMENTS.contains(&name.as_str())) {
                out.push_str(&rest[..end]);
            }
            i += end;
        } else if let Some(tag) = parse_start_tag(rest) {
            i += tag.text.len();
            let lname = local(tag.name);
            let attr = |n: &str| tag.attrs.iter().find(|a| local(a.name) == n).and_then(|a| a.value);
            if !tag.terminated {
                ch.add("닫히지 않은 태그");
            } else if REMOVE_ELEMENTS.contains(&lname.as_str()) {
                ch.add(match lname.as_str() {
                    "script" => "script 요소",
                    "foreignobject" => "foreignObject 요소",
                    _ => "object·embed·iframe류 요소",
                });
                if !(tag.self_closing || lname == "embed") {
                    i = skip_element(src, i, tag.name);
                }
            } else if matches!(lname.as_str(), "animate" | "set" | "animatetransform" | "animatemotion") && attr("attributename").is_some_and(|v| local(v) == "href") {
                ch.add("href 재작성 애니메이션");
                if !tag.self_closing {
                    i = skip_element(src, i, tag.name);
                }
            } else if lname == "meta" && attr("http-equiv").is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh")) {
                ch.add("meta refresh");
            } else if lname == "link" && attr("href").is_some_and(|h| !keep_url(h, true)) {
                ch.add("원격 link");
            } else if DROP_TAGS.contains(&lname.as_str()) {
                ch.add(if lname == "base" { "base 요소" } else { "폼 요소" });
            } else if lname == "style" && !tag.self_closing {
                let close = src[i..].to_ascii_lowercase().find("</style").map_or(src.len(), |k| i + k);
                let css = &src[i..close];
                // XHTML·SVG에서는 `<style>` 안의 마크업도 실제로 파싱된다. CDATA 표지를 빼고도 `<`가 남으면 요소째 지운다.
                if css.replace("<![CDATA[", "").replace("]]>", "").contains('<') {
                    ch.add("style 내 마크업");
                    i = skip_element(src, i, tag.name);
                } else {
                    out.push_str(&rewrite_tag(&tag, &lname, &mut ch));
                    let cleaned = strip_css(css);
                    if cleaned != css {
                        ch.add("style 요소 정리");
                    }
                    out.push_str(&cleaned);
                    i = close;
                }
            } else {
                out.push_str(&rewrite_tag(&tag, &lname, &mut ch));
            }
        } else {
            out.push('<');
            i += 1;
        }
    }
    (out, ch)
}
