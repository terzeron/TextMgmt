//! EPUB 구조 검사 테스트 (옛 인코딩 검사 도구의 epub-structure 테스트를 이식).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use bookdoctor::cli::Options;
use bookdoctor::epub::scan_epub_file;
use bookdoctor::epub::structure::{ISSUE_TO_CODE, inspect, inspect_with};
use common::*;
use scan_common::{Category, FileReport, Format, Severity};

fn write(p: &Path, text: &str) {
    fs::write(p, text).unwrap();
}

fn inspect_report(path: &Path) -> FileReport {
    let mut r = FileReport::new(path, Some(Format::Epub));
    inspect(path, &mut r);
    r
}

fn issues(path: &Path) -> Vec<String> {
    inspect_report(path).findings.iter().map(|f| f.code.clone()).collect()
}

/// 코드의 첫 지표 위치(옛 `examples[code]`).
fn location(r: &FileReport, code: &str) -> String {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음")).location.clone()
}

fn epub_with(name: &str, f: impl FnOnce(&mut EpubSpec)) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    let mut spec = EpubSpec::default();
    f(&mut spec);
    make_epub_with(&p, spec);
    (d, p)
}

#[test]
fn epub_structure_accepts_a_standard_epub() {
    let (_d, p) = epub_with("ok.epub", |_| {});
    assert!(issues(&p).is_empty());
}

#[test]
fn epub_structure_flags_prefixed_root() {
    let (_d, p) = epub_with("p.epub", |s| s.chapter = prefixed_chapter());
    let r = inspect_report(&p);
    assert_eq!(codes_of(&r), ["PREFIXED_ROOT"]);
    assert_eq!(location(&r, "PREFIXED_ROOT"), "OEBPS/ch 1.xhtml");
}

#[test]
fn epub_structure_reports_each_issue_once_with_first_example() {
    let opf = OPF_XML
        .replace(r#"<itemref idref="c1"/>"#, r#"<itemref idref="c1"/><itemref idref="c2"/>"#)
        .replace("</manifest>", r#"<item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/></manifest>"#);
    let (_d, p) = epub_with("t.epub", |s| {
        s.chapter = prefixed_chapter();
        s.opf = Some(opf);
    });
    // 두 번째 챕터는 원본 테스트처럼 ZIP 끝에 덧붙인다.
    let tmp = p.with_file_name("t2.epub");
    {
        use std::io::Write;
        let mut zin = zip::ZipArchive::new(fs::File::open(&p).unwrap()).unwrap();
        let mut zout = zip::ZipWriter::new(fs::File::create(&tmp).unwrap());
        for i in 0..zin.len() {
            let f = zin.by_index_raw(i).unwrap();
            zout.raw_copy_file(f).unwrap();
        }
        zout.start_file("OEBPS/ch2.xhtml", zip::write::SimpleFileOptions::default()).unwrap();
        zout.write_all(prefixed_chapter().as_bytes()).unwrap();
        zout.finish().unwrap();
    }
    let r = inspect_report(&tmp);
    assert_eq!(codes_of(&r), ["PREFIXED_ROOT"]);
    assert_eq!(location(&r, "PREFIXED_ROOT"), "OEBPS/ch 1.xhtml");
}

#[test]
fn epub_structure_does_not_misread_binary_spine_item_as_prefixed_root() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("audio.epub");
    make_zip_with_media(&p, "audio/mpeg", "Audio/001.mp3", b"\xff\xfb\x90\x00 <ab:cd garbage");
    assert!(issues(&p).is_empty());
}

#[test]
fn epub_structure_still_checks_markup_spine_items() {
    for (media, name) in [("application/xhtml+xml", "c.xhtml"), ("text/html", "c.html"), ("image/svg+xml", "c.svg"), ("application/xml", "c.xml")] {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("m.epub");
        make_zip_with_media(&p, media, name, prefixed_chapter().as_bytes());
        assert_eq!(issues(&p), ["PREFIXED_ROOT"], "{media}");
    }
}

#[test]
fn epub_structure_falls_back_to_extension_when_media_type_is_missing() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("e.epub");
    make_zip_with_media(&p, "", "c.xhtml", prefixed_chapter().as_bytes());
    assert_eq!(issues(&p), ["PREFIXED_ROOT"]);
}

