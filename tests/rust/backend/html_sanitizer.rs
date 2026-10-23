//! HTML 미리보기 sanitizer(`BookManager::_sanitize_html_for_viewer`)의 적대적 입력 검증.
//! Python 테스트는 대표 사례만 확인한다. 여기서는 실행 가능한 구성요소가 출력에 남지 않는다는 보장을 여러 우회 형태로 확인한다.
//! 출력은 서버가 `Content-Security-Policy: sandbox`와 함께 내려주지만, sanitizer 자체도 실행 가능한 요소를 남기면 안 된다.

use backend::book_manager::BookManager;

const BASE: &str = "/api/books/1/html-resource";

fn sanitize(html: &str) -> String {
    BookManager::_sanitize_html_for_viewer(html, BASE)
}

/// 출력의 모든 태그에서 (태그 이름, 속성 이름 목록)을 뽑는다. 속성 값은 따옴표를 존중해서 건너뛴다(값 안의 텍스트는 속성이 아니다).
fn tags_with_attribute_names(output: &str) -> Vec<(String, Vec<String>)> {
    let chars: Vec<char> = output.chars().collect();
    let mut tags = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '<' || !chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        i += 1;
        let mut name = String::new();
        while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '/' | '>') {
            name.push(chars[i].to_ascii_lowercase());
            i += 1;
        }
        let mut attrs = Vec::new();
        while i < chars.len() && chars[i] != '>' {
            if chars[i].is_whitespace() || chars[i] == '/' {
                i += 1;
                continue;
            }
            let mut attr = String::new();
            while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '=' | '>' | '/') {
                attr.push(chars[i].to_ascii_lowercase());
                i += 1;
            }
            attrs.push(attr);
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '=' {
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                match chars.get(i) {
                    Some(&q) if q == '"' || q == '\'' => {
                        i += 1;
                        while i < chars.len() && chars[i] != q {
                            i += 1;
                        }
                        i += 1;
                    }
                    _ => {
                        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '>' {
                            i += 1;
                        }
                    }
                }
            }
        }
        tags.push((name, attrs));
    }
    tags
}

/// 출력에서 브라우저가 실행하거나 외부로 나가는 구성요소를 찾는다. 찾으면 이유를, 없으면 `None`을 돌려준다.
fn danger(output: &str) -> Option<String> {
    // 태그와 속성은 파싱해서 본다. 속성 값 안의 텍스트(`title="onerror=..."`)는 실행되지 않으므로 오탐하지 않는다.
    for (name, attrs) in tags_with_attribute_names(output) {
        if ["script", "iframe", "frame", "object", "embed", "form", "base"].contains(&name.as_str()) {
            return Some(format!("<{name}> 태그가 남았다"));
        }
        if name == "meta" && attrs.iter().any(|a| a == "http-equiv") {
            return Some("<meta http-equiv>가 남았다".to_string());
        }
        if let Some(attr) = attrs.iter().find(|a| a.starts_with("on") || ["srcdoc", "formaction"].contains(&a.as_str())) {
            return Some(format!("<{name}>의 {attr} 속성이 남았다"));
        }
    }
    // 속성 값은 엔티티와 공백·제어 문자를 접어서 `jav&#x09;ascript:` 같은 우회도 잡는다. 텍스트 안의 언급은 걸리지 않게 `="`/`='` 뒤만 본다.
    let folded: String = output.to_lowercase().replace("&#x09;", "").replace("&#9;", "").replace("&#x0a;", "").replace("&#10;", "").replace("&tab;", "").replace("&newline;", "").chars().filter(|c| !c.is_whitespace() && !c.is_control()).collect();
    for needle in ["=\"javascript:", "='javascript:", "=javascript:", "=\"vbscript:", "=\"data:text/html", "expression("] {
        if folded.contains(needle) {
            return Some(format!("{needle} 이(가) 남았다"));
        }
    }
    None
}

#[test]
fn removes_executable_tags_in_every_nesting() {
    for html in [
        "<script>alert(1)</script>",
        "<SCRIPT SRC=//evil/x.js></SCRIPT>",
        "<div><p><script>alert(1)</script></p></div>",
        "<svg><script>alert(1)</script></svg>",
        "<math><script>alert(1)</script></math>",
        "<iframe src=//evil></iframe>",
        "<iframe srcdoc=\"<script>alert(1)</script>\"></iframe>",
        "<object data=//evil/x.swf></object>",
        "<embed src=//evil/x.swf>",
        "<form action=//evil><input name=x></form>",
        "<base href=//evil/>",
        "<meta http-equiv=refresh content=\"0;url=javascript:alert(1)\">",
        "<frameset><frame src=//evil></frameset>",
    ] {
        let out = sanitize(html);
        assert_eq!(danger(&out), None, "입력 {html:?} -> 출력 {out:?}");
    }
}

