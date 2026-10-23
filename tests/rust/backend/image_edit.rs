//! 원본: tests/test_image_edit.py
//!
//! crop_pil: PDF 쪽과 EPUB 이미지가 함께 쓰는 크롭 기하.
//!
//! 가정한 API:
//! - `backend::image_edit::crop_pil(image: &image::DynamicImage, rect: (f64, f64, f64, f64, f64)) -> Result<image::DynamicImage, ImageCropError>`
//!   (rect는 x0, y0, x1, y1, angle. 앞의 네 값은 이미지 크기에 대한 비율이다)
//! - `backend::image_edit::ImageCropError`: `Display`가 사유 문장을 돌려준다 (`pytest.raises(match=...)`는 부분 문자열로 확인한다)
//!
//! `image` crate에는 1비트 모드가 없다. PIL `Image.new("1", ...)`는 값이 전부 255인 `ImageLuma8`로 대신하고
//! 결과가 8비트 회색(`ColorType::L8`)인지 확인한다.

use backend::image_edit::{ImageCropError, crop_pil};
use image::{ColorType, DynamicImage, GenericImageView, ImageBuffer, Luma, Rgb, RgbImage};

fn rgb(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::new(width, height))
}

#[test]
fn test_axis_aligned_crop_uses_fractions_of_the_image() {
    let out = crop_pil(&rgb(200, 100), (0.25, 0.1, 0.75, 0.9, 0.0)).unwrap();
    assert_eq!(out.dimensions(), (100, 80));
}

#[test]
fn test_tilted_crop_straightens_the_region() {
    let source = DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 200, Rgb([255, 255, 255])));
    let out = crop_pil(&source, (0.3, 0.3, 0.7, 0.7, 10.0)).unwrap();
    assert_eq!(out.dimensions(), (80, 80));
}

#[test]
fn test_one_bit_image_is_rotated_as_grey() {
    let source = DynamicImage::ImageLuma8(ImageBuffer::<Luma<u8>, _>::from_pixel(200, 200, Luma([255])));
    let out = crop_pil(&source, (0.3, 0.3, 0.7, 0.7, 5.0)).unwrap();
    assert_eq!(out.color(), ColorType::L8);
}

#[test]
fn test_invalid_regions_raise_with_a_reason() {
    let cases: [((f64, f64, f64, f64, f64), &str); 5] = [
        ((0.5, 0.1, 0.4, 0.9, 0.0), "밖이거나 비어"),
        ((0.0, 0.0, 1.2, 1.0, 0.0), "밖이거나 비어"),
        ((0.0, 0.0, f64::NAN, 1.0, 0.0), "밖이거나 비어"),
        ((0.0, 0.0, 0.01, 1.0, 0.0), "너무 작음"),
        ((0.0, 0.0, 1.0, 1.0, 30.0), "쪽 밖으로 나감"),
    ];
    for (rect, reason) in cases {
        let err: ImageCropError = crop_pil(&rgb(200, 200), rect).expect_err(&format!("rect={rect:?}: 오류가 나야 한다"));
        assert!(err.to_string().contains(reason), "rect={rect:?}: {err} 에 {reason:?}가 없다");
    }
}
