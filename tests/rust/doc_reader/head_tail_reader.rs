//! 파일 앞뒤 읽기 테스트 (`head_tail_reader` 모듈). ISBN 추출 테스트에서 읽기만 검사하는 것을 옮겼다.

use std::fs;
use std::io::Write;
use std::path::Path;

use doc_reader::head_tail_reader::*;

#[path = "../shared/zipx.rs"]
mod zipx;

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

    // `>`가 없는 `<`는 한 글자만 넘기고, 스트림 끝의 `\`에서 문자열 읽기를 멈춘다.
    fs::write(&f, b"%PDF-1.4\n1 0 obj\n<< >>\nstream\n<41 (xy\\endstream\n").unwrap();
    assert_eq!(pdf_text_fallback(&f).unwrap(), "xy ");
}

/// 인코딩 손상 판정용 본문 표본(`extract_*_text`).
mod sampling {
    use super::zipx::{corrupt_local_header, make_zip};
    use doc_reader::head_tail_reader::*;
    use std::fs;

    const KOREAN: &str = "그는 아무 말도 하지 않았다 그리고 다시 길을 걸었다 바람이 불었다 ";

    #[test]
    fn extract_txt_text_missing_file() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(extract_txt_text(&d.path().join("없는파일.txt"), 100, 10), "");
    }

    #[test]
    fn extract_txt_text_head_only() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        fs::write(&f, KOREAN).unwrap();
        assert_eq!(extract_txt_text(&f, 1000, 100).trim(), KOREAN.trim());
    }

    #[test]
    fn extract_txt_text_reads_head_and_tail() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("b.txt");
        fs::write(&f, format!("시작{}끝부분", "가".repeat(5000))).unwrap();
        let text = extract_txt_text(&f, 50, 20);
        assert!(text.contains("시작"));
        assert!(text.contains("끝부분"));
    }

    #[test]
    fn extract_txt_text_tail_disabled() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("c.txt");
        fs::write(&f, format!("시작{}끝부분", "가".repeat(5000))).unwrap();
        assert!(!extract_txt_text(&f, 50, 0).contains("끝부분"));
    }

    #[test]
    fn extract_txt_text_unopenable_path_is_empty() {
        let d = tempfile::tempdir().unwrap();
        // 디렉토리는 stat은 되지만 파일로 열어 읽을 수 없다.
        assert_eq!(extract_txt_text(d.path(), 100, 10), "");
    }

    #[test]
    fn extract_epub_text_without_html_documents() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.epub");
        make_zip(&p, &[("mimetype", "application/epub+zip")]);
        assert_eq!(extract_epub_text(&p, 100, 10), "");
    }

    #[test]
    fn extract_epub_text_strips_tags_and_skips_front_matter() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("b.epub");
        let cover = format!("<html><body>{}</body></html>", "표지".repeat(100));
        let body = format!("<html><body>{}</body></html>", KOREAN.repeat(5));
        make_zip(&p, &[("cover.xhtml", &cover), ("body1.xhtml", &body)]);
        let text = extract_epub_text(&p, 2000, 0);
        assert!(!text.contains("표지"));
        assert!(text.contains("바람이"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn extract_epub_text_rotates_when_many_documents() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.epub");
        let docs: Vec<(String, String)> = (0..6).map(|i| (format!("ch{i}.xhtml"), format!("<p>장면{i} {}</p>", KOREAN.repeat(3)))).collect();
        let refs: Vec<(&str, &str)> = docs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        make_zip(&p, &refs);
        let text = extract_epub_text(&p, 400, 100);
        assert!(text.contains("장면2"));
        let head: String = text.chars().take(400).collect();
        assert!(!head.contains("장면0"));
    }

    #[test]
    fn extract_epub_text_skips_short_documents() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("d.epub");
        let long = format!("<p>{}</p>", KOREAN.repeat(5));
        make_zip(&p, &[("a.xhtml", "<p>짧음</p>"), ("b.xhtml", &long)]);
        assert!(!extract_epub_text(&p, 2000, 0).contains("짧음"));
    }

    #[test]
    fn extract_epub_text_on_broken_zip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("broken.epub");
        fs::write(&p, b"not a zip at all").unwrap();
        assert_eq!(extract_epub_text(&p, 100, 10), "");
    }

    #[test]
    fn extract_epub_text_ignores_unreadable_member() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.epub");
        let body = format!("<p>{}</p>", KOREAN.repeat(5));
        make_zip(&p, &[("a.xhtml", &body)]);
        corrupt_local_header(&p, "a.xhtml");
        assert_eq!(extract_epub_text(&p, 2000, 0), "");
    }

    #[test]
    fn extract_document_text_dispatches_by_extension() {
        let d = tempfile::tempdir().unwrap();
        let txt = d.path().join("a.txt");
        fs::write(&txt, KOREAN).unwrap();
        assert!(extract_document_text(&txt, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS).contains("바람이"));

        let epub = d.path().join("a.epub");
        let body = format!("<p>{}</p>", KOREAN.repeat(5));
        make_zip(&epub, &[("body.xhtml", &body)]);
        assert!(extract_document_text(&epub, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS).contains("바람이"));

        let other = d.path().join("a.pdf");
        fs::write(&other, b"%PDF-1.4").unwrap();
        assert_eq!(extract_document_text(&other, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS), "");
    }

    #[test]
    fn extract_epub_text_limited_stops_at_the_total_budget() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("bomb.epub");
        // 멤버마다 2MB로 풀리는데 거의 0바이트로 압축되는 문서 40개. 예산 없이 읽으면 전부 푼다.
        let zeros = "\0".repeat(2 * 1024 * 1024);
        let docs: Vec<(String, &str)> = (0..40).map(|i| (format!("ch{i}.xhtml"), zeros.as_str())).collect();
        let refs: Vec<(&str, &str)> = docs.iter().map(|(a, b)| (a.as_str(), *b)).collect();
        make_zip(&p, &refs);
        let started = std::time::Instant::now();
        let (_, cut) = extract_epub_text_limited(&p, 20_000, 10_000, 1024 * 1024, usize::MAX);
        assert!(cut, "예산이 바닥나면 잘렸다고 알려야 한다");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "예산이 읽기량을 묶어야 한다: {:?}", started.elapsed());
    }

    #[test]
    fn extract_epub_text_limited_stops_at_the_member_count() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("tags.epub");
        // 태그뿐인 문서는 50자 미만이라 건너뛰는데, 건너뛴 것도 읽은 것으로 센다.
        let docs: Vec<(String, &str)> = (0..100).map(|i| (format!("ch{i}.xhtml"), "<p></p>")).collect();
        let refs: Vec<(&str, &str)> = docs.iter().map(|(a, b)| (a.as_str(), *b)).collect();
        make_zip(&p, &refs);
        let (text, cut) = extract_epub_text_limited(&p, 20_000, 10_000, u64::MAX, 10);
        assert!(cut);
        assert_eq!(text, "");
    }

    #[test]
    fn extract_epub_text_limited_is_not_cut_when_the_sample_fits() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ok.epub");
        let body = format!("<p>{}</p>", KOREAN.repeat(5));
        make_zip(&p, &[("a.xhtml", &body)]);
        let (text, cut) = extract_epub_text_limited(&p, 2000, 0, 1024 * 1024, 100);
        assert!(!cut);
        assert!(text.contains("바람이"));
        assert_eq!(text, extract_epub_text(&p, 2000, 0));
    }

    #[test]
    fn extract_epub_text_limited_charges_members_that_inflate_and_then_fail() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("badcrc.epub");
        let zeros = "\0".repeat(2 * 1024 * 1024);
        let docs: Vec<(String, &str)> = (0..40).map(|i| (format!("ch{i}.xhtml"), zeros.as_str())).collect();
        let refs: Vec<(&str, &str)> = docs.iter().map(|(a, b)| (a.as_str(), *b)).collect();
        make_zip(&p, &refs);
        // 로컬 헤더(CRC는 14행 오프셋)와 중앙 디렉터리(16)의 CRC를 깨서, 끝까지 푼 뒤 읽기가 실패하게 한다.
        let mut bytes = fs::read(&p).unwrap();
        for (sig, off) in [(&b"PK\x03\x04"[..], 14usize), (&b"PK\x01\x02"[..], 16usize)] {
            let mut i = 0;
            while i + 4 <= bytes.len() {
                if &bytes[i..i + 4] == sig {
                    bytes[i + off] ^= 0xff;
                    i += 4;
                } else {
                    i += 1;
                }
            }
        }
        fs::write(&p, &bytes).unwrap();
        let started = std::time::Instant::now();
        // 예산 3MB. 멤버 하나가 2MB라 예산이 없으면 40개(80MB)를 전부 푼다.
        let (_, cut) = extract_epub_text_limited(&p, 20_000, 10_000, 3 * 1024 * 1024, usize::MAX);
        assert!(cut);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    }
}
