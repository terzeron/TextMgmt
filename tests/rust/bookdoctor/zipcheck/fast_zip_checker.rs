#[path = "../support/mod.rs"]
mod support;
use std::fs;
use std::path::Path;
use bookdoctor::cli_parser::{Options, parse};
use bookdoctor::format_dispatcher::scan_file;
use bookdoctor::{Category, FileReport, Format, Severity};
use bookdoctor::zipcheck::fast_zip_checker::resolve_name;
use crate::support::shared_fixtures::{finding, scan_path as scan};

fn deep(path: &Path) -> FileReport {
    scan_file(path, &Options { zip_deep: true, ..Options::default() })
}

fn text(n: usize) -> Vec<u8> {
    "소림사 장로가 강호에서 내공을 운기조식하며 화산파 검법을 수련했다.\n".repeat(n).into_bytes()
}

#[test]
fn valid_zip_has_no_findings() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("ok.zip");
    support::zip_of(&p, &[("a.txt", &text(50)), ("dir/b.txt", &text(10))]);
    let r = scan(&p);
    assert_eq!(r.format, Some(Format::Zip));
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn plain_text_with_zip_extension_is_not_a_zip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("fake.zip");
    fs::write(&p, text(20)).unwrap();
    let f = scan(&p);
    let f = finding(&f, "NOT_ZIP");
    assert_eq!((f.category, f.severity), (Category::Defect, Severity::High));
}

#[test]
fn entry_with_a_corrupt_deflate_stream_is_found_by_the_probe_without_deep() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("bad.zip");
    support::zip_of(&p, &[("good.txt", &text(30)), ("bad.txt", &text(400))]);
    let mut b = fs::read(&p).unwrap();
    let at = support::data_offset(&b, "bad.txt");
    b[at] = 0xFF; // BFINAL=1, BTYPE=11(예약): 풀 수 없는 deflate 블록
    fs::write(&p, b).unwrap();
    let r = scan(&p);
    let f = finding(&r, "ZIP_ENTRY_UNREADABLE");
    assert_eq!((f.category, f.severity, f.location.as_str()), (Category::Defect, Severity::High, "bad.txt"));
    assert!(!r.findings.iter().any(|f| f.location == "good.txt"));
}

#[test]
fn crc_mismatch_beyond_the_probe_is_only_found_with_zip_deep() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("crc.zip");
    let body = vec![b'x'; 8192];
    support::zip_stored(&p, &[("data.bin", &body)]);
    let mut b = fs::read(&p).unwrap();
    let at = support::data_offset(&b, "data.bin");
    b[at + 8191] ^= 0xFF; // 마지막 바이트: 앞 4096바이트 시험 밖
    fs::write(&p, b).unwrap();
    assert!(!scan(&p).has("ZIP_CRC_MISMATCH") && !scan(&p).has("ZIP_ENTRY_UNREADABLE"), "probe must stay shallow");
    let f = deep(&p);
    let f = finding(&f, "ZIP_CRC_MISMATCH");
    assert_eq!((f.category, f.severity, f.location.as_str()), (Category::Defect, Severity::High, "data.bin"));
}

#[test]
fn small_member_crc_mismatch_is_found_even_without_deep() {
    // 4096바이트 이하는 시험이 끝까지 읽으므로 CRC도 확인된다.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("small.zip");
    support::zip_stored(&p, &[("s.bin", &[b'y'; 100])]);
    let mut b = fs::read(&p).unwrap();
    let at = support::data_offset(&b, "s.bin");
    b[at + 50] ^= 0xFF;
    fs::write(&p, b).unwrap();
    assert!(scan(&p).has("ZIP_CRC_MISMATCH"));
}

#[test]
fn zip_slip_is_critical_and_named_after_the_member() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slip.zip");
    support::zip_of(&p, &[("../../evil.sh", b"echo hi\n"), ("ok.txt", b"ok")]);
    let r = scan(&p);
    let f = finding(&r, "ZIP_PATH_TRAVERSAL");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::Critical));
    assert!(f.location.contains("evil.sh"));
}

