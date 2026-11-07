//! `common::xml_parser` 테스트.

use common::xml_parser::parse_xml;

#[test]
fn parse_xml_accepts_a_doctype_declaration() {
    let text = "<?xml version=\"1.0\"?><!DOCTYPE html><html><body>본문</body></html>";
    let doc = parse_xml(text).unwrap();
    assert_eq!(doc.root_element().tag_name().name(), "html");
}

#[test]
fn parse_xml_reports_malformed_xml() {
    assert!(parse_xml("<a><b></a>").is_err());
}

mod element_tree {
    use common::xml_parser::{Node, declared_prefixes, decode, parse_lenient, parse_package, parse_str, to_string};

    #[test]
    fn namespaced_lookups_text_and_serialization_escapes() {
        let mut root = parse_str(r#"<r xmlns:a="urn:a"><a:x n="1"/><a:x n="2"/><!--c--><?pi data?><?bare?><t>v</t></r>"#).unwrap();
        assert_eq!(root.find_all("urn:a", "x").count(), 2);
        // 주석과 처리 명령은 글에 넣지 않는다.
        assert_eq!(root.text_content(), "v");
        root.find_mut("urn:a", "x").unwrap().set_attr("n", "<&>\"\n\r\t");
        root.children.push(Node::Text("&<>\r".into()));
        let text = to_string(&root);
        assert!(text.contains(r#"<a:x n="&lt;&amp;&gt;&quot;&#10;&#13;&#9;"/>"#), "{text}");
        assert!(text.contains("<?pi data?><?bare?>"), "{text}");
        assert!(text.ends_with("&amp;&lt;&gt;&#13;</r>"), "{text}");
        assert_eq!(declared_prefixes(&root).into_iter().collect::<Vec<_>>(), [("a".to_string(), "urn:a".to_string())]);
    }

    #[test]
    fn decode_follows_byte_order_marks() {
        assert_eq!(decode(&[0xFE, 0xFF, 0x00, b'a']).unwrap(), "a");
        assert!(decode(&[0xEF, 0xBB, 0xBF, 0xFF]).is_err());
    }

    #[test]
    fn lenient_parsing_gives_up_on_unstrippable_entities_and_too_many_prefixes() {
        // 정규식이 지울 수 없는 비ASCII 엔티티 이름은 고칠 수 없다.
        assert!(parse_lenient("<p>&가;</p>".as_bytes()).is_none());
        // 선언을 16번 더해도 남는 접두사가 있으면 포기한다.
        let many: String = (0..17).map(|i| format!("<p{i}:x/>")).collect();
        assert!(parse_lenient(format!("<r>{many}</r>").as_bytes()).is_none());
    }

    #[test]
    fn package_with_an_undeclared_opf_element_is_rejected() {
        assert!(parse_package(b"<opf:package><opf:metadata/></opf:package>").is_err());
    }
}
