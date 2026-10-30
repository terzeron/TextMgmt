//! MS-OVBA 압축 해제와 VBA 프로젝트 읽기 테스트.

mod common;

use common::make_cfb;
use ole_reader::vba::{VbaError, decompress, read_project};
use ole_reader::{Limits, OleFile};

/// MS-OVBA 3.2.2 일반 압축 예제의 압축 바이트와 해제 결과.
const SPEC_COMPRESSED: [u8; 51] = [
    0x01, 0x2F, 0xB0, 0x00, 0x23, 0x61, 0x61, 0x61, 0x62, 0x63, 0x64, 0x65, 0x82, 0x66, 0x00, 0x70, 0x61, 0x67, 0x68, 0x69, 0x6A, 0x01, 0x38, 0x08, 0x61, 0x6B, 0x6C, 0x00, 0x20, 0x6D, 0x6E, 0x6F, 0x70, 0x06, 0x71, 0x02, 0x70, 0x04,
    0x00, 0x72, 0x73, 0x74, 0x75, 0x76, 0x10, 0x77, 0x78, 0x79, 0x7A, 0x00, 0x2C,
];
const SPEC_TEXT: &str = "#aaabcdefaaaaghijaaaaaklaaamnopqaaaaaaaaaaaarstuvwxyzaaa";

/// 테스트용 압축기: 리터럴 토큰만 쓴다(압축은 안 되지만 유효한 컨테이너다). 청크가 4096바이트를 채우면 비압축 청크로 둔다.
fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x01];
    for chunk in data.chunks(4096) {
        let mut body = Vec::new();
        for group in chunk.chunks(8) {
            body.push(0u8); // 플래그: 전부 리터럴
            body.extend_from_slice(group);
        }
        // 청크 크기 필드는 12비트다(청크 전체 길이 - 3 <= 4095). 리터럴 토큰이 이에 안 맞는 큰 청크는 비압축 청크로 둔다.
        if body.len() + 2 - 3 <= 0x0FFF {
            let header: u16 = 0xB000 | (body.len() as u16 + 2 - 3);
            out.extend_from_slice(&header.to_le_bytes());
            out.extend_from_slice(&body);
        } else {
            let header: u16 = 0x3000 | (chunk.len() as u16 + 2 - 3);
            out.extend_from_slice(&header.to_le_bytes());
            out.extend_from_slice(chunk);
        }
    }
    out
}

fn pseudo(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

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

fn record(id: u16, data: &[u8]) -> Vec<u8> {
    let mut r = id.to_le_bytes().to_vec();
    r.extend_from_slice(&(data.len() as u32).to_le_bytes());
    r.extend_from_slice(data);
    r
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// `dir` 스트림(압축 전). 앞에 모듈과 무관한 레코드 몇 개를 둔다.
fn dir_stream(modules: &[(&str, &str, u32)]) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend(record(0x0001, &1u32.to_le_bytes())); // PROJECTSYSKIND
    d.extend(record(0x0002, &0x0409u32.to_le_bytes())); // PROJECTLCID
    d.extend(record(0x0003, &1252u16.to_le_bytes())); // PROJECTCODEPAGE
    d.extend(record(0x000F, &(modules.len() as u16).to_le_bytes())); // PROJECTMODULES
    d.extend(record(0x0013, &0xFFFFu16.to_le_bytes())); // PROJECTCOOKIE
    for (name, stream, offset) in modules {
        d.extend(record(0x0019, name.as_bytes())); // MODULENAME
        let mut sn = stream.as_bytes().to_vec();
        sn.extend_from_slice(&0x0032u16.to_le_bytes());
        sn.extend_from_slice(&((stream.len() * 2) as u32).to_le_bytes());
        sn.extend(utf16(stream));
        // MODULESTREAMNAME: 크기 필드는 MBCS 이름의 길이이고 뒤에 예약값과 유니코드 이름이 이어진다.
        d.extend_from_slice(&0x001Au16.to_le_bytes());
        d.extend_from_slice(&(stream.len() as u32).to_le_bytes());
        d.extend_from_slice(&sn);
        d.extend(record(0x0031, &offset.to_le_bytes())); // MODULEOFFSET
        d.extend(record(0x002B, &[])); // 종결자
    }
    d
}

fn module_stream(junk: usize, source: &str) -> Vec<u8> {
    let mut s = pseudo(junk, 77);
    s.extend(compress(source.as_bytes()));
    s
}

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