#[test]
fn highly_compressed_member_is_flagged_without_reading_it_all() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("bomb.zip");
    support::zip_of(&p, &[("zeros.bin", &vec![0u8; 8 * 1024 * 1024])]);
    let r = scan(&p);
    assert_eq!(finding(&r, "HIGH_COMPRESSION_RATIO").severity, Severity::Medium);
}

#[test]
fn forged_small_declared_size_does_not_make_deep_scan_read_everything() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("forged.zip");
    support::zip_of(&p, &[("zeros.bin", &vec![0u8; 32 * 1024 * 1024])]);
    let mut b = fs::read(&p).unwrap();
    // 중앙 디렉터리의 비압축 크기(오프셋 24)를 10으로 속인다.
    let central = (0..b.len() - 46).find(|&i| &b[i..i + 4] == b"PK\x01\x02").unwrap();
    b[central + 24..central + 28].copy_from_slice(&10u32.to_le_bytes());
    fs::write(&p, b).unwrap();
    let r = scan_file(&p, &Options { zip_deep: true, max_total_mb: 2, ..Options::default() });
    // 읽기는 상한에서 멈춘다: 끝까지 풀었다고 통과시키지 않고 미완료나 압축비 지표로 남긴다.
    assert!(r.incomplete || r.has("HIGH_COMPRESSION_RATIO") || r.has("ZIP_SIZE_MISMATCH") || r.has("ZIP_ENTRY_UNREADABLE") || r.has("ZIP_CRC_MISMATCH"), "{:?} {:?}", r.findings, r.reasons);
    assert_eq!(r.findings.iter().filter(|f| f.code == "ZIP_SIZE_MISMATCH").count(), 1, "one report per member: {:?}", r.findings);
}

#[test]
fn directory_over_the_size_limit_is_incomplete_and_not_opened() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("hugedir.zip");
    support::zip_of(&p, &[("a.txt", b"a")]);
    let mut b = fs::read(&p).unwrap();
    let eocd = (0..=b.len() - 22).rev().find(|&i| &b[i..i + 4] == b"PK\x05\x06").unwrap();
    b[eocd + 12..eocd + 16].copy_from_slice(&0x2000_0000u32.to_le_bytes()); // 512MiB
    fs::write(&p, b).unwrap();
    let r = scan(&p);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("Central directory too large")), "{:?}", r.reasons);
    assert!(r.findings.is_empty());
}

#[test]
fn nested_zip_is_flagged_and_recursed_unless_disabled() {
    let d = tempfile::tempdir().unwrap();
    let inner = d.path().join("inner.zip");
    support::zip_of(&inner, &[("../../deep-evil", b"x")]);
    let p = d.path().join("outer.zip");
    support::zip_of(&p, &[("inner.dat", &fs::read(&inner).unwrap())]);
    // 기본은 P4의 재귀 검사가 켜져 있다: 안쪽 ZIP의 경로 위험이 `멤버!안쪽이름` 위치로 올라온다.
    let r = scan(&p);
    assert_eq!(finding(&r, "NESTED_ARCHIVE").severity, Severity::Medium);
    let nested = finding(&r, "ZIP_PATH_TRAVERSAL");
    assert!(nested.location.starts_with("inner.dat!"), "{}", nested.location);
    // `--no-recurse`면 표시만 하고 안쪽은 보지 않는다.
    let off = bookdoctor::format_dispatcher::scan_file(&p, &bookdoctor::cli_parser::Options { recurse: false, ..Default::default() });
    assert!(off.has("NESTED_ARCHIVE") && !off.has("ZIP_PATH_TRAVERSAL"));
}

