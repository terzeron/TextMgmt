//! EPUB 구조 검사 테스트 (detect_mojibake의 epub-structure 테스트를 이식).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use epubscan::structure::*;

fn quiet(_: &str) {}

fn write(p: &Path, text: &str) {
    fs::write(p, text).unwrap();
}

fn issues(path: &Path) -> Vec<String> {
    inspect_epub_structure(path.to_str().unwrap()).issues
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
    let r = inspect_epub_structure(p.to_str().unwrap());
    assert_eq!(r.issues, ["prefixed_root"]);
    assert_eq!(r.examples["prefixed_root"], "OEBPS/ch 1.xhtml");
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
    let r = inspect_epub_structure(tmp.to_str().unwrap());
    assert_eq!(r.issues, ["prefixed_root"]);
    assert_eq!(r.examples["prefixed_root"], "OEBPS/ch 1.xhtml");
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
        assert_eq!(issues(&p), ["prefixed_root"], "{media}");
    }
}

#[test]
fn epub_structure_falls_back_to_extension_when_media_type_is_missing() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("e.epub");
    make_zip_with_media(&p, "", "c.xhtml", prefixed_chapter().as_bytes());
    assert_eq!(issues(&p), ["prefixed_root"]);
}

#[test]
fn epub_structure_flags_non_zip_truncated_and_missing() {
    let d = tempfile::tempdir().unwrap();
    let bad = d.path().join("bad.epub");
    fs::write(&bad, b"not a zip").unwrap();
    assert_eq!(issues(&bad), ["not_zip"]);

    let ok = d.path().join("ok.epub");
    make_epub(&ok);
    let data = fs::read(&ok).unwrap();
    let cut = d.path().join("cut.epub");
    fs::write(&cut, &data[..data.len() - 100]).unwrap();
    assert_eq!(issues(&cut), ["not_zip"]);

    assert_eq!(issues(&d.path().join("none.epub")), ["not_zip"]);
}

#[test]
fn epub_structure_flags_mimetype_not_first_or_stored() {
    let (_d, p) = epub_with("m.epub", |s| s.mimetype_first = false);
    assert_eq!(issues(&p), ["bad_mimetype"]);
}

#[test]
fn epub_structure_flags_empty_zip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("empty.epub");
    zip::ZipWriter::new(fs::File::create(&p).unwrap()).finish().unwrap();
    assert_eq!(issues(&p), ["bad_mimetype", "no_container"]);
}

#[test]
fn epub_structure_flags_unusable_container() {
    for container in [None, Some("<container"), Some("<container><rootfiles/></container>")] {
        let (_d, p) = epub_with("c.epub", |s| s.container = container);
        assert_eq!(issues(&p), ["no_container"], "{container:?}");
    }
}

#[test]
fn epub_structure_flags_unreadable_members_not_missing() {
    for member in ["OEBPS/content.opf", "META-INF/container.xml", "OEBPS/ch 1.xhtml"] {
        let (_d, p) = epub_with("u.epub", |_| {});
        corrupt_local_header(&p, member);
        let r = inspect_epub_structure(p.to_str().unwrap());
        assert_eq!(r.issues, ["member_unreadable"], "{member}");
        assert_eq!(r.examples["member_unreadable"], member);
    }
}

#[test]
fn epub_structure_records_unexpected_errors_instead_of_aborting() {
    let (_d, p) = epub_with("e.epub", |_| {});
    let boom = |_: &[u8]| -> Option<String> { panic!("예상 못 한 오류") };
    let r = inspect_epub_structure_with(p.to_str().unwrap(), &boom);
    assert_eq!(r.issues, ["scan_error"]);
    assert!(r.examples["scan_error"].contains("예상 못 한 오류"));
}

#[test]
fn scan_epub_structure_survives_a_file_that_breaks_the_inspector() {
    let d = tempfile::tempdir().unwrap();
    make_epub(&d.path().join("a_ok.epub"));
    let bad = d.path().join("b_bad.epub");
    make_epub(&bad);
    corrupt_local_header(&bad, "OEBPS/ch 1.xhtml");
    make_epub_with(&d.path().join("c_ok.epub"), EpubSpec { chapter: prefixed_chapter(), ..EpubSpec::default() });
    let report = scan_epub_structure(d.path(), 1, 0, &quiet);
    assert_eq!(report.total, 3);
    assert_eq!(report.counts["member_unreadable"], 1);
    assert_eq!(report.counts["prefixed_root"], 1);
}

#[test]
fn epub_structure_flags_missing_opf_unparsable_opf_and_missing_spine_member() {
    let (_d, p) = epub_with("o.epub", |s| s.opf = None);
    let r = inspect_epub_structure(p.to_str().unwrap());
    assert_eq!(r.issues, ["opf_missing"]);
    assert_eq!(r.examples["opf_missing"], "OEBPS/content.opf");

    let (_d, p) = epub_with("o.epub", |s| s.opf = Some("<package".into()));
    assert_eq!(issues(&p), ["opf_unparsable"]);

    let (_d, p) = epub_with("s.epub", |s| s.chapter_name = None);
    let r = inspect_epub_structure(p.to_str().unwrap());
    assert_eq!(r.issues, ["spine_member_missing"]);
    assert_eq!(r.examples["spine_member_missing"], "OEBPS/ch 1.xhtml");
}

#[test]
fn epub_structure_ignores_spine_items_without_manifest_entry() {
    let (_d, p) = epub_with("g.epub", |s| s.opf = Some(OPF_XML.replace(r#"<itemref idref="c1"/>"#, r#"<itemref idref="ghost"/>"#)));
    assert!(issues(&p).is_empty());
}

#[test]
fn epub_structure_flags_oversized_xml_member_as_unreadable() {
    let (_d, p) = epub_with("b.epub", |s| s.opf = Some(format!("{OPF_XML}{}", " ".repeat(5 * 1024 * 1024))));
    assert_eq!(issues(&p), ["member_unreadable"]);
}

#[test]
fn epub_structure_chapter_without_root_tag_is_not_flagged() {
    let (_d, p) = epub_with("r.epub", |s| s.chapter = "plain text".into());
    assert!(issues(&p).is_empty());
}

#[test]
fn scan_epub_structure_counts_only_epubs_and_respects_limit() {
    let d = tempfile::tempdir().unwrap();
    make_epub(&d.path().join("a.epub"));
    make_epub_with(&d.path().join("b.epub"), EpubSpec { chapter: prefixed_chapter(), ..EpubSpec::default() });
    write(&d.path().join("c.txt"), "본문");
    let report = scan_epub_structure(d.path(), 1, 0, &quiet);
    assert_eq!(report.total, 2);
    assert_eq!(report.flagged_count, 1);
    assert_eq!(report.counts["prefixed_root"], 1);
    assert_eq!(Path::new(&report.flagged[0].path).file_name().unwrap(), "b.epub");
    assert_eq!(scan_epub_structure(d.path(), 1, 1, &quiet).total, 1);
}
