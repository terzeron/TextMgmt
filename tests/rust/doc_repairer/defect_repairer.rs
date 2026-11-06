#[path = "../shared/doc_support/mod.rs"]
mod support;
use doc_repairer::defect_repairer::*;

const OLE_AS_UTF16_CHARS: &str = "\u{cfd0}\u{e011}\u{b1a1}\u{e11a}";
const OLE_MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

/// (원래 바이너리, 망가진 상태) 쌍. 문자열에서 만들어야 UTF-16LE 왕복이 보장된다.
fn binary_pair() -> (Vec<u8>, Vec<u8>) {
    let text = format!("{OLE_AS_UTF16_CHARS}{}", "FileHeader 본문 내용 가나다라마바사".repeat(20));
    (text.encode_utf16().flat_map(u16::to_le_bytes).collect(), text.into_bytes())
}

#[test]
fn unmangle_restores_original_bytes() {
    let (orig, mangled) = binary_pair();
    assert!(orig.starts_with(&OLE_MAGIC));
    assert_eq!(unmangle_utf16(&mangled), Some(orig));
}

#[test]
fn unmangle_none_for_plain_text_tolerates_truncation_and_gives_up() {
    assert_eq!(unmangle_utf16("평범한 한국어 텍스트입니다. ".repeat(20).as_bytes()), None);
    let (_, mangled) = binary_pair();
    assert!(unmangle_utf16(&mangled[..mangled.len() - 1]).is_some());
    assert_eq!(unmangle_utf16(&[0xff, 0xfe, 0xfd, 0xfc, 0xfb]), None);
}

#[test]
fn true_extension_identifies_binaries() {
    let (_, mangled) = binary_pair();
    assert_eq!(true_extension(&mangled), Some(".hwp"));
    assert_eq!(true_extension("무림맹의 장문인. ".repeat(40).as_bytes()), None);
    assert_eq!(true_extension_with(b"whatever", &|_| Some(b"ZZZZ not a known magic".to_vec())), None);
}

// ---- xhtml_fix (하위 모듈을 합치며 옮긴 테스트) ----

mod xhtml_fix {
    use std::fs;

    use crate::support::shared_fixtures::{epub_bytes, member, names};
    use crate::support::*;
    use doc_checker::epub::render_checker::xhtml_defect;
    use doc_repairer::defect_repairer::{Verdict, rebuild, repair_markup, verify};

    fn fixed(src: &str) -> String {
        let out = repair_markup(src);
        assert_eq!(xhtml_defect(out.as_bytes()), None, "수리 결과가 아직 결함이다: {out}");
        out
    }

    fn text(xml: &str) -> String {
        let doc = roxmltree::Document::parse_with_options(xml, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).unwrap();
        doc.descendants()
            .filter(|n| n.is_text())
            .filter(|n| n.parent_element().is_none_or(|p| !matches!(p.tag_name().name(), "style" | "script" | "title")))
            .filter_map(|n| n.text())
            .collect::<String>()
            .split_whitespace()
            .collect()
    }

    // ---- 마크업 수리 ----

