//! HTML 미리보기 sanitizer(`BookManager::sanitize_html_for_viewer`)의 적대적 입력 검증.
//! 실행 가능한 구성요소가 출력에 남지 않는다는 보장을 여러 우회 형태로 확인한다.
//! 출력은 서버가 `Content-Security-Policy: sandbox`와 함께 내려주지만, sanitizer 자체도 실행 가능한 요소를 남기면 안 된다.

#[path = "../common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{block_on, deps, make_doc, make_manager, tmp_dir};
use backend::book_manager::BookManager;
use backend::main::BookResponse;
use common::es_fake::FakeEsManager;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::Arc;
use std::{fs, io};

const BASE: &str = "/api/books/1/html-resource";

fn sanitize(html: &str) -> String {
    BookManager::sanitize_html_for_viewer(html, BASE)
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
    let folded: String = output
        .to_lowercase()
        .replace("&#x09;", "")
        .replace("&#9;", "")
        .replace("&#x0a;", "")
        .replace("&#10;", "")
        .replace("&tab;", "")
        .replace("&newline;", "")
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
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
    for html in
        ["<img src=\"http://evil.test/t.png\">", "<img src=\"//evil.test/t.png\">", "<link rel=stylesheet href=\"http://evil.test/a.css\">", "<video poster=\"https://evil.test/p.png\"></video>"]
    {
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

/// `BookResponse` 읽기 도우미(테스트 로컬). `is_file`은 본문이 있으면 참이다(파일을 읽어 담은 응답).
trait ResponseExt {
    fn header(&self, name: &str) -> Option<&str>;
    fn is_file(&self) -> bool;
}

impl ResponseExt for BookResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn is_file(&self) -> bool {
        !self.body.is_empty()
    }
}

#[test]
fn test_get_html_resource_allows_local_whitelisted_files_only() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    let html_file = tmp_path.join("A").join("test.html");
    fs::write(&html_file, "<html></html>").unwrap();
    let css_file = tmp_path.join("A").join("style.css");
    fs::write(&css_file, "body{}").unwrap();
    let doc = make_doc("A/test.html", "html");
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(doc));

    let resp = block_on(manager.get_html_resource(1, "style.css"));
    assert!(resp.is_file());
    assert_eq!(resp.status_code, 200);
    assert!(resp.header("content-security-policy").expect("csp").starts_with("sandbox;"));
    assert_eq!(resp.header("x-content-type-options"), Some("nosniff"));

    let bad = block_on(manager.get_html_resource(1, "../secret.txt"));
    assert_eq!(bad.status_code, 400);

    let unsupported = tmp_path.join("A").join("script.js");
    fs::write(&unsupported, "alert(1)").unwrap();
    let resp2 = block_on(manager.get_html_resource(1, "script.js"));
    assert_eq!(resp2.status_code, 400);
}

#[test]
fn test_svg_download_blocks_active_content() {
    for name in ["untrusted.svg", "untrusted.SVG", "한글 표지.svg"] {
        let (media_type, headers) = BookManager::file_response_meta(Path::new(name));
        assert_eq!(media_type, "image/svg+xml");
        let header = |name: &str| headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str());
        assert!(header("Content-Disposition").is_some_and(|v| v.starts_with("attachment;")));
        assert!(header("Content-Security-Policy").is_some_and(|v| v.starts_with("sandbox;") && v.contains("script-src 'none'")));
        assert_eq!(header("X-Content-Type-Options"), Some("nosniff"));
    }
}

// ── sanitize_html_for_viewer: external src 속성 제거 ─────────────

#[test]
fn test_sanitize_html_external_src_attribute_removed() {
    // 외부 URL src 는 build_html_resource_url 이 None 을 반환하므로 속성이 제거된다.
    let html = r#"<html><body><img src="http://external.com/img.png"></body></html>"#;
    let result = BookManager::sanitize_html_for_viewer(html, "/base");
    let dom = scraper::Html::parse_document(&result);
    let sel = scraper::Selector::parse("img").unwrap();
    let img = dom.select(&sel).next();
    assert!(img.is_none() || img.unwrap().value().attr("src").is_none());
}

#[test]
fn test_empty_string_returns_none() {
    assert_eq!(BookManager::build_html_resource_url("/base", ""), None);
}

#[test]
fn test_whitespace_only_returns_none() {
    assert_eq!(BookManager::build_html_resource_url("/base", "   "), None);
}

