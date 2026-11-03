#[path = "../ole_common.rs"]
mod ole_common;

// --- 압축 해제 ---

use bookdoctor::hwp::inflate::{InflateError, decompress};
use flate2::Compression;
use flate2::write::{DeflateEncoder, ZlibEncoder};
use std::io::Write;

fn raw_deflate(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

#[test]
fn zlib_and_raw_deflate_both_decode() {
    let text = b"hello hwp ".repeat(50);
    assert_eq!(decompress(&raw_deflate(&text), 1 << 20), Ok(text.clone()));
    assert_eq!(decompress(&zlib(&text), 1 << 20), Ok(text));
}

#[test]
fn output_over_the_limit_is_too_large_without_allocating_it() {
    // 1GiB의 0을 압축하면 1MiB 남짓이다. 상한 1MiB로 풀면 TooLarge이고 메모리를 쓰지 않는다.
    let bomb = raw_deflate(&vec![0u8; 64 * 1024 * 1024]);
    assert!(bomb.len() < 128 * 1024, "fixture sanity: {}", bomb.len());
    assert_eq!(decompress(&bomb, 1 << 20), Err(InflateError::TooLarge));
    assert_eq!(decompress(&zlib(&vec![0u8; 8 << 20]), 1 << 20), Err(InflateError::TooLarge));
    // 상한과 같은 크기는 허용한다.
    assert_eq!(decompress(&raw_deflate(&vec![1u8; 1 << 20]), 1 << 20).map(|v| v.len()), Ok(1 << 20));
}

#[test]
fn truncated_and_garbage_are_invalid() {
    let full = raw_deflate(&b"some text that compresses ".repeat(100));
    assert_eq!(decompress(&full[..full.len() / 2], 1 << 20), Err(InflateError::Invalid));
    assert_eq!(decompress(&[0xFF, 0xEE, 0xDD, 0xCC, 0xBB], 1 << 20), Err(InflateError::Invalid));
    let mut seed = 99u32;
    for _ in 0..200 {
        let bytes: Vec<u8> = (0..128).map(|_| { seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); (seed >> 24) as u8 }).collect();
        let _ = decompress(&bytes, 1 << 16); // panic하지 않는다.
    }
}

#[test]
fn empty_input_is_an_empty_output() {
    assert_eq!(decompress(&[], 1 << 20), Ok(Vec::new()));
}

// --- BinData (명세 표 17, 18) ---

use bookdoctor::hwp::inflate::decompress_prefix;

#[test]
fn decompress_prefix_stops_after_n_bytes_without_error() {
    let bomb = raw_deflate(&vec![0u8; 64 * 1024 * 1024]);
    assert_eq!(decompress_prefix(&bomb, 4096).map(|v| v.len()), Ok(4096));
    // 끊긴 스트림도 그때까지 풀린 앞부분을 돌려준다.
    let full = raw_deflate(&b"abcdefgh".repeat(4096));
    let cut = &full[..full.len() / 2];
    assert!(decompress_prefix(cut, 16).is_ok_and(|v| v == b"abcdefghabcdefgh"));
    assert_eq!(decompress_prefix(&[0xFF, 0xEE, 0xDD], 16), Err(InflateError::Invalid));
}
