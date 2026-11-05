//! `doc_checker::epub::security_checker` 단위 테스트.

#[path = "../support/mod.rs"]
mod support;
use crate::support::shared_fixtures::{binary, has, patch_central};
use doc_checker::epub::{security_checker::Limits, security_checker::decode_calls, security_checker::scan as scan_into};
use doc_checker::{FileReport, Format, Severity};
use std::fs;
use std::io::Write;
use std::path::Path;
use support::*;

fn scan_counting(path: &Path) -> (FileReport, usize) {
    let before = decode_calls();
    let mut r = FileReport::new(path, Some(Format::Epub));
    scan_into(path, &Limits::default(), &mut r);
    (r, decode_calls() - before)
}

#[test]
fn text_members_are_decoded() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    support::epub_with_chapter(&p, &support::xhtml("<p>본문</p>"));
    let (_, calls) = scan_counting(&p);
    assert!(calls >= 2, "xhtml와 opf는 변환해서 검사한다: {calls}");
}

#[test]
fn non_text_members_are_not_decoded() {
    let d = tempfile::tempdir().unwrap();
    let plain = d.path().join("plain.epub");
    support::epub_with_chapter(&plain, &support::xhtml("<p>본문</p>"));
    let (_, base_calls) = scan_counting(&plain);

    let with_media = d.path().join("media.epub");
    let jpg = binary(1 << 20, 1);
    let png = binary(1 << 19, 2);
    let ttf = binary(1 << 18, 3);
    support::epub_with_files(&with_media, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/a.jpg", &jpg), ("OEBPS/b.png", &png), ("OEBPS/c.ttf", &ttf), ("OEBPS/d.bin", &jpg)]);
    let (_, media_calls) = scan_counting(&with_media);
    assert_eq!(media_calls, base_calls, "이미지·폰트·알 수 없는 확장자 멤버는 UTF-8 변환 대상이 아니다");
}

#[test]
fn binary_member_checks_still_run_without_decoding() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("disguised.epub");
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    support::epub_with_files(&p, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/cover.jpg", &pe)]);
    let (r, _) = scan_counting(&p);
    let f = r.findings.iter().find(|f| f.code == "PE_EXECUTABLE_CONTENT").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!(f.severity, doc_checker::Severity::Critical);
}

#[test]
fn txt_members_are_not_decoded_either_because_other_kind_has_no_text_rules() {
    // Kind::Other의 txt에는 텍스트 규칙이 없다(scan_text가 아무것도 하지 않는다). 변환해도 결과가 달라지지 않는다.
    let d = tempfile::tempdir().unwrap();
    let plain = d.path().join("plain.epub");
    support::epub_with_chapter(&plain, &support::xhtml("<p>본문</p>"));
    let (_, base_calls) = scan_counting(&plain);
    let with_txt = d.path().join("txt.epub");
    support::epub_with_files(&with_txt, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/notes.txt", "소림사 장로\n".as_bytes())]);
    let (r, txt_calls) = scan_counting(&with_txt);
    assert_eq!(txt_calls, base_calls);
    assert!(!r.findings.iter().any(|f| f.location.contains("notes.txt")), "{:?}", r.findings);
}

fn scan(path: &Path) -> FileReport {
    let mut r = FileReport::new(path, Some(Format::Epub));
    scan_into(path, &Limits::default(), &mut r);
    r
}

// ---- 매니페스트와 매직 ----

#[test]
fn manifest_items_with_script_unc_and_unknown_schemes_are_flagged() {
    let items = [
        opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""),
        opf_item("a", "javascript:alert(1)", "text/plain", ""),
        opf_item("b", "\\\\host\\share\\x", "text/plain", ""),
        opf_item("c", "weird-app://x", "text/plain", ""),
    ]
    .concat();
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &opf(&items), &xhtml("<p>x</p>"), &[]);
    let r = scan(&p);
    for code in ["JS_URL", "UNC_PATH", "UNKNOWN_URI_SCHEME"] {
        assert!(has(&r, code), "{code}");
    }
}

#[test]
fn mach_o_universal_binaries_are_told_apart_from_java_classes() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/a.bin2", b"\xca\xfe\xba\xbe\x00\x00\x00\x02rest")]);
    assert!(has(&scan(&p), "MACHO_BINARY_CONTENT"));
}

#[test]
fn directory_entries_and_very_long_names_are_handled() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    {
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.add_directory("OEBPS/", o).unwrap();
        let long = format!("OEBPS/{}", ["dd"; 600].join("/"));
        z.start_file(format!("{long}/f.txt"), o).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
    }
    let r = scan(&p);
    assert!(has(&r, "ZIP_NAME_TOO_LONG"), "{:?}", r.findings);
}

#[test]
fn encrypted_and_unsupported_compression_entries_are_noted_not_read() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_chapter(&p, &xhtml("<p>x</p>"));
    patch_central(&p, "OEBPS/ch1.xhtml", 8, &[0x01, 0x00]); // 일반 목적 비트 0: 암호화
    let r = scan(&p);
    assert!(r.incomplete);
    assert!(r.reasons.iter().any(|n| n.contains("ZIP-encrypted")), "{:?}", r.reasons);

    let q = d.path().join("u.epub");
    epub_with_chapter(&q, &xhtml("<p>x</p>"));
    patch_central(&q, "OEBPS/ch1.xhtml", 10, &[12, 0]); // bzip2
    let r = scan(&q);
    assert!(r.reasons.iter().any(|n| n.contains("unsupported ZIP compression")), "{:?}", r.reasons);
}

mod budget_and_encoding_evasion {
    use super::scan_chapter;
    use super::support;
    use crate::support::shared_fixtures::{codes, forge_declared_size, has, scan_with, scan_with_stats};
    use doc_checker::epub::security_checker::Limits;
    use doc_checker::{FileReport, Severity};
    use support::*;

    fn sev(r: &FileReport, code: &str) -> Severity {
        r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", codes(r))).severity
    }

