//! PDF 쪽 크롭과 EPUB 이미지 크롭이 함께 쓰는 크롭 기하. Python `backend/image_edit.py`에 대응한다.

use std::fmt;

use image::{DynamicImage, GenericImageView, GrayImage, RgbImage, RgbaImage};

/// x0, y0, x1, y1(이미지 크기 대비 0~1, 원점은 좌상단), angle(도, 시계 방향, 박스 중심 기준).
pub type Rect = (f64, f64, f64, f64, f64);

pub const MIN_CROP_PIXELS: u32 = 8;

/// 크롭할 수 없는 사유. 호출부가 앞에 위치 정보를 붙인다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageCropError(pub String);

impl fmt::Display for ImageCropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImageCropError {}

fn fail(reason: &str) -> ImageCropError {
    ImageCropError(reason.to_string())
}

/// Python `round`: 0.5는 짝수 쪽으로 보낸다.
fn py_round(value: f64) -> i64 {
    let floor = value.floor();
    let diff = value - floor;
    let rounded = if diff < 0.5 {
        floor
    } else if diff > 0.5 {
        floor + 1.0
    } else if floor % 2.0 == 0.0 {
        floor
    } else {
        floor + 1.0
    };
    rounded as i64
}

/// `source`에서 `rect` 영역만 잘라 새 이미지로 돌려준다 (`MIN_CROP_PIXELS` 기준).
///
/// angle이 0이 아니면 기울어진 영역을 곧게 펴서 잘라내며(재샘플링), 네 꼭짓점이 이미지 안에 있어야 한다.
/// 영역이 이미지 밖이거나 비어 있거나 한 변이 최소 픽셀 미만이면 `ImageCropError`를 돌려준다.
pub fn crop_pil(source: &DynamicImage, rect: Rect) -> Result<DynamicImage, ImageCropError> {
    crop_pil_min(source, rect, MIN_CROP_PIXELS)
}

pub fn crop_pil_min(source: &DynamicImage, rect: Rect, min_pixels: u32) -> Result<DynamicImage, ImageCropError> {
    let (x0, y0, x1, y1, angle) = rect;
    let valid = [x0, y0, x1, y1, angle].iter().all(|v| v.is_finite()) && 0.0 <= x0 && x0 < x1 && x1 <= 1.0 && 0.0 <= y0 && y0 < y1 && y1 <= 1.0 && angle.abs() <= 360.0;
    if !valid {
        return Err(fail("크롭 영역이 쪽 밖이거나 비어 있음"));
    }
    let (width, height) = source.dimensions();
    let (w, h) = (f64::from(width), f64::from(height));
    let straight = angle.abs() < 0.01;
    let (left, top, out_w, out_h) = if straight {
        let (left, top, right, bottom) = (py_round(x0 * w), py_round(y0 * h), py_round(x1 * w), py_round(y1 * h));
        (left, top, right - left, bottom - top)
    } else {
        (0, 0, py_round((x1 - x0) * w), py_round((y1 - y0) * h))
    };
    if out_w < i64::from(min_pixels) || out_h < i64::from(min_pixels) {
        return Err(fail("크롭 영역이 너무 작음"));
    }
    if straight {
        return Ok(source.crop_imm(left as u32, top as u32, out_w as u32, out_h as u32));
    }
    let theta = angle.to_radians();
    let (cos_t, sin_t) = (theta.cos(), theta.sin());
    let (center_x, center_y) = ((x0 + x1) / 2.0 * w, (y0 + y1) / 2.0 * h);
    let (half_w, half_h) = (out_w as f64 / 2.0, out_h as f64 / 2.0);
    for (sign_x, sign_y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let corner_x = center_x + cos_t * sign_x * half_w - sin_t * sign_y * half_h;
        let corner_y = center_y + sin_t * sign_x * half_w + cos_t * sign_y * half_h;
        if !(-1.5 <= corner_x && corner_x <= w + 1.5 && -1.5 <= corner_y && corner_y <= h + 1.5) {
            return Err(fail("기울어진 크롭 영역이 쪽 밖으로 나감"));
        }
    }
    // 출력 픽셀 (x, y) → 원본 좌표: 박스 중심을 기준으로 angle 만큼 돌린 위치
    let coefficients = [cos_t, -sin_t, center_x - cos_t * half_w + sin_t * half_h, sin_t, cos_t, center_y - sin_t * half_w - cos_t * half_h];
    let (out_w, out_h) = (out_w as u32, out_h as u32);
    Ok(match source {
        DynamicImage::ImageLuma8(img) => DynamicImage::ImageLuma8(GrayImage::from_raw(out_w, out_h, affine_bicubic(img.as_raw(), width, height, 1, out_w, out_h, &coefficients)).expect("크기 일치")),
        DynamicImage::ImageRgba8(img) => DynamicImage::ImageRgba8(RgbaImage::from_raw(out_w, out_h, affine_bicubic(img.as_raw(), width, height, 4, out_w, out_h, &coefficients)).expect("크기 일치")),
        other => {
            let rgb = other.to_rgb8();
            DynamicImage::ImageRgb8(RgbImage::from_raw(out_w, out_h, affine_bicubic(rgb.as_raw(), width, height, 3, out_w, out_h, &coefficients)).expect("크기 일치"))
        }
    })
}

/// Pillow `Geometry.c`의 4점 bicubic polynomial 순서를 따른다.
fn cubic_interpolate(v1: f64, v2: f64, v3: f64, v4: f64, delta: f64) -> f64 {
    let p1 = v2;
    let p2 = -v1 + v3;
    let p3 = 2.0 * (v1 - v2) + v3 - v4;
    let p4 = -v1 + v2 - v3 + v4;
    p1 + delta * (p2 + delta * (p3 + delta * p4))
}

/// PIL `Image.transform(AFFINE, BICUBIC)`과 같은 역방향 사상. 원본 밖은 흰색(255)으로 채운다.
fn affine_bicubic(src: &[u8], width: u32, height: u32, channels: usize, out_w: u32, out_h: u32, c: &[f64; 6]) -> Vec<u8> {
    let (w, h) = (width as i64, height as i64);
    let mut out = vec![255u8; out_w as usize * out_h as usize * channels];
    for y in 0..out_h {
        for x in 0..out_w {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let xin = c[0] * px + c[1] * py + c[2];
            let yin = c[3] * px + c[4] * py + c[5];
            if xin < 0.0 || yin < 0.0 || xin >= w as f64 || yin >= h as f64 {
                continue;
            }
            let (fx, fy) = (xin - 0.5, yin - 0.5);
            let (ix, iy) = (fx.floor() as i64, fy.floor() as i64);
            let (dx, dy) = (fx - ix as f64, fy - iy as f64);
            let base = (y as usize * out_w as usize + x as usize) * channels;
            for ch in 0..channels {
                let mut rows = [0.0; 4];
                for (j, row) in rows.iter_mut().enumerate() {
                    let sy = (iy + j as i64 - 1).clamp(0, h - 1);
                    let mut samples = [0.0; 4];
                    for (i, sample) in samples.iter_mut().enumerate() {
                        let sx = (ix + i as i64 - 1).clamp(0, w - 1);
                        *sample = f64::from(src[(sy as usize * width as usize + sx as usize) * channels + ch]);
                    }
                    *row = cubic_interpolate(samples[0], samples[1], samples[2], samples[3], dx);
                }
                let acc = cubic_interpolate(rows[0], rows[1], rows[2], rows[3], dy);
                out[base + ch] = acc.trunc().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}
