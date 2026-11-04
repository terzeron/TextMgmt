//! BeautifulSoup `html.parser`로 하던 가벼운 탐색만 담은 작은 DOM. 표지 후보 추출이 쓰는 질의만 지원한다.

const VOID_ELEMENTS: [&str; 14] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

#[derive(Debug)]
enum Child {
    Node(usize),
    Text(String),
}

#[derive(Debug)]
struct Node {
    tag: String,
    attrs: Vec<(String, String)>,
    parent: Option<usize>,
    children: Vec<Child>,
}

pub struct Soup {
    nodes: Vec<Node>,
}

fn is_name_end(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'/' | b'>')
}

/// `<`에서 시작하는 태그를 읽는다. (이름, 속성, 닫는 태그 여부, 자체 닫힘 여부, 다음 위치)
fn read_tag(text: &str, start: usize) -> (String, Vec<(String, String)>, bool, bool, usize) {
    let bytes = text.as_bytes();
    let closing = bytes[start + 1] == b'/';
    let mut i = start + 1 + usize::from(closing);
    let name_start = i;
    while i < bytes.len() && !is_name_end(bytes[i]) {
        i += 1;
    }
    let name = text[name_start..i].to_lowercase();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    while i < bytes.len() && bytes[i] != b'>' {
        if bytes[i] == b'/' {
            self_closing = bytes.get(i + 1) == Some(&b'>');
            i += 1;
            continue;
        }
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let key_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let key = text[key_start..i].to_lowercase();
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
                let quote = bytes[i];
                let value_start = i + 1;
                i = value_start;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = htmlkit::decode_entities(&text[value_start..i]);
                i = (i + 1).min(bytes.len());
            } else {
                let value_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = htmlkit::decode_entities(&text[value_start..i]);
            }
        }
        if !key.is_empty() {
            attrs.push((key, value));
        }
    }
    (name, attrs, closing, self_closing, (i + 1).min(bytes.len()))
}

impl Soup {
    pub fn parse(html: &str) -> Soup {
        let mut nodes = vec![Node { tag: String::new(), attrs: Vec::new(), parent: None, children: Vec::new() }];
        let mut current = 0usize;
        let bytes = html.as_bytes();
        let lower = html.to_ascii_lowercase();
        let mut i = 0;
        while i < html.len() {
            let Some(off) = html[i..].find('<') else {
                nodes[current].children.push(Child::Text(htmlkit::decode_entities(&html[i..])));
                break;
            };
            if off > 0 {
                nodes[current].children.push(Child::Text(htmlkit::decode_entities(&html[i..i + off])));
            }
            i += off;
            let rest = &html[i..];
            if rest.starts_with("<!--") {
                i = html[i + 4..].find("-->").map_or(html.len(), |e| i + 4 + e + 3);
            } else if rest.starts_with("<!") || rest.starts_with("<?") {
                i = rest.find('>').map_or(html.len(), |e| i + e + 1);
            } else if bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic) || (bytes.get(i + 1) == Some(&b'/') && bytes.get(i + 2).is_some_and(u8::is_ascii_alphabetic)) {
                let (name, attrs, closing, self_closing, next) = read_tag(html, i);
                i = next;
                if closing {
                    let mut walk = Some(current);
                    while let Some(idx) = walk {
                        if idx != 0 && nodes[idx].tag == name {
                            current = nodes[idx].parent.unwrap_or(0);
                            break;
                        }
                        walk = nodes[idx].parent;
                    }
                    continue;
                }
                let idx = nodes.len();
                nodes.push(Node { tag: name.clone(), attrs, parent: Some(current), children: Vec::new() });
                nodes[current].children.push(Child::Node(idx));
                if name == "script" || name == "style" {
                    // 본문은 글자로 세지 않는다. 닫는 태그까지 건너뛴다.
                    let close = format!("</{name}");
                    i = lower[i..].find(&close).map_or(html.len(), |e| i + e);
                } else if !self_closing && !VOID_ELEMENTS.contains(&name.as_str()) {
                    current = idx;
                }
            } else {
                nodes[current].children.push(Child::Text("<".to_string()));
                i += 1;
            }
        }
        Soup { nodes }
    }

    pub fn root(&self) -> usize {
        0
    }

    pub fn tag(&self, idx: usize) -> &str {
        &self.nodes[idx].tag
    }

    pub fn attr(&self, idx: usize, name: &str) -> Option<&str> {
        self.nodes[idx].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// 문서 순서로 모든 자손 요소.
    pub fn descendants(&self, idx: usize) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect(idx, &mut out);
        out
    }

    fn collect(&self, idx: usize, out: &mut Vec<usize>) {
        for child in &self.nodes[idx].children {
            if let Child::Node(n) = child {
                out.push(*n);
                self.collect(*n, out);
            }
        }
    }

    pub fn find_all(&self, idx: usize, tag: &str) -> Vec<usize> {
        self.descendants(idx).into_iter().filter(|n| self.nodes[*n].tag == tag).collect()
    }

    pub fn parent(&self, idx: usize) -> Option<usize> {
        self.nodes[idx].parent.filter(|p| *p != 0)
    }

    pub fn has_ancestor(&self, idx: usize, pred: impl Fn(&Soup, usize) -> bool) -> bool {
        let mut walk = self.parent(idx);
        while let Some(p) = walk {
            if pred(self, p) {
                return true;
            }
            walk = self.parent(p);
        }
        false
    }

    pub fn has_class(&self, idx: usize, class: &str) -> bool {
        self.attr(idx, "class").is_some_and(|c| c.split_whitespace().any(|t| t == class))
    }

    /// `meta[attr="value"]`의 첫 요소의 `content`(없으면 빈 문자열).
    pub fn meta_content(&self, attr: &str, value: &str) -> Option<String> {
        self.find_all(0, "meta").into_iter().find(|m| self.attr(*m, attr) == Some(value)).map(|m| self.attr(m, "content").unwrap_or("").to_string())
    }

    /// 문자열 노드를 문서 순서로 모은다.
    fn strings(&self, idx: usize, out: &mut Vec<String>) {
        for child in &self.nodes[idx].children {
            match child {
                Child::Text(t) => out.push(t.clone()),
                Child::Node(n) => self.strings(*n, out),
            }
        }
    }

    /// `get_text()`.
    pub fn text(&self, idx: usize) -> String {
        let mut parts = Vec::new();
        self.strings(idx, &mut parts);
        parts.concat()
    }

    /// `get_text(strip=True)`.
    pub fn text_strip(&self, idx: usize) -> String {
        let mut parts = Vec::new();
        self.strings(idx, &mut parts);
        parts.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().concat()
    }

    /// `get_text(separator=" ", strip=True)`.
    pub fn text_strip_sep(&self, idx: usize) -> String {
        let mut parts = Vec::new();
        self.strings(idx, &mut parts);
        parts.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
    }
}