#[test]
fn removes_event_handler_attributes_in_every_spelling() {
    for html in [
        "<img src=x onerror=alert(1)>",
        "<img src=x ONERROR=alert(1)>",
        "<img src=x OnErRoR = alert(1)>",
        "<img/src=x/onerror=alert(1)>",
        "<body onload=alert(1)>",
        "<svg onload=alert(1)>",
        "<div onmouseover=\"alert(1)\">x</div>",
        "<a href=#x onclick=alert(1)>x</a>",
        "<input autofocus onfocus=alert(1)>",
        "<video><source onerror=alert(1)></video>",
        "<details open ontoggle=alert(1)>",
    ] {
        let out = sanitize(html);
        assert_eq!(danger(&out), None, "입력 {html:?} -> 출력 {out:?}");
    }
}

#[test]
fn script_urls_are_dropped_from_every_url_attribute() {
    for html in [
        "<a href=\"javascript:alert(1)\">x</a>",
        "<a href=\"  JaVaScRiPt:alert(1)\">x</a>",
        "<a href=\"jav&#x09;ascript:alert(1)\">x</a>",
        "<a href=\"jav&#10;ascript:alert(1)\">x</a>",
        "<a href=\"&#106;avascript:alert(1)\">x</a>",
        "<a href=\"java\nscript:alert(1)\">x</a>",
        "<a href=\"\u{1}javascript:alert(1)\">x</a>",
        "<a href=\"vbscript:msgbox(1)\">x</a>",
        "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>",
        "<img src=\"javascript:alert(1)\">",
        "<video poster=\"javascript:alert(1)\"></video>",
        "<svg><a xlink:href=\"javascript:alert(1)\"><text>x</text></a></svg>",
        "<svg><a href=\"javascript:alert(1)\"><text>x</text></a></svg>",
        "<svg><use href=\"data:image/svg+xml;base64,PHN2Zz48c2NyaXB0PmFsZXJ0KDEpPC9zY3JpcHQ+PC9zdmc+#x\"/></svg>",
        "<button formaction=\"javascript:alert(1)\">x</button>",
        "<link rel=stylesheet href=\"javascript:alert(1)\">",
    ] {
        let out = sanitize(html);
        assert_eq!(danger(&out), None, "입력 {html:?} -> 출력 {out:?}");
    }
}

#[test]
fn links_keep_only_in_page_anchors_and_get_a_safe_rel() {
    let out = sanitize("<a href=\"#chapter1\">a</a><a href=\"http://evil.test/x\">b</a><a href=\"chapter2.html\">c</a>");
    assert!(out.contains("href=\"#chapter1\""), "{out}");
    assert!(!out.contains("evil.test"), "{out}");
    assert!(!out.contains("chapter2.html"), "{out}");
    assert!(out.matches("nofollow noopener noreferrer").count() >= 3, "{out}");
}

#[test]
fn external_resources_are_not_loaded() {
    for html in ["<img src=\"http://evil.test/t.png\">", "<img src=\"//evil.test/t.png\">", "<link rel=stylesheet href=\"http://evil.test/a.css\">", "<video poster=\"https://evil.test/p.png\"></video>"] {
        let out = sanitize(html);
        assert!(!out.contains("evil.test"), "입력 {html:?} -> 출력 {out:?}");
    }
}

#[test]
fn mutation_xss_vectors_do_not_leave_executable_content() {
    // 파서가 다르면 한 번 걸러낸 뒤 다시 해석할 때 살아나는 형태(mXSS). 출력을 다시 sanitize해도 같아야 한다.
    for html in [
        "<math><mtext><table><mglyph><style><!--</style><img title=\"--&gt;&lt;img src=1 onerror=alert(1)&gt;\">",
        "<svg></p><style><a id=\"</style><img src=1 onerror=alert(1)>\">",
        "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\">",
        "<form><math><mtext></form><form><mglyph><style></math><img src onerror=alert(1)>",
        "<a href=\"#\" title=\"&quot; onmouseover=&quot;alert(1)\">x</a>",
    ] {
        let once = sanitize(html);
        let twice = sanitize(&once);
        assert_eq!(danger(&once), None, "1차: 입력 {html:?} -> 출력 {once:?}");
        assert_eq!(danger(&twice), None, "2차: 입력 {html:?} -> 출력 {twice:?}");
    }
}

#[test]
fn ordinary_book_markup_survives() {
    let html = "<html><body><h1 id=\"t\">제목</h1><p class=\"a\">본문 <b>굵게</b> <i>기울임</i></p><ul><li>하나</li></ul><table><tr><td>셀</td></tr></table></body></html>";
    let out = sanitize(html);
    for text in ["제목", "본문", "굵게", "기울임", "하나", "셀"] {
        assert!(out.contains(text), "{text} 이(가) 사라졌다: {out}");
    }
    assert_eq!(danger(&out), None);
}
