//! `cover_manager::cover_sources::html_dom` 단위 테스트. 표지 후보 추출이 쓰는 작은 DOM 질의를 본다.

use cover_manager::cover_sources::html_dom::HtmlDom;

const PAGE: &str = r#"<html><head>
<meta property="og:image" content="https://img.example/cover.jpg">
<meta name="description">
</head><body>
<div class="book main"><a href="/b/1"><img src="a.jpg" alt="표지"></a><span>제목&nbsp;하나</span></div>
<div class="other"><p>  둘째   문단 </p><br/><img src="b.jpg"></div>
</body></html>"#;

#[test]
fn find_all_and_attr_follow_document_order() {
    let dom = HtmlDom::parse(PAGE);
    let imgs = dom.find_all(dom.root(), "img");
    assert_eq!(imgs.len(), 2);
    assert_eq!(dom.attr(imgs[0], "src"), Some("a.jpg"));
    assert_eq!(dom.attr(imgs[1], "src"), Some("b.jpg"));
    assert_eq!(dom.attr(imgs[1], "alt"), None);
    assert_eq!(dom.tag(imgs[0]), "img");
}

#[test]
fn parent_ancestor_and_class_checks() {
    let dom = HtmlDom::parse(PAGE);
    let img = dom.find_all(dom.root(), "img")[0];
    let a = dom.parent(img).unwrap();
    assert_eq!(dom.tag(a), "a");
    assert!(dom.has_ancestor(img, |s, i| s.has_class(i, "book")));
    assert!(!dom.has_ancestor(img, |s, i| s.has_class(i, "other")));
    let divs = dom.find_all(dom.root(), "div");
    assert!(dom.has_class(divs[0], "main") && !dom.has_class(divs[0], "boo"));
    assert!(dom.descendants(divs[0]).len() >= 3);
}

#[test]
fn meta_content_and_text_helpers() {
    let dom = HtmlDom::parse(PAGE);
    assert_eq!(dom.meta_content("property", "og:image").as_deref(), Some("https://img.example/cover.jpg"));
    assert_eq!(dom.meta_content("name", "description").as_deref(), Some(""));
    assert_eq!(dom.meta_content("property", "og:title"), None);
    let p = dom.find_all(dom.root(), "p")[0];
    assert_eq!(dom.text_strip(p), "둘째   문단");
    let other = dom.find_all(dom.root(), "div")[1];
    assert_eq!(dom.text_strip_sep(other), "둘째   문단");
    assert!(dom.text(dom.root()).contains("제목"));
}
