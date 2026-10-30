//! MS-OVBA 압축 해제와 VBA 프로젝트 읽기 테스트.

mod common;

use common::{compress, dir_stream, make_cfb, module_stream, pseudo};
use ole_reader::vba::{VbaError, decompress, read_project};
use ole_reader::{Limits, OleFile};

/// MS-OVBA 3.2.2 일반 압축 예제의 압축 바이트와 해제 결과.
const SPEC_COMPRESSED: [u8; 51] = [
    0x01, 0x2F, 0xB0, 0x00, 0x23, 0x61, 0x61, 0x61, 0x62, 0x63, 0x64, 0x65, 0x82, 0x66, 0x00, 0x70, 0x61, 0x67, 0x68, 0x69, 0x6A, 0x01, 0x38, 0x08, 0x61, 0x6B, 0x6C, 0x00, 0x20, 0x6D, 0x6E, 0x6F, 0x70, 0x06, 0x71, 0x02, 0x70, 0x04,
    0x00, 0x72, 0x73, 0x74, 0x75, 0x76, 0x10, 0x77, 0x78, 0x79, 0x7A, 0x00, 0x2C,
];
const SPEC_TEXT: &str = "#aaabcdefaaaaghijaaaaaklaaamnopqaaaaaaaaaaaarstuvwxyzaaa";

#[test]
fn decompresses_the_spec_example() {
    assert_eq!(decompress(&SPEC_COMPRESSED, 1 << 20).unwrap(), SPEC_TEXT.as_bytes());
}

#[test]
fn literal_round_trip_for_sizes_around_the_chunk_boundary() {
    for n in [0usize, 1, 7, 8, 9, 4095, 4096, 4097, 8192, 10_000] {
        let data = pseudo(n, n as u64 + 3);
        assert_eq!(decompress(&compress(&data), 1 << 20).unwrap(), data, "n={n}");
    }
}

#[test]
fn an_uncompressed_chunk_is_copied_as_is() {
    let data = pseudo(4096, 9);
    let mut c = vec![0x01];
    c.extend_from_slice(&(0x3000u16 | 4095).to_le_bytes());
    c.extend_from_slice(&data);
    assert_eq!(decompress(&c, 1 << 20).unwrap(), data);
}

#[test]
fn a_copy_token_reaching_before_the_chunk_start_is_an_error_not_a_panic() {
    // 첫 토큰이 복사 토큰이다. 아직 해제한 바이트가 없는데 앞을 가리킨다.
    let c = [0x01, 0x02, 0xB0, 0x01, 0x00, 0x00];
    assert_eq!(decompress(&c, 1 << 20), Err(VbaError::BadOffset));
}

#[test]
fn copy_tokens_cannot_inflate_past_the_output_limit() {
    // 6바이트 청크(리터럴 'a' + 길이 4098짜리 복사 토큰)를 1000개: 해제하면 4MiB 가까이 된다.
    let mut c = vec![0x01];
    for _ in 0..1000 {
        c.extend_from_slice(&[0x03, 0xB0, 0x02, b'a', 0xFF, 0x0F]);
    }
    assert_eq!(decompress(&c, 1 << 20), Err(VbaError::OutputLimit));
    assert!(decompress(&c, 8 << 20).is_ok());
}

#[test]
fn bad_signature_and_truncated_chunks_are_errors() {
    assert_eq!(decompress(&[], 100), Err(VbaError::NotCompressed));
    assert_eq!(decompress(&[0x02, 0x00], 100), Err(VbaError::NotCompressed));
    assert_eq!(decompress(&SPEC_COMPRESSED[..5], 100), Err(VbaError::Truncated));
    assert_eq!(decompress(&[0x01, 0x2F], 100), Err(VbaError::Truncated));
}

// --- 프로젝트 읽기 ---

#[test]
fn read_project_extracts_module_sources() {
    let src1 = "Attribute VB_Name = \"Module1\"\r\nSub AutoOpen()\r\n  Shell \"calc.exe\"\r\nEnd Sub\r\n";
    let src2 = "Attribute VB_Name = \"ThisDocument\"\r\nPrivate Sub Document_Open()\r\nEnd Sub\r\n";
    let dir = compress(&dir_stream(&[("Module1", "Module1", 40), ("ThisDocument", "ThisDocument", 12)]));
    let m1 = module_stream(40, src1);
    let m2 = module_stream(12, src2);
    let cfb = make_cfb(&[("Macros/VBA/dir", &dir), ("Macros/VBA/Module1", &m1), ("Macros/VBA/ThisDocument", &m2), ("WordDocument", b"x")]);
    let mut f = OleFile::from_bytes(cfb.bytes, &Limits::default()).unwrap();
    let project = read_project(&mut f, "Macros/VBA", 1 << 20).unwrap();
    assert!(project.errors.is_empty(), "{:?}", project.errors);
    let by: Vec<(&str, &str)> = project.modules.iter().map(|m| (m.name.as_str(), m.source.as_str())).collect();
    assert_eq!(by, vec![("Module1", src1), ("ThisDocument", src2)]);
}