#[test]
fn test_data_uri_returned_as_is() {
    let uri = "data:image/png;base64,abc123";
    assert_eq!(BookManager::build_html_resource_url("/base", uri), Some(uri.to_string()));
}

#[test]
fn test_blob_uri_returned_as_is() {
    let uri = "blob:http://example.com/uuid";
    assert_eq!(BookManager::build_html_resource_url("/base", uri), Some(uri.to_string()));
}

#[test]
fn test_external_http_url_returns_none() {
    assert_eq!(BookManager::build_html_resource_url("/base", "http://external.com/img.png"), None);
}

#[test]
fn test_protocol_relative_url_returns_none() {
    assert_eq!(BookManager::build_html_resource_url("/base", "//cdn.example.com/img.png"), None);
}

#[test]
fn test_missing_required_es_fields_returns_server_error() {
    let tmp = tmp_dir();
    let html = tmp.path().join("A/book.html");
    fs::create_dir_all(html.parent().unwrap()).unwrap();
    fs::write(&html, "<html/>").unwrap();
    fs::write(html.parent().unwrap().join("style.css"), "body{}").unwrap();

    let mut doc = make_doc("A/book.html", "html");
    doc.as_object_mut().unwrap().remove("category");
    let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));
    let resp = block_on(manager.get_html_resource(1, "style.css"));

    assert_eq!(resp.status_code, 500);
}

#[test]
fn test_book_not_found_returns_404() {
    // ES 에 책이 없으면 404.
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    let resp = block_on(manager.get_html_resource(99, "image.jpg"));
    assert_eq!(resp.status_code, 404);
}

#[test]
fn test_non_html_file_returns_400() {
    // HTML 이 아닌 파일(.epub)은 400.
    let tmp = tmp_dir();
    let doc = make_doc("A/book.epub", "epub");
    let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));
    let resp = block_on(manager.get_html_resource(1, "image.jpg"));
    assert_eq!(resp.status_code, 400);
}

#[test]
fn test_path_traversal_via_symlink_returns_400() {
    // html_dir 밖을 가리키는 심볼릭 링크는 400.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let html_file = tmp_path.join("A").join("book.html");
    fs::create_dir_all(html_file.parent().unwrap()).unwrap();
    fs::write(&html_file, "<html/>").unwrap();

    // html_dir 밖 파일을 가리키는 심볼릭 링크 생성
    let outside = tmp_path.join("secret.jpg");
    fs::File::create(&outside).unwrap();
    let link = html_file.parent().unwrap().join("evil.jpg");
    symlink(&outside, &link).unwrap();

    let doc = make_doc("A/book.html", "html");
    let manager = make_manager(tmp_path, &FakeEsManager::with_doc(doc));
    let resp = block_on(manager.get_html_resource(1, "evil.jpg"));
    assert_eq!(resp.status_code, 400);
}

#[test]
fn test_resource_file_not_found_returns_404() {
    // html_dir 내 존재하지 않는 리소스는 404.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let html_file = tmp_path.join("A").join("book.html");
    fs::create_dir_all(html_file.parent().unwrap()).unwrap();
    fs::write(&html_file, "<html/>").unwrap();

    let doc = make_doc("A/book.html", "html");
    let manager = make_manager(tmp_path, &FakeEsManager::with_doc(doc));
    let resp = block_on(manager.get_html_resource(1, "nonexistent.jpg"));
    assert_eq!(resp.status_code, 404);
}

#[test]
fn test_is_relative_to_oserror_returns_400() {
    // is_relative_to 가 OSError 를 던지면 400.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let html_file = tmp_path.join("A").join("book.html");
    fs::create_dir_all(html_file.parent().unwrap()).unwrap();
    fs::write(&html_file, "<html/>").unwrap();
    fs::File::create(html_file.parent().unwrap().join("image.jpg")).unwrap();

    // 가정한 seam: 경로 포함 관계 검사(is_relative_to)를 실패시키는 훅. 문서에 없는 seam이다(파일 머리말 참고).
    let doc = make_doc("A/book.html", "html");
    let es = FakeEsManager::with_doc(doc);
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_path_is_relative_to(Arc::new(|_path, _other| Err(io::Error::other("path too long")))));
    let resp = block_on(manager.get_html_resource(1, "image.jpg"));
    assert_eq!(resp.status_code, 400);
}