    #[test]
    fn named_entities_and_void_tags_become_well_formed() {
        let out = fixed("<html><head><title>t</title></head><body><p>가&nbsp;나<br>다<img src=\"a.png\"></p></body></html>");
        assert!(out.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"), "{out}");
        assert!(out.contains("xmlns=\"http://www.w3.org/1999/xhtml\""), "{out}");
        assert!(out.contains("<br />") && out.contains("<img src=\"a.png\" />"), "{out}");
        assert_eq!(text(&out), "가\u{a0}나다".split_whitespace().collect::<String>());
    }

    #[test]
    fn mismatched_and_unclosed_tags_are_balanced_without_losing_text() {
        let out = fixed("<html><head></head><body><div><p>하나<b>둘</p><p>셋</div></body></html>");
        assert_eq!(text(&out), "하나둘셋");
    }

    #[test]
    fn stray_angle_brackets_are_escaped_and_stay_visible() {
        let out = fixed("<html><head></head><body><p>스킬 <불주먹> 과 <...> 그리고 <완.></p></body></html>");
        assert_eq!(text(&out), "스킬<불주먹>과<...>그리고<완.>");
        assert!(out.contains("&lt;불주먹&gt;"), "{out}");
    }

    #[test]
    fn malformed_end_tags_are_dropped_like_a_browser_does() {
        let out = fixed("<html><head></head><body><p>앞</p></...><p>뒤</p></ ></body></html>");
        assert_eq!(text(&out), "앞뒤");
    }

    #[test]
    fn an_end_tag_without_a_closing_bracket_stays_visible_instead_of_swallowing_text() {
        let out = fixed("<html><head></head><body><p>앞</브레먀\n<b>뒤</b></p></body></html>");
        assert_eq!(text(&out), "앞</브레먀뒤");
    }

    #[test]
    fn entities_without_a_semicolon_are_decoded() {
        let out = fixed("<html><head></head><body><p>&nbsp화려한 &amp 소파</p></body></html>");
        assert_eq!(text(&out), "\u{a0}화려한&소파".split_whitespace().collect::<String>());
    }

    #[test]
    fn an_unclosed_attribute_quote_does_not_swallow_the_text() {
        let out = fixed("<html><head></head><body><p><font face=\"batang>그럼 왜 탔는데?</font></p><p>다음</p></body></html>");
        assert_eq!(text(&out), "그럼왜탔는데?다음");
    }

    #[test]
    fn text_after_the_closing_html_tag_is_kept() {
        let out = fixed("<html><head></head><body><p>본문</p></body></html>\n<p>해바라기.syk</p>");
        assert_eq!(text(&out), "본문해바라기.syk");
    }

    #[test]
    fn an_unclosed_style_is_closed_before_the_body_text() {
        let out = fixed("<html><head></head><body><p>앞</p><style>p { color: red }\n</body></html>");
        assert_eq!(text(&out), "앞");
    }

    #[test]
    fn a_missing_head_is_added_and_a_bare_fragment_gets_a_document() {
        assert!(fixed("<html><body><p>글</p></body></html>").contains("<head>"));
        let out = fixed("\n<img alt=\"이미지\" src=\"dropped_image.png\" />");
        assert!(out.contains("<head>") && out.contains("<body>") && out.contains("dropped_image.png"), "{out}");
    }

    #[test]
    fn namespaces_for_epub_xlink_and_svg_are_declared() {
        let out =
            fixed("<html><head></head><body><p><a epub:type=\"noteref\" href=\"#n\">1</a></p><svg viewBox=\"0 0 1 1\"><image xlink:href=\"c.jpg\" width=\"1\" height=\"1\"/></svg></body></html>");
        assert!(out.contains("xmlns:epub=\"http://www.idpf.org/2007/ops\""), "{out}");
        assert!(out.contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\""), "{out}");
        assert!(out.contains("<svg xmlns=\"http://www.w3.org/2000/svg\""), "{out}");
        assert!(out.contains("xlink:href=\"c.jpg\""), "{out}");
    }

    #[test]
    fn xml_forbidden_characters_and_invalid_names_are_dropped() {
        let out = fixed("<html><head></head><body><p>가\u{7}나&#0;다</p><p <bad=\"1\" a\u{b7}b=\"2\">라</p></body></html>");
        assert!(text(&out).starts_with("가나다"), "{out}");
    }

    #[test]
    fn duplicate_attributes_and_odd_doctype_are_normalized() {
        let out = fixed(
            "<?xml version='1.0' encoding='utf-8'?>\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head></head><body><p class=\"a\" class=\"b\">글</p></body></html>",
        );
        assert_eq!(text(&out), "글");
        assert!(out.contains("<!DOCTYPE html>") && !out.contains("xhtml11.dtd"), "{out}");
    }

    // ---- EPUB 재구성 ----

