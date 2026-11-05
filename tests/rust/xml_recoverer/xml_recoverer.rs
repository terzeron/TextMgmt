//! `xml_recoverer` 단위 테스트.

use xml_recoverer::{close_open_tags, declare_prefix, declare_unbound_prefixes, strip_doctype, strip_unknown_entities};

fn strict_parses(xml: &str) -> bool {
    roxmltree::Document::parse(xml).is_ok()
}

// ---- 접두사 선언 ----

#[test]
fn declare_prefix_uses_the_known_uri_after_the_prolog() {
    let src = r#"<?xml version="1.0"?><!-- c --><!DOCTYPE p [<!ENTITY e "x">]><package><a opf:role="aut"/></package>"#;
    let fixed = declare_prefix(src, "opf").unwrap();
    assert!(fixed.contains(r#"<package xmlns:opf="http://www.idpf.org/2007/opf">"#), "{fixed}");
}

#[test]
fn declare_prefix_uses_a_placeholder_urn_for_unknown_prefixes() {
    let fixed = declare_prefix("<html><x:y/></html>", "x").unwrap();
    assert_eq!(fixed, r#"<html xmlns:x="urn:unknown-prefix:x"><x:y/></html>"#);
    assert!(strict_parses(&fixed));
}

#[test]
fn declare_prefix_without_root_element_is_none() {
    assert_eq!(declare_prefix("not xml", "x"), None);
}

#[test]
fn declare_unbound_prefixes_declares_each_missing_prefix_once() {
    let src = r#"<package xmlns="http://www.idpf.org/2007/opf" xml:lang="ko"><a opf:x="1" opf:y="2"/><b xsi:z="1"/><calibre:c/></package>"#;
    let fixed = declare_unbound_prefixes(src);
    assert_eq!(fixed.matches("xmlns:opf=").count(), 1);
    assert!(fixed.contains(r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#));
    assert!(fixed.contains(r#"xmlns:calibre="urn:unknown-prefix:calibre""#));
    assert!(!fixed.contains("xmlns:xml="));
    assert!(strict_parses(&fixed));
}

#[test]
fn declare_unbound_prefixes_ignores_declared_prefixes_comments_and_values() {
    let src = r#"<?xml version="1.0"?><!-- <fake:tag a:b="1"/> --><package xmlns:opf="urn:a"><meta opf:x="1" property="dcterms:modified">a:b</meta></package>"#;
    assert_eq!(declare_unbound_prefixes(src), src);
}

#[test]
fn declare_unbound_prefixes_without_root_element_is_unchanged() {
    assert_eq!(declare_unbound_prefixes("not xml"), "not xml");
}

// ---- DOCTYPE·엔티티 ----

#[test]
fn strip_doctype_removes_the_internal_subset_too() {
    let src = r#"<?xml version="1.0"?><!DOCTYPE p [<!ENTITY e SYSTEM "file:///etc/passwd">]><p>&e;</p>"#;
    assert_eq!(strip_doctype(src), r#"<?xml version="1.0"?><p>&e;</p>"#);
    assert_eq!(strip_doctype("<p/>"), "<p/>");
}

#[test]
fn strip_unknown_entities_keeps_only_the_predefined_five() {
    assert_eq!(strip_unknown_entities("a&nbsp;b&amp;&lt;&gt;&quot;&apos;&e;&#160;"), "ab&amp;&lt;&gt;&quot;&apos;&#160;");
    assert!(strict_parses(&strip_unknown_entities("<p>a&nbsp;b</p>")));
}

// ---- 태그 구조 ----

#[test]
fn close_open_tags_closes_truncated_documents() {
    assert_eq!(close_open_tags("<a><b>t"), "<a><b>t</b></a>");
    assert_eq!(close_open_tags("<a><b>t</b><c"), "<a><b>t</b></a>");
}

#[test]
fn close_open_tags_matches_closing_tags_to_the_nearest_open_tag() {
    assert_eq!(close_open_tags("<a><b><c>t</a>"), "<a><b><c>t</c></b></a>");
    assert_eq!(close_open_tags("<a>t</x></a>"), "<a>t</a>");
}

#[test]
fn close_open_tags_keeps_comments_cdata_pi_and_quoted_brackets() {
    let src = r#"<?xml version="1.0"?><a x="1>2"><!-- <b> --><![CDATA[<c>]]><d/></a>"#;
    assert_eq!(close_open_tags(src), src);
    assert_eq!(close_open_tags("<a><!-- open"), "<a></a>");
    assert_eq!(close_open_tags("<a><!DOCTYPE x>t"), "<a>t</a>");
}
