//! ole_reader 테스트: 읽기 코어.

mod common;

use common::{make_cfb, make_cfb_v4};
use ole_reader::{EntryKind, Limits, OleError, OleFile};

fn bytes_of(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed)).collect()
}

fn open(cfb: &common::Cfb) -> OleFile {
    OleFile::from_bytes(cfb.bytes.clone(), &Limits::default()).expect("opens")
}

#[test]
fn opens_and_lists_streams() {
    let small_a = bytes_of(100, 1);
    let small_b = bytes_of(4000, 2);
    let big = bytes_of(5000, 3);
    let file = open(&make_cfb(&[("a", &small_a), ("b", &small_b), ("big", &big)]));
    let names: Vec<(&str, EntryKind, u64)> = file.entries().iter().map(|e| (e.name.as_str(), e.kind, e.size)).collect();
    assert!(names.contains(&("Root Entry", EntryKind::Root, file.entries()[0].size)));
    assert!(names.contains(&("a", EntryKind::Stream, 100)));
    assert!(names.contains(&("b", EntryKind::Stream, 4000)));
    assert!(names.contains(&("big", EntryKind::Stream, 5000)));
    assert_eq!(file.major_version(), 3);
}

#[test]
fn reads_mini_stream_and_regular_stream_exactly() {
    let small = bytes_of(333, 9);
    let mid = bytes_of(4095, 4);
    let big = bytes_of(70_000, 5);
    let mut file = open(&make_cfb(&[("small", &small), ("mid", &mid), ("big", &big)]));
    for (name, want) in [("small", &small), ("mid", &mid), ("big", &big)] {
        let e = file.find(name).unwrap_or_else(|| panic!("{name}")).clone();
        assert_eq!(&file.read_stream(&e).unwrap(), want, "{name}");
    }
}

#[test]
fn find_is_case_insensitive_and_follows_storage_paths() {
    let dir = bytes_of(50, 1);
    let other = bytes_of(60, 2);
    let file = open(&make_cfb(&[("Macros/VBA/dir", &dir), ("Macros/PROJECT", &other), ("WordDocument", &other)]));
    assert!(file.find("Macros/VBA/dir").is_some());
    assert!(file.find("macros/vba/DIR").is_some());
    assert!(file.find("Macros/PROJECT").is_some());
    assert!(file.find("macros").is_some_and(|e| e.kind == EntryKind::Storage));
    assert!(file.find("VBA/dir").is_none(), "경로는 루트에서 시작한다");
    assert!(file.find("Macros/missing").is_none());
}

#[test]
fn version_4_with_4096_byte_sectors_is_read() {
    let small = bytes_of(200, 1);
    let big = bytes_of(9000, 2);
    let mut file = open(&make_cfb_v4(&[("s", &small), ("b", &big)]));
    assert_eq!(file.major_version(), 4);
    let e = file.find("b").unwrap().clone();
    assert_eq!(file.read_stream(&e).unwrap(), big);
    let e = file.find("s").unwrap().clone();
    assert_eq!(file.read_stream(&e).unwrap(), small);
}

#[test]
fn not_ole_and_too_small_are_errors() {
    assert!(matches!(OleFile::from_bytes(b"plain text, not an OLE file at all".to_vec(), &Limits::default()), Err(OleError::NotOle)));
    assert!(matches!(OleFile::from_bytes(Vec::new(), &Limits::default()), Err(OleError::NotOle)));
    let mut short = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    short.resize(511, 0);
    assert!(matches!(OleFile::from_bytes(short, &Limits::default()), Err(OleError::TooSmall)));
}

#[test]
fn stream_larger_than_the_limit_is_an_error_not_an_allocation() {
    let data = bytes_of(5000, 1);
    let cfb = make_cfb(&[("big", &data)]);
    let mut bytes = cfb.bytes.clone();
    // 스트림 크기를 10GB로 속인다(항목 120바이트 오프셋의 u64).
    let at = cfb.entry_offset(cfb.id_of("big")) + 120;
    bytes[at..at + 8].copy_from_slice(&(10u64 << 30).to_le_bytes());
    let mut file = OleFile::from_bytes(bytes, &Limits::default()).unwrap();
    let e = file.find("big").unwrap().clone();
    assert!(matches!(file.read_stream(&e), Err(OleError::Limit(_))));
}

#[test]
fn the_directory_walk_does_not_trust_the_sibling_tree() {
    let a = bytes_of(10, 1);
    let b = bytes_of(10, 2);
    let cfb = make_cfb(&[("a", &a), ("b", &b)]);
    let mut bytes = cfb.bytes.clone();
    // a의 오른쪽 형제와 자식을 자기 자신으로 돌린다. 순환이어도 멈춰야 하고 이름 찾기는 계속 된다.
    let ia = cfb.id_of("a");
    let at = cfb.entry_offset(ia);
    bytes[at + 72..at + 76].copy_from_slice(&(ia as u32).to_le_bytes());
    bytes[at + 76..at + 80].copy_from_slice(&(ia as u32).to_le_bytes());
    let start = std::time::Instant::now();
    let file = OleFile::from_bytes(bytes, &Limits::default()).unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert!(file.entries().iter().any(|e| e.name == "a") && file.entries().iter().any(|e| e.name == "b"));
}

#[test]
fn open_reads_a_file_from_disk_and_respects_the_total_limit() {
    let data = bytes_of(3000, 1);
    let cfb = make_cfb(&[("x", &data)]);
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.ole");
    std::fs::write(&p, &cfb.bytes).unwrap();
    assert!(OleFile::open(&p, &Limits::default()).is_ok());
    let tiny = Limits { max_total: 1024, ..Limits::default() };
    assert!(matches!(OleFile::open(&p, &tiny), Err(OleError::Limit(_))));
}

#[test]
fn path_of_joins_storage_names_from_the_root() {
    let data = bytes_of(50, 1);
    let file = open(&make_cfb(&[("Macros/VBA/dir", &data), ("WordDocument", &data)]));
    let dir = file.find("Macros/VBA/dir").unwrap().clone();
    assert_eq!(file.path_of(&dir), "Macros/VBA/dir");
    let vba = file.find("macros/vba").unwrap().clone();
    assert_eq!(file.path_of(&vba), "Macros/VBA");
    assert_eq!(file.path_of(&file.entries()[0].clone()), "");
}

#[test]
fn prefix_read_returns_only_the_requested_bytes() {
    let big = bytes_of(70_000, 5);
    let small = bytes_of(300, 6);
    let mut file = open(&make_cfb(&[("big", &big), ("small", &small)]));
    let e = file.find("big").unwrap().clone();
    assert_eq!(file.read_stream_prefix(&e, 1000).unwrap(), big[..1000].to_vec());
    let e = file.find("small").unwrap().clone();
    assert_eq!(file.read_stream_prefix(&e, 1000).unwrap(), small, "스트림이 요청보다 짧으면 전부");
    assert_eq!(file.read_stream_prefix(&e, 10).unwrap(), small[..10].to_vec());
}
