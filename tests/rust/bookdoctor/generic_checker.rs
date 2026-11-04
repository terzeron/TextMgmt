mod support;

use bookdoctor::cli_parser::Options;
use bookdoctor::generic_checker::sniff_head;
use bookdoctor::format_dispatcher::scan_file;
use bookdoctor::magic_sniffer::Sniffed;

fn count(r: &bookdoctor::FileReport, code: &str) -> usize {
    r.findings.iter().filter(|f| f.code == code).count()
}

#[test]
fn epub_embedding_a_pdf_early_is_still_a_zip() {
    let mut bytes = b"PK\x03\x04".to_vec();
    bytes.extend_from_slice(b"....%PDF-1.4....");
    assert_eq!(sniff_head(&bytes), Sniffed::Zip);
    let f = support::file_with("a.epub", &bytes);
    assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "FORMAT_MISMATCH"), 0);
}
