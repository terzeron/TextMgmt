//! PDF 표지 렌더링. Python `_extract_cover_thumbnail`의 PDF 갈래와 `_is_mostly_blank_cover`에 대응한다.
//!
//! TODO(pdf): Python은 pdfium으로 쪽을 그린다. pdfium 바인딩(`pdfium-render`)은 런타임에 `libpdfium`이 있어야 해서 이 crate의 의존에 넣지 않았다.
//! 여기서는 빈 쪽과 쪽 전체를 덮는 이미지 한 장뿐인 쪽을 지원한다. JPEG 및 8비트 RGB/Gray 픽셀 스트림을 읽는다.
//! 그 밖의 쪽(텍스트, 벡터 등)은 `None`이고, 호출자는 표지를 만들 수 없는 파일로 취급한다.
//!
//! 안전: 파일 크기, 이미지 바이트, 이미지 화소 수를 읽기와 디코딩 전에 상한과 비교한다.

use std::path::Path;

use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, Rgb, RgbImage};
use lopdf::{Document, Object, ObjectId};

use super::BookManager;

/// 이 크기를 넘는 PDF는 읽지 않는다. `lopdf`는 파일 전체를 메모리에 올린다.
pub const MAX_PDF_COVER_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// 표지로 쓸 쪽 이미지 스트림 하나의 바이트 상한.
pub const MAX_PDF_COVER_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// 표지로 쓸 쪽 이미지의 화소 수 상한(PIL 기본 한도와 비슷하다).
pub const MAX_PDF_COVER_IMAGE_PIXELS: u64 = 89_000_000;

/// 쪽의 내용. 이 렌더러가 그릴 수 있는 것만 구분한다.
enum PageContent {
    Blank,
    Image(DynamicImage),
}

fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(f64::from(*v)),
        _ => None,
    }
}

/// 쪽의 MediaBox를 `(폭, 높이)`로. 상위 `Pages`에서 물려받은 값도 읽는다.
fn page_size(doc: &Document, page_id: ObjectId) -> Option<(f64, f64)> {
    let mut id = page_id;
    for _ in 0..16 {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(media_box) = dict.get(b"MediaBox") {
            let (_, media_box) = doc.dereference(media_box).ok()?;
            let values: Vec<f64> = media_box.as_array().ok()?.iter().filter_map(|o| doc.dereference(o).ok().and_then(|(_, o)| number(o))).collect();
            if let [x0, y0, x1, y1] = values[..] {
                let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
                return (w > 0.0 && h > 0.0).then_some((w, h));
            }
            return None;
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// 쪽이 비었는가, 이미지 한 장뿐인가를 가른다. 그리지 못하는 쪽은 `None`이다.
fn page_content(doc: &Document, page_id: ObjectId) -> Option<PageContent> {
    let content = doc.get_page_content(page_id);
    let blank_stream = content.iter().all(|b| b.is_ascii_whitespace());
    let (resources, _) = doc.get_page_resources(page_id).ok()?;
    let xobjects = resources.and_then(|r| r.get(b"XObject").ok()).and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_dict().ok());
    let mut images = Vec::new();
    if let Some(xobjects) = xobjects {
        for (_, object) in xobjects.iter() {
            let (_, object) = doc.dereference(object).ok()?;
            if let Ok(stream) = object.as_stream()
                && stream.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Image")
            {
                images.push(stream);
            }
        }
    }
    if images.is_empty() {
        return blank_stream.then_some(PageContent::Blank);
    }
    // 쪽 전체를 덮는 이미지 한 장(`q W 0 0 H x y cm /Im Do Q`)만 지원한다.
    if images.len() != 1 || content.len() > 512 || !String::from_utf8_lossy(&content).contains("Do") {
        return None;
    }
    let stream = images[0];
    if stream.content.len() > MAX_PDF_COVER_IMAGE_BYTES {
        return None;
    }
    let filter = stream.dict.get(b"Filter").ok().and_then(|f| match f {
        Object::Name(name) => Some(name.clone()),
        Object::Array(list) if list.len() == 1 => list[0].as_name().ok().map(<[u8]>::to_vec),
        _ => None,
    });
    if filter.as_deref() == Some(b"DCTDecode") {
        let (w, h) = image::ImageReader::new(std::io::Cursor::new(&stream.content)).with_guessed_format().ok()?.into_dimensions().ok()?;
        if u64::from(w) * u64::from(h) > MAX_PDF_COVER_IMAGE_PIXELS {
            return None;
        }
        return image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg).ok().map(PageContent::Image);
    }
    // PDF의 픽셀 스트림은 PNG/JPEG 컨테이너가 없다. 선언한 크기로 출력 상한을 정한 뒤 압축을 푼다.
    if stream.dict.get(b"BitsPerComponent").ok()?.as_i64().ok()? != 8 || [b"SMask".as_slice(), b"Mask", b"Decode"].iter().any(|key| stream.dict.has(key)) {
        return None;
    }
    let w = u32::try_from(stream.dict.get(b"Width").ok()?.as_i64().ok()?).ok()?;
    let h = u32::try_from(stream.dict.get(b"Height").ok()?.as_i64().ok()?).ok()?;
    let pixels = u64::from(w) * u64::from(h);
    if pixels == 0 || pixels > MAX_PDF_COVER_IMAGE_PIXELS {
        return None;
    }
    let colorspace = stream.dict.get(b"ColorSpace").ok()?.as_name().ok()?;
    let channels = match colorspace {
        b"DeviceRGB" => 3,
        b"DeviceGray" => 1,
        _ => return None,
    };
    let expected = usize::try_from(pixels.checked_mul(channels)?).ok()?;
    // PNG predictor는 복원 전 각 행에 필터 바이트 하나를 더 쓴다. 중간 버퍼도 이 한도 안에서만 허용한다.
    let data = stream.get_plain_content_with_limit(expected.checked_add(h as usize)?).ok()?;
    if data.len() != expected {
        return None;
    }
    let image = if channels == 3 { DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, data)?) } else { DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, data)?) };
    Some(PageContent::Image(image))
}