    fn scan_extra(name: &str, data: &[u8]) -> FileReport {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &clean_opf(), &xhtml("<p>본문</p>"), &[(name, data)]);
        scan_with(&p, &Limits::default())
    }

    // ---- 1. 읽기 상한은 선언 크기를 믿지 않는다 ----

    const REMOTE_ITEM: &str = r#"<item id="r" href="http://evil.example/a.css" media-type="text/css"/>"#;

    fn padded_opf() -> String {
        // 매니페스트 앞쪽을 큰 주석으로 부풀리고, 원격 항목을 맨 뒤에 둔다.
        opf(&format!("<!--{}-->{}{}", "x".repeat(6000), opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""), REMOTE_ITEM))
    }

    #[test]
    fn opf_prepass_reads_are_charged_to_the_total_budget() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &padded_opf(), &xhtml("<p>x</p>"), &[]);
        // 예산(3000)보다 큰 OPF를 선검사가 통째로 읽으면 예산 밖에서 항목이 검사된다.
        let (r, stats) = scan_with_stats(&p, &Limits { max_member: 1 << 20, max_total: 3000, max_files: 10_000 });
        assert!(!has(&r, "REMOTE_MANIFEST_ITEM"), "예산을 넘겨 읽었다: {:?}", codes(&r));
        assert!(r.incomplete && r.reasons.iter().any(|n| n.contains("Total decompressed-byte scan limit")), "{:?}", r.reasons);
        assert!(stats.inspected_bytes <= 3000, "{stats:?}");
        // 음성: 예산이 충분하면 같은 OPF에서 항목이 그대로 검출된다.
        let r = scan_with(&p, &Limits::default());
        assert!(has(&r, "REMOTE_MANIFEST_ITEM"), "{:?}", codes(&r));
    }

    #[test]
    fn opf_prepass_is_capped_at_max_files() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        let extra_opf = opf(&format!("{}{}", opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""), REMOTE_ITEM));
        epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/extra.opf", extra_opf.as_bytes())]);
        // 항목은 mimetype, container, content.opf, ch1, extra.opf 순이라 max_files=4면 extra.opf는 범위 밖이다.
        let r = scan_with(&p, &Limits { max_files: 4, ..Limits::default() });
        assert!(!has(&r, "REMOTE_MANIFEST_ITEM"), "{:?}", codes(&r));
        assert!(r.reasons.iter().any(|n| n.contains("Too many entries")), "{:?}", r.reasons);
        // 음성: 범위 안이면 검출된다.
        let r = scan_with(&p, &Limits { max_files: 5, ..Limits::default() });
        assert!(has(&r, "REMOTE_MANIFEST_ITEM"), "{:?}", codes(&r));
    }

    // 이 테스트는 zip 크레이트의 선언 크기 강제를 지키는 가드다. 실제 RED 근거는 위의 OPF 선검사 테스트다.
    #[test]
    fn forged_small_declared_size_cannot_exceed_the_total_budget() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/bomb.bin", &vec![0u8; 4 << 20])]);
        forge_declared_size(&p, "OEBPS/bomb.bin", 10);
        let limits = Limits { max_member: 8 << 20, max_total: 100_000, max_files: 10_000 };
        let (r, stats) = scan_with_stats(&p, &limits);
        assert!(stats.inspected_bytes <= limits.max_total, "{stats:?}");
        assert!(!r.reasons.iter().any(|n| n.contains("panic")), "{:?}", r.reasons);
        // 음성: 속이지 않은 같은 멤버는 선언 크기가 예산을 넘으므로 incomplete 한 건으로 건너뛴다.
        let q = d.path().join("honest.epub");
        epub_with_files(&q, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/bomb.bin", &vec![0u8; 4 << 20])]);
        let (r, stats) = scan_with_stats(&q, &limits);
        assert!(r.incomplete && stats.inspected_bytes <= limits.max_total);
    }

    #[test]
    fn honest_zip_bomb_is_still_flagged_with_the_old_code_and_severity() {
        let r = scan_extra("OEBPS/bomb.bin", &vec![0u8; 4 << 20]);
        assert_eq!(sev(&r, "HIGH_COMPRESSION_RATIO"), Severity::Medium);
    }

    // ---- 2. URL 속성은 접두사와 무관하게 로컬 이름으로 본다 ----

    #[test]
    fn xlink_href_literal_spelling_is_still_flagged() {
        let r = scan_chapter(r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><a xlink:href="javascript:alert(1)"><text>x</text></a></svg>"#);
        assert_eq!(sev(&r, "JS_URL"), Severity::High);
    }

    #[test]
    fn url_attributes_with_any_namespace_prefix_are_checked() {
        for body in [r#"<a x:href="javascript:alert(1)">x</a>"#, r#"<img foo:src="javascript:alert(1)"/>"#, r#"<form bar:action="javascript:alert(1)"></form>"#] {
            let r = scan_chapter(body);
            assert_eq!(sev(&r, "JS_URL"), Severity::High, "{body}: {:?}", codes(&r));
        }
        // 접두사 없는 같은 값과 같은 코드·심각도다.
        assert_eq!(sev(&scan_chapter(r#"<a href="javascript:alert(1)">x</a>"#), "JS_URL"), Severity::High);
    }

    #[test]
    fn prefixed_url_attribute_with_a_safe_value_or_other_local_name_is_not_flagged() {
        assert!(!has(&scan_chapter(r#"<a x:href="chapter2.xhtml">x</a>"#), "JS_URL"));
        assert!(!has(&scan_chapter(r#"<a x:myhref="javascript:alert(1)">x</a>"#), "JS_URL"));
        assert!(!has(&scan_chapter(r#"<a x:href="https://example.com/">x</a>"#), "JS_URL"));
        assert!(!has(&scan_chapter(r#"<div d:data="plain text" x:action="/submit">x</div>"#), "JS_URL"));
    }

    #[test]
    fn animate_attribute_name_is_matched_by_local_name() {
        let r = scan_chapter(r#"<svg><a><animate attributeName="x:href" to="javascript:alert(1)"/></a></svg>"#);
        assert_eq!(sev(&r, "SVG_ANIMATE_HREF"), Severity::Medium);
        assert_eq!(sev(&r, "JS_URL"), Severity::High);
        // 음성: href가 아닌 속성을 바꾸는 애니메이션은 그대로 통과한다.
        let r = scan_chapter(r#"<svg><circle><animate attributeName="x:fill" to="javascript:alert(1)"/></circle></svg>"#);
        assert!(!has(&r, "SVG_ANIMATE_HREF") && !has(&r, "JS_URL"), "{:?}", codes(&r));
        // 기존 철자는 그대로다.
        let r = scan_chapter(r#"<svg><a><set attributeName="xlink:href" to="javascript:alert(1)"/></a></svg>"#);
        assert_eq!(sev(&r, "SVG_ANIMATE_HREF"), Severity::Medium);
    }

    // ---- 3. UTF-16/32 마크업 ----

    fn utf16(text: &str, le: bool, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if bom {
            out.extend(if le { [0xFF, 0xFE] } else { [0xFE, 0xFF] });
        }
        for u in text.encode_utf16() {
            out.extend(if le { u.to_le_bytes() } else { u.to_be_bytes() });
        }
        out
    }

    const SCRIPT_DOC: &str = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><script>alert(1)</script></body></html>";

    #[test]
    fn utf16_markup_with_bom_is_transcoded_and_scanned() {
        for le in [true, false] {
            let r = scan_extra("OEBPS/u16.xhtml", &utf16(SCRIPT_DOC, le, true));
            assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High, "le={le}: {:?}", codes(&r));
            assert!(!has(&r, "MARKUP_UNDECODABLE"));
        }
    }

    #[test]
    fn utf16_markup_without_bom_is_detected_by_nul_interleave() {
        for le in [true, false] {
            let r = scan_extra("OEBPS/u16.xhtml", &utf16(SCRIPT_DOC, le, false));
            assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High, "le={le}: {:?}", codes(&r));
        }
    }

    #[test]
    fn utf32_markup_is_reported_as_undecodable() {
        for bytes in [[&[0xFF, 0xFE, 0, 0][..], b"<\0\0\0s\0\0\0"].concat(), [&[0, 0, 0xFE, 0xFF][..], b"\0\0\0<\0\0\0s"].concat()] {
            let r = scan_extra("OEBPS/u32.xhtml", &bytes);
            assert_eq!(sev(&r, "MARKUP_UNDECODABLE"), Severity::Medium, "{:?}", codes(&r));
            assert!(r.incomplete && r.reasons.iter().any(|n| n.contains("unsupported encoding utf-32") && n.contains("OEBPS/u32.xhtml")), "{:?}", r.reasons);
        }
    }

    #[test]
    fn utf8_and_utf8_bom_markup_behave_as_before() {
        let doc = xhtml("<script>alert(1)</script>");
        let r = scan_extra("OEBPS/a.xhtml", doc.as_bytes());
        assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High);
        assert!(!r.reasons.iter().any(|n| n.contains("non-utf8")), "{:?}", r.reasons);
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend(doc.as_bytes());
        let r = scan_extra("OEBPS/b.xhtml", &bom);
        assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High);
        assert!(!r.incomplete && !has(&r, "MARKUP_UNDECODABLE"), "{:?}", r.reasons);
        let r = scan_extra("OEBPS/c.xhtml", xhtml("<p>한글 본문</p>").as_bytes());
        assert!(!r.incomplete && r.findings.is_empty(), "{:?}", r.reasons);
    }

    #[test]
    fn invalid_utf8_markup_makes_the_scan_incomplete_with_one_summary_note() {
        let mut bad = xhtml("<p>caf").into_bytes();
        bad.push(0xE9);
        bad.extend(b"</p><script>1</script>");
        let r = scan_extra("OEBPS/latin.xhtml", &bad);
        let notes: Vec<_> = r.reasons.iter().filter(|n| n.contains("non-utf8 markup")).collect();
        assert_eq!(notes.len(), 1, "{:?}", r.reasons);
        assert!(notes[0].contains("1 members") && notes[0].contains("first: OEBPS/latin.xhtml"), "{notes:?}");
        assert!(r.incomplete);
        assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High);
    }

    fn scan_many(members: &[(String, Vec<u8>)]) -> FileReport {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        let refs: Vec<(&str, &[u8])> = members.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &refs);
        scan_with(&p, &Limits::default())
    }

    #[test]
    fn many_undecodable_members_leave_one_note_per_kind_and_capped_findings() {
        let mut members = Vec::new();
        for i in 0..40 {
            members.push((format!("OEBPS/l{i}.xhtml"), vec![b'<', b'p', b'>', 0xE9, b'<', b'/', b'p', b'>']));
            members.push((format!("OEBPS/w{i}.xhtml"), [&[0xFF, 0xFE, 0, 0][..], b"<\0\0\0"].concat()));
        }
        let r = scan_many(&members);
        for kind in ["non-utf8 markup", "unsupported encoding utf-32"] {
            let n = r.reasons.iter().filter(|m| m.contains(kind)).count();
            assert_eq!(n, 1, "{kind}: {:?}", r.reasons);
        }
        let f: Vec<_> = r.findings.iter().filter(|f| f.code == "MARKUP_UNDECODABLE").collect();
        assert_eq!(f.len(), 20, "{}", f.len());
        assert!(f.iter().any(|x| x.detail.contains("more")), "{f:?}");
        let note = r.reasons.iter().find(|m| m.contains("utf-32")).unwrap();
        assert!(note.contains("40 members"), "{note}");
    }

    // ---- 3b. 옛 탐지를 가리지 않는다 / NUL을 품은 마크업 ----

    #[test]
    fn utf16_bom_followed_by_ascii_markup_keeps_the_old_finding() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(b"<script>alert(1)</script>");
        let r = scan_extra("OEBPS/tricky.xhtml", &bytes);
        assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High, "{:?}", codes(&r));
        let mut bytes = vec![0xFE, 0xFF];
        bytes.extend(b"<script>alert(1)</script>");
        assert_eq!(sev(&scan_extra("OEBPS/tricky2.xhtml", &bytes), "SCRIPT_TAG"), Severity::High);
    }

    #[test]
    fn nul_interleaved_prefix_followed_by_ascii_markup_keeps_the_old_finding() {
        let mut bytes: Vec<u8> = (0..28).flat_map(|_| [b' ', 0]).collect();
        bytes.extend(b"<script>alert(1)</script>");
        let r = scan_extra("OEBPS/tricky.xhtml", &bytes);
        assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High, "{:?}", codes(&r));
    }

    #[test]
    fn bomless_utf32_markup_is_undecodable() {
        let le: Vec<u8> = "<script>alert(1)</script>".bytes().flat_map(|b| [b, 0, 0, 0]).collect();
        let be: Vec<u8> = "<script>alert(1)</script>".bytes().flat_map(|b| [0, 0, 0, b]).collect();
        for bytes in [le, be] {
            let r = scan_extra("OEBPS/u32.xhtml", &bytes);
            assert_eq!(sev(&r, "MARKUP_UNDECODABLE"), Severity::Medium, "{:?}", codes(&r));
            assert!(r.incomplete);
        }
    }

    #[test]
    fn valid_utf8_markup_with_a_nul_is_undecodable_but_still_scanned() {
        let doc = xhtml("<p>a\0b</p><script>1</script>");
        let r = scan_extra("OEBPS/nul.xhtml", doc.as_bytes());
        assert_eq!(sev(&r, "MARKUP_UNDECODABLE"), Severity::Medium, "{:?}", codes(&r));
        assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High);
        assert!(r.incomplete);
        let r = scan_extra("OEBPS/clean.xhtml", xhtml("<p>ok</p>").as_bytes());
        assert!(!has(&r, "MARKUP_UNDECODABLE") && !r.incomplete);
        // 변환한 UTF-16은 NUL이 있어도 undecodable이 아니다.
        let r = scan_extra("OEBPS/u16.xhtml", &utf16(SCRIPT_DOC, true, true));
        assert!(!has(&r, "MARKUP_UNDECODABLE"), "{:?}", codes(&r));
    }

    // ---- 4. data: URL ----

    #[test]
    fn active_data_url_mime_types_are_flagged_like_data_text_html() {
        for url in [
            "data:image/svg+xml;base64,PHN2Zy8+",
            "data:application/xml,%3Cx/%3E",
            "data:text/xml,%3Cx/%3E",
            "data:application/atom+xml,x",
            "data:text/javascript,alert(1)",
            "data:application/javascript,alert(1)",
            "data:application/x-javascript,alert(1)",
            "data:application/xhtml+xml,x",
            "  DATA:Image/SVG+XML;base64,PHN2Zy8+",
            "data: text/xml ,x",
        ] {
            let r = scan_chapter(&format!(r#"<img src="{url}"/>"#));
            assert_eq!(sev(&r, "JS_URL"), Severity::High, "{url}: {:?}", codes(&r));
        }
        let r = scan_chapter(r#"<a href="data:text/html;base64,PGI+">x</a>"#);
        assert_eq!(sev(&r, "JS_URL"), Severity::High);
    }

    #[test]
    fn benign_data_urls_in_img_are_not_flagged() {
        for url in ["data:image/png;base64,iVBORw0KGgo=", "data:image/jpeg;base64,/9j/", "data:image/gif;base64,R0lGOD", "data:text/plain,hello"] {
            let r = scan_chapter(&format!(r#"<img src="{url}"/>"#));
            assert!(!has(&r, "JS_URL"), "{url}: {:?}", codes(&r));
        }
    }

    #[test]
    fn data_urls_in_embedding_elements_must_be_images_or_fonts() {
        for body in [
            r#"<iframe src="data:text/plain,hi"></iframe>"#,
            r#"<iframe src="data:,hi"></iframe>"#,
            r#"<object data="data:application/pdf;base64,AAAA"></object>"#,
            r#"<embed src="data:application/octet-stream;base64,AAAA"/>"#,
            r#"<frame src="data:image/svg+xml;base64,PHN2Zy8+"/>"#,
        ] {
            let r = scan_chapter(body);
            assert_eq!(sev(&r, "JS_URL"), Severity::High, "{body}: {:?}", codes(&r));
        }
        for body in [
            r#"<iframe src="data:image/png;base64,iVBORw0KGgo="></iframe>"#,
            r#"<object data="data:image/webp;base64,AAAA"></object>"#,
            r#"<embed src="data:font/woff2;base64,AAAA"/>"#,
            r#"<object data="data:application/font-woff;base64,AAAA"></object>"#,
        ] {
            let r = scan_chapter(body);
            assert!(!has(&r, "JS_URL"), "{body}: {:?}", codes(&r));
        }
    }
}

mod active_content {
    use super::support;
    use super::{scan, scan_chapter};
    use crate::support::shared_fixtures::{codes, has, patch_central, scan_with, scan_with_stats};
    use doc_checker::epub::{security_checker::Limits, security_checker::guarded};
    use doc_checker::{Category, FileReport, Format, Severity};
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use support::*;

    fn severity_of(r: &FileReport, code: &str) -> Severity {
        r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", codes(r))).severity
    }

    fn scan_extra(name: &str, data: &[u8]) -> FileReport {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &clean_opf(), &xhtml("<p>본문</p>"), &[(name, data)]);
        scan(&p)
    }

    // ---- 기준선 ----

    #[test]
    fn clean_epub_has_no_findings() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_chapter(&p, &xhtml("<p>안녕하세요</p>"));
        let (r, stats) = scan_with_stats(&p, &Limits::default());
        assert!(!r.incomplete && r.findings.is_empty(), "{:?}", r.findings);
        assert!(r.findings.is_empty() && r.reasons.is_empty());
        assert_eq!(stats.entries, 4);
        assert!(stats.inspected_bytes > 0);
    }

    // ---- HTML 활성 요소 ----

    #[test]
    fn script_tags_are_flagged_and_marked_as_undeclared_without_scripted_property() {
        let r = scan_chapter("<script>var a = 1;</script>");
        assert_eq!(severity_of(&r, "SCRIPT_TAG"), Severity::High);
        assert!(has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
        assert!(!r.incomplete && !r.findings.is_empty());
    }

    #[test]
    fn declared_scripted_property_suppresses_the_undeclared_finding() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &scripted_opf(), &xhtml("<script>1</script>"), &[]);
        let r = scan(&p);
        assert!(has(&r, "SCRIPT_TAG") && !has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
    }

    #[test]
    fn non_javascript_script_types_are_low() {
        let r = scan_chapter(r#"<script type="application/ld+json">{"a":1}</script>"#);
        assert_eq!(severity_of(&r, "SCRIPT_TAG"), Severity::Low);
        assert!(!has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
    }

    #[test]
    fn remote_script_is_high_and_reported_separately() {
        let r = scan_chapter(r#"<script src="https://evil.example/x.js"></script>"#);
        assert_eq!(severity_of(&r, "REMOTE_SCRIPT"), Severity::High);
    }

    #[test]
    fn script_body_goes_through_the_javascript_rules() {
        let r = scan_chapter("<script>eval(atob('YQ=='))</script>");
        assert!(has(&r, "DYNAMIC_CODE_EXEC") && has(&r, "OBFUSCATION"));
    }

    #[test]
    fn event_handler_attributes_are_high() {
        let r = scan_chapter(r#"<p onclick="alert(1)">x</p><img src="a.png" onerror=alert(1)>"#);
        assert_eq!(severity_of(&r, "EVENT_HANDLER_ATTR"), Severity::High);
        assert!(has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
    }

    #[test]
    fn javascript_urls_are_found_even_when_obfuscated() {
        for href in ["javascript:alert(1)", "JaVaScRiPt:alert(1)", "jav&#x09;ascript:alert(1)", "  javascript:alert(1)", "java\nscript:alert(1)", "&#106;avascript:alert(1)", "vbscript:msgbox(1)"] {
            let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
            assert!(has(&r, "JS_URL"), "{href}: {:?}", codes(&r));
        }
    }

    #[test]
    fn data_html_urls_and_dangerous_schemes_are_flagged() {
        let r = scan_chapter(r#"<a href="data:text/html;base64,PHNjcmlwdD4=">x</a>"#);
        assert!(has(&r, "JS_URL"));
        for href in ["file:///etc/passwd", "smb://host/share"] {
            let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
            assert_eq!(severity_of(&r, "DANGEROUS_URI_SCHEME"), Severity::High, "{href}");
        }
        for href in ["ms-msdt:/id", "search-ms:query=x", "ms-appinstaller:?source=x", "MS-MSDT:/id"] {
            let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
            assert_eq!(severity_of(&r, "DANGEROUS_URI_SCHEME"), Severity::Critical, "{href}");
        }
        let r = scan_chapter(r#"<a href="weird-app://open">x</a>"#);
        assert_eq!(severity_of(&r, "UNKNOWN_URI_SCHEME"), Severity::Low);
    }

    #[test]
    fn ordinary_links_are_clean() {
        let r = scan_chapter(r##"<a href="#top">a</a><a href="ch2.xhtml">b</a><a href="mailto:a@b.c">c</a><a href="https://example.com/">d</a><a href="tel:123">e</a>"##);
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    #[test]
    fn unc_paths_are_flagged() {
        // 클릭해야 열리는 링크는 High, 자원으로 자동 로드되면 클릭 없이 인증 정보가 나가므로 Critical이다.
        let r = scan_chapter(r#"<a href="\\attacker\share\a.png">x</a>"#);
        assert_eq!(severity_of(&r, "UNC_PATH"), Severity::High);
        let r = scan_chapter(r#"<img src="\\attacker\share\a.png">"#);
        assert_eq!(severity_of(&r, "UNC_PATH"), Severity::Critical);
    }

    #[test]
    fn active_elements_are_flagged() {
        for (body, sev) in [
            ("<object data=\"a.swf\"></object>", Severity::High),
            ("<embed src=\"a.swf\">", Severity::High),
            ("<applet code=\"A\"></applet>", Severity::High),
            ("<iframe src=\"a.xhtml\"></iframe>", Severity::Medium),
            ("<iframe srcdoc=\"<b>\"></iframe>", Severity::High),
            ("<iframe src=\"https://x.example/\"></iframe>", Severity::High),
        ] {
            let r = scan_chapter(body);
            assert_eq!(severity_of(&r, "ACTIVE_ELEMENT"), sev, "{body}");
        }
    }

    #[test]
    fn forms_meta_refresh_and_base_are_flagged() {
        let r = scan_chapter(r#"<form action="a.xhtml"><input name="x"></form>"#);
        assert_eq!(severity_of(&r, "FORM_ELEMENT"), Severity::Medium);
        assert!(has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
        let r = scan_chapter(r#"<form action="https://evil.example/post"></form>"#);
        assert_eq!(severity_of(&r, "REMOTE_FORM_ACTION"), Severity::High);

        let r = scan_chapter(r#"<meta http-equiv="refresh" content="0;url=ch2.xhtml">"#);
        assert_eq!(severity_of(&r, "META_REFRESH"), Severity::Medium);
        let r = scan_chapter(r#"<meta http-equiv="Refresh" content="0; URL=https://evil.example/">"#);
        assert_eq!(severity_of(&r, "META_REFRESH"), Severity::High);

        let r = scan_chapter(r#"<base href="https://evil.example/">"#);
        assert_eq!(severity_of(&r, "BASE_HREF"), Severity::Medium);
    }

    #[test]
    fn local_script_src_and_non_remote_refresh_keep_their_expected_severity() {
        let r = scan_chapter(r#"<script src="javascript:alert(1)"></script>"#);
        assert_eq!(severity_of(&r, "JS_URL"), Severity::High);
        assert!(!has(&r, "REMOTE_SCRIPT"));

        let r = scan_chapter(r#"<meta http-equiv="refresh" content="5">"#);
        assert_eq!(severity_of(&r, "META_REFRESH"), Severity::Medium);

        let r = scan_chapter(r#"<svg><animate attributeName="href" from="javascript:alert(1)"/></svg>"#);
        assert!(has(&r, "SVG_ANIMATE_HREF") && has(&r, "JS_URL"));
    }

    #[test]
    fn remote_resources_are_low_and_require_the_opf_declaration() {
        let r =
            scan_chapter(r#"<img src="https://tracker.example/p.gif"><link rel="stylesheet" href="http://x.example/a.css"><video src="https://x.example/v.mp4"></video><source src="//x.example/a">"#);
        assert_eq!(severity_of(&r, "REMOTE_RESOURCE"), Severity::Low);
        assert!(has(&r, "UNDECLARED_REMOTE_RESOURCES"));

        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &opf(&opf_item("c1", "ch1.xhtml", "application/xhtml+xml", "remote-resources")), &xhtml(r#"<img src="https://tracker.example/p.gif">"#), &[]);
        assert!(!has(&scan(&p), "UNDECLARED_REMOTE_RESOURCES"));
    }

    #[test]
    fn svg_foreign_object_and_href_animation_are_flagged() {
        let r = scan_chapter(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><div/></foreignObject><a><set attributeName="href" to="javascript:alert(1)"/></a><animate attributeName="xlink:href" values="x"/></svg>"#,
        );
        assert_eq!(severity_of(&r, "SVG_FOREIGN_OBJECT"), Severity::Medium);
        assert_eq!(severity_of(&r, "SVG_ANIMATE_HREF"), Severity::Medium);
    }

    #[test]
    fn comments_and_cdata_do_not_hide_or_fake_findings() {
        // 주석 안의 태그는 실행되지 않는다.
        let r = scan_chapter("<!-- <script>alert(1)</script> --><p>x</p>");
        assert!(!has(&r, "SCRIPT_TAG"));
        // 대소문자와 따옴표 없는 속성, 줄바꿈이 낀 태그도 잡는다.
        let r = scan_chapter("<SCRIPT\n type=text/javascript\n>alert(1)</SCRIPT>");
        assert!(has(&r, "SCRIPT_TAG"));
    }

    #[test]
    fn style_blocks_and_attributes_use_the_css_rules() {
        let r = scan_chapter("<style>p { width: expression(alert(1)); }</style>");
        assert_eq!(severity_of(&r, "CSS_EXPRESSION"), Severity::High);
        let r = scan_chapter(r#"<p style="behavior:url(x.htc); -moz-binding:url(http://x/y.xml#z)">x</p>"#);
        assert!(has(&r, "CSS_BEHAVIOR") && has(&r, "CSS_MOZ_BINDING"));
    }

    #[test]
    fn css_files_are_scanned() {
        let r = scan_extra(
            "OEBPS/a.css",
            b"@import url(http://evil.example/a.css); p{background:url(javascript:alert(1))} q{background:url('https://x.example/i.png')} @import 'https://y.example/b.css';",
        );
        assert!(has(&r, "REMOTE_CSS_IMPORT") && has(&r, "JS_URL") && has(&r, "REMOTE_RESOURCE"));
        let clean = scan_extra("OEBPS/a.css", b"p { color: red; background: url(img/a.png); }");
        assert!(clean.findings.is_empty(), "{:?}", clean.findings);
    }

    // ---- JavaScript 파일 ----

    #[test]
    fn javascript_members_are_flagged_and_scanned() {
        let r = scan_extra("OEBPS/a.js", b"eval('x'); new Function('return 1'); setTimeout('go()', 10); document.write('x'); var x = new XMLHttpRequest(); fetch('/a'); new WebSocket('ws://x'); navigator.sendBeacon('/b'); window.open('x'); location.href = 'y'; location.replace('z'); new ActiveXObject('WScript.Shell'); require('child_process'); localStorage.setItem('a','b'); document.cookie; importScripts('x'); import('x'); unescape('%41'); String.fromCharCode(1,2,3,4,5,6,7,8,9,10,11,12);");
        assert_eq!(severity_of(&r, "SCRIPT_FILE"), Severity::Medium);
        for code in ["DYNAMIC_CODE_EXEC", "DOCUMENT_WRITE", "NETWORK_API", "NAVIGATION", "HOST_API", "STORAGE_API", "DYNAMIC_IMPORT", "OBFUSCATION"] {
            assert!(has(&r, code), "{code}: {:?}", codes(&r));
        }
        let mjs = scan_extra("OEBPS/a.mjs", b"export const a = 1;");
        assert!(has(&mjs, "SCRIPT_FILE"));
    }

    #[test]
    fn plain_function_names_do_not_trigger_dynamic_code_rules() {
        let r = scan_extra("OEBPS/a.js", b"function evaluate() { return 1; } const retrieval = 2;");
        assert_eq!(codes(&r), ["SCRIPT_FILE"]);
    }

    #[test]
    fn long_hex_escape_runs_count_as_obfuscation() {
        let r = scan_extra("OEBPS/a.js", format!("var s = \"{}\";", "\\x41".repeat(40)).as_bytes());
        assert!(has(&r, "OBFUSCATION"));
    }

    // ---- ZIP 멤버 내용 ----

    #[test]
    fn executable_extensions_and_magic_bytes_are_flagged() {
        assert_eq!(severity_of(&scan_extra("OEBPS/x.exe", b"hello"), "EMBEDDED_EXECUTABLE"), Severity::Critical);
        assert!(has(&scan_extra("OEBPS/run.sh", b"echo"), "EMBEDDED_EXECUTABLE"));
        // 번들 실행 파일은 확장자로 위장했든 아니든 전부 Critical이다.
        assert_eq!(severity_of(&scan_extra("OEBPS/a.exe", b"MZ\x90\x00more"), "PE_EXECUTABLE_CONTENT"), Severity::Critical);
        assert_eq!(severity_of(&scan_extra("OEBPS/a.so", b"\x7fELF\x02\x01"), "ELF_BINARY_CONTENT"), Severity::Critical);
        assert_eq!(severity_of(&scan_extra("OEBPS/a.jpg", b"MZ\x90\x00more"), "PE_EXECUTABLE_CONTENT"), Severity::Critical);
        assert_eq!(severity_of(&scan_extra("OEBPS/a.jpg", b"\x7fELF\x02\x01"), "ELF_BINARY_CONTENT"), Severity::Critical);
        assert_eq!(severity_of(&scan_extra("OEBPS/a.jpg", b"\xcf\xfa\xed\xfe\x07"), "MACHO_BINARY_CONTENT"), Severity::Critical);
        assert_eq!(severity_of(&scan_extra("OEBPS/a.jpg", b"\xca\xfe\xba\xbe\x00\x00\x00\x34"), "JAVA_CLASS_CONTENT"), Severity::Medium);
        assert_eq!(severity_of(&scan_extra("OEBPS/a.txt", b"#!/bin/sh\nrm -rf /"), "SCRIPT_SHEBANG"), Severity::Medium);
        assert_eq!(severity_of(&scan_extra("OEBPS/a.bin", b"\xca\xfe\xba\xbe\x00\x00\x00\x02"), "MACHO_BINARY_CONTENT"), Severity::Medium);
        assert!(scan_extra("OEBPS/a.png", b"\x89PNG\r\n\x1a\nrest").findings.is_empty());
    }

    #[test]
    fn nested_archives_are_low() {
        assert_eq!(severity_of(&scan_extra("OEBPS/inner.zip", b"PK\x03\x04rest"), "NESTED_ARCHIVE"), Severity::Low);
    }

    #[test]
    fn high_compression_ratio_is_flagged_as_a_possible_bomb() {
        let r = scan_extra("OEBPS/zeros.bin", &vec![0u8; 3 * 1024 * 1024]);
        assert_eq!(severity_of(&r, "HIGH_COMPRESSION_RATIO"), Severity::Medium);
        // 작으면 비율이 높아도 무시한다.
        assert!(!has(&scan_extra("OEBPS/zeros.bin", &vec![0u8; 100 * 1024]), "HIGH_COMPRESSION_RATIO"));
    }

    // ---- ZIP 이름과 구조 ----

    fn raw_zip(path: &Path, names: &[&str]) {
        let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", opts).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.start_file("META-INF/container.xml", opts).unwrap();
        z.write_all(CONTAINER_XML.as_bytes()).unwrap();
        z.start_file("OEBPS/content.opf", opts).unwrap();
        z.write_all(clean_opf().as_bytes()).unwrap();
        z.start_file("OEBPS/ch1.xhtml", opts).unwrap();
        z.write_all(xhtml("<p>x</p>").as_bytes()).unwrap();
        for n in names {
            z.start_file(*n, opts).unwrap();
            z.write_all(b"data").unwrap();
        }
        z.finish().unwrap();
    }

    fn scan_names(names: &[&str]) -> FileReport {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        raw_zip(&p, names);
        scan(&p)
    }

    #[test]
    fn path_traversal_is_critical_and_absolute_names_are_high() {
        for n in ["../evil.txt", "OEBPS/../../evil.txt", "OEBPS/%2e%2e/evil.txt", "..\\evil.txt"] {
            assert_eq!(severity_of(&scan_names(&[n]), "ZIP_PATH_TRAVERSAL"), Severity::Critical, "{n}");
        }
        for n in ["/etc/passwd", "\\windows\\x", "C:/x.txt", "c:\\x.txt"] {
            assert_eq!(severity_of(&scan_names(&[n]), "ZIP_ABSOLUTE_PATH"), Severity::High, "{n}");
        }
    }

    #[test]
    fn backslash_control_reserved_and_stream_names_are_flagged() {
        assert_eq!(severity_of(&scan_names(&["OEBPS\\a.txt"]), "ZIP_BACKSLASH_PATH"), Severity::High);
        assert_eq!(severity_of(&scan_names(&["OEBPS/a\u{1b}b.txt"]), "ZIP_CONTROL_CHAR_NAME"), Severity::Medium);
        for n in ["OEBPS/CON.txt", "OEBPS/nul", "OEBPS/Com1.html", "OEBPS/lpt9.x"] {
            assert!(has(&scan_names(&[n]), "WINDOWS_RESERVED_NAME"), "{n}");
        }
        assert!(!has(&scan_names(&["OEBPS/console.txt"]), "WINDOWS_RESERVED_NAME"));
        assert_eq!(severity_of(&scan_names(&["OEBPS/a.xhtml:evil.exe"]), "ZIP_ALT_DATA_STREAM"), Severity::High);
        assert_eq!(severity_of(&scan_names(&[&format!("OEBPS/{}.txt", "a".repeat(300))]), "ZIP_NAME_TOO_LONG"), Severity::Low);
        assert_eq!(severity_of(&scan_names(&[&format!("OEBPS/{}.txt", "a".repeat(1_025))]), "ZIP_NAME_TOO_LONG"), Severity::Low);
    }

    #[test]
    fn names_that_collide_on_windows_are_flagged() {
        let r = scan_names(&["OEBPS/Page.xhtml", "OEBPS/page.xhtml "]);
        assert_eq!(severity_of(&r, "WINDOWS_FILENAME_COLLISION"), Severity::Medium);
        let r = scan_names(&["OEBPS/a.txt", "OEBPS/A.TXT"]);
        assert!(has(&r, "WINDOWS_FILENAME_COLLISION"));
        assert!(!has(&scan_names(&["OEBPS/a.txt", "OEBPS/b.txt"]), "WINDOWS_FILENAME_COLLISION"));
    }

    #[test]
    fn symlink_entries_are_flagged() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", stored).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.add_symlink("OEBPS/link", "/etc/passwd", stored).unwrap();
        z.finish().unwrap();
        assert_eq!(severity_of(&scan(&p), "ZIP_SYMLINK"), Severity::Medium);
    }

    #[test]
    fn damaged_local_headers_are_high() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_chapter(&p, &xhtml("<p>x</p>"));
        corrupt_local_header(&p, "OEBPS/ch1.xhtml");
        let r = scan(&p);
        assert_eq!(severity_of(&r, "BAD_LOCAL_HEADER"), Severity::High);

        let q = d.path().join("u.epub");
        epub_with_chapter(&q, &xhtml("<p>x</p>"));
        mangle_local_name(&q, "OEBPS/ch1.xhtml");
        assert_eq!(severity_of(&scan(&q), "ZIP_HEADER_NAME_MISMATCH"), Severity::High);
    }

    #[test]
    fn encrypted_and_unsupported_members_are_reported_as_incomplete() {
        let d = tempfile::tempdir().unwrap();
        let encrypted = d.path().join("encrypted.epub");
        epub_with_chapter(&encrypted, &xhtml("<p>x</p>"));
        patch_central(&encrypted, "OEBPS/ch1.xhtml", 8, &[0x01, 0x00]);
        let r = scan(&encrypted);
        assert!(r.incomplete);
        assert!(r.reasons.iter().any(|n| n.contains("ZIP-encrypted")), "{:?}", r.reasons);

        let unsupported = d.path().join("unsupported.epub");
        epub_with_chapter(&unsupported, &xhtml("<p>x</p>"));
        patch_central(&unsupported, "OEBPS/ch1.xhtml", 10, &[12, 0]);
        let r = scan(&unsupported);
        assert!(r.incomplete);
        assert!(r.reasons.iter().any(|n| n.contains("unsupported ZIP compression")), "{:?}", r.reasons);
    }

    // ---- mimetype / container ----

    #[test]
    fn epub_basics_are_checked() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            z.start_file("OEBPS/a.xhtml", o).unwrap();
            z.write_all(b"<p/>").unwrap();
            z.finish().unwrap();
        }
        let r = scan(&p);
        assert!(has(&r, "MISSING_MIMETYPE") && has(&r, "MISSING_CONTAINER_XML"));

        let q = d.path().join("u.epub");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&q).unwrap());
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            z.start_file("META-INF/container.xml", o).unwrap();
            z.write_all(CONTAINER_XML.as_bytes()).unwrap();
            z.start_file("mimetype", o).unwrap();
            z.write_all(b"text/plain").unwrap();
            z.finish().unwrap();
        }
        let r = scan(&q);
        assert_eq!(severity_of(&r, "NONSTANDARD_MIMETYPE"), Severity::Low);
        assert_eq!(severity_of(&r, "BAD_MIMETYPE"), Severity::Medium);
    }

    // ---- XML / OPF ----

    #[test]
    fn xml_external_entities_stylesheets_and_xinclude_are_flagged() {
        let r = scan_extra("OEBPS/a.xml", br#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY x SYSTEM "file:///etc/passwd">]><r>&x;</r>"#);
        assert_eq!(severity_of(&r, "XXE_ENTITY"), Severity::Critical);
        let r = scan_extra("OEBPS/a.xml", br#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY x SYSTEM "http://evil.example/e">]><r>&x;</r>"#);
        assert_eq!(severity_of(&r, "XXE_ENTITY"), Severity::High);
        let r = scan_extra("OEBPS/a.xml", br#"<!DOCTYPE r SYSTEM "http://evil.example/a.dtd"><r/>"#);
        assert_eq!(severity_of(&r, "EXTERNAL_DTD"), Severity::Medium);
        let r = scan_extra("OEBPS/a.xml", br#"<!DOCTYPE r [<!ENTITY a "aaaa">]><r>&a;</r>"#);
        assert_eq!(severity_of(&r, "ENTITY_DECLARATION"), Severity::High);
        let r = scan_extra("OEBPS/a.xml", br#"<?xml-stylesheet type="text/xsl" href="http://evil.example/a.xsl"?><r/>"#);
        assert_eq!(severity_of(&r, "XML_STYLESHEET"), Severity::Medium);
        let r = scan_extra("OEBPS/a.xml", br#"<r xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="/etc/passwd" parse="text"/></r>"#);
        assert_eq!(severity_of(&r, "XINCLUDE"), Severity::Medium);
        assert!(scan_extra("OEBPS/a.xml", b"<r><a>plain</a></r>").findings.is_empty());
    }

    #[test]
    fn standard_public_doctypes_are_low_noise() {
        let r = scan_chapter("<p>x</p>");
        assert!(r.findings.is_empty());
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        let page = format!(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n{}",
            xhtml("<p>x</p>").replace("<?xml version=\"1.0\"?>", "")
        );
        epub_with_files(&p, &clean_opf(), &page, &[]);
        let r = scan(&p);
        assert_eq!(severity_of(&r, "EXTERNAL_DTD"), Severity::Low);
    }

    #[test]
    fn opf_manifest_is_checked_for_escapes_remote_items_and_script_media_types() {
        let items = format!(
            "{}{}{}{}{}",
            opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""),
            opf_item("e1", "../../outside.txt", "text/plain", ""),
            opf_item("r1", "https://x.example/a.png", "image/png", ""),
            opf_item("f1", "file:///etc/passwd", "text/plain", ""),
            opf_item("s1", "a.js", "application/javascript", "")
        );
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &opf(&items), &xhtml("<p>x</p>"), &[("OEBPS/a.js", b"1")]);
        let r = scan(&p);
        assert_eq!(severity_of(&r, "MANIFEST_PATH_ESCAPE"), Severity::High);
        assert_eq!(severity_of(&r, "REMOTE_MANIFEST_ITEM"), Severity::Medium);
        assert_eq!(severity_of(&r, "DANGEROUS_URI_SCHEME"), Severity::High);
        assert_eq!(severity_of(&r, "SCRIPT_MEDIA_TYPE"), Severity::High);
    }

    #[test]
    fn opf_manifest_reports_unc_and_unknown_schemes() {
        let items = [opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""), opf_item("u1", "\\\\host\\share\\x", "text/plain", ""), opf_item("x1", "weird-app://open", "text/plain", "")].concat();
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &opf(&items), &xhtml("<p>x</p>"), &[]);
        let r = scan(&p);
        assert!(has(&r, "UNC_PATH") && has(&r, "UNKNOWN_URI_SCHEME"), "{:?}", codes(&r));
    }

    #[test]
    fn malformed_opf_is_noted_without_aborting() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, "<package", &xhtml("<p>x</p>"), &[]);
        let r = scan(&p);
        assert!(r.reasons.iter().any(|n| n.contains("OEBPS/content.opf")), "{:?}", r.reasons);
        assert!(r.incomplete);
    }

    // ---- 한도와 불완전 ----

    #[test]
    fn limits_make_the_scan_incomplete_instead_of_failing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/big.bin", &vec![7u8; 4096])]);
        let r = scan_with(&p, &Limits { max_member: 1024, max_total: 200 * 1024 * 1024, max_files: 10_000 });
        assert!(r.incomplete);
        assert!(r.reasons.iter().any(|n| n.contains("OEBPS/big.bin") && n.contains("max inspection size")), "{:?}", r.reasons);

        let r = scan_with(&p, &Limits { max_member: 1 << 20, max_total: 100, max_files: 10_000 });
        assert!(r.reasons.iter().any(|n| n.contains("Total decompressed-byte scan limit")), "{:?}", r.reasons);

        let (r, stats) = scan_with_stats(&p, &Limits { max_member: 1 << 20, max_total: 1 << 28, max_files: 2 });
        assert!(r.reasons.iter().any(|n| n.contains("Too many entries")), "{:?}", r.reasons);
        assert!(stats.entries >= 5);
    }

    #[test]
    fn unreadable_members_are_noted_when_the_data_is_corrupt() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_chapter(&p, &xhtml(&"<p>안녕하세요</p>".repeat(50)));
        // 압축 데이터 한가운데를 망가뜨려 CRC·해제 오류를 낸다.
        let mut data = fs::read(&p).unwrap();
        let marker = b"OEBPS/ch1.xhtml";
        let off = data.windows(marker.len()).position(|w| w == marker).unwrap() + marker.len() + 8;
        for b in &mut data[off..off + 16] {
            *b ^= 0xff;
        }
        fs::write(&p, data).unwrap();
        let r = scan(&p);
        assert!(r.reasons.iter().any(|n| n.contains("Could not decompress")), "{:?} {:?}", r.reasons, r.findings);
    }

    #[test]
    fn non_zip_and_missing_files_are_incomplete() {
        let d = tempfile::tempdir().unwrap();
        let bad = d.path().join("bad.epub");
        fs::write(&bad, b"not a zip").unwrap();
        for p in [bad, d.path().join("none.epub")] {
            let (r, stats) = scan_with_stats(&p, &Limits::default());
            assert!(r.incomplete);
            assert!(r.reasons[0].contains("Cannot open as ZIP EPUB"));
            assert_eq!(stats.entries, 0);
        }
    }

    // ---- 정렬과 출력 안전 ----

    #[test]
    fn findings_are_sorted_by_severity_then_code_then_location() {
        let r = scan_chapter(r#"<script>eval(1)</script><img src="https://x.example/a.gif"><base href="x">"#);
        let sev: Vec<Severity> = r.findings.iter().map(|f| f.severity).collect();
        let mut sorted = sev.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(sev, sorted);
    }

    #[test]
    fn report_strings_never_carry_control_characters() {
        let r = scan_names(&["OEBPS/a\u{1b}[2J\u{85}b.exe"]);
        for f in &r.findings {
            assert!(!f.location.chars().any(|c| c.is_control()) && !f.detail.chars().any(|c| c.is_control()), "{f:?}");
        }
        assert!(r.findings.iter().all(|f| f.location.chars().count() <= 160 && f.detail.chars().count() <= 160));
    }

    #[test]
    fn report_serializes_findings_with_stable_severity_labels() {
        let r = scan_chapter("<script>1</script>");
        let v = serde_json::to_value(&r).unwrap();
        // 옛 `status` 라벨은 FileReport에 없다(요약은 Task 3·4의 report가 맡는다).
        assert_eq!(v["incomplete"], false);
        assert_eq!(v["findings"][0]["severity"], "HIGH");
        assert!(v["findings"][0]["code"].is_string());
    }

    // ---- 파서 차이를 이용한 우회 ----

    #[test]
    fn declared_media_type_decides_which_scanner_runs() {
        let items = format!(
            "{}{}{}{}",
            opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""),
            opf_item("p1", "payload.dat", "application/xhtml+xml", ""),
            opf_item("p2", "pic.png", "text/javascript", ""),
            opf_item("p3", "style.txt", "text/css", "")
        );
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(
            &p,
            &opf(&items),
            &xhtml("<p>x</p>"),
            &[("OEBPS/payload.dat", b"<html><script>alert(1)</script></html>"), ("OEBPS/pic.png", b"eval(document.cookie)"), ("OEBPS/style.txt", b"p { width: expression(alert(1)); }")],
        );
        let r = scan(&p);
        for code in ["SCRIPT_TAG", "DYNAMIC_CODE_EXEC", "CSS_EXPRESSION"] {
            assert!(has(&r, code), "{code}: {:?}", codes(&r));
        }
        let mismatches: Vec<&str> = r.findings.iter().filter(|f| f.code == "MEDIA_TYPE_MISMATCH").map(|f| f.location.as_str()).collect();
        assert_eq!(severity_of(&r, "MEDIA_TYPE_MISMATCH"), Severity::Medium);
        assert_eq!(mismatches.len(), 3, "{mismatches:?}");
    }

    #[test]
    fn compatible_declarations_are_not_mismatches() {
        let items = format!("{}{}{}", opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""), opf_item("n1", "toc.ncx", "application/x-dtbncx+xml", ""), opf_item("i1", "a.png", "image/png", ""));
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &opf(&items), &xhtml("<p>x</p>"), &[("OEBPS/toc.ncx", b"<ncx/>"), ("OEBPS/a.png", b"\x89PNG\r\n\x1a\n")]);
        assert!(!has(&scan(&p), "MEDIA_TYPE_MISMATCH"));
    }

    #[test]
    fn the_package_document_named_by_container_xml_is_found_without_an_opf_extension() {
        // rootfile이 .opf가 아닌 이름을 가리켜도 매니페스트를 읽어 선언 검사를 한다.
        let container = CONTAINER_XML.replace("OEBPS/content.opf", "OEBPS/package.xml");
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            z.start_file("mimetype", o).unwrap();
            z.write_all(b"application/epub+zip").unwrap();
            z.start_file("META-INF/container.xml", o).unwrap();
            z.write_all(container.as_bytes()).unwrap();
            z.start_file("OEBPS/package.xml", o).unwrap();
            z.write_all(scripted_opf().replace("scripted", "").as_bytes()).unwrap();
            z.start_file("OEBPS/ch1.xhtml", o).unwrap();
            z.write_all(xhtml("<script>1</script>").as_bytes()).unwrap();
            z.finish().unwrap();
        }
        assert!(has(&scan(&p), "UNDECLARED_SCRIPTED_CONTENT"));
    }

    #[test]
    fn every_javascript_mime_type_and_unknown_types_count_as_executable() {
        for ty in [
            "text/jscript.encode",
            "TEXT/JAVASCRIPT",
            "text/javascript1.2",
            "application/x-ecmascript",
            "text/livescript",
            "text/x-javascript",
            "module",
            "application/javascript; charset=utf-8",
            "  text/javascript  ",
            "text/vbscript",
            "text/x-custom",
        ] {
            let r = scan_chapter(&format!(r#"<script type="{ty}">alert(1)</script>"#));
            assert_eq!(severity_of(&r, "SCRIPT_TAG"), Severity::High, "{ty:?}");
        }
        for ty in ["application/json", "application/ld+json", "text/plain", "text/x-handlebars-template", "importmap", "speculationrules"] {
            let r = scan_chapter(&format!(r#"<script type="{ty}">{{"a":1}}</script>"#));
            assert_eq!(severity_of(&r, "SCRIPT_TAG"), Severity::Low, "{ty:?}");
        }
    }

    #[test]
    fn markup_hidden_after_cdata_or_odd_comments_is_still_scanned() {
        for body in [
            "<![CDATA[ <script>alert(1)</script> ]]>",
            "<!--> <script>alert(1)</script>",
            "<!---> <script>alert(1)</script>",
            "<!-- x --!> <script>alert(1)</script>",
            "<p>x</p><!-- <script>alert(1)</script>",
        ] {
            let r = scan_chapter(body);
            assert!(has(&r, "SCRIPT_TAG"), "{body}: {:?}", codes(&r));
        }
        // 닫히지 않은 주석은 파서마다 해석이 달라 따로 알린다.
        let r = scan_chapter("<p>x</p><!-- never closed");
        assert_eq!(severity_of(&r, "MALFORMED_MARKUP"), Severity::Low);
        // 정상적으로 닫힌 주석은 여전히 숨긴다.
        assert!(!has(&scan_chapter("<!-- <script>alert(1)</script> --><p>x</p>"), "SCRIPT_TAG"));
        assert!(!has(&scan_chapter("<!-- a --><p>x</p>"), "MALFORMED_MARKUP"));
    }

    // ---- CRITICAL: 정상 문서에 있을 이유가 없는 수법 ----

    #[test]
    fn host_command_and_obfuscated_loader_are_critical_but_library_activex_is_not() {
        let r = scan_extra("OEBPS/a.js", b"var s = new ActiveXObject('WScript.Shell'); s.Run('calc');");
        assert_eq!(severity_of(&r, "HOST_COMMAND_EXEC"), Severity::Critical);
        let r = scan_extra("OEBPS/a.js", b"var cp = require('child_process');");
        assert_eq!(severity_of(&r, "HOST_COMMAND_EXEC"), Severity::Critical);
        for payload in ["eval(atob('YQ=='))", "eval(unescape('%61'))", "new Function(String.fromCharCode(97))()"] {
            let r = scan_extra("OEBPS/a.js", payload.as_bytes());
            assert_eq!(severity_of(&r, "OBFUSCATED_LOADER"), Severity::Critical, "{payload}");
        }
        // jQuery 1.x가 IE에서 XML을 파싱할 때 쓰는 ActiveXObject와 `new Function("return " + data)`는 정상이다.
        let jq = scan_extra("OEBPS/a.js", br#"xml = new ActiveXObject("Microsoft.XMLDOM"); return (new Function("return " + b))();"#);
        assert!(has(&jq, "DYNAMIC_CODE_EXEC") && !has(&jq, "HOST_API"), "{:?}", codes(&jq));
        assert!(!has(&jq, "HOST_COMMAND_EXEC") && !has(&jq, "OBFUSCATED_LOADER"), "{:?}", codes(&jq));
        assert!(jq.findings.iter().all(|f| f.severity != Severity::Critical));
    }

    #[test]
    fn script_tag_without_malicious_pattern_never_reaches_critical() {
        let r = scan_chapter("<script>var n = eval('1+1'); document.getElementById('a').onclick = function () {};</script><button onclick=\"go()\">x</button>");
        assert!(r.findings.iter().all(|f| f.severity != Severity::Critical), "{:?}", r.findings);
    }

    #[test]
    fn host_api_ignores_library_activex_fallbacks_and_feature_detection_but_keeps_the_rest() {
        let benign: [&[u8]; 5] = [
            br#"var x = window.ActiveXObject ? new ActiveXObject("Microsoft.XMLHTTP") : new XMLHttpRequest();"#,
            br#"xml = new window.ActiveXObject( 'Msxml2.XMLHTTP.6.0' );"#,
            br#"var d = new ActiveXObject("MSXML2.DOMDocument");"#,
            br#"if ("ActiveXObject" in window) { var ie = true; }"#,
            br#"var a = typeof ActiveXObject !== 'undefined';"#,
        ];
        for js in benign {
            let r = scan_extra("OEBPS/a.js", js);
            assert!(!has(&r, "HOST_API") && !has(&r, "HOST_COMMAND_EXEC"), "{}: {:?}", String::from_utf8_lossy(js), codes(&r));
        }
        // 모르는 ProgID, 동적 인자, 호스트 객체는 계속 High다.
        let risky: [&[u8]; 6] = [
            br#"new ActiveXObject("Some.Unknown.Control");"#,
            br#"new ActiveXObject(prog);"#,
            br#"new ActiveXObject ( "Microsoft.XMLDOM" + x );"#,
            b"WScript.Echo(1);",
            b"var fs = require('fs');",
            b"process.binding('natives');",
        ];
        for js in risky {
            let r = scan_extra("OEBPS/a.js", js);
            assert_eq!(severity_of(&r, "HOST_API"), Severity::High, "{}", String::from_utf8_lossy(js));
        }
        // 레거시 AJAX 라이브러리가 쓰는 MSXML2.XMLHTTP는 Critical이 아니고, 서버측 다운로더만 Critical이다.
        let r = scan_extra("OEBPS/a.js", br#"new ActiveXObject("MSXML2.ServerXMLHTTP");"#);
        assert_eq!(severity_of(&r, "HOST_COMMAND_EXEC"), Severity::Critical);
    }

    #[test]
    fn activex_aliasing_and_comment_tricks_do_not_hide_host_calls() {
        let evasions: [&[u8]; 11] = [
            b"function f(C) { return new C('x'); } f(ActiveXObject);",
            b"var C = [ActiveXObject][0]; new C(prog);",
            b"function g() { return ActiveXObject; }",
            br#"var s = " // "; new ActiveXObject("Some.Unknown.Control");"#,
            br#"var s = new (window.ActiveXObject)('Some.Unknown.Control');"#,
            br#"var o = { A: window.ActiveXObject }; new o.A(prog);"#,
            br#"var A = window.ActiveXObject; var s = new A("WScript.Shell");"#,
            br#"var A = ActiveXObject; new A(prog);"#,
            br#"var A = window["ActiveXObject"]; new A(prog);"#,
            br#"new ActiveXObject/* x */("Some.Unknown.Control");"#,
            br#"var s = new (window.ActiveXObject)('Scripting.FileSystemObject');"#,
        ];
        for js in evasions {
            let r = scan_extra("OEBPS/a.js", js);
            assert!(has(&r, "HOST_API") || has(&r, "HOST_COMMAND_EXEC"), "{}: {:?}", String::from_utf8_lossy(js), codes(&r));
        }
        // 위험 ProgID는 생성자 모양과 무관하게 Critical이다.
        for js in [&br#"var A = window.ActiveXObject; new A("WScript.Shell");"#[..], &br#"new (window.ActiveXObject)('Scripting.FileSystemObject');"#[..]] {
            assert_eq!(severity_of(&scan_extra("OEBPS/a.js", js), "HOST_COMMAND_EXEC"), Severity::Critical, "{}", String::from_utf8_lossy(js));
        }
    }

    #[test]
    fn boolean_feature_detection_is_not_mistaken_for_activex_use() {
        let cases: [&[u8]; 3] = [br#"if (ready && window.ActiveXObject) { useLegacy(); }"#, br#"if (ready || self.ActiveXObject) { useLegacy(); }"#, br#"var supported = !window.ActiveXObject;"#];
        for js in cases {
            let r = scan_extra("OEBPS/a.js", js);
            assert!(!has(&r, "HOST_API"), "{}: {:?}", String::from_utf8_lossy(js), codes(&r));
        }
    }

    #[test]
    fn directory_entries_are_skipped_without_affecting_epub_findings() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.add_directory("OEBPS/empty/", o).unwrap();
        z.start_file("META-INF/container.xml", o).unwrap();
        z.write_all(CONTAINER_XML.as_bytes()).unwrap();
        z.start_file("OEBPS/content.opf", o).unwrap();
        z.write_all(clean_opf().as_bytes()).unwrap();
        z.start_file("OEBPS/ch1.xhtml", o).unwrap();
        z.write_all(xhtml("<p>ok</p>").as_bytes()).unwrap();
        z.finish().unwrap();

        let (r, stats) = scan_with_stats(&p, &Limits::default());
        assert!(!r.incomplete && r.findings.is_empty(), "{:?}", r.findings);
        assert_eq!(stats.entries, 5);
    }

    #[test]
    fn a_later_critical_finding_is_not_hidden_by_an_earlier_high_one_at_the_same_location() {
        for body in [r##"<a href="smb://h/s">a</a><a href="ms-msdt:/id">b</a>"##, r##"<a href="ms-msdt:/id">b</a><a href="smb://h/s">a</a>"##] {
            let r = scan_chapter(body);
            assert_eq!(severity_of(&r, "DANGEROUS_URI_SCHEME"), Severity::Critical, "{body}");
            assert_eq!(r.findings.iter().filter(|f| f.code == "DANGEROUS_URI_SCHEME").count(), 1);
        }
    }

    // ---- 큰 파일과 메모리 ----

    #[test]
    fn a_panicking_scan_becomes_an_incomplete_report_instead_of_killing_the_batch() {
        let mut r = FileReport::new(Path::new("/x/y.epub"), Some(Format::Epub));
        guarded(&mut r, |_| panic!("parser bug"));
        assert!(r.incomplete);
        assert!(r.findings.is_empty());
        assert!(r.path.ends_with("y.epub") && r.reasons.iter().any(|n| n.starts_with("panic:") && n.contains("parser bug")), "{:?}", r.reasons);
    }

    #[test]
    fn findings_found_before_a_panic_are_kept() {
        let mut r = FileReport::new(Path::new("/x/y.epub"), Some(Format::Epub));
        guarded(&mut r, |rep| {
            rep.push(Category::Security, Severity::High, "SCRIPT_TAG", "a.xhtml", "d");
            panic!("boom");
        });
        assert!(r.incomplete && r.has("SCRIPT_TAG"));
    }

    /// ZIP을 파일 중간에서 잘라도 panic하지 않고 incomplete이거나 지표를 낸다(옛 도구와 같다).
    #[test]
    fn truncated_zip_is_incomplete_not_abort() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_chapter(&p, &xhtml("<p>안녕하세요</p>"));
        let full = fs::read(&p).unwrap();
        for cut in [full.len() / 2, full.len() - 10, 30, 3] {
            fs::write(&p, &full[..cut]).unwrap();
            let r = scan(&p);
            assert!(r.incomplete || !r.findings.is_empty(), "cut={cut}");
            assert!(!r.reasons.iter().any(|n| n.starts_with("panic:")), "cut={cut}: {:?}", r.reasons);
        }
    }

    /// 구조 성격의 네 코드는 Render로, 나머지는 Security로 보고한다.
    #[test]
    fn structure_codes_are_render_and_the_rest_are_security() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"text/plain").unwrap();
        z.start_file("a.xhtml", o).unwrap();
        z.write_all(b"<script>1</script>").unwrap();
        z.finish().unwrap();
        let r = scan(&p);
        for code in ["MISSING_CONTAINER_XML", "BAD_MIMETYPE"] {
            let f = r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음"));
            assert_eq!(f.category, Category::Render, "{code}");
        }
        let f = r.findings.iter().find(|f| f.code == "SCRIPT_TAG").unwrap();
        assert_eq!(f.category, Category::Security);
    }

    #[test]
    fn missing_and_nonstandard_mimetype_are_render() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("a.txt", o).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        let r = scan(&p);
        assert_eq!(r.findings.iter().find(|f| f.code == "MISSING_MIMETYPE").map(|f| f.category), Some(Category::Render));

        let q = d.path().join("u.epub");
        let mut z = zip::ZipWriter::new(fs::File::create(&q).unwrap());
        z.start_file("a.txt", o).unwrap();
        z.write_all(b"x").unwrap();
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.finish().unwrap();
        let r = scan(&q);
        assert_eq!(r.findings.iter().find(|f| f.code == "NONSTANDARD_MIMETYPE").map(|f| f.category), Some(Category::Render));
    }

    /// 끝에 zip64 EOCD만 있는 파일. 항목 수와 중앙 디렉터리 크기를 마음대로 적을 수 있다.
    fn zip64_stub(entries: u64, dir_bytes: u64) -> Vec<u8> {
        let mut rec = vec![0u8; 56];
        rec[..4].copy_from_slice(b"PK\x06\x06");
        rec[32..40].copy_from_slice(&entries.to_le_bytes());
        rec[40..48].copy_from_slice(&dir_bytes.to_le_bytes());
        let mut loc = vec![0u8; 20];
        loc[..4].copy_from_slice(b"PK\x06\x07"); // zip64 EOCD 위치 = 0
        let mut eocd = vec![0u8; 22];
        eocd[..4].copy_from_slice(b"PK\x05\x06");
        eocd[10..12].copy_from_slice(&0xFFFFu16.to_le_bytes());
        eocd[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        [rec, loc, eocd].concat()
    }

    #[test]
    fn zip64_directories_that_would_exhaust_memory_are_refused_before_opening() {
        let d = tempfile::tempdir().unwrap();
        for (entries, dir_bytes, needle) in [(5_000_000u64, 1_000u64, "Too many entries"), (10, 10u64 << 30, "Central directory too large")] {
            let p = d.path().join("huge.epub");
            fs::write(&p, zip64_stub(entries, dir_bytes)).unwrap();
            let (r, stats) = scan_with_stats(&p, &Limits::default());
            assert!(r.incomplete, "{needle}");
            assert!(r.reasons.iter().any(|n| n.contains(needle) && n.contains("not opened")), "{:?}", r.reasons);
            assert_eq!(stats.entries, 0);
        }
        // 정상 크기의 zip64 표기는 막지 않는다(열기를 시도하고, 못 열면 평소의 메모를 남긴다).
        let p = d.path().join("small.epub");
        fs::write(&p, zip64_stub(3, 100)).unwrap();
        assert!(scan(&p).reasons.iter().any(|n| n.contains("Cannot open")));
        // zip64 로케이터나 레코드가 없으면 알아보지 못한 것으로 보고 평소처럼 연다.
        let mut broken = zip64_stub(5_000_000, 1);
        broken[76 - 20] = b'X';
        fs::write(&p, broken).unwrap();
        assert!(scan(&p).reasons.iter().any(|n| n.contains("Cannot open")));
        let mut invalid_record = zip64_stub(5_000_000, 1);
        invalid_record[..4].copy_from_slice(b"NOPE");
        fs::write(&p, invalid_record).unwrap();
        assert!(scan(&p).reasons.iter().any(|n| n.contains("Cannot open")));
    }

    #[test]
    fn the_total_scan_limit_leaves_one_note_not_one_per_member() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        let extra: Vec<(String, Vec<u8>)> = (0..20).map(|i| (format!("OEBPS/f{i}.bin"), vec![1u8; 2048])).collect();
        let refs: Vec<(&str, &[u8])> = extra.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &refs);
        let r = scan_with(&p, &Limits { max_member: 1 << 20, max_total: 100, max_files: 10_000 });
        let notes: Vec<_> = r.reasons.iter().filter(|n| n.contains("Total decompressed-byte scan limit")).collect();
        assert_eq!(notes.len(), 1, "{:?}", r.reasons);
        assert!(notes[0].contains("entries not inspected"));
    }
}

/// 본문 한 장짜리 EPUB을 만들어 검사한다. OPF는 scripted 선언이 없는 기본형이다.
fn scan_chapter(body: &str) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_chapter(&p, &xhtml(body));
    scan(&p)
}

fn scan_epub(chapter: &str, extra: &[(&str, &[u8])]) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &clean_opf(), &xhtml(chapter), extra);
    let mut r = FileReport::new(&p, Some(Format::Epub));
    scan_into(&p, &Limits::default(), &mut r);
    r
}

fn sev(r: &FileReport, code: &str) -> Option<Severity> {
    r.findings.iter().filter(|f| f.code == code).map(|f| f.severity).max()
}

fn js(body: &str) -> FileReport {
    scan_epub(&format!("<script>{body}</script>"), &[])
}

// ---- 토크나이저 가장자리 ----

#[test]
fn truncated_and_odd_markup_never_panics_and_still_finds_what_is_there() {
    for body in [
        "<a href=\"x",
        "<p class=a/>",
        "<img src=x onerror=1",
        "<",
        "<1>",
        "x < y",
        "</",
        "<a",
        "<!doctype",
        "<?xml",
        "<p =x>",
        "<p a= >",
        "<p a='b>",
        "<p a=b c>",
        "&",
        "&#",
        "&#x",
        "&unknown; &#xZZ; &#99999999;",
    ] {
        let _ = scan_chapter(body);
    }
    // 따옴표 없는 속성값과 값 없는 속성이 섞여도 핸들러를 잡는다.
    assert!(has(&scan_chapter("<p hidden onclick=alert(1)>x</p>"), "EVENT_HANDLER_ATTR"));
    assert!(has(&scan_chapter("<script></script"), "SCRIPT_TAG"));
    assert!(has(&scan_chapter("<script>alert(1)"), "SCRIPT_TAG"));
}

#[test]
fn named_and_numeric_entities_are_decoded_before_scheme_checks() {
    for href in ["javascript&colon;alert(1)", "java&Tab;script:alert(1)", "java&NewLine;script:alert(1)", "javascript&#58;alert(1)", "&#x6A;avascript:alert(1)", "&bsol;&bsol;host\\share"] {
        let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
        assert!(has(&r, "JS_URL") || has(&r, "UNC_PATH") || has(&r, "UNKNOWN_URI_SCHEME"), "{href}");
    }
    // 괄호 엔티티는 스킴으로 이어지지 않는다(`javascript(1):x`는 JS URL이 아니다).
    assert!(!has(&scan_chapter(r#"<a href="javascript&lpar;1&rpar;:x">x</a>"#), "JS_URL"));
    // 그 밖의 이름 있는 엔티티는 그대로 두되 스킴 판정에는 영향이 없다.
    for ent in ["amp", "lt", "gt", "quot", "apos", "sol", "nbsp"] {
        let r = scan_chapter(&format!(r#"<a href="a&{ent};b">x</a>"#));
        assert!(!has(&r, "JS_URL"), "{ent}");
    }
}

#[test]
fn script_src_with_a_local_or_dangerous_url_is_classified() {
    assert!(has(&scan_chapter(r#"<script src="javascript:alert(1)"></script>"#), "JS_URL"));
    assert!(!has(&scan_chapter(r#"<script src="a.js"></script>"#), "REMOTE_SCRIPT"));
}

#[test]
fn svg_animation_values_with_script_urls_are_flagged() {
    let r = scan_chapter(r#"<svg xmlns="http://www.w3.org/2000/svg"><animate attributeName="href" values="javascript:alert(1)"/><set attributeName="xlink:href" from="vbscript:x" to="ok"/></svg>"#);
    assert!(has(&r, "JS_URL") && has(&r, "SVG_ANIMATE_HREF"));
    // href가 아닌 속성을 바꾸는 애니메이션은 무시한다.
    assert!(!has(&scan_chapter(r#"<svg xmlns="http://www.w3.org/2000/svg"><animate attributeName="opacity" values="0;1"/></svg>"#), "SVG_ANIMATE_HREF"));
}

#[test]
fn meta_without_refresh_and_base_without_href_are_ignored() {
    let r = scan_chapter(r#"<meta name="viewport" content="x"><base target="_blank"><meta http-equiv="refresh" content="5">"#);
    assert!(!has(&r, "BASE_HREF"));
    assert_eq!(r.findings.iter().find(|f| f.code == "META_REFRESH").map(|f| f.severity), Some(Severity::Medium));
}

// ---- 번들 실행 파일은 무조건 CRITICAL ----

#[test]
fn bundled_executables_are_always_critical() {
    for name in ["OEBPS/Misc/setup.exe", "OEBPS/a.dll", "OEBPS/run.sh", "OEBPS/x.lnk", "OEBPS/a.bat", "OEBPS/a.jar"] {
        let r = scan_epub("<p>t</p>", &[(name, b"hello")]);
        assert_eq!(sev(&r, "EMBEDDED_EXECUTABLE"), Some(Severity::Critical), "{name}");
    }
    let r = scan_epub("<p>t</p>", &[("OEBPS/Misc/a.exe", b"MZ\x90\x00more")]);
    assert_eq!(sev(&r, "PE_EXECUTABLE_CONTENT"), Some(Severity::Critical));
    let r = scan_epub("<p>t</p>", &[("OEBPS/a.so", b"\x7fELF\x02\x01")]);
    assert_eq!(sev(&r, "ELF_BINARY_CONTENT"), Some(Severity::Critical));
    let r = scan_epub("<p>t</p>", &[("OEBPS/a.dylib", b"\xcf\xfa\xed\xfe\x07")]);
    assert_eq!(sev(&r, "MACHO_BINARY_CONTENT"), Some(Severity::Critical));
    // 실행 확장자가 아니고 내용도 실행 코드가 아니면 지표가 없다.
    assert!(scan_epub("<p>t</p>", &[("OEBPS/a.png", b"\x89PNG\r\n\x1a\nrest")]).findings.is_empty());
}

// ---- 이벤트 핸들러·javascript: URL 본문 ----

#[test]
fn event_handler_bodies_are_scanned_like_scripts() {
    let r = scan_epub(r#"<div onclick="eval(atob('YWxlcnQoMSk='))">x</div>"#, &[]);
    assert_eq!(sev(&r, "OBFUSCATED_LOADER"), Some(Severity::Critical), "{:?}", r.findings);
    let r = scan_epub(r#"<div onclick="go()">x</div>"#, &[]);
    assert_eq!(sev(&r, "EVENT_HANDLER_ATTR"), Some(Severity::High));
    assert_eq!(sev(&r, "OBFUSCATED_LOADER"), None);
}

#[test]
fn javascript_url_bodies_are_scanned_like_scripts() {
    for href in ["javascript:eval(atob('YWxlcnQoMSk='))", "JaVa\tScRiPt:eval(atob('x'))", "javascript:document.write(atob('PHNjcmlwdD4='))"] {
        let r = scan_epub(&format!(r#"<a href="{href}">x</a>"#), &[]);
        assert_eq!(sev(&r, "OBFUSCATED_LOADER"), Some(Severity::Critical), "{href}: {:?}", r.findings);
    }
    let r = scan_epub(r#"<a href="javascript:void(0)">x</a>"#, &[]);
    assert_eq!(sev(&r, "JS_URL"), Some(Severity::High));
    assert_eq!(sev(&r, "OBFUSCATED_LOADER"), None);
}

// ---- 난독화 로더 우회 ----

#[test]
fn decoded_payload_loaders_are_critical_in_every_shape() {
    for body in [
        "eval(atob('YWxlcnQoMSk='))",
        "setTimeout(atob('YWxlcnQoMSk='),1)",
        "setInterval(unescape('%61%6c'),1)",
        "document.writeln(atob('PHNjcmlwdD4='))",
        "document.write(String.fromCharCode(60,115))",
        "var e=eval; var p=atob('YWxlcnQoMSk='); e(p);",
        "var f=window.Function; f(String.fromCharCode(97,108))()",
        "window['ev'+'al'](atob('YWxlcnQoMSk='))",
        "self[\"ev\"+\"al\"](decodeURIComponent('%61'))",
        "(0,eval)(atob('YWxlcnQoMSk='))",
    ] {
        assert_eq!(sev(&js(body), "OBFUSCATED_LOADER"), Some(Severity::Critical), "{body}");
    }
}

#[test]
fn analytics_style_document_write_unescape_and_distant_decode_calls_are_not_critical() {
    // Google Analytics 방식 로더: High로만 남는다.
    let r = js("document.write(unescape('%3Cscript src=\"' + document.location.protocol + '//x.example/a.js\"%3E%3C/script%3E'))");
    assert_eq!(sev(&r, "OBFUSCATED_LOADER"), Some(Severity::High), "{:?}", r.findings);
    // jQuery의 globalEval과 멀리 떨어진 decodeURIComponent는 한 로더로 묶지 않는다.
    let filler = " ".repeat(2000);
    let r = js(&format!("function g(d){{ window[ \"eval\" ].call( window, d ); }}{filler} var q = decodeURIComponent(location.search);"));
    assert_eq!(sev(&r, "OBFUSCATED_LOADER"), None, "{:?}", r.findings);
    assert_eq!(sev(&r, "DYNAMIC_CODE_EXEC"), Some(Severity::High));
}

#[test]
fn aliased_exec_without_decoding_is_high_and_ordinary_code_is_not_critical() {
    let r = js("window['ev'+'al']('1+1')");
    assert_eq!(sev(&r, "DYNAMIC_CODE_EXEC"), Some(Severity::High), "{:?}", r.findings);
    assert_eq!(sev(&r, "OBFUSCATED_LOADER"), None);
    for body in [
        "document.write('hello')",
        "var s = unescape('%41');",
        "var x = atob('YQ==');",
        "function evaluate(a){return a}; var retrieval = 2;",
        "var o = {eval: 1}; o.evaluate = 2;",
        "var s = 'a' + 'b'; var t = s + atob('YQ==');",
    ] {
        assert_ne!(sev(&js(body), "OBFUSCATED_LOADER"), Some(Severity::Critical), "{body}");
    }
}

// ---- 클릭 없는 UNC·원격 file 로드, meta refresh ----

#[test]
fn zero_click_network_resource_loads_are_critical() {
    for tag in [
        r#"<img src="\\evil\share\x.png">"#,
        r#"<img src="smb://evil/share/x.png">"#,
        r#"<img src="file://evil/share/x.png">"#,
        r#"<link rel="stylesheet" href="\\evil\share\a.css">"#,
        r#"<iframe src="smb://evil/s/a.html"></iframe>"#,
    ] {
        let r = scan_epub(tag, &[]);
        let worst = r.findings.iter().filter(|f| matches!(f.code.as_str(), "UNC_PATH" | "DANGEROUS_URI_SCHEME")).map(|f| f.severity).max();
        assert_eq!(worst, Some(Severity::Critical), "{tag}: {:?}", r.findings);
    }
}

#[test]
fn clicked_links_and_local_file_loads_stay_high() {
    for tag in [
        r#"<a href="\\evil\share\x.png">x</a>"#,
        r#"<a href="smb://evil/share/x">x</a>"#,
        r#"<a href="file://evil/share/x">x</a>"#,
        r#"<img src="file:///C:/Users/a/x.png">"#,
        r#"<img src="file://localhost/C:/x.png">"#,
    ] {
        let r = scan_epub(tag, &[]);
        let worst = r.findings.iter().filter(|f| matches!(f.code.as_str(), "UNC_PATH" | "DANGEROUS_URI_SCHEME")).map(|f| f.severity).max();
        assert_eq!(worst, Some(Severity::High), "{tag}: {:?}", r.findings);
    }
}

#[test]
fn meta_refresh_to_an_exploit_scheme_is_critical() {
    let r = scan_epub(r#"<meta http-equiv="refresh" content="0;url=ms-msdt:/id PCWDiagnostic">"#, &[]);
    assert_eq!(sev(&r, "META_REFRESH"), Some(Severity::Critical));
    let r = scan_epub(r#"<meta http-equiv="refresh" content="0;url=search-ms:query=x">"#, &[]);
    assert_eq!(sev(&r, "META_REFRESH"), Some(Severity::Critical));
    // 일반 원격 이동과 그 밖의 위험 스킴은 High로 남는다.
    let r = scan_epub(r#"<meta http-equiv="refresh" content="0;url=https://evil.example/">"#, &[]);
    assert_eq!(sev(&r, "META_REFRESH"), Some(Severity::High));
    let r = scan_epub(r#"<meta http-equiv="refresh" content="0;url=file:///etc/passwd">"#, &[]);
    assert_eq!(sev(&r, "META_REFRESH"), Some(Severity::High));
}

// ---- 번들 실행 파일의 SHA-256 ----

#[test]
fn bundled_executable_findings_carry_the_sha256_of_the_member() {
    // sha256("hello")
    let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    let r = scan_epub("<p>t</p>", &[("OEBPS/Misc/setup.exe", b"hello")]);
    let f = r.findings.iter().find(|f| f.code == "EMBEDDED_EXECUTABLE").unwrap();
    assert!(f.detail.contains(&format!("sha256={hello}")), "{}", f.detail);
    let mut pe = b"MZ\x90\x00".to_vec();
    pe.extend(b"hello");
    let r = scan_epub("<p>t</p>", &[("OEBPS/Misc/a.exe", &pe)]);
    let f = r.findings.iter().find(|f| f.code == "PE_EXECUTABLE_CONTENT").unwrap();
    assert!(f.detail.contains("sha256=") && f.detail.len() > 64, "{}", f.detail);
    // 실행 파일이 아닌 멤버에는 붙지 않는다.
    let r = scan_epub("<p>t</p>", &[("OEBPS/a.png", b"\x89PNG\r\n\x1a\nrest")]);
    assert!(r.findings.iter().all(|f| !f.detail.contains("sha256=")));
}