#[test]
fn executable_member_is_flagged_by_name_and_by_magic() {
    let d = tempfile::tempdir().unwrap();
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let p = d.path().join("exe.zip");
    support::zip_of(&p, &[("setup.exe", b"not really"), ("picture.dat", &pe), ("readme.txt", b"hi")]);
    let r = scan(&p);
    let locs: Vec<&str> = r.findings.iter().filter(|f| f.code == "EMBEDDED_EXECUTABLE").map(|f| f.location.as_str()).collect();
    assert!(locs.contains(&"setup.exe") && locs.contains(&"picture.dat") && !locs.contains(&"readme.txt"), "{locs:?}");
    assert_eq!(finding(&r, "EMBEDDED_EXECUTABLE").severity, Severity::Critical);
}

#[test]
fn encrypted_member_is_incomplete_not_clean() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("enc.zip");
    support::zip_of(&p, &[("secret.txt", &text(20)), ("open.txt", &text(5))]);
    support::mark_encrypted(&p, "secret.txt");
    let r = scan(&p);
    assert!(r.incomplete);
    assert!(r.reasons.iter().any(|x| x.contains("secret.txt") && FileReport::is_encrypted_reason(x)), "{:?}", r.reasons);
}

#[test]
fn zip_deep_is_off_by_default() {
    assert!(!Options::default().zip_deep);
    assert!(!parse(&["scan".to_string(), "a.zip".to_string()]).unwrap().zip_deep);
}

#[test]
fn traversal_is_found_when_the_raw_bytes_are_not_valid_utf8() {
    // 원본 이름 `AAAA/evil.txt`의 앞 4바이트를 `../` + 0xFF로 바꾼다. zip crate는 이를 CP437로 읽는다.
    // 디코딩한 이름과 원본 바이트의 손실 변환 이름을 모두 검사하므로 어느 쪽으로 읽어도 `..`를 놓치지 않는다.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("rawname.zip");
    support::zip_of(&p, &[("AAAA/evil.txt", b"x")]);
    let mut b = fs::read(&p).unwrap();
    let mut from = 0;
    while let Some(at) = b[from..].windows(13).position(|w| w == b"AAAA/evil.txt") {
        let at = from + at;
        b[at..at + 4].copy_from_slice(&[b'.', b'.', b'/', 0xFF]);
        from = at + 13;
    }
    fs::write(&p, b).unwrap();
    assert!(scan(&p).has("ZIP_PATH_TRAVERSAL"));
}

#[test]
fn resolve_name_falls_back_to_the_raw_bytes_and_flags_a_failed_decode() {
    let (name, undecodable) = resolve_name::<()>(Err(()), b"../x\xFF");
    assert!(undecodable && name.starts_with("../x"), "{name}");
    let (name, undecodable) = resolve_name::<()>(Ok("a/b.txt".to_string()), b"a/b.txt");
    assert!(!undecodable && name == "a/b.txt");
}

#[test]
fn stored_member_whose_declared_size_differs_from_its_extent_is_a_size_mismatch() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("forged.zip");
    support::zip_stored(&p, &[("a.bin", &vec![b'x'; 8192])]);
    let mut b = fs::read(&p).unwrap();
    let central = b.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    b[central + 24..central + 28].copy_from_slice(&10u32.to_le_bytes());
    fs::write(&p, b).unwrap();
    let r = scan(&p);
    let f = finding(&r, "ZIP_SIZE_MISMATCH");
    assert_eq!((f.category, f.severity, f.location.as_str()), (Category::Security, Severity::High, "a.bin"));
}

fn forge_declared_size(p: &Path, size: u32) {
    let mut b = fs::read(p).unwrap();
    let central = b.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    b[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
    fs::write(p, b).unwrap();
}

#[test]
fn deflate_member_declaring_less_than_the_probe_window_is_a_size_mismatch_not_unreadable() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("forged-small.zip");
    support::zip_of(&p, &[("zeros.bin", &vec![0u8; 8 * 1024 * 1024])]);
    forge_declared_size(&p, 10);
    let r = scan(&p);
    let f = finding(&r, "ZIP_SIZE_MISMATCH");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert!(!r.has("ZIP_ENTRY_UNREADABLE"), "{:?}", r.findings);
}

