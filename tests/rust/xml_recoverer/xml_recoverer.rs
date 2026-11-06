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

// ---- OPF 접두사 선언 (book_manager의 바이트 감싸개 테스트에서 옮김) ----

const OPF_NS_STR: &str = "http://www.idpf.org/2007/opf";

fn opf(sep: &str, attr: &str, extra_decl: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>{}<package{sep}xmlns="{OPF_NS_STR}"{extra_decl} version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator {attr}>x</dc:creator></metadata><manifest/><spine/></package>"#,
        "\n"
    )
}

#[test]
fn test_declare_unbound_prefixes_handles_any_whitespace_after_package() {
    for sep in [" ", "\n  ", "\t"] {
        let fixed = declare_unbound_prefixes(&opf(sep, r#"opf:role="aut""#, ""));
        assert!(strict_parses(&fixed), "sep={sep:?}");
        assert!(fixed.contains(&format!(r#"xmlns:opf="{OPF_NS_STR}""#)), "sep={sep:?}");
    }
}

#[test]
fn test_declare_unbound_prefixes_handles_prefixes_other_than_opf() {
    for (attr, prefix) in [(r#"xsi:type="x""#, "xsi"), (r#"calibre:series="s""#, "calibre")] {
        let fixed = declare_unbound_prefixes(&opf(" ", attr, ""));
        assert!(strict_parses(&fixed), "attr={attr}");
        assert!(fixed.contains(&format!("xmlns:{prefix}=")), "attr={attr}, prefix={prefix}");
    }
}

#[test]
fn test_declare_unbound_prefixes_returns_input_unchanged_when_nothing_is_missing() {
    let clean = opf(" ", r#"id="a""#, "");
    assert_eq!(declare_unbound_prefixes(&clean), clean);
    let declared = opf(" ", r#"opf:role="aut""#, &format!(r#" xmlns:opf="{OPF_NS_STR}""#));
    assert_eq!(declare_unbound_prefixes(&declared), declared);
}

#[test]
fn test_declare_unbound_prefixes_ignores_prefixes_that_only_appear_in_values_and_text() {
    let data = r#"<?xml version="1.0"?><!-- <fake:tag a:b="1"/> --><package xmlns="http://www.idpf.org/2007/opf"><meta property="dcterms:modified">urn:uuid:x</meta></package>"#;
    assert_eq!(declare_unbound_prefixes(data), data);
}

#[test]
fn test_declare_unbound_prefixes_keeps_xml_prefix_and_declares_each_prefix_once() {
    let fixed = declare_unbound_prefixes(r#"<package xmlns="http://www.idpf.org/2007/opf" xml:lang="ko"><a opf:x="1" opf:y="2"/><b xsi:z="1"/></package>"#);
    assert!(fixed.matches("xmlns:opf=").count() == 1 && !fixed.contains("xmlns:xml="));
    assert!(strict_parses(&fixed));
}

#[test]
fn test_declare_unbound_prefixes_without_root_element_is_unchanged_for_plain_text() {
    assert_eq!(declare_unbound_prefixes("not xml"), "not xml");
}