#[test]
fn epub_structure_flags_non_zip_truncated_and_missing() {
    let d = tempfile::tempdir().unwrap();
    let bad = d.path().join("bad.epub");
    fs::write(&bad, b"not a zip").unwrap();
    assert_eq!(issues(&bad), ["NOT_ZIP"]);

    let ok = d.path().join("ok.epub");
    make_epub(&ok);
    let data = fs::read(&ok).unwrap();
    let cut = d.path().join("cut.epub");
    fs::write(&cut, &data[..data.len() - 100]).unwrap();
    assert_eq!(issues(&cut), ["NOT_ZIP"]);

    assert_eq!(issues(&d.path().join("none.epub")), ["NOT_ZIP"]);
}

#[test]
fn epub_structure_flags_mimetype_not_first_or_stored() {
    let (_d, p) = epub_with("m.epub", |s| s.mimetype_first = false);
    assert_eq!(issues(&p), ["BAD_MIMETYPE"]);
}

#[test]
fn epub_structure_flags_empty_zip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("empty.epub");
    zip::ZipWriter::new(fs::File::create(&p).unwrap()).finish().unwrap();
    assert_eq!(issues(&p), ["BAD_MIMETYPE", "NO_CONTAINER"]);
}

#[test]
fn epub_structure_flags_unusable_container() {
    for container in [None, Some("<container"), Some("<container><rootfiles/></container>")] {
        let (_d, p) = epub_with("c.epub", |s| s.container = container);
        assert_eq!(issues(&p), ["NO_CONTAINER"], "{container:?}");
    }
}

#[test]
fn epub_structure_flags_unreadable_members_not_missing() {
    for member in ["OEBPS/content.opf", "META-INF/container.xml", "OEBPS/ch 1.xhtml"] {
        let (_d, p) = epub_with("u.epub", |_| {});
        corrupt_local_header(&p, member);
        let r = inspect_report(&p);
        assert_eq!(codes_of(&r), ["MEMBER_UNREADABLE"], "{member}");
        assert_eq!(location(&r, "MEMBER_UNREADABLE"), member);
    }
}

#[test]
fn epub_structure_records_unexpected_errors_instead_of_aborting() {
    let (_d, p) = epub_with("e.epub", |_| {});
    let boom = |_: &[u8]| -> Option<String> { panic!("예상 못 한 오류") };
    let mut r = FileReport::new(&p, Some(Format::Epub));
    inspect_with(&p, &mut r, &boom);
    // scan_error는 지표가 아니라 불완전로 보고한다.
    assert!(r.findings.is_empty());
    assert!(r.reasons.iter().any(|m| m.starts_with("panic: ") && m.contains("예상 못 한 오류")));
}

#[test]
fn parser_panic_becomes_incomplete_not_abort() {
    let (_d, p) = epub_with("e.epub", |s| s.mimetype_first = false);
    let boom = |_: &[u8]| -> Option<String> { panic!("파서 폭발") };
    let mut r = FileReport::new(&p, Some(Format::Epub));
    inspect_with(&p, &mut r, &boom);
    assert!(r.incomplete && !r.reasons.is_empty());
    // panic 전에 찾은 지표는 남는다.
    assert!(r.has("BAD_MIMETYPE"));
}

#[test]
fn scan_epub_structure_survives_a_file_that_breaks_the_inspector() {
    let d = tempfile::tempdir().unwrap();
    make_epub(&d.path().join("a_ok.epub"));
    let bad = d.path().join("b_bad.epub");
    make_epub(&bad);
    corrupt_local_header(&bad, "OEBPS/ch 1.xhtml");
    make_epub_with(&d.path().join("c_ok.epub"), EpubSpec { chapter: prefixed_chapter(), ..EpubSpec::default() });
    let reports: Vec<FileReport> = ["a_ok.epub", "b_bad.epub", "c_ok.epub"].iter().map(|n| inspect_report(&d.path().join(n))).collect();
    assert_eq!(reports.len(), 3);
    assert_eq!(reports.iter().filter(|r| r.has("MEMBER_UNREADABLE")).count(), 1);
    assert_eq!(reports.iter().filter(|r| r.has("PREFIXED_ROOT")).count(), 1);
}

