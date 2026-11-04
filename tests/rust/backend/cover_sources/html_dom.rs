//! `backend::cover_sources::html_dom` 단위 테스트. 표지 후보 추출이 쓰는 작은 DOM 질의를 본다.

use backend::cover_sources::html_dom::Soup;

const PAGE: &str = r#"<html><head>
<meta property="og:image" content="https://img.example/cover.jpg">
<meta name="description">
</head><body>
<div class="book main"><a href="/b/1"><img src="a.jpg" alt="표지"></a><span>제목&nbsp;하나</span></div>
<div class="other"><p>  둘째   문단 </p><br/><img src="b.jpg"></div>
</body></html>"#;

#[test]
fn find_all_and_attr_follow_document_order() {
    let soup = Soup::parse(PAGE);
    let imgs = soup.find_all(soup.root(), "img");
    assert_eq!(imgs.len(), 2);
    assert_eq!(soup.attr(imgs[0], "src"), Some("a.jpg"));
    assert_eq!(soup.attr(imgs[1], "src"), Some("b.jpg"));
    assert_eq!(soup.attr(imgs[1], "alt"), None);
    assert_eq!(soup.tag(imgs[0]), "img");
}

#[test]
fn parent_ancestor_and_class_checks() {
    let soup = Soup::parse(PAGE);
    let img = soup.find_all(soup.root(), "img")[0];
    let a = soup.parent(img).unwrap();
    assert_eq!(soup.tag(a), "a");
    assert!(soup.has_ancestor(img, |s, i| s.has_class(i, "book")));
    assert!(!soup.has_ancestor(img, |s, i| s.has_class(i, "other")));
    let divs = soup.find_all(soup.root(), "div");
    assert!(soup.has_class(divs[0], "main") && !soup.has_class(divs[0], "boo"));
    assert!(soup.descendants(divs[0]).len() >= 3);
}

#[test]
fn meta_content_and_text_helpers() {
    let soup = Soup::parse(PAGE);
    assert_eq!(soup.meta_content("property", "og:image").as_deref(), Some("https://img.example/cover.jpg"));
    assert_eq!(soup.meta_content("name", "description").as_deref(), Some(""));
    assert_eq!(soup.meta_content("property", "og:title"), None);
    let p = soup.find_all(soup.root(), "p")[0];
    assert_eq!(soup.text_strip(p), "둘째   문단");
    let other = soup.find_all(soup.root(), "div")[1];
    assert_eq!(soup.text_strip_sep(other), "둘째   문단");
    assert!(soup.text(soup.root()).contains("제목"));
}
