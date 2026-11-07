//! `doc_repairer::encoding_repairer` 단위 테스트. 훅을 쓰는 시나리오는 loader 통합 테스트에 있다.

use doc_repairer::encoding_repairer::reencode_text_file_to_utf8;
use std::fs;
use std::path::Path;

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn reencode_converts_cp949_in_place_and_leaves_utf8_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요 반갑습니다\n둘째 줄입니다\n");
    let path = write(tmp.path(), "k.txt", &bytes);
    let inode = std::os::unix::fs::MetadataExt::ino(&fs::metadata(&path).unwrap());

    let dry = reencode_text_file_to_utf8(&path, true, None, Path::new("k.txt"));
    assert!(dry.changed && dry.reason.contains("[dry-run]") && dry.reason.contains("euc-kr"), "{dry:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes.as_ref(), "dry-run은 파일을 바꾸지 않는다");

    let backup = tmp.path().join("backup");
    let done = reencode_text_file_to_utf8(&path, false, Some(&backup), Path::new("sub/k.txt"));
    assert!(done.changed, "{done:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "안녕하세요 반갑습니다\n둘째 줄입니다\n");
    assert_eq!(fs::read(backup.join("sub/k.txt")).unwrap(), bytes.as_ref());
    assert_eq!(std::os::unix::fs::MetadataExt::ino(&fs::metadata(&path).unwrap()), inode, "같은 inode에 덮어쓴다");
    assert!(!tmp.path().join("k.txt.utf8.tmp").exists());

    let again = reencode_text_file_to_utf8(&path, false, None, Path::new("k.txt"));
    assert_eq!((again.changed, again.reason.as_str()), (false, "이미 UTF-8"));
    let empty = write(tmp.path(), "e.txt", b"");
    assert_eq!(reencode_text_file_to_utf8(&empty, false, None, Path::new("e.txt")).reason, "빈 파일");
    assert!(reencode_text_file_to_utf8(&tmp.path().join("none"), false, None, Path::new("n")).reason.starts_with("읽기 실패"));
}

#[test]
fn reencode_refuses_when_nothing_decodes_losslessly_or_the_backup_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let garbage = write(tmp.path(), "g.txt", b"\x80\x81\x82\xff\xfe\x00\x01 abc \xfd");
    let r = reencode_text_file_to_utf8(&garbage, false, None, Path::new("g.txt"));
    assert!(!r.changed, "{r:?}");

    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요 반갑습니다\n");
    let path = write(tmp.path(), "k.txt", &bytes);
    let blocker = write(tmp.path(), "blocker", b"x");
    let r = reencode_text_file_to_utf8(&path, false, Some(&blocker), Path::new("k.txt"));
    assert!(!r.changed && r.reason.starts_with("백업 실패"), "{r:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes.as_ref(), "백업이 실패하면 변환하지 않는다");
}

#[test]
fn western_single_byte_text_is_converted_via_cp1252_not_cp949() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(tmp.path(), "w.txt", b"Don Sabas\x97a man called caf\xe9 and more text here\n");
    let r = reencode_text_file_to_utf8(&path, false, None, Path::new("w.txt"));
    assert!(r.changed && r.reason.contains("cp1252"), "{r:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "Don Sabas\u{2014}a man called café and more text here\n");
}

#[test]
fn custom_decoders_must_round_trip_and_read_as_real_text() {
    use doc_repairer::encoding_repairer::{ReencodeHooks, reencode_text_file_to_utf8_with};
    use text_decoder::text_transcoder::Candidate;
    let latin1_decode: fn(&[u8]) -> String = |raw| raw.iter().map(|&b| char::from(b)).collect();
    let latin1_encode: fn(&str) -> Option<Vec<u8>> = |text| text.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect();
    let rejected: [(&str, Candidate<'static>); 3] = [
        ("대체 문자", Candidate::Custom { name: "replacement", decode: |_| "\u{fffd}".to_string(), encode: |_| None }),
        ("되돌릴 수 없음", Candidate::Custom { name: "one-way", decode: |raw| raw.iter().map(|&b| char::from(b)).collect(), encode: |_| None }),
        (
            "글자가 없음",
            Candidate::Custom {
                name: "symbols",
                decode: |raw| raw.iter().map(|&b| char::from_u32(0x2500 + u32::from(b)).unwrap()).collect(),
                encode: |text| text.chars().map(|c| u8::try_from(u32::from(c) - 0x2500).ok()).collect(),
            },
        ),
    ];
    let tmp = tempfile::tempdir().unwrap();
    for (why, candidate) in rejected {
        let path = write(tmp.path(), "c.txt", b"caf\xe9 au lait\n");
        let hooks = ReencodeHooks { encodings: Some(vec![candidate]), ..ReencodeHooks::default() };
        let r = reencode_text_file_to_utf8_with(&path, false, None, Path::new("c.txt"), &hooks);
        assert!(!r.changed, "{why}: {r:?}");
    }
    let path = write(tmp.path(), "c.txt", b"caf\xe9 au lait\n");
    let hooks = ReencodeHooks { encodings: Some(vec![Candidate::Custom { name: "latin1", decode: latin1_decode, encode: latin1_encode }]), ..ReencodeHooks::default() };
    let r = reencode_text_file_to_utf8_with(&path, false, None, Path::new("c.txt"), &hooks);
    assert!(r.changed, "{r:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "café au lait\n");
}
