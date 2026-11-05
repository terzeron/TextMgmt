//! XML 문서 해석. EPUB 패키지·XHTML과 OOXML 파트를 읽는 crate가 같은 설정을 쓴다.

/// XML을 해석한다. EPUB과 OOXML 파트에는 DOCTYPE 선언이 흔해서 DTD를 허용한다(외부 엔티티는 읽지 않는다).
/// 앞의 BOM은 떼지 않는다. 필요하면 호출자가 뗀다.
pub fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
    roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
}
