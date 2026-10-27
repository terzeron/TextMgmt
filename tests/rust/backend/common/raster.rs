//! 픽셀 비교용 래스터. Python PIL 이미지에 대응한다(book_fixtures의 `Raster`를 옮겼다). `pdf.rs`의 `pixel_image`가 돌려준다.

#![allow(dead_code)]

/// mode는 "RGB" | "L" | "1" | "P".
#[derive(Clone, Debug)]
pub struct Raster {
    pub mode: &'static str,
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<[u8; 3]>,
}

impl Raster {
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb[y * self.w + x]
    }

    /// L/1 모드의 `getpixel`처럼 단일 값을 돌려준다.
    pub fn gray(&self, x: usize, y: usize) -> u8 {
        self.pixel(x, y)[0]
    }

    /// 픽셀 영역 `[x0, x1) x [y0, y1)`을 잘라낸다. 편집 인자 `edits::crop`과는 다른 것이다.
    pub fn crop(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Raster {
        let mut rgb = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                rgb.push(self.pixel(x, y));
            }
        }
        Raster { mode: self.mode, w: x1 - x0, h: y1 - y0, rgb }
    }
}

/// `_same_pixels`
pub fn same_pixels(a: &Raster, b: &Raster) -> bool {
    a.mode == b.mode && a.w == b.w && a.h == b.h && a.rgb == b.rgb
}
