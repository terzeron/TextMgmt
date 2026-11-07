//! 표지 후보 캐시 준비물: 검색·이미지 내려받기를 가짜로 바꾸고 후보와 이미지를 캐시에 채운다.
//! `cover_manager`와 backend 테스트가 함께 쓴다.

#![allow(dead_code)]

use std::io::Cursor;
use std::sync::{Arc, Mutex};

use cover_manager::cover_management::{CandidateView, CoverManagement};
use cover_manager::cover_sources::{SearchStatus, StoreCandidate, StoreSearch};
use image::{ImageFormat, Rgb, RgbImage};

use super::image::RED;

/// 단색 이미지를 `format`으로 인코딩한다.
pub fn encode_image(width: u32, height: u32, rgb: [u8; 3], format: ImageFormat) -> Vec<u8> {
    let image = RgbImage::from_pixel(width, height, Rgb(rgb));
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).unwrap();
    out.into_inner()
}

/// 120x180 빨강 JPEG.
pub fn image_bytes() -> Vec<u8> {
    encode_image(120, 180, RED, ImageFormat::Jpeg)
}

pub struct Cached {
    pub candidates: Vec<CandidateView>,
    pub fetches: Arc<Mutex<Vec<String>>>,
}

impl Cached {
    pub fn id(&self, index: usize) -> &str {
        &self.candidates[index].candidate_id
    }
}

/// 검색과 이미지 내려받기를 가짜로 바꾸고 후보와 이미지를 캐시에 채운다.
pub fn cached_candidates(service: &mut CoverManagement, relative: &str, stores: &[&str]) -> Cached {
    let fetches = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = fetches.clone();
    let hooks = service
        .hooks()
        .clone()
        .with_search(|store, metadata| {
            let candidates = (0..2)
                .map(|index| StoreCandidate {
                    title: metadata.title.clone(),
                    author: "저자".into(),
                    detail_url: "https://www.yes24.com/".into(),
                    image_url: format!("https://image.yes24.com/{store}-{index}.jpg"),
                })
                .collect();
            Ok(StoreSearch { status: SearchStatus::Success, search_url: String::new(), candidates })
        })
        .with_fetch_bytes(move |_store, url| {
            seen.lock().unwrap().push(url.to_string());
            Ok(image_bytes())
        });
    service.set_hooks(hooks);
    let mut candidates = Vec::new();
    for store in stores {
        candidates.extend(service.candidates(store, relative, false, false).unwrap().candidates);
    }
    for candidate in &candidates {
        service.image(&candidate.candidate_id).unwrap();
    }
    Cached { candidates, fetches }
}
