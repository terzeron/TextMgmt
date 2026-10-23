//! 쪽·이미지 편집 요청의 JSON 인자 빌더. 픽셀을 다루는 `Raster::crop`과 이름이 부딪치지 않게 book_fixtures의 `crop`, `crop_at`,
//! `split`, `insert`, `upload`를 이 모듈로 옮겼다(설계 5장). 호출은 `edits::crop(..)`처럼 모듈 이름을 붙인다.
//!
//! 사용: `use common::edits;` `edits::crop(1, 0.0, 0.0, 0.5, 0.5)`

#![allow(dead_code)]

use serde_json::{json, Value};

use super::image::b64;

/// 회전 각도까지 정하는 자르기 요청.
pub fn crop_at(page: i64, x0: f64, y0: f64, x1: f64, y1: f64, angle: f64) -> Value {
    // serde_json은 NaN을 null로 바꾼다. 비유한(non-finite) 각도는 null로 전달된다.
    json!({"page": page, "x0": x0, "y0": y0, "x1": x1, "y1": y1, "angle": angle})
}

pub fn crop(page: i64, x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    crop_at(page, x0, y0, x1, y1, 0.0)
}

pub fn split(page: i64, first: [f64; 4], second: [f64; 4]) -> Value {
    let rect = |r: [f64; 4]| json!({"x0": r[0], "y0": r[1], "x1": r[2], "y1": r[3]});
    json!({"page": page, "first": rect(first), "second": rect(second)})
}

/// image_env의 왼쪽은 빨강, 오른쪽은 파랑이다.
pub const LEFT: [f64; 4] = [0.0, 0.0, 0.5, 1.0];
pub const RIGHT: [f64; 4] = [0.5, 0.0, 1.0, 1.0];

pub fn insert_url(page: i64, url: &str) -> Value {
    json!({"page": page, "url": url})
}

pub fn insert(page: i64) -> Value {
    insert_url(page, "https://img.example.com/a.png")
}

pub fn upload(page: i64, data: &[u8]) -> Value {
    json!({"page": page, "image_data": b64(data)})
}