#[test]
fn epub_structure_flags_missing_opf_unparsable_opf_and_missing_spine_member() {
    let (_d, p) = epub_with("o.epub", |s| s.opf = None);
    let r = inspect_report(&p);
    assert_eq!(codes_of(&r), ["OPF_MISSING"]);
    assert_eq!(location(&r, "OPF_MISSING"), "OEBPS/content.opf");

    let (_d, p) = epub_with("o.epub", |s| s.opf = Some("<package".into()));
    assert_eq!(issues(&p), ["OPF_UNPARSABLE"]);

    let (_d, p) = epub_with("s.epub", |s| s.chapter_name = None);
    let r = inspect_report(&p);
    assert_eq!(codes_of(&r), ["SPINE_MEMBER_MISSING"]);
    assert_eq!(location(&r, "SPINE_MEMBER_MISSING"), "OEBPS/ch 1.xhtml");
}

#[test]
fn epub_structure_ignores_spine_items_without_manifest_entry() {
    let (_d, p) = epub_with("g.epub", |s| s.opf = Some(OPF_XML.replace(r#"<itemref idref="c1"/>"#, r#"<itemref idref="ghost"/>"#)));
    assert!(issues(&p).is_empty());
}

#[test]
fn epub_structure_flags_oversized_xml_member_as_unreadable() {
    let (_d, p) = epub_with("b.epub", |s| s.opf = Some(format!("{OPF_XML}{}", " ".repeat(5 * 1024 * 1024))));
    assert_eq!(issues(&p), ["MEMBER_UNREADABLE"]);
}

#[test]
fn epub_structure_chapter_without_root_tag_is_not_flagged() {
    let (_d, p) = epub_with("r.epub", |s| s.chapter = "plain text".into());
    assert!(issues(&p).is_empty());
}

#[test]
fn scan_epub_structure_counts_only_epubs() {
    // 디렉토리 순회·limit은 bookdoctor 공용 경로(scan.rs)가 맡는다. 여기서는 EPUB만 검사해 센다.
    let d = tempfile::tempdir().unwrap();
    make_epub(&d.path().join("a.epub"));
    make_epub_with(&d.path().join("b.epub"), EpubSpec { chapter: prefixed_chapter(), ..EpubSpec::default() });
    write(&d.path().join("c.txt"), "본문");
    let mut epubs: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "epub")).collect();
    epubs.sort();
    let reports: Vec<FileReport> = epubs.iter().map(|p| inspect_report(p)).collect();
    assert_eq!(reports.len(), 2);
    let flagged: Vec<&FileReport> = reports.iter().filter(|r| !r.findings.is_empty()).collect();
    assert_eq!(flagged.len(), 1);
    assert!(flagged[0].has("PREFIXED_ROOT"));
    assert_eq!(Path::new(&flagged[0].path).file_name().unwrap(), "b.epub");
}

fn codes_of(r: &FileReport) -> Vec<String> {
    r.findings.iter().map(|f| f.code.clone()).collect()
}

fn row(old: &str) -> (&'static str, &'static str, Category, Severity) {
    *ISSUE_TO_CODE.iter().find(|r| r.0 == old).unwrap_or_else(|| panic!("{old} 없음"))
}

fn assert_row(r: &FileReport, old: &str, code: &str, category: Category, severity: Severity) {
    assert_eq!(row(old), (row(old).0, code, category, severity), "{old} 표");
    let f = r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", codes_of(r)));
    assert_eq!((f.category, f.severity), (category, severity), "{code}");
}

#[test]
fn issue_table_has_nine_rows_and_unique_codes() {
    assert_eq!(ISSUE_TO_CODE.len(), 9);
    let mut codes: Vec<&str> = ISSUE_TO_CODE.iter().map(|r| r.1).filter(|c| !c.is_empty()).collect();
    let mut olds: Vec<&str> = ISSUE_TO_CODE.iter().map(|r| r.0).collect();
    codes.sort();
    olds.sort();
    codes.dedup();
    olds.dedup();
    assert_eq!(codes.len(), 8);
    assert_eq!(olds.len(), 9);
    // scan_error만 지표 코드가 없다(불완전로 보고).
    assert_eq!(row("scan_error").1, "");
}

#[test]
fn row_not_zip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("bad.epub");
    fs::write(&p, b"not a zip").unwrap();
    assert_row(&inspect_report(&p), "not_zip", "NOT_ZIP", Category::Defect, Severity::High);
}

#[test]
fn row_bad_mimetype() {
    let (_d, p) = epub_with("m.epub", |s| s.mimetype_first = false);
    assert_row(&inspect_report(&p), "bad_mimetype", "BAD_MIMETYPE", Category::Render, Severity::Low);
}

#[test]
fn row_no_container() {
    let (_d, p) = epub_with("c.epub", |s| s.container = None);
    assert_row(&inspect_report(&p), "no_container", "NO_CONTAINER", Category::Render, Severity::High);
}

