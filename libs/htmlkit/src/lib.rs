//! HTML/XML 마크업 토큰화와 텍스트 추출.
//!
//! 토크나이저와 속성 엔티티 해석은 EPUB 보안 검사가 쓰고(`epubscan`), `text_of`는 BeautifulSoup
//! `html.parser`의 `get_text()`와 같은 텍스트를 만든다(`isbn`, 이후 `loader`).

mod entities;

use entities::ENTITIES;

#[derive(Debug)]
pub struct Tag {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub closing: bool,
    /// `<script>`·`<style>` 안의 원시 텍스트.
    pub body: Option<String>,
}

impl Tag {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn parse_tag(text: &str, start: usize) -> (Tag, usize) {
    let bytes = text.as_bytes();
    let closing = bytes[start + 1] == b'/';
    let mut i = start + 1 + usize::from(closing);
    let name_start = i;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'/' | b'>') {
        i += 1;
    }
    let name = text[name_start..i].to_lowercase();
    let mut attrs = Vec::new();
    while i < bytes.len() && bytes[i] != b'>' {
        if bytes[i].is_ascii_whitespace() || bytes[i] == b'/' {
            i += 1;
            continue;
        }
        let ns = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let key = text[ns..i].to_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && matches!(bytes[i], b'"' | b'\'') {
                let q = bytes[i];
                let vs = i + 1;
                i = vs;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                value = text[vs..i].to_owned();
                i = (i + 1).min(bytes.len());
            } else {
                let vs = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = text[vs..i].to_owned();
            }
        }
        attrs.push((key, value));
    }
    (Tag { name, attrs, closing, body: None }, (i + 1).min(bytes.len()))
}

