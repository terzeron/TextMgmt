//! `bookdoctor::file_repairer` 테스트: `repair`가 파일을 모으고, 결함 수리·바이너리 복원을 휴지통 보관과 함께 적용하는 흐름.
//! 판별·재구성 기법 자체는 `tests/rust/doc_repairer/defect_repairer.rs`, `defect_repairer.rs`의 `xhtml_fix`가 검증한다.

mod support;

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::support::shared_fixtures::{member, run_in};
use bookdoctor::cli_parser::{execute, parse};
use bookdoctor::file_repairer::*;
use bookdoctor::option_model::Options;
use doc_checker::epub::render_checker::xhtml_defect;

mod xhtml {
    use super::*;
    use crate::support::*;

    fn opf_with_chapters(items: &[(&str, &str)], extra_manifest: &str) -> String {
        let manifest: String = items.iter().map(|(id, href)| opf_item(id, href, "application/xhtml+xml", "")).collect();
        let spine: String = items.iter().map(|(id, _)| format!(r#"<itemref idref="{id}"/>"#)).collect();
        format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest>{manifest}{extra_manifest}</manifest><spine>{spine}</spine></package>"#)
    }

    const BROKEN: &str = "<html><head><title>t</title></head><body><p>본문&nbsp;글<br></body></html>";

    const GARBAGE: &[u8] = &[0xef, 0xc8, 0x1f, 0xa7, 0xe3, 0xd1, 0xeb, 0x93, 0x5d, 0xee, 0x27, 0x0b, 0x60, 0xbc, 0x87, 0x93, 0x87, 0x00, 0x34, 0xe3, 0x17, 0xb5, 0xbd, 0xf0, 0x38, 0xf4, 0xd0, 0xda];

    fn broken_epub(dir: &Path, name: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        epub_with_files(&p, &clean_opf(), BROKEN, &[]);
        p
    }

    #[test]
    fn apply_moves_the_original_to_trash_and_overwrites_in_place() {
        let d = tempfile::tempdir().unwrap();
        let trash = d.path().join("0_trash");
        let p = broken_epub(d.path(), "a.epub");
        let original = fs::read(&p).unwrap();
        let (ino, mtime) = {
            let m = fs::metadata(&p).unwrap();
            (m.ino(), m.modified().unwrap())
        };
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        let m = fs::metadata(&p).unwrap();
        assert_eq!((m.ino(), m.modified().unwrap()), (ino, mtime), "inode와 mtime을 유지한다");
        assert_eq!(fs::read(trash.join("a.epub")).unwrap(), original);
        assert_eq!(xhtml_defect(member(&fs::read(&p).unwrap(), "OEBPS/ch1.xhtml").as_bytes()), None);
        // 다시 돌려도 바꿀 것이 없고 휴지통에 새 사본이 늘지 않는다.
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 1);
    }

    #[test]
    fn trash_name_collisions_get_a_suffix_and_healthy_files_are_left_alone() {
        let d = tempfile::tempdir().unwrap();
        let trash = d.path().join("0_trash");
        fs::create_dir_all(&trash).unwrap();
        fs::write(trash.join("a.epub"), b"old").unwrap();
        let p = broken_epub(d.path(), "a.epub");
        let healthy = d.path().join("ok.epub");
        epub_with_chapter(&healthy, &xhtml("<p>글</p>"));
        let healthy_before = fs::read(&healthy).unwrap();
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap(), healthy.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(trash.join("a.1.epub").exists());
        assert_eq!(fs::read(&healthy).unwrap(), healthy_before);
        assert!(!trash.join("ok.epub").exists());
    }

    #[test]
    fn a_deletion_candidate_is_reported_and_left_in_place() {
        let d = tempfile::tempdir().unwrap();
        let opf = opf_with_chapters(&[("c1", "ch1.xhtml"), ("c2", "ch2.xhtml"), ("c3", "ch3.xhtml")], "");
        let p = d.path().join("bad.epub");
        epub_with_files(&p, &opf, &xhtml("<p>글</p>"), &[("OEBPS/ch2.xhtml", GARBAGE), ("OEBPS/ch3.xhtml", GARBAGE)]);
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("DELETE_CANDIDATE"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
    }

    #[test]
    fn a_file_with_security_findings_is_skipped_and_left_for_sanitize() {
        let d = tempfile::tempdir().unwrap();
        let trash = d.path().join("0_trash");
        let p = d.path().join("evil.epub");
        // 깨진 마크업(XHTML_PARSE_FAILED)과 스크립트가 함께 있다. 수리하면 스크립트가 되살아날 수 있으므로 먼저 --sanitize로 정리해야 한다.
        epub_with_files(&p, &clean_opf(), "<html><head></head><body><p title=\"a><script>alert(1)</script></p><br></body></html>", &[]);
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("SKIP") && out.contains("--sanitize"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
        assert!(!trash.exists() || fs::read_dir(&trash).unwrap().count() == 0);
    }

    #[test]
    fn a_file_with_visible_security_findings_is_skipped_before_any_rebuild() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("evil.epub");
        // 스크립트가 첫 검사에서 이미 보인다. 수리하기 전에 먼저 --sanitize로 정리해야 한다.
        epub_with_files(&p, &clean_opf(), "<html><head><title>t</title></head><body><p>본문<br><script>alert(1)</script></p></body></html>", &[]);
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("SKIP") && out.contains("보안 지표(HIGH 이상)가 있다"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
    }

    #[test]
    fn an_unreadable_non_markup_member_makes_the_rebuild_fail_and_the_file_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.epub");
        epub_with_files(&p, &clean_opf(), BROKEN, &[("OEBPS/data.bin", b"payload payload payload")]);
        // 압축 데이터 한가운데 한 바이트를 뒤집는다. 헤더는 멀쩡해서 검사는 통과하고 읽을 때만 실패한다.
        let mut bytes = fs::read(&p).unwrap();
        let name = b"OEBPS/data.bin";
        let at = bytes.windows(name.len()).position(|w| w == name).unwrap();
        let data_start = at + name.len() + u16::from_le_bytes([bytes[at - 2], bytes[at - 1]]) as usize;
        bytes[data_start + 2] ^= 0xff;
        fs::write(&p, &bytes).unwrap();
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("SKIP") && out.contains("data.bin"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
    }

    #[test]
    fn a_member_that_is_not_markup_is_dropped_and_listed() {
        let d = tempfile::tempdir().unwrap();
        let chapters: Vec<(String, String)> = (1..=6).map(|i| (format!("c{i}"), format!("ch{i}.xhtml"))).collect();
        let refs: Vec<(&str, &str)> = chapters.iter().map(|(id, href)| (id.as_str(), href.as_str())).collect();
        let p = d.path().join("a.epub");
        let extra: Vec<(String, &[u8])> = (3..=6).map(|i| (format!("OEBPS/ch{i}.xhtml"), &b"<html><head><title>t</title></head><body><p>x</p></body></html>"[..])).collect();
        let mut files: Vec<(&str, &[u8])> = extra.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        files.push(("OEBPS/ch2.xhtml", GARBAGE));
        epub_with_files(&p, &opf_with_chapters(&refs, ""), BROKEN, &files);
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("FIXED") && out.contains("- 제거 ") && out.contains("마크업이 아닌 데이터"), "{out}");
    }

    #[test]
    fn a_defect_in_a_member_the_rebuild_does_not_treat_as_markup_is_reported_unchanged() {
        // 스펙상 본문(spine)이지만 확장자가 .dat라서 수리 대상 멤버로 보지 않는다.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.epub");
        epub_with_files(&p, &opf_with_chapters(&[("c1", "ch1.xhtml"), ("c2", "ch2.dat")], ""), &xhtml("<p>글</p>"), &[("OEBPS/ch2.dat", BROKEN.as_bytes())]);
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("UNCHANGED"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
    }

    #[test]
    fn a_write_failure_is_reported_and_exits_1() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("0_trash"), b"not a directory").unwrap(); // 휴지통 디렉터리를 만들 수 없다
        let p = broken_epub(d.path(), "a.epub");
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("FAIL") && out.contains("실패 1건"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
    }
}

mod binary {
    use super::*;

    const OLE_AS_UTF16_CHARS: &str = "\u{cfd0}\u{e011}\u{b1a1}\u{e11a}";

    /// (원래 바이너리, 망가진 상태) 쌍. 문자열에서 만들어야 UTF-16LE 왕복이 보장된다.
    fn binary_pair() -> (Vec<u8>, Vec<u8>) {
        let text = format!("{OLE_AS_UTF16_CHARS}{}", "FileHeader 본문 내용 가나다라마바사".repeat(20));
        (text.encode_utf16().flat_map(u16::to_le_bytes).collect(), text.into_bytes())
    }

    #[test]
    fn restore_keeps_the_inode_and_mtime_and_trashes_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("0_trash");
        let (orig, mangled) = binary_pair();
        let p = dir.path().join("경혼기.txt");
        fs::write(&p, &mangled).unwrap();
        let before = fs::metadata(&p).unwrap();
        let res = restore_binary(&p, &trash).unwrap();
        assert!(matches!(res, FixResult::Restored { .. }), "{res:?}");
        assert!(!p.exists());
        let fixed = dir.path().join("경혼기.hwp");
        assert_eq!(fs::read(&fixed).unwrap(), orig);
        let after = fs::metadata(&fixed).unwrap();
        assert_eq!((after.ino(), after.mtime()), (before.ino(), before.mtime()), "book_id가 inode다");
        assert_eq!(fs::read(trash.join("경혼기.txt")).unwrap(), mangled);
    }

    #[test]
    fn normal_text_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("소설.txt");
        fs::write(&plain, "무림맹의 장문인은 제자들을 이끌었다. ".repeat(30)).unwrap();
        assert!(restore_binary(&plain, &dir.path().join("0_trash")).is_none());
        assert!(plain.exists());
    }