/// 쪽의 `[x0, x1] x [y0, y1]`(0~1, 위가 0) 영역을 `out` 크기로 그린다.
fn render_region(content: &PageContent, region: (f64, f64, f64, f64), out: (u32, u32)) -> RgbImage {
    let (out_w, out_h) = (out.0.max(1), out.1.max(1));
    match content {
        PageContent::Blank => ImageBuffer::from_pixel(out_w, out_h, Rgb([255, 255, 255])),
        PageContent::Image(img) => {
            let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
            let (x0, y0) = ((region.0 * iw).floor() as u32, (region.2 * ih).floor() as u32);
            let (x1, y1) = (((region.1 * iw).ceil() as u32).min(img.width()), ((region.3 * ih).ceil() as u32).min(img.height()));
            let cropped = img.crop_imm(x0.min(x1.saturating_sub(1)), y0.min(y1.saturating_sub(1)), x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
            image::imageops::resize(&cropped.to_rgb8(), out_w, out_h, FilterType::Lanczos3)
        }
    }
}

impl BookManager {
    /// PDF 쪽 하나를 표지 모양으로 그린다. 쪽 모양에 따라 자르는 규칙은 Python과 같다:
    /// 가로의 2배 이상 긴 쪽은 위쪽 표지 영역만, 가로 쪽은 좌측 절반(펼침면), 그 밖에는 쪽 전체를 비율대로 줄인다.
    fn render_pdf_cover_page(doc: &Document, page_id: ObjectId, width: u32) -> Option<RgbImage> {
        let (pw, ph) = page_size(doc, page_id)?;
        let content = page_content(doc, page_id)?;
        let height = (f64::from(width) / Self::COVER_ASPECT_RATIO).round() as u32;
        if ph >= pw * Self::COVER_TALL_RATIO {
            let cover_height = pw / Self::COVER_ASPECT_RATIO;
            return Some(render_region(&content, (0.0, 1.0, 0.0, (cover_height / ph).min(1.0)), (width, height)));
        }
        if pw > ph {
            let half = pw / 2.0;
            let scale = (f64::from(width) / half).min(f64::from(height) / ph);
            return Some(render_region(&content, (0.0, 0.5, 0.0, 1.0), ((half * scale).round() as u32, (ph * scale).round() as u32)));
        }
        let scale = (f64::from(width) / pw).min(f64::from(height) / ph);
        Some(render_region(&content, (0.0, 1.0, 0.0, 1.0), ((pw * scale).round() as u32, (ph * scale).round() as u32)))
    }

    /// PDF 첫 쪽(거의 빈 쪽이면 둘째 쪽)을 표지로 그린다. 그리지 못하면 `None`이다.
    pub(super) fn render_pdf_cover(path: &Path, width: u32) -> Option<RgbImage> {
        if std::fs::metadata(path).ok()?.len() > MAX_PDF_COVER_FILE_BYTES {
            return None;
        }
        let doc = Document::load(path).ok()?;
        let pages = doc.get_pages();
        let first = Self::render_pdf_cover_page(&doc, *pages.get(&1)?, width)?;
        if pages.len() > 1 && Self::_is_mostly_blank_cover(&DynamicImage::ImageRgb8(first.clone())) {
            return pages.get(&2).and_then(|id| Self::render_pdf_cover_page(&doc, *id, width)).or(Some(first));
        }
        Some(first)
    }

    /// 작은 단색 영역만 있는 이미지를 빈 표지로 판정한다. 64x96으로 줄여 가장 큰 색 덩어리의 비율과 윤곽선 화소 비율을 본다.
    pub fn _is_mostly_blank_cover(img: &DynamicImage) -> bool {
        let sample = image::imageops::resize(&img.to_rgb8(), 64, 96, FilterType::Triangle);
        let pixel_count = f64::from(sample.width() * sample.height());
        // 16단계로 줄인 색의 도수. PIL `quantize(colors=16)`의 근사로 채널마다 2비트씩 4단계로 나눈 64칸 중 가장 큰 칸을 본다.
        let mut buckets = std::collections::HashMap::new();
        for pixel in sample.pixels() {
            *buckets.entry((pixel[0] >> 6, pixel[1] >> 6, pixel[2] >> 6)).or_insert(0u32) += 1;
        }
        let Some(largest) = buckets.values().max() else { return false };
        let background_ratio = f64::from(*largest) / pixel_count;
        // FIND_EDGES: 가장자리 한 줄은 원본을 그대로 두고 안쪽만 커널(-1 ... 8 ... -1)을 적용한다. L로 바꿔 24를 넘는 화소를 센다.
        let (w, h) = (sample.width() as usize, sample.height() as usize);
        let luma = |p: &Rgb<u8>| (u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000;
        let mut detail = 0u32;
        for y in 0..h {
            for x in 0..w {
                let center = sample.get_pixel(x as u32, y as u32);
                let value = if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                    luma(center)
                } else {
                    let mut channels = [0i32; 3];
                    for (c, slot) in channels.iter_mut().enumerate() {
                        let mut sum = 8 * i32::from(center[c]);
                        for dy in -1i32..=1 {
                            for dx in -1i32..=1 {
                                if dx != 0 || dy != 0 {
                                    sum -= i32::from(sample.get_pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32)[c]);
                                }
                            }
                        }
                        *slot = sum.clamp(0, 255);
                    }
                    luma(&Rgb([channels[0] as u8, channels[1] as u8, channels[2] as u8]))
                };
                if value > 24 {
                    detail += 1;
                }
            }
        }
        let detail_ratio = f64::from(detail) / pixel_count;
        background_ratio >= Self::COVER_BACKGROUND_RATIO_THRESHOLD && detail_ratio <= Self::COVER_DETAIL_RATIO_THRESHOLD
    }
}