    fn opf_with_chapters(items: &[(&str, &str)], extra_manifest: &str) -> String {
        let manifest: String = items.iter().map(|(id, href)| opf_item(id, href, "application/xhtml+xml", "")).collect();
        let spine: String = items.iter().map(|(id, _)| format!(r#"<itemref idref="{id}"/>"#)).collect();
        format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest>{manifest}{extra_manifest}</manifest><spine>{spine}</spine></package>"#)
    }

    fn plan(v: Verdict) -> doc_repairer::defect_repairer::Plan {
        match v {
            Verdict::Fix(p) => p,
            Verdict::Delete(why) => panic!("삭제 후보로 판정했다: {why}"),
        }
    }

    const BROKEN: &str = "<html><head><title>t</title></head><body><p>본문&nbsp;글<br></body></html>";
    const GARBAGE: &[u8] = &[0xef, 0xc8, 0x1f, 0xa7, 0xe3, 0xd1, 0xeb, 0x93, 0x5d, 0xee, 0x27, 0x0b, 0x60, 0xbc, 0x87, 0x93, 0x87, 0x00, 0x34, 0xe3, 0x17, 0xb5, 0xbd, 0xf0, 0x38, 0xf4, 0xd0, 0xda];

    #[test]
    fn a_broken_chapter_is_repaired_and_the_rest_is_byte_identical() {
        let src = epub_bytes(BROKEN, &clean_opf(), &[("OEBPS/img.png", b"\x89PNG\r\n\x1a\nxx")]);
        let p = plan(rebuild(&src).unwrap());
        assert_eq!(names(&p.bytes), names(&src));
        assert_eq!(xhtml_defect(member(&p.bytes, "OEBPS/ch1.xhtml").as_bytes()), None);
        assert_eq!(member(&p.bytes, "OEBPS/content.opf"), member(&src, "OEBPS/content.opf"));
        assert_eq!(p.edited.len(), 1);
        verify(&src, &p.bytes).unwrap();
    }

    #[test]
    fn an_empty_member_gets_a_skeleton() {
        let opf = opf_with_chapters(&[("c1", "ch1.xhtml"), ("c2", "ch2.xhtml")], "");
        let src = epub_bytes(&xhtml("<p>글</p>"), &opf, &[("OEBPS/ch2.xhtml", b"")]);
        let p = plan(rebuild(&src).unwrap());
        let skeleton = member(&p.bytes, "OEBPS/ch2.xhtml");
        assert_eq!(xhtml_defect(skeleton.as_bytes()), None);
        assert!(p.edited.iter().any(|(n, what)| n == "OEBPS/ch2.xhtml" && what.contains("뼈대")), "{:?}", p.edited);
        verify(&src, &p.bytes).unwrap();
    }

    #[test]
    fn a_binary_garbage_member_is_dropped_with_its_references() {
        let ncx = r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="n1" playOrder="1"><navLabel><text>하나</text></navLabel><content src="ch1.xhtml"/></navPoint><navPoint id="n2" playOrder="2"><navLabel><text>둘</text></navLabel><content src="ch2.xhtml"/></navPoint></navMap></ncx>"#;
        let opf = opf_with_chapters(&[("c1", "ch1.xhtml"), ("c2", "ch2.xhtml"), ("c3", "ch3.xhtml")], r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#);
        let link = xhtml(r#"<p><a href="ch2.xhtml#x">둘로</a> 와 <a href="ch3.xhtml">셋</a></p>"#);
        let src = epub_bytes(&link, &opf, &[("OEBPS/ch2.xhtml", GARBAGE), ("OEBPS/ch3.xhtml", xhtml("<p>셋</p>").as_bytes()), ("OEBPS/toc.ncx", ncx.as_bytes())]);
        let p = plan(rebuild(&src).unwrap());
        assert!(!names(&p.bytes).contains(&"OEBPS/ch2.xhtml".to_string()));
        let opf_after = member(&p.bytes, "OEBPS/content.opf");
        assert!(!opf_after.contains("ch2.xhtml") && !opf_after.contains(r#"idref="c2""#), "{opf_after}");
        assert!(opf_after.contains("ch3.xhtml") && opf_after.contains(r#"idref="c3""#), "{opf_after}");
        let ncx_after = member(&p.bytes, "OEBPS/toc.ncx");
        assert!(!ncx_after.contains("ch2.xhtml") && ncx_after.contains("ch1.xhtml"), "{ncx_after}");
        let link_after = member(&p.bytes, "OEBPS/ch1.xhtml");
        assert!(!link_after.contains("ch2.xhtml") && link_after.contains("둘로") && link_after.contains("ch3.xhtml"), "{link_after}");
        assert_eq!(p.dropped.len(), 1);
        verify(&src, &p.bytes).unwrap();
    }

    #[test]
    fn a_member_that_cannot_be_read_is_dropped_like_garbage() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("t.epub");
        let opf = opf_with_chapters(&[("c1", "ch1.xhtml"), ("c2", "ch2.xhtml"), ("c3", "ch3.xhtml"), ("c4", "ch4.xhtml"), ("c5", "ch5.xhtml"), ("c6", "ch6.xhtml")], "");
        let body = xhtml("<p>본문</p>");
        epub_with_files(
            &path,
            &opf,
            &body,
            &[
                ("OEBPS/ch2.xhtml", body.as_bytes()),
                ("OEBPS/ch3.xhtml", body.as_bytes()),
                ("OEBPS/ch4.xhtml", body.as_bytes()),
                ("OEBPS/ch5.xhtml", body.as_bytes()),
                ("OEBPS/ch6.xhtml", body.as_bytes()),
            ],
        );
        corrupt_local_header(&path, "OEBPS/ch3.xhtml");
        let p = plan(rebuild(&fs::read(&path).unwrap()).unwrap());
        assert!(!names(&p.bytes).contains(&"OEBPS/ch3.xhtml".to_string()));
        assert!(!member(&p.bytes, "OEBPS/content.opf").contains("ch3.xhtml"));
        assert!(names(&p.bytes).contains(&"OEBPS/ch4.xhtml".to_string()));
    }

    #[test]
    fn many_broken_members_make_the_file_a_deletion_candidate() {
        let opf = opf_with_chapters(&[("c1", "ch1.xhtml"), ("c2", "ch2.xhtml"), ("c3", "ch3.xhtml")], "");
        let src = epub_bytes(&xhtml("<p>글</p>"), &opf, &[("OEBPS/ch2.xhtml", GARBAGE), ("OEBPS/ch3.xhtml", GARBAGE)]);
        match rebuild(&src).unwrap() {
            Verdict::Delete(why) => assert!(why.contains("깨져"), "{why}"),
            Verdict::Fix(_) => panic!("많이 깨진 파일은 고치지 않고 삭제 후보로 알려야 한다"),
        }
    }

    #[test]
    fn a_non_utf8_chapter_with_real_markup_is_not_touched() {
        let euckr = encoding_rs::EUC_KR.encode("<html><head></head><body><p>한국어 본문<br></p></body></html>").0.into_owned();
        let opf = opf_with_chapters(&[("c1", "ch1.xhtml"), ("c2", "ch2.xhtml")], "");
        let src = epub_bytes(&xhtml("<p>글</p>"), &opf, &[("OEBPS/ch2.xhtml", &euckr)]);
        assert!(rebuild(&src).is_err());
    }

    #[test]
    fn a_healthy_epub_has_nothing_to_fix() {
        let src = epub_bytes(&xhtml("<p>글</p>"), &clean_opf(), &[]);
        let p = plan(rebuild(&src).unwrap());
        assert!(p.edited.is_empty() && p.dropped.is_empty());
    }

    #[test]
    fn verify_rejects_a_rewrite_that_loses_text() {
        let src = epub_bytes(BROKEN, &clean_opf(), &[]);
        let lost = epub_bytes(&xhtml("<p>다른 글</p>"), &clean_opf(), &[]);
        assert!(verify(&src, &lost).unwrap_err().contains("본문"));
    }
}
