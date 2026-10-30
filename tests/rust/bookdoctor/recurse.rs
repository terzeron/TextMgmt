//! 재귀 검사 테스트: 컨테이너 안의 컨테이너를 깊이·개수·바이트 상한 안에서 검사한다.

mod common;

use std::fs;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bookdoctor::cli::{Options, run};
use bookdoctor::recurse::ScratchDir;
use bookdoctor::scan::scan_file;
use scan_common::FileReport;

const OLE_STUB: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 macro";

fn bytes_of(build: impl FnOnce(&Path)) -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("x");
    build(&p);
    fs::read(&p).unwrap()
}

/// 매크로 프로젝트가 든 `.docx`의 바이트(매크로 없는 형식에 든 `vbaProject.bin`이라 Critical).
fn vba_docx() -> Vec<u8> {
    bytes_of(|p| common::docx_with(p, &[], &[("word/vbaProject.bin", OLE_STUB)]))
}

fn zip_bytes(members: &[(&str, &[u8])]) -> Vec<u8> {
    bytes_of(|p| common::zip_of(p, members))
}

fn scan_bytes(name: &str, data: &[u8], opts: &Options) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, data).unwrap();
    scan_file(&p, opts)
}

fn has_vba_under(r: &FileReport, prefix: &str) -> bool {
    r.findings.iter().any(|f| f.code == "OOXML_VBA_PROJECT" && f.location.starts_with(prefix) && f.location.contains("vbaProject.bin"))
}

#[test]
fn a_docx_inside_a_zip_is_scanned_with_a_path_prefix() {
    let outer = zip_bytes(&[("note.txt", b"hello"), ("inner.docx", &vba_docx())]);
    let r = scan_bytes("a.zip", &outer, &Options::default());
    assert!(has_vba_under(&r, "inner.docx!"), "{:?} {:?}", r.findings, r.reasons);
    // 바깥 보고서의 점수에도 반영된다.
    assert!(r.risk_score > 0, "{}", r.risk_score);
}

#[test]
fn an_embedded_docx_inside_a_docx_is_scanned() {
    let inner = vba_docx();
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("outer.docx");
    common::docx_with(&p, &[], &[("word/embeddings/e.docx", &inner)]);
    let r = scan_file(&p, &Options::default());
    assert!(has_vba_under(&r, "word/embeddings/e.docx!") || has_vba_under(&r, "e.docx!"), "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn nesting_levels_up_to_the_limit_are_reached_and_deeper_ones_are_incomplete() {
    // 바깥 ⊃ l1 ⊃ l2 ⊃ l3.docx: 3단계 아래까지는 닿는다.
    let l3 = vba_docx();
    let l2 = zip_bytes(&[("l3.docx", &l3)]);
    let l1 = zip_bytes(&[("l2.zip", &l2)]);
    let outer = zip_bytes(&[("l1.zip", &l1)]);
    let r = scan_bytes("a.zip", &outer, &Options::default());
    assert!(has_vba_under(&r, "l1.zip!l2.zip!l3.docx!"), "{:?} {:?}", r.findings, r.reasons);
    assert!(!r.reasons.iter().any(|m| m.contains("recursion")), "{:?}", r.reasons);
    // 한 단계 더 깊으면 닿지 못하고 미완료다.
    let l1 = zip_bytes(&[("l2.zip", &zip_bytes(&[("l3.zip", &zip_bytes(&[("l4.docx", &l3)]))]))]);
    let outer = zip_bytes(&[("l1.zip", &l1)]);
    let r = scan_bytes("b.zip", &outer, &Options::default());
    assert!(!r.findings.iter().any(|f| f.code == "OOXML_VBA_PROJECT"), "{:?}", r.findings);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("recursion depth")), "{:?}", r.reasons);
}

#[test]
fn a_very_deep_chain_is_bounded_and_fast() {
    let mut data = vba_docx();
    for i in 0..30 {
        data = zip_bytes(&[(if i % 2 == 0 { "n.zip" } else { "m.zip" }, &data)]);
    }
    let started = Instant::now();
    let r = scan_bytes("deep.zip", &data, &Options::default());
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("recursion")), "{:?}", r.reasons);
}

#[test]
fn too_many_nested_files_is_incomplete() {
    let tiny = zip_bytes(&[("a.txt", b"x")]);
    let names: Vec<String> = (0..100).map(|i| format!("n{i:03}.zip")).collect();
    let members: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), tiny.as_slice())).collect();
    let r = scan_bytes("many.zip", &zip_bytes(&members), &Options::default());
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("recursion file limit")), "{:?}", r.reasons);
}

