//! 파일 앞뒤 읽기 테스트 (`head_tail_reader` 모듈). ISBN 추출 테스트에서 읽기만 검사하는 것을 옮겼다.

use std::fs;
use std::io::Write;
use std::path::Path;

use doc_reader::head_tail_reader::*;

fn write(p: &Path, text: &str) {
    fs::write(p, text).unwrap();
}

/// 텍스트 한 줄을 그리는 1쪽짜리 PDF.
fn text_pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
    out
}

#[test]
fn head_and_tail_of_a_file_and_of_a_string() {
    let d = tempfile::tempdir().unwrap();
    let small = d.path().join("small.txt");
    write(&small, "hello world");
    assert_eq!(read_head_tail_from_file(&small, 8 * 1024).unwrap(), "hello world");

    let large = d.path().join("large.txt");
    write(&large, &"A".repeat(20_000));
    let r = read_head_tail_from_file(&large, 100).unwrap();
    assert_eq!(r.len(), 200);
    assert!(r.chars().all(|c| c == 'A'));
    assert!(read_head_tail_from_file(&d.path().join("none.txt"), 100).is_err());

    assert_eq!(read_head_tail_from_content("hello", 8 * 1024), "hello");
    assert_eq!(read_head_tail_from_content(&"A".repeat(20_000), 100).len(), 200);
    // 문자 수 기준으로 자른다.
    assert_eq!(read_head_tail_from_content(&"가".repeat(500), 100).chars().count(), 200);
}

#[test]
fn head_and_tail_page_windows() {
    assert_eq!(head_tail_page_indices(3, 5), [0, 1, 2]);
    assert_eq!(head_tail_page_indices(6, 5), [0, 1, 2, 3, 4, 5]);
    assert_eq!(head_tail_page_indices(20, 5), [0, 1, 2, 3, 4, 15, 16, 17, 18, 19]);
    assert!(head_tail_page_indices(0, 5).is_empty());
}

#[test]
fn the_fallback_scanner_inflates_streams_and_reads_text_arrays_and_hex_strings() {
    use flate2::{Compression, write::ZlibEncoder};
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(b"BT [(ISBN ) -20 (978) 5 (8994492032)] TJ <49534244> Tj (esc \\(x\\) \\101 \\n) Tj ET").unwrap();
    let body = enc.finish().unwrap();
    let mut pdf = b"%PDF-1.4\n1 0 obj\n<< /Filter /FlateDecode >>\nstream\n".to_vec();
    pdf.extend(&body);
    pdf.extend(b"\nendstream\nendobj\n2 0 obj\n<< >>\nstream\nBT (plain) Tj ET\nendstream\nendobj\n");
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("c.pdf");
    fs::write(&f, pdf).unwrap();
    let text = pdf_text_fallback(&f).unwrap();
    assert!(text.contains("ISBN 9788994492032"), "{text:?}");
    assert!(text.contains("ISBD"), "{text:?}");
    assert!(text.contains("esc (x) A \n"), "{text:?}");
    assert!(text.contains("plain"));
    assert!(pdf_text_fallback(&d.path().join("none.pdf")).is_err());
}

#[test]
fn the_poppler_backend_reads_text_and_reports_page_count() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("book.pdf");
    fs::write(&f, text_pdf("ISBN 9788994492032")).unwrap();
    let (text, pages) = pdf_text_poppler(&f).unwrap();
    assert_eq!(pages, 1);
    assert!(text.contains("9788994492032"), "{text:?}");
    assert!(pdf_text_auto(&f).unwrap().0.contains("9788994492032"));
    assert!(pdf_text_poppler(&d.path().join("none.pdf")).is_err());
    let junk = d.path().join("junk.pdf");
    write(&junk, "not a pdf");
    assert!(pdf_text_poppler(&junk).is_err());
}

#[test]
fn the_pure_rust_pdf_extractor_remains_available_as_fallback() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("book.pdf");
    fs::write(&f, text_pdf("ISBN 9788994492032")).unwrap();
    let (text, pages) = pdf_text_primary(&f).unwrap();
    assert_eq!(pages, 1);
    assert!(text.contains("9788994492032"), "{text:?}");
    assert!(pdf_text_primary(&d.path().join("none.pdf")).is_err());
}

#[test]
fn the_fallback_scanner_handles_odd_stream_and_string_syntax() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("odd.pdf");
    let mut pdf = b"%PDF-1.4\n1 0 obj\n<< >>\nstream\r\n".to_vec();
    // 중첩 괄호, \r \t 이스케이프, 8진수 한 자리, 딕셔너리 `<<`, 홀수 길이 16진수, 끝에서 끊긴 이스케이프.
    pdf.extend(br"BT (a(b)c\r\t\1x\) ) Tj << /K 1 >> [(p) (q)] TJ <41 42 4> Tj (tail \");
    pdf.extend(b"\nendstream\n2 0 obj\n<< >>\nstream\n(unterminated");
    fs::write(&f, pdf).unwrap();
    let text = pdf_text_fallback(&f).unwrap();
    assert!(text.contains("a(b)c\r\t\u{1}x) "), "{text:?}");
    assert!(text.contains("pq"), "{text:?}");
    assert!(text.contains("AB"), "{text:?}");
}
