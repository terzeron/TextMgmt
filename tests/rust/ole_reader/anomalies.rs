//! ole_reader 테스트: 손상·악의적 구조를 Anomaly로 알린다. 읽을 수 있는 만큼 읽고 panic하지 않는다.

mod common;

use std::time::{Duration, Instant};

use common::{Cfb, make_cfb, make_cfb_v4};
use ole_reader::{Anomaly, Limits, OleError, OleFile};

fn bytes_of(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(13).wrapping_add(seed)).collect()
}

fn sample() -> Cfb {
    let small = bytes_of(120, 1);
    let mid = bytes_of(3000, 2);
    let big = bytes_of(5000, 3);
    let big2 = bytes_of(6000, 4);
    make_cfb(&[("small", &small), ("mid", &mid), ("big", &big), ("big2", &big2), ("Macros/VBA/dir", &small)])
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// 구조를 망가뜨린 바이트를 열어 (결과, 걸린 시간)을 돌려준다. 어떤 입력도 2초를 넘기면 안 된다.
fn open_timed(bytes: Vec<u8>, limits: &Limits) -> (Result<OleFile, OleError>, Duration) {
    let start = Instant::now();
    let r = OleFile::from_bytes(bytes, limits);
    (r, start.elapsed())
}

fn anomalies(bytes: Vec<u8>) -> Vec<Anomaly> {
    let (r, took) = open_timed(bytes, &Limits::default());
    assert!(took < Duration::from_secs(2), "{took:?}");
    r.map(|f| f.anomalies().to_vec()).unwrap_or_default()
}

fn start_of(cfb: &Cfb, name: &str) -> u32 {
    OleFile::from_bytes(cfb.bytes.clone(), &Limits::default()).unwrap().find(name).unwrap().start
}

#[test]
fn clean_files_have_no_anomalies() {
    assert!(anomalies(sample().bytes).is_empty());
    let small = bytes_of(100, 1);
    let big = bytes_of(9000, 2);
    assert!(anomalies(make_cfb_v4(&[("s", &small), ("b", &big), ("St/x", &small)]).bytes).is_empty());
}

#[test]
fn a_fat_chain_that_loops_is_reported() {
    let cfb = sample();
    let start = start_of(&cfb, "big");
    let mut b = cfb.bytes.clone();
    put32(&mut b, cfb.fat_entry_offset(start), start);
    assert!(anomalies(b).contains(&Anomaly::FatChainCycle { start }));
}

#[test]
fn a_chain_that_leaves_the_file_is_reported() {
    let cfb = sample();
    let start = start_of(&cfb, "big");
    let mut b = cfb.bytes.clone();
    put32(&mut b, cfb.fat_entry_offset(start), 0x00FF_FFFF);
    assert!(anomalies(b).contains(&Anomaly::SectorOutOfRange { sector: 0x00FF_FFFF }));
}

#[test]
fn a_false_fat_sector_count_in_the_header_is_reported() {
    let cfb = sample();
    let mut b = cfb.bytes.clone();
    put32(&mut b, 44, 7);
    assert!(anomalies(b).contains(&Anomaly::FatSectorCountMismatch { header: 7, found: 1 }));
}

#[test]
fn a_stream_declared_larger_than_the_whole_file_is_reported() {
    let cfb = sample();
    let id = cfb.id_of("big");
    let mut b = cfb.bytes.clone();
    let at = cfb.entry_offset(id) + 120;
    b[at..at + 8].copy_from_slice(&50_000_000u64.to_le_bytes());
    assert!(anomalies(b).contains(&Anomaly::StreamSizeExceedsFile { id: id as u32, size: 50_000_000 }));
}

#[test]
fn two_streams_sharing_sectors_are_reported_as_cross_linked() {
    let cfb = sample();
    let big_start = start_of(&cfb, "big");
    let mut b = cfb.bytes.clone();
    let at = cfb.entry_offset(cfb.id_of("big2")) + 116;
    put32(&mut b, at, big_start);
    assert!(anomalies(b).iter().any(|a| matches!(a, Anomaly::CrossLinkedSectors { .. })));
}

#[test]
fn more_entries_than_the_limit_is_reported() {
    let cfb = sample();
    let limits = Limits { max_entries: 3, ..Limits::default() };
    let (r, _) = open_timed(cfb.bytes.clone(), &limits);
    let f = r.unwrap();
    assert!(f.anomalies().iter().any(|a| matches!(a, Anomaly::TooManyEntries(n) if *n > 3)));
    assert!(f.entries().len() <= 3);
}

#[test]
fn a_wrong_name_length_and_an_invalid_object_type_are_reported() {
    let cfb = sample();
    let id = cfb.id_of("mid");
    let mut b = cfb.bytes.clone();
    let at = cfb.entry_offset(id);
    b[at + 64..at + 66].copy_from_slice(&2u16.to_le_bytes());
    assert!(anomalies(b).contains(&Anomaly::NameLengthMismatch { id: id as u32 }));
    let mut b = cfb.bytes.clone();
    b[at + 66] = 9;
    assert!(anomalies(b).contains(&Anomaly::InvalidObjectType { id: id as u32, value: 9 }));
}

#[test]
fn false_header_fields_are_reported_and_reading_continues() {
    let cfb = sample();
    let mut b = cfb.bytes.clone();
    b[28..30].copy_from_slice(&0xFEFFu16.to_le_bytes());
    assert!(anomalies(b).contains(&Anomaly::BadByteOrder));
    let mut b = cfb.bytes.clone();
    b[30..32].copy_from_slice(&10u16.to_le_bytes());
    let a = anomalies(b.clone());
    assert!(a.contains(&Anomaly::BadSectorShift(10)), "{a:?}");
    assert!(OleFile::from_bytes(b, &Limits::default()).is_ok(), "주버전에 맞춰 계속 읽는다");
    let mut b = cfb.bytes.clone();
    b[32..34].copy_from_slice(&7u16.to_le_bytes());
    assert!(anomalies(b).contains(&Anomaly::BadMiniSectorShift(7)));
    let mut b = cfb.bytes.clone();
    put32(&mut b, 56, 8192);
    assert!(anomalies(b).contains(&Anomaly::BadMiniCutoff(8192)));
}

#[test]
fn bytes_after_the_last_sector_are_reported() {
    let mut b = sample().bytes;
    b.extend_from_slice(&[0xAB; 100]);
    assert!(anomalies(b).contains(&Anomaly::TrailingGarbage(100)));
}

#[test]
fn a_difat_chain_that_ends_early_is_reported() {
    let cfb = sample();
    let mut b = cfb.bytes.clone();
    put32(&mut b, 68, 0x00FF_FFFF);
    put32(&mut b, 72, 1);
    assert!(anomalies(b).contains(&Anomaly::DifatTruncated));
}

#[test]
fn a_directory_chain_that_continues_past_the_end_of_the_file_is_reported() {
    let streams: Vec<(String, Vec<u8>)> = (0..12).map(|i| (format!("s{i:02}"), bytes_of(40, i as u8))).collect();
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    let cfb = make_cfb(&refs);
    assert!(cfb.dir_sectors >= 2);
    let mut b = cfb.bytes.clone();
    // 첫 디렉터리 섹터의 다음을 FAT 안에 있지만 파일에는 없는 섹터(100번)로 돌린다.
    put32(&mut b, cfb.fat_entry_offset(cfb.dir_sector), 100);
    let (r, _) = open_timed(b, &Limits::default());
    let f = r.expect("첫 섹터의 항목은 읽을 수 있다");
    assert!(f.anomalies().contains(&Anomaly::DirectoryTruncated), "{:?}", f.anomalies());
    assert!(f.entries().len() >= 3);
}

#[test]
fn a_looping_directory_chain_is_reported() {
    let streams: Vec<(String, Vec<u8>)> = (0..12).map(|i| (format!("s{i:02}"), bytes_of(40, i as u8))).collect();
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    let cfb = make_cfb(&refs);
    let mut b = cfb.bytes.clone();
    let last = cfb.dir_sector + cfb.dir_sectors - 1;
    put32(&mut b, cfb.fat_entry_offset(last), cfb.dir_sector);
    assert!(anomalies(b).contains(&Anomaly::DirectoryCycle));
}

#[test]
fn small_streams_without_a_mini_stream_are_reported() {
    let cfb = sample();
    let mut b = cfb.bytes.clone();
    let at = cfb.entry_offset(0);
    put32(&mut b, at + 116, 0xFFFF_FFFE);
    b[at + 120..at + 128].copy_from_slice(&0u64.to_le_bytes());
    assert!(anomalies(b).contains(&Anomaly::MiniStreamMissing));
}

#[test]
fn hostile_header_counts_do_not_hang_or_allocate_wildly() {
    let cfb = sample();
    let mut b = cfb.bytes.clone();
    put32(&mut b, 44, 0xFFFF_FFFF);
    put32(&mut b, 72, 0xFFFF_FFFF);
    put32(&mut b, 68, cfb.fat_sectors[0]);
    let (r, took) = open_timed(b, &Limits::default());
    assert!(took < Duration::from_secs(2), "{took:?}");
    let _ = r; // 열려도 오류여도 된다. 멈추거나 panic하지 않으면 된다.
    // 모든 섹터가 서로를 가리키는 FAT
    let mut b = cfb.bytes.clone();
    for s in 0..128u32 {
        put32(&mut b, cfb.fat_entry_offset(s), (s + 1) % 128);
    }
    let (_, took) = open_timed(b, &Limits::default());
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn anomaly_codes_are_stable_names() {
    let table: [(Anomaly, &str); 17] = [
        (Anomaly::BadByteOrder, "OLE_BAD_BYTE_ORDER"),
        (Anomaly::BadSectorShift(1), "OLE_BAD_SECTOR_SHIFT"),
        (Anomaly::BadMiniSectorShift(1), "OLE_BAD_MINI_SECTOR_SHIFT"),
        (Anomaly::BadMiniCutoff(1), "OLE_BAD_MINI_CUTOFF"),
        (Anomaly::FatChainCycle { start: 1 }, "OLE_FAT_CYCLE"),
        (Anomaly::SectorOutOfRange { sector: 1 }, "OLE_SECTOR_OUT_OF_RANGE"),
        (Anomaly::FatSectorCountMismatch { header: 1, found: 2 }, "OLE_FAT_COUNT_MISMATCH"),
        (Anomaly::DifatTruncated, "OLE_DIFAT_TRUNCATED"),
        (Anomaly::DirectoryTruncated, "OLE_DIRECTORY_TRUNCATED"),
        (Anomaly::DirectoryCycle, "OLE_DIRECTORY_CYCLE"),
        (Anomaly::TooManyEntries(1), "OLE_TOO_MANY_ENTRIES"),
        (Anomaly::StreamSizeExceedsFile { id: 1, size: 2 }, "OLE_STREAM_SIZE_EXCEEDS_FILE"),
        (Anomaly::MiniStreamMissing, "OLE_MINI_STREAM_MISSING"),
        (Anomaly::CrossLinkedSectors { sector: 1 }, "OLE_CROSS_LINKED"),
        (Anomaly::TrailingGarbage(1), "OLE_TRAILING_GARBAGE"),
        (Anomaly::NameLengthMismatch { id: 1 }, "OLE_NAME_LENGTH_MISMATCH"),
        (Anomaly::InvalidObjectType { id: 1, value: 9 }, "OLE_INVALID_OBJECT_TYPE"),
    ];
    for (a, code) in table {
        assert_eq!(a.code(), code);
        assert!(!a.detail().is_empty());
    }
}

#[test]
fn free_directory_slots_filled_with_0xff_are_not_anomalies() {
    // 실제 파일(HWP 5.x를 쓰는 작성기)은 안 쓰는 디렉터리 슬롯을 0xFF로 채운다. 유형 0xFF, 시작·크기가 모두 0xFF인 슬롯은 빈 슬롯으로 본다.
    let one = bytes_of(60, 1);
    let cfb = make_cfb(&[("a", &one), ("b", &one), ("c", &one), ("d", &one)]); // 루트 + 4개 = 5항목, 섹터 둘에 슬롯 8개
    let used = cfb.names.len();
    let slots = (cfb.dir_sectors as usize * 512) / 128;
    assert!(slots > used, "fixture needs spare slots");
    let mut b = cfb.bytes.clone();
    let at = cfb.entry_offset(used);
    b[at..at + 128].fill(0xFF);
    assert!(anomalies(b).is_empty());
    // 시작·크기가 0xFF가 아닌 유형 0xFF 항목은 그대로 이상으로 본다.
    let mut b = cfb.bytes.clone();
    b[at..at + 128].fill(0);
    b[at + 66] = 0xFF;
    assert!(anomalies(b).iter().any(|a| matches!(a, Anomaly::InvalidObjectType { value: 0xFF, .. })));
}