/// 태그 목록. 주석·CDATA·선언은 건너뛰고, 닫히지 않은 주석은 나머지를 주석으로 본다(브라우저와 같다).
pub fn tokenize(text: &str) -> (Vec<Tag>, bool) {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut tags = Vec::new();
    let mut malformed = false;
    let mut i = 0;
    while let Some(off) = text[i..].find('<') {
        i += off;
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            // HTML 규칙: `<!-->`와 `<!--->`는 빈 주석이고 `--!>`도 주석을 닫는다. 닫히지 않은 주석은 파서마다
            // 해석이 달라(XML은 오류, HTML은 나머지 전부 주석) 뒤쪽도 계속 검사한다.
            let after = &text[i + 4..];
            if after.starts_with('>') {
                i += 5;
            } else if after.starts_with("->") {
                i += 6;
            } else {
                let close = [("-->", 3), ("--!>", 4)].iter().filter_map(|(m, l)| after.find(m).map(|e| (e, *l))).min();
                match close {
                    Some((e, l)) => i += 4 + e + l,
                    None => {
                        malformed = true;
                        i += 4;
                    }
                }
            }
        } else if rest.starts_with("<![CDATA[") {
            // XML에서는 CDATA, HTML에서는 가짜 주석이라 파서마다 안쪽을 다르게 본다. 안쪽도 마크업으로 검사한다.
            i += 9;
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            match text[i..].find('>') {
                Some(e) => i += e + 1,
                None => break,
            }
        } else if bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic) || (bytes.get(i + 1) == Some(&b'/') && bytes.get(i + 2).is_some_and(u8::is_ascii_alphabetic)) {
            let (mut tag, next) = parse_tag(text, i);
            i = next;
            let local = local_name(&tag.name).to_owned();
            if !tag.closing && (local == "script" || local == "style") {
                let close = format!("</{}", tag.name);
                let end = lower[i..].find(&close).map_or(text.len(), |e| i + e);
                tag.body = Some(text[i..end].to_owned());
                i = end;
            }
            tags.push(tag);
        } else {
            i += 1;
        }
    }
    (tags, malformed)
}

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '#' && c != '&').unwrap_or(rest.len());
        let body = &rest[1..end];
        let consumed = end + usize::from(rest[end..].starts_with(';'));
        let decoded = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = body.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match body {
                "tab" | "Tab" => Some('\t'),
                "newline" | "NewLine" => Some('\n'),
                "colon" => Some(':'),
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lpar" => Some('('),
                "rpar" => Some(')'),
                "sol" => Some('/'),
                "bsol" => Some('\\'),
                "nbsp" => Some('\u{a0}'),
                _ => None,
            }
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[consumed..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- text_of

/// 모르는 이름이거나 정의되지 않은 번호일 때 Python이 쓰는 대체 문자.
const REPLACEMENT: char = '\u{fffd}';

/// windows-1252에서 0x80~0x9f 자리. 정의되지 않은 자리는 `None`이다.
const CP1252_HIGH: [Option<char>; 32] = [
    Some('€'),
    None,
    Some('‚'),
    Some('ƒ'),
    Some('„'),
    Some('…'),
    Some('†'),
    Some('‡'),
    Some('ˆ'),
    Some('‰'),
    Some('Š'),
    Some('‹'),
    Some('Œ'),
    None,
    Some('Ž'),
    None,
    None,
    Some('‘'),
    Some('’'),
    Some('“'),
    Some('”'),
    Some('•'),
    Some('–'),
    Some('—'),
    Some('˜'),
    Some('™'),
    Some('š'),
    Some('›'),
    Some('œ'),
    None,
    Some('ž'),
    Some('Ÿ'),
];

/// BeautifulSoup `handle_charref`: 256 미만은 windows-1252로 읽고, 그 밖에는 코드포인트로 읽는다.
fn charref(number: u32) -> char {
    match number {
        0x80..=0x9f => CP1252_HIGH[(number - 0x80) as usize].or_else(|| char::from_u32(number)).unwrap_or(REPLACEMENT),
        n => char::from_u32(n).unwrap_or(REPLACEMENT),
    }
}

/// `&#...` 숫자 참조를 읽는다. 반환값은 (문자, 소비한 바이트 수)다.
/// html.parser와 같이 종결 문자(16진 숫자가 아닌 글자)가 있어야 참조로 본다. 종결 문자는 `;`일 때만 먹는다.
fn read_charref(s: &str) -> Option<(char, usize)> {
    let rest = s.strip_prefix("&#")?;
    let bytes = rest.as_bytes();
    let (hex, start) = if matches!(bytes.first(), Some(b'x' | b'X')) { (true, 1) } else { (false, 0) };
    let digits = bytes[start..].iter().take_while(|b| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() }).count();
    if digits == 0 {
        return None;
    }
    let end = start + digits;
    let terminator = *bytes.get(end)?;
    if terminator.is_ascii_hexdigit() {
        return None;
    }
    let number = u32::from_str_radix(&rest[start..end], if hex { 16 } else { 10 }).unwrap_or(u32::MAX);
    Some((charref(number), 2 + end + usize::from(terminator == b';')))
}

/// `&name` 이름 참조를 읽는다. 모르는 이름은 BeautifulSoup처럼 `&name`(세미콜론 없이)이 된다.
fn read_entityref(s: &str) -> Option<(String, usize)> {
    let rest = s.strip_prefix('&')?;
    let bytes = rest.as_bytes();
    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }
    let len = bytes.iter().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.')).count();
    // 이름은 영숫자를 모두 먹었으므로 종결 문자는 영숫자일 수 없다. 끝에 닿았다면 참조가 아니다.
    let terminator = *bytes.get(len)?;
    let name = &rest[..len];
    let text = match ENTITIES.binary_search_by(|(k, _)| (*k).cmp(name)) {
        Ok(i) => ENTITIES[i].1.to_owned(),
        Err(_) => format!("&{name}"),
    };
    Some((text, 1 + len + usize::from(terminator == b';')))
}

/// BeautifulSoup이 시작 태그에서 곧바로 닫는 void 요소(`HTMLTreeBuilder.empty_element_tags`).
const VOID_ELEMENTS: [&str; 24] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "menuitem", "meta", "param", "source", "track", "wbr", "basefont", "bgsound", "command", "frame", "image", "isindex",
    "nextid", "spacer",
];

