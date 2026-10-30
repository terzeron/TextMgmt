//! ZIP 계열 멤버의 엔트로피 검사 테스트.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use bookdoctor::cli::Options;
use bookdoctor::scan::scan_file;
use scan_common::{FileReport, Severity};

fn random_bytes(n: usize, mut seed: u32) -> Vec<u8> {
    (0..n)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        })
        .collect()
}

fn zip_with(name: &str, members: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    for (n, data) in members {
        // 무작위 바이트가 압축되지 않아도 같은 결과가 되도록 저장 방식으로 쓴다.
        z.start_file(*n, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap();
    (d, p)
}

fn scan(name: &str, members: &[(&str, &[u8])]) -> FileReport {
    let (_d, p) = zip_with(name, members);
    scan_file(&p, &Options::default())
}

#[test]
fn random_bytes_named_xml_in_a_zip_are_flagged_medium() {
    let r = scan("a.zip", &[("word/document.xml", &random_bytes(8192, 1))]);
    let f = r.findings.iter().find(|f| f.code == "HIGH_ENTROPY_TEXT_MEMBER").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!(f.severity, Severity::Medium);
    assert!(f.location.contains("document.xml"), "{}", f.location);
    // DOCX와 HWPX도 같다.
    assert!(scan("b.docx", &[("[Content_Types].xml", b"<Types/>"), ("word/styles.xml", &random_bytes(4096, 2))]).has("HIGH_ENTROPY_TEXT_MEMBER"));
    assert!(scan("c.hwpx", &[("Contents/section0.xml", &random_bytes(4096, 3))]).has("HIGH_ENTROPY_TEXT_MEMBER"));
}

#[test]
fn random_bytes_named_like_compressed_data_are_not_flagged() {
    let r = scan("a.zip", &[("a.jpg", &random_bytes(8192, 4)), ("b.png", &random_bytes(8192, 5)), ("c.zip", &random_bytes(8192, 6)), ("BinData/BIN0001.bin", &random_bytes(8192, 7))]);
    assert!(!r.has("HIGH_ENTROPY_TEXT_MEMBER"), "{:?}", r.findings);
}

#[test]
fn ordinary_text_members_are_not_flagged() {
    let xml = format!("<?xml version=\"1.0\"?><root>{}</root>", "<item name=\"value\">한글 텍스트 and english text</item>".repeat(200));
    let js = "function f(a, b) { return a + b; }\n".repeat(300);
    let r = scan("a.zip", &[("a.xml", xml.as_bytes()), ("b.js", js.as_bytes()), ("c.txt", "가나다라마바사 ".repeat(500).as_bytes())]);
    assert!(!r.has("HIGH_ENTROPY_TEXT_MEMBER"), "{:?}", r.findings);
}

#[test]
fn tiny_members_are_not_judged() {
    // 표본이 작으면 엔트로피가 불안정하다.
    let r = scan("a.zip", &[("a.xml", &random_bytes(100, 8))]);
    assert!(!r.has("HIGH_ENTROPY_TEXT_MEMBER"), "{:?}", r.findings);
}

#[test]
fn only_the_probe_prefix_is_sampled() {
    // 앞 4KiB가 평범하면 뒤가 무작위여도 이 검사는 보지 않는다(추가로 읽지 않는 대신 한계를 둔다).
    let mut data = b"<a>text</a>".repeat(500);
    data.truncate(4096);
    data.extend(random_bytes(8192, 9));
    let r = scan("a.zip", &[("a.xml", &data)]);
    assert!(!r.has("HIGH_ENTROPY_TEXT_MEMBER"), "{:?}", r.findings);
}