#[test]
fn row_opf_missing() {
    let (_d, p) = epub_with("o.epub", |s| s.opf = None);
    assert_row(&inspect_report(&p), "opf_missing", "OPF_MISSING", Category::Render, Severity::High);
}

#[test]
fn row_member_unreadable() {
    let (_d, p) = epub_with("u.epub", |_| {});
    corrupt_local_header(&p, "OEBPS/content.opf");
    assert_row(&inspect_report(&p), "member_unreadable", "MEMBER_UNREADABLE", Category::Defect, Severity::High);
}

#[test]
fn row_scan_error_is_incomplete_not_a_finding() {
    let (_d, p) = epub_with("e.epub", |_| {});
    let boom = |_: &[u8]| -> Option<String> { panic!("x") };
    let mut r = FileReport::new(&p, Some(Format::Epub));
    inspect_with(&p, &mut r, &boom);
    assert_eq!(row("scan_error").1, "");
    assert!(r.incomplete && r.findings.is_empty());
}

#[test]
fn row_opf_unparsable() {
    let (_d, p) = epub_with("o.epub", |s| s.opf = Some("<package".into()));
    assert_row(&inspect_report(&p), "opf_unparsable", "OPF_UNPARSABLE", Category::Render, Severity::High);
}

#[test]
fn row_spine_member_missing() {
    let (_d, p) = epub_with("s.epub", |s| s.chapter_name = None);
    assert_row(&inspect_report(&p), "spine_member_missing", "SPINE_MEMBER_MISSING", Category::Render, Severity::High);
}

#[test]
fn row_prefixed_root() {
    let (_d, p) = epub_with("p.epub", |s| s.chapter = prefixed_chapter());
    assert_row(&inspect_report(&p), "prefixed_root", "PREFIXED_ROOT", Category::Render, Severity::Low);
}

#[test]
fn same_code_from_security_and_structure_is_reported_once() {
    let (_d, p) = epub_with("m.epub", |s| s.mimetype_first = false);
    let mut r = FileReport::new(&p, Some(Format::Epub));
    scan_epub_file(&p, &Options::default(), &mut r);
    let hits: Vec<_> = r.findings.iter().filter(|f| f.code == "BAD_MIMETYPE").collect();
    assert_eq!(hits.len(), 1, "{:?}", codes_of(&r));
    // 구조 검사 쪽(Render, ISSUE_TO_CODE의 심각도)이 남는다.
    assert_eq!((hits[0].category, hits[0].severity), (Category::Render, Severity::Low));
}

#[test]
fn different_codes_from_security_and_structure_both_stay() {
    let (_d, p) = epub_with("c.epub", |s| s.container = None);
    let mut r = FileReport::new(&p, Some(Format::Epub));
    scan_epub_file(&p, &Options::default(), &mut r);
    assert!(r.has("NO_CONTAINER") && r.has("MISSING_CONTAINER_XML"), "{:?}", codes_of(&r));
}

#[test]
fn oversized_zip_directory_skips_every_stage_but_security() {
    // 끝에 zip64 EOCD만 있고 항목 수가 한도를 넘는 파일. 구조·렌더링·손상 검사가 열면 안 된다(최종 검토 지적).
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("huge.epub");
    let mut rec = vec![0u8; 56];
    rec[..4].copy_from_slice(b"PK\x06\x06");
    rec[32..40].copy_from_slice(&5_000_000u64.to_le_bytes());
    let mut loc = vec![0u8; 20];
    loc[..4].copy_from_slice(b"PK\x06\x07");
    let mut eocd = vec![0u8; 22];
    eocd[..4].copy_from_slice(b"PK\x05\x06");
    eocd[10..12].copy_from_slice(&0xFFFFu16.to_le_bytes());
    eocd[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    fs::write(&p, [rec, loc, eocd].concat()).unwrap();
    let mut r = FileReport::new(&p, Some(Format::Epub));
    scan_epub_file(&p, &Options::default(), &mut r);
    assert!(r.incomplete);
    assert!(r.reasons.iter().any(|n| n.contains("Too many entries") && n.contains("not opened")), "{:?}", r.reasons);
    assert!(r.findings.is_empty(), "{:?}", codes_of(&r));
    assert!(!r.reasons.iter().any(|n| n.contains("no text extracted")), "{:?}", r.reasons);
}
