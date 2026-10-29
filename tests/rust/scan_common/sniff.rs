//! scan_common::sniff, scan_common::names 테스트: 매직바이트 판별과 Windows 예약 이름.

use scan_common::names::windows_reserved;
use scan_common::sniff::{Sniffed, sniff};

#[test]
fn sniffs_pdf_even_at_offset_300() {
    assert_eq!(sniff(b"%PDF-1.7\n"), Sniffed::Pdf);
    let mut b = vec![b' '; 300];
    b.extend_from_slice(b"%PDF-1.4");
    assert_eq!(sniff(&b), Sniffed::Pdf);
}

#[test]
fn sniffs_zip() {
    assert_eq!(sniff(b"PK\x03\x04rest"), Sniffed::Zip);
    assert_eq!(sniff(b"PK\x05\x06"), Sniffed::Zip);
}

#[test]
fn sniffs_elf() {
    assert_eq!(sniff(b"\x7fELF\x02"), Sniffed::Elf);
}

#[test]
fn sniffs_macho() {
    for m in [[0xFE, 0xED, 0xFA, 0xCE], [0xFE, 0xED, 0xFA, 0xCF], [0xCE, 0xFA, 0xED, 0xFE], [0xCF, 0xFA, 0xED, 0xFE], [0xCA, 0xFE, 0xBA, 0xBE]] {
        assert_eq!(sniff(&m), Sniffed::MachO);
    }
}

#[test]
fn sniffs_pe() {
    assert_eq!(sniff(b"MZ\x90\x00"), Sniffed::Pe);
}

#[test]
fn sniffs_html_after_bom_and_whitespace() {
    assert_eq!(sniff(b"\xEF\xBB\xBF \n\t<HTML><body>"), Sniffed::Html);
    assert_eq!(sniff(b"<!DOCTYPE html>"), Sniffed::Html);
}

#[test]
fn sniffs_plain_text_as_unknown() {
    assert_eq!(sniff("평문 text".as_bytes()), Sniffed::Unknown);
    assert_eq!(sniff(b""), Sniffed::Unknown);
}

#[test]
fn windows_reserved_matches_device_names() {
    assert!(windows_reserved("CON"));
    assert!(windows_reserved("com1.txt"));
    assert!(windows_reserved("aux"));
    assert!(windows_reserved("nul. "));
    assert!(!windows_reserved("console"));
}