#[test]
fn the_byte_budget_is_shared_across_members() {
    // 안쪽 ZIP 둘이 각각 700KB로 선언되지만 0 채움이라 바깥 파일은 작다. 예산 1MiB는 첫째만 허용한다.
    let zeros = vec![0u8; 700_000];
    let big = bytes_of(|p| common::zip_stored(p, &[("pad.bin", &zeros)]));
    assert!(big.len() > 700_000);
    let outer = zip_bytes(&[("a.zip", &big), ("b.zip", &big)]);
    assert!(outer.len() < 100_000, "{}", outer.len());
    let opts = Options { max_total_mb: 1, ..Options::default() };
    let r = scan_bytes("budget.zip", &outer, &opts);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("recursion byte budget")), "{:?}", r.reasons);
    // 예산이 넉넉하면 둘 다 본다.
    let r = scan_bytes("budget2.zip", &outer, &Options::default());
    assert!(!r.reasons.iter().any(|m| m.contains("recursion")), "{:?}", r.reasons);
}

#[test]
fn a_large_nested_archive_does_not_blow_up_time_or_disk() {
    // 안쪽 ZIP이 100MiB의 0을 압축해 담는다. 앞부분만 읽고 컨테이너가 아닌 멤버는 풀지 않으므로 빠르다.
    let zeros = vec![0u8; 100 * 1024 * 1024];
    let inner = bytes_of(|p| {
        let mut z = zip::ZipWriter::new(fs::File::create(p).unwrap());
        z.start_file("zeros.bin", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        z.write_all(&zeros).unwrap();
        z.finish().unwrap();
    });
    let outer = zip_bytes(&[("inner.zip", &inner)]);
    let started = Instant::now();
    let r = scan_bytes("bomb.zip", &outer, &Options::default());
    assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
    assert!(!r.reasons.iter().any(|m| m.contains("panic")), "{:?}", r.reasons);
}

#[test]
fn a_pdf_member_is_scanned_and_a_plain_member_is_not() {
    let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /OpenAction << /S /JavaScript /JS (app.alert(1)) >> >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";
    let r = scan_bytes("a.zip", &zip_bytes(&[("doc.pdf", pdf), ("pic.png", b"\x89PNG\r\n\x1a\n data")]), &Options::default());
    assert!(r.findings.iter().any(|f| f.location.starts_with("doc.pdf!")) || r.reasons.iter().any(|m| m.starts_with("doc.pdf")), "{:?} {:?}", r.findings, r.reasons);
    assert!(!r.findings.iter().any(|f| f.location.starts_with("pic.png!")), "{:?}", r.findings);
}

#[test]
fn member_names_never_reach_the_filesystem() {
    let probe_dir = std::env::temp_dir();
    let marks = ["bd_probe_escape.zip", "bd_probe_abs.zip"];
    for m in marks {
        let _ = fs::remove_file(probe_dir.join(m));
    }
    let tiny = zip_bytes(&[("a.txt", b"x")]);
    let long = format!("{}.zip", "L".repeat(300));
    let abs = format!("{}/bd_probe_abs.zip", probe_dir.display());
    let outer = zip_bytes(&[("../../bd_probe_escape.zip", &tiny), (abs.as_str(), &tiny), (long.as_str(), &tiny), ("evil\u{202e}gpj.zip", &tiny)]);
    let r = scan_bytes("names.zip", &outer, &Options::default());
    for m in marks {
        assert!(!probe_dir.join(m).exists(), "{m} was created from a member name");
    }
    assert!(!r.reasons.iter().any(|m| m.contains("panic")), "{:?}", r.reasons);
}

#[test]
fn the_scratch_dir_is_private_and_removed_on_drop_and_on_panic() {
    let kept: PathBuf;
    {
        let s = ScratchDir::new().unwrap();
        let f = s.write(b"data", "zip").unwrap();
        assert!(f.exists() && f.file_name().unwrap().to_string_lossy().ends_with(".zip"));
        kept = s.path().to_path_buf();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&kept).unwrap().permissions().mode() & 0o777, 0o700);
    }
    assert!(!kept.exists());
    let leaked = std::sync::Mutex::new(PathBuf::new());
    let result = catch_unwind(AssertUnwindSafe(|| {
        let s = ScratchDir::new().unwrap();
        *leaked.lock().unwrap() = s.path().to_path_buf();
        s.write(b"x", "zip").unwrap();
        panic!("boom");
    }));
    assert!(result.is_err());
    assert!(!leaked.lock().unwrap().exists());
}

#[test]
fn no_recurse_turns_the_nested_scan_off() {
    let outer = zip_bytes(&[("inner.docx", &vba_docx())]);
    let opts = Options { recurse: false, ..Options::default() };
    let r = scan_bytes("a.zip", &outer, &opts);
    assert!(!has_vba_under(&r, "inner.docx!"), "{:?}", r.findings);
    assert!(Options::default().recurse);
    // 명령행 옵션도 받는다.
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.zip"), &outer).unwrap();
    let mut out = Vec::new();
    run(&["--no-recurse".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().join("a.zip").to_string_lossy().into_owned()], &mut out);
    assert!(!String::from_utf8(out).unwrap().contains("inner.docx!"));
}
