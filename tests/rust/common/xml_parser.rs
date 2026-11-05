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