/// 공백만 있는 텍스트 노드는 개행이 있으면 `"\n"`, 없으면 `" "` 하나로 줄인다(BeautifulSoup `endData`).
/// `<pre>`·`<textarea>` 안은 그대로 둔다. 노드 하나는 태그·주석·선언 사이의 연속 구간이다.
fn flush(out: &mut String, node: &mut String, preserve: bool) {
    let blank = !node.is_empty() && node.chars().all(|c| matches!(c, ' ' | '\n' | '\t' | '\u{c}' | '\r'));
    if blank && !preserve {
        out.push(if node.contains('\n') { '\n' } else { ' ' });
    } else {
        out.push_str(node);
    }
    node.clear();
}

/// BeautifulSoup(html, "html.parser").get_text()와 같은 텍스트. `<script>`·`<style>` 본문, 주석, 선언,
/// 처리 명령은 빼고 CDATA는 넣는다. 끝까지 닫히지 않은 태그는 버린다.
pub fn text_of(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let (mut out, mut node) = (String::new(), String::new());
    let mut preserve = 0usize;
    // 시작 태그에서 이미 닫은 void 요소 이름. 뒤따르는 `</meta>` 같은 끝 태그는 노드 경계가 아니다.
    let mut already_closed: Vec<String> = Vec::new();
    let mut i = 0;
    while i < html.len() {
        let rest = &html[i..];
        let c = rest.as_bytes()[0];
        if c == b'&' {
            if let Some((ch, used)) = read_charref(rest) {
                node.push(ch);
                i += used;
            } else if let Some((s, used)) = read_entityref(rest) {
                node.push_str(&s);
                i += used;
            } else {
                node.push('&');
                i += 1;
            }
        } else if c != b'<' {
            let end = rest.find(['&', '<']).unwrap_or(rest.len());
            node.push_str(&rest[..end]);
            i += end;
        } else if rest.starts_with("<!--") {
            flush(&mut out, &mut node, preserve > 0);
            match html[i + 4..].find("-->") {
                Some(e) => i += 4 + e + 3,
                None => break,
            }
        } else if rest.starts_with("<![CDATA[") {
            flush(&mut out, &mut node, preserve > 0);
            let (body, used, closed) = match html[i + 9..].find("]]>") {
                Some(e) => (&html[i + 9..i + 9 + e], 9 + e + 3, true),
                None => (&html[i + 9..], html.len() - i, false),
            };
            node.push_str(body);
            flush(&mut out, &mut node, preserve > 0);
            i += used;
            if !closed {
                break;
            }
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            flush(&mut out, &mut node, preserve > 0);
            match rest.find('>') {
                Some(e) => i += e + 1,
                None => break,
            }
        } else if bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic) || (bytes.get(i + 1) == Some(&b'/') && bytes.get(i + 2).is_some_and(u8::is_ascii_alphabetic)) {
            if !rest.contains('>') {
                break;
            }
            let (tag, next) = parse_tag(html, i);
            let self_closing = html[..next].ends_with("/>");
            i = next;
            let ignored_end = tag.closing && already_closed.iter().position(|n| *n == tag.name).map(|k| already_closed.remove(k)).is_some();
            if !ignored_end {
                flush(&mut out, &mut node, preserve > 0);
            }
            if !tag.closing && !self_closing && VOID_ELEMENTS.contains(&tag.name.as_str()) {
                already_closed.push(tag.name.clone());
            }
            let keeps_whitespace = tag.name == "pre" || tag.name == "textarea";
            if keeps_whitespace {
                preserve = if tag.closing { preserve.saturating_sub(1) } else { preserve + 1 };
            }
            let local = local_name(&tag.name).to_owned();
            if !tag.closing && (local == "script" || local == "style") {
                // 본문은 건너뛰고 닫는 태그부터 다시 읽는다. 닫는 태그가 없으면 나머지를 버린다.
                match lower[i..].find(&format!("</{}", tag.name)) {
                    Some(e) => i += e,
                    None => break,
                }
            }
        } else {
            node.push('<');
            i += 1;
        }
    }
    flush(&mut out, &mut node, preserve > 0);
    out
}
