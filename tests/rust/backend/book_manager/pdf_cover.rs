//! `backend::book_manager::pdf_cover` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use backend::book_manager::BookManager;
use image::{DynamicImage, Rgb, RgbImage};

#[test]
fn test_blank_cover_check_returns_false_when_image_has_no_colors() {
    // 색 목록을 얻지 못하는 이미지를 흉내 낼 수 없어 크기 0 이미지를 쓴다.
    let image = image::DynamicImage::new_rgb8(0, 0);
    assert!(!BookManager::_is_mostly_blank_cover(&image));
}

#[test]
fn cover_blank_detection_does_not_merge_sixteen_nearby_colors_into_one_background() {
    let image = RgbImage::from_fn(64, 96, |x, y| {
        if y < 58 {
            let color_x = (x / 16).min(3);
            let color_y = (y / 15).min(3);
            Rgb([64 + color_x as u8 * 4, 64 + color_y as u8 * 4, 64])
        } else {
            Rgb([0, 0, 0])
        }
    });

    assert!(!BookManager::_is_mostly_blank_cover(&DynamicImage::ImageRgb8(image)));
}

#[test]
fn cover_blank_detection_matches_pillow_on_repository_image_fixtures() {
    // Python Pillow 10.4.0 oracle for the repository's actual RGB/GIF/JPEG fixtures.
    let fixtures: [(&str, &[u8], bool); 5] = [
        ("test.png", include_bytes!("../../../books/_image/test.png"), false),
        ("150.gif", include_bytes!("../../../books/_image/150.gif"), false),
        ("151.gif", include_bytes!("../../../books/_image/151.gif"), false),
        ("42sik.jpg", include_bytes!("../../../books/_image/42sik.jpg"), false),
        ("42siks.jpg", include_bytes!("../../../books/_image/42siks.jpg"), false),
    ];
    for (name, bytes, expected) in fixtures {
        let image = image::load_from_memory(bytes).unwrap();
        assert_eq!(BookManager::_is_mostly_blank_cover(&image), expected, "fixture={name}");
    }
}

#[test]
fn cover_blank_detection_matches_pillow_on_synthetic_color_patterns() {
    // Python Pillow 10.4.0 MEDIANCUT + FIND_EDGES oracle, generated from the same patterns.
    let expected = [false, false, false, false, false, false, true, true];
    for seed in 0..32u32 {
        let kind = seed % 8;
        let image = RgbImage::from_fn(64, 96, |x, y| match kind {
            0 => {
                let delta = ((x * 17 + y * 31 + seed * 13) % 9) as i16 - 4;
                Rgb([(128 + delta) as u8, (128 - delta) as u8, (128 + delta) as u8])
            }
            1 => {
                let value = if (x / 8 + y / 12) % 2 == 0 { 220 } else { 20 };
                Rgb([value, value, value])
            }
            2 if y < 58 => Rgb([64 + (x / 16).min(3) as u8 * 4, 64 + (y / 15).min(3) as u8 * 4, 64]),
            2 => Rgb([0, 0, 0]),
            3 => Rgb([(x * 4 % 256) as u8, (y * 3 % 256) as u8, ((x + y) * 2 % 256) as u8]),
            4 if (x * 7 + y * 11 + seed) % 23 < 3 => Rgb([0, 0, 0]),
            4 => Rgb([255, 255, 255]),
            5 => {
                let n = (x * 92821 + y * 68917 + seed * 421) % 256;
                Rgb([n as u8, (n * 37 % 256) as u8, (n * 83 % 256) as u8])
            }
            6 if x < 55 && y < 85 => Rgb([180, 175, 160]),
            6 => Rgb([0, 0, 0]),
            _ => Rgb([(seed * 31 % 256) as u8, (seed * 17 % 256) as u8, (seed * 7 % 256) as u8]),
        });
        assert_eq!(BookManager::_is_mostly_blank_cover(&DynamicImage::ImageRgb8(image)), expected[kind as usize], "seed={seed}");
    }
}

#[test]
fn cover_blank_detection_matches_pillow_after_bilinear_resizing() {
    // Pillow 10.4.0 oracle for 600x900 sources resized to 64x96 with BILINEAR.
    let expected = "10001011100010111000101110001011";
    let results: String = (0..32u32)
        .map(|seed| {
            let kind = seed % 8;
            let image = RgbImage::from_fn(600, 900, |x, y| {
                let sx = x * 64 / 600;
                let sy = y * 96 / 900;
                match kind {
                    0 => {
                        let delta = ((x * 17 + y * 31 + seed * 13) % 9) as i16 - 4;
                        Rgb([(128 + delta) as u8, (128 - delta) as u8, (128 + delta) as u8])
                    }
                    1 => {
                        let value = if (sx / 8 + sy / 12) % 2 == 0 { 220 } else { 20 };
                        Rgb([value, value, value])
                    }
                    2 if sy < 58 => Rgb([64 + (sx / 16).min(3) as u8 * 4, 64 + (sy / 15).min(3) as u8 * 4, 64]),
                    2 => Rgb([0, 0, 0]),
                    3 => Rgb([(x * 4 % 256) as u8, (y * 3 % 256) as u8, ((x + y) * 2 % 256) as u8]),
                    4 if (x * 7 + y * 11 + seed) % 23 < 3 => Rgb([0, 0, 0]),
                    4 => Rgb([255, 255, 255]),
                    5 => {
                        let n = (x * 92821 + y * 68917 + seed * 421) % 256;
                        Rgb([n as u8, (n * 37 % 256) as u8, (n * 83 % 256) as u8])
                    }
                    6 if sx < 55 && sy < 85 => Rgb([180, 175, 160]),
                    6 => Rgb([0, 0, 0]),
                    _ => Rgb([(seed * 31 % 256) as u8, (seed * 17 % 256) as u8, (seed * 7 % 256) as u8]),
                }
            });
            if BookManager::_is_mostly_blank_cover(&DynamicImage::ImageRgb8(image)) { '1' } else { '0' }
        })
        .collect();
    assert_eq!(results, expected);
}