#[test]
fn forged_size_above_the_probe_window_is_only_seen_by_zip_deep() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("forged-big.zip");
    support::zip_of(&p, &[("zeros.bin", &vec![0u8; 8 * 1024 * 1024])]);
    forge_declared_size(&p, 5000);
    assert!(!scan(&p).has("ZIP_SIZE_MISMATCH"), "probe-only reads 4096 bytes and cannot see it (documented limit)");
    let r = deep(&p);
    assert!(r.has("ZIP_SIZE_MISMATCH") && !r.has("ZIP_ENTRY_UNREADABLE"), "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn honest_headers_never_get_a_size_mismatch() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("ok.zip");
    support::zip_of(&p, &[("a.txt", &text(200)), ("b.bin", &vec![1u8; 10_000])]);
    support::zip_stored(&d.path().join("ok2.zip"), &[("c.bin", &vec![2u8; 9000])]);
    for name in ["ok.zip", "ok2.zip"] {
        let path = d.path().join(name);
        assert!(!scan(&path).has("ZIP_SIZE_MISMATCH") && !deep(&path).has("ZIP_SIZE_MISMATCH"), "{name}");
    }
}

// --- 최종 리뷰: 실행 파일 확장자, .bin, 중복 이름 ---

#[test]
fn common_dropper_types_in_a_zip_are_flagged_by_name() {
    for name in ["invoice.js", "a.pif", "a.msc", "a.scf", "a.url", "a.iso", "a.img", "a.vhd", "a.vhdx", "a.chm", "a.xll", "a.wsc", "a.one", "a.appx", "a.msix", "a.exe", "a.lnk"] {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("z.zip");
        support::zip_of(&p, &[(name, b"payload")]);
        let r = scan(&p);
        assert!(r.findings.iter().any(|f| f.code == "EMBEDDED_EXECUTABLE" && f.location == name), "{name}: {:?}", r.findings);
    }
}

#[test]
fn bin_members_are_data_unless_their_content_is_executable() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("z.zip");
    support::zip_of(&p, &[("printerSettings1.bin", b"\x01\x02 plain data"), ("a.bin", b"data")]);
    assert!(scan(&p).findings.is_empty());
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let q = d.path().join("pe.zip");
    support::zip_of(&q, &[("real.bin", &pe)]);
    assert!(scan(&q).has("EMBEDDED_EXECUTABLE"));
}

#[test]
fn duplicate_member_names_are_reported_because_the_zip_crate_hides_one_copy() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("dup.zip");
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    support::zip_of(&p, &[("readmeA.txt", &pe), ("readmeB.txt", b"hello")]);
    let mut b = fs::read(&p).unwrap();
    let mut from = 0;
    while let Some(at) = b[from..].windows(11).position(|w| w == b"readmeB.txt") {
        let at = from + at;
        b[at..at + 11].copy_from_slice(b"readmeA.txt");
        from = at + 11;
    }
    fs::write(&p, b).unwrap();
    let r = scan(&p);
    let f = finding(&r, "ZIP_DUPLICATE_NAME");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert!(r.incomplete, "한쪽 사본을 못 본 검사는 완전하다고 할 수 없다");
}

#[test]
fn archives_without_duplicates_never_get_the_duplicate_finding() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("ok.zip");
    support::zip_of(&p, &[("a.txt", b"a"), ("dir/", b""), ("dir/b.txt", b"b")]);
    assert!(!scan(&p).has("ZIP_DUPLICATE_NAME"));
}

mod text_member_entropy {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use bookdoctor::cli_parser::Options;
    use bookdoctor::format_dispatcher::scan_file;
    use bookdoctor::{FileReport, Severity};

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
}
