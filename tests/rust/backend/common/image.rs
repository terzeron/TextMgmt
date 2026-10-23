//! 이미지 바이트 생성·해독 도구. cover_fixtures, book_fixtures, cover, book_manager_1, book_manager_gaps, epub_cover_set에
//! 흩어져 있던 `b64`, `image_size`, `encode_image`/`encode`, `png_solid`, `solid`, `jpeg_bytes`, `image_bytes`를 하나로 모았다.
//! 설계: docs/rust-test-seams.md 5장.
//!
//! 사용: `mod common;` 다음에 `use common::image::{red_jpeg, png_solid};`

#![allow(dead_code)]

use std::io::Cursor;

use base64::Engine;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

pub const RED: [u8; 3] = [255, 0, 0];
pub const BLUE: [u8; 3] = [0, 0, 255];

/// Python `base64.b64encode(data).decode()`.
pub fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// 인코딩된 이미지의 (너비, 높이). 해독에 실패하면 panic한다.
pub fn image_size(data: &[u8]) -> (u32, u32) {
    let img = image::load_from_memory(data).expect("이미지 해독");
    (img.width(), img.height())
}

/// 단색 `RgbImage`. Python `Image.new("RGB", size, color)`.
pub fn solid(width: u32, height: u32, rgb: [u8; 3]) -> RgbImage {
    RgbImage::from_pixel(width, height, Rgb(rgb))
}

/// 이미지를 `format`으로 인코딩한다. `RgbImage`와 `DynamicImage`를 모두 받는다.
pub fn encode(img: impl Into<DynamicImage>, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.into().write_to(&mut out, format).unwrap();
    out.into_inner()
}

/// 단색 이미지를 `format`으로 인코딩한다. `encode(solid(..), format)`와 같다.
pub fn encode_image(width: u32, height: u32, rgb: [u8; 3], format: ImageFormat) -> Vec<u8> {
    encode(solid(width, height, rgb), format)
}

pub fn png_solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    encode_image(width, height, rgb, ImageFormat::Png)
}

pub fn jpeg_solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    encode_image(width, height, rgb, ImageFormat::Jpeg)
}

/// cover.rs의 `png_bytes`. 진한 빨강 단색 PNG.
pub fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    png_solid(width, height, [200, 30, 30])
}

/// 화질을 지정하는 JPEG 인코딩.
pub fn jpeg_bytes(img: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(img).unwrap();
    out
}

/// 120x180 빨강 JPEG. Python `image_bytes()`. cover_fixtures, epub_cover_set, book_manager_gaps가 각자 정의하던 것이다.
pub fn red_jpeg() -> Vec<u8> {
    jpeg_solid(120, 180, RED)
}

/// 이름 색의 120x180 JPEG. epub_cover_set의 `image_bytes_of(color)`.
pub fn color_jpeg(name: &str) -> Vec<u8> {
    jpeg_solid(120, 180, color(name))
}

/// PIL 색 이름 중 테스트가 쓰는 것.
pub fn color(name: &str) -> [u8; 3] {
    match name {
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "pink" => [255, 192, 203],
        "white" => [255, 255, 255],
        other => panic!("모르는 색: {other}"),
    }
}

/// PIL `image.paste(color, (x0, y0, x1, y1))`.
pub fn paste(image: &mut RgbImage, (x0, y0, x1, y1): (u32, u32, u32, u32), rgb: [u8; 3]) {
    for y in y0..y1 {
        for x in x0..x1 {
            image.put_pixel(x, y, Rgb(rgb));
        }
    }
}

/// `_png()`: 위쪽 절반은 초록, 아래쪽 절반은 노랑인 PNG.
pub fn png_halves(width: u32, height: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(width, height, |_, y| if y < height / 2 { Rgb([0, 200, 0]) } else { Rgb([230, 230, 0]) });
    encode(img, ImageFormat::Png)
}

/// 왼쪽 절반은 빨강, 오른쪽 절반은 파랑인 이미지. 자르기·나누기 결과를 색으로 구분할 때 쓴다.
pub fn red_blue_halves(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, _| if x < width / 2 { Rgb(RED) } else { Rgb(BLUE) })
}

/// 이미 만든 JPEG의 SOI 바로 뒤에 EXIF 방향(Orientation) APP1 조각을 끼운다.
pub fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\x00\x2a\x00\x00\x00\x08"); // big-endian, IFD0 오프셋 8
    tiff.extend_from_slice(&1u16.to_be_bytes()); // 항목 1개
    tiff.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
    tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_be_bytes()); // 다음 IFD 없음
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

pub fn jpeg_with_orientation(orientation: u16, width: u32, height: u32) -> Vec<u8> {
    with_exif_orientation(&jpeg_bytes(&solid(width, height, [10, 120, 200]), 75), orientation)
}

/// multipart/form-data 파일 필드 한 개짜리 본문.
pub fn multipart_file(boundary: &str, field: &str, filename: &str, content_type: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n").into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// pytest.approx(기본 상대 오차 1e-6)에 대응한다.
pub fn assert_approx(actual: f64, expected: f64, what: &str) {
    let tolerance = (1e-6 * expected.abs()).max(1e-12);
    assert!((actual - expected).abs() <= tolerance, "{what}: {actual} != {expected}");
}