    #[test]
    fn restore_avoids_overwriting_an_existing_target() {
        let dir = tempfile::tempdir().unwrap();
        let (_, mangled) = binary_pair();
        let p = dir.path().join("책.txt");
        fs::write(&p, &mangled).unwrap();
        fs::write(dir.path().join("책.hwp"), "먼저 있던 것").unwrap();
        let Some(FixResult::Restored { restored_to, .. }) = restore_binary(&p, &dir.path().join("0_trash")) else { panic!("not restored") };
        assert_eq!(Path::new(&restored_to).file_name().unwrap(), "책__1.hwp");
        assert_eq!(fs::read_to_string(dir.path().join("책.hwp")).unwrap(), "먼저 있던 것");
    }

    #[test]
    fn an_unreadable_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(restore_binary(dir.path(), &dir.path().join("0_trash")), Some(FixResult::Error { .. })));
    }

    #[test]
    fn a_lossy_roundtrip_is_skipped_and_a_flaky_unmangle_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("0_trash");
        let (orig, mangled) = binary_pair();
        let f = dir.path().join("a.txt");
        fs::write(&f, &mangled).unwrap();
        // 무손실이 아니면 손대지 않는다. 원본을 잃는 것이 오탐보다 나쁘다.
        let lossy = |_: &[u8]| Some([orig.clone(), vec![0, 0]].concat());
        let Some(FixResult::Skipped { reason, .. }) = restore_binary_with(&f, &trash, &lossy) else { panic!("expected skip") };
        assert_eq!(reason, "무손실 왕복 실패");
        assert_eq!(fs::read(&f).unwrap(), mangled);
        assert!(!trash.exists());

        // 앞부분 검사는 통과하고 전체 검사에서 실패하는 경계.
        let calls = Cell::new(0);
        let flaky = |_: &[u8]| {
            calls.set(calls.get() + 1);
            (calls.get() == 1).then(|| orig.clone())
        };
        assert!(restore_binary_with(&f, &trash, &flaky).is_none());
    }

    #[test]
    fn repair_command_restores_binaries_under_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let (orig, mangled) = binary_pair();
        fs::create_dir(dir.path().join("cat")).unwrap();
        let p = dir.path().join("cat/책.txt");
        fs::write(&p, &mangled).unwrap();
        let argv: Vec<String> = ["repair".to_string(), dir.path().to_string_lossy().into_owned()].into();
        let opts = Options { library_root: dir.path().to_path_buf(), ..parse(&argv).unwrap() };
        let mut out = Vec::new();
        let code = execute(&opts, &mut out, false, true);
        let out = String::from_utf8(out).unwrap();
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("RESTORED") && out.contains("바이너리 복원 1건"), "{out}");
        assert_eq!(fs::read(dir.path().join("cat/책.hwp")).unwrap(), orig);
        assert!(dir.path().join("0_trash/책.txt").exists());
    }

    #[test]
    fn rewriting_commands_skip_the_trash_and_quarantine_directories() {
        // 루트 전체를 고쳐도 휴지통에 보관한 원본과 격리한 파일은 건드리지 않는다.
        let dir = tempfile::tempdir().unwrap();
        let (_, mangled) = binary_pair();
        for sub in ["0_trash", "0_broken/cat"] {
            fs::create_dir_all(dir.path().join(sub)).unwrap();
            fs::write(dir.path().join(sub).join("old.txt"), &mangled).unwrap();
        }
        let argv: Vec<String> = ["repair".to_string(), dir.path().to_string_lossy().into_owned()].into();
        let opts = Options { library_root: dir.path().to_path_buf(), ..parse(&argv).unwrap() };
        let mut out = Vec::new();
        assert_eq!(execute(&opts, &mut out, false, true), 0);
        assert!(String::from_utf8(out).unwrap().contains("TXT 검사 0건"));
        assert!(dir.path().join("0_trash/old.txt").exists() && dir.path().join("0_broken/cat/old.txt").exists());
    }

    #[test]
    fn a_file_that_already_has_the_right_extension_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (_, mangled) = binary_pair();
        let p = dir.path().join("책.hwp");
        fs::write(&p, &mangled).unwrap();
        assert!(restore_binary(&p, &dir.path().join("0_trash")).is_none());
    }

    #[test]
    fn a_rename_failure_is_reported_and_exits_1() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (_, mangled) = binary_pair();
        let cat = dir.path().join("cat");
        fs::create_dir(&cat).unwrap();
        fs::write(cat.join("책.txt"), &mangled).unwrap();
        fs::set_permissions(&cat, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(cat.join("probe"), b"").is_ok() {
            fs::set_permissions(&cat, fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skip: root는 읽기 전용 디렉터리에도 쓴다");
            return;
        }
        let argv: Vec<String> = ["repair".to_string(), dir.path().to_string_lossy().into_owned()].into();
        let opts = Options { library_root: dir.path().to_path_buf(), ..parse(&argv).unwrap() };
        let mut out = Vec::new();
        let code = execute(&opts, &mut out, false, true);
        fs::set_permissions(&cat, fs::Permissions::from_mode(0o755)).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("FAIL") && out.contains("실패 1건"), "{out}");
    }

    #[test]
    fn repair_command_lists_a_lossy_restore_as_skipped_and_leaves_the_file() {
        // 끝에 잘린 바이트가 붙어 있으면 되돌린 바이트가 원본과 같지 않다. 무손실이 아니면 건드리지 않는다.
        let d = tempfile::tempdir().unwrap();
        let mut raw = format!("{OLE_AS_UTF16_CHARS}{}", "FileHeader 본문 내용".repeat(20)).into_bytes();
        raw.push(0xe4);
        let p = d.path().join("mangled.txt");
        fs::write(&p, &raw).unwrap();
        let (code, out) = run_in(d.path(), &["repair", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("SKIP") && out.contains("무손실 왕복 실패") && out.contains("건너뜀 1건"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), raw);
    }
}