#[test]
fn a_module_with_a_bad_offset_is_reported_and_the_others_are_still_read() {
    let good = "Sub Good()\r\nEnd Sub\r\n";
    let dir = compress(&dir_stream(&[("Good", "Good", 8), ("Bad", "Bad", 99_999)]));
    let cfb = make_cfb(&[("Macros/VBA/dir", &dir), ("Macros/VBA/Good", &module_stream(8, good)), ("Macros/VBA/Bad", &module_stream(8, "x"))]);
    let mut f = OleFile::from_bytes(cfb.bytes, &Limits::default()).unwrap();
    let project = read_project(&mut f, "Macros/VBA", 1 << 20).unwrap();
    assert_eq!(project.modules.len(), 1);
    assert_eq!(project.modules[0].name, "Good");
    assert_eq!(project.errors.len(), 1, "{:?}", project.errors);
}

#[test]
fn a_missing_or_garbage_dir_stream_is_an_error() {
    let cfb = make_cfb(&[("WordDocument", b"x")]);
    let mut f = OleFile::from_bytes(cfb.bytes, &Limits::default()).unwrap();
    assert!(matches!(read_project(&mut f, "Macros/VBA", 1 << 20), Err(VbaError::BadDir(_))));
    let cfb = make_cfb(&[("Macros/VBA/dir", b"this is not compressed")]);
    let mut f = OleFile::from_bytes(cfb.bytes, &Limits::default()).unwrap();
    assert!(read_project(&mut f, "Macros/VBA", 1 << 20).is_err());
}

#[test]
fn the_total_output_budget_is_enforced_across_modules() {
    let big = "A".repeat(3000);
    let dir = compress(&dir_stream(&[("M1", "M1", 0), ("M2", "M2", 0)]));
    let cfb = make_cfb(&[("Macros/VBA/dir", &dir), ("Macros/VBA/M1", &module_stream(0, &big)), ("Macros/VBA/M2", &module_stream(0, &big))]);
    let mut f = OleFile::from_bytes(cfb.bytes, &Limits::default()).unwrap();
    // dir를 푸는 데 쓴 바이트까지 포함해 4000바이트만 허용하면 둘째 모듈은 못 푼다.
    let project = read_project(&mut f, "Macros/VBA", 4000).unwrap();
    assert!(project.modules.len() <= 1 && !project.errors.is_empty(), "{} {:?}", project.modules.len(), project.errors);
}

#[test]
fn modules_that_fail_to_decompress_still_consume_the_output_budget() {
    // 리터럴 3000바이트를 낸 뒤 같은 청크 안에서 잘못된 복사 토큰을 만나 오류가 나는 모듈 스트림.
    let bad_after_output = || {
        let mut body = Vec::new();
        for group in vec![b'A'; 3000].chunks(8) {
            body.push(0u8);
            body.extend_from_slice(group);
        }
        // 마지막 그룹 뒤에 복사 토큰 하나: 플래그 비트 0이 1이고 오프셋이 청크 시작 앞을 가리킨다(해제한 3000바이트보다 먼 오프셋).
        body.push(0x01);
        body.extend_from_slice(&[0xFF, 0xFF]);
        let header: u16 = 0xB000 | (body.len() as u16 + 2 - 3);
        let mut c = vec![0x01];
        c.extend_from_slice(&header.to_le_bytes());
        c.extend_from_slice(&body);
        c
    };
    assert_eq!(decompress(&bad_after_output(), 1 << 20), Err(VbaError::BadOffset), "fixture must fail after producing output");
    let names: Vec<String> = (0..6).map(|i| format!("M{i}")).collect();
    let modules: Vec<(&str, &str, u32)> = names.iter().map(|n| (n.as_str(), n.as_str(), 0u32)).collect();
    let dir = compress(&dir_stream(&modules));
    let streams: Vec<(String, Vec<u8>)> = names.iter().map(|n| (format!("Macros/VBA/{n}"), bad_after_output())).collect();
    let mut all: Vec<(&str, &[u8])> = vec![("Macros/VBA/dir", dir.as_slice())];
    all.extend(streams.iter().map(|(n, b)| (n.as_str(), b.as_slice())));
    let cfb = make_cfb(&all);
    let mut f = OleFile::from_bytes(cfb.bytes, &Limits::default()).unwrap();
    // 예산 7000바이트: dir 해제분을 빼고, 실패한 모듈이 낸 3000바이트씩이 예산에서 빠지면 서너 개째에서 멈춘다.
    let project = read_project(&mut f, "Macros/VBA", 7000).unwrap();
    assert!(project.errors.len() < 6, "실패한 해제도 예산에서 빠져야 한다: {} errors", project.errors.len());
}

#[test]
fn the_same_module_stream_is_not_decompressed_twice() {
    let src = "Sub A()\r\nEnd Sub\r\n";
    let dir = compress(&dir_stream(&[("A", "Same", 0), ("B", "Same", 0), ("C", "Same", 0)]));
    let cfb = make_cfb(&[("Macros/VBA/dir", &dir), ("Macros/VBA/Same", &module_stream(0, src))]);
    let mut f = OleFile::from_bytes(cfb.bytes, &Limits::default()).unwrap();
    let project = read_project(&mut f, "Macros/VBA", 1 << 20).unwrap();
    assert_eq!(project.modules.len(), 1, "{:?}", project.modules);
    assert_eq!(project.errors.len(), 2, "{:?}", project.errors);
}
