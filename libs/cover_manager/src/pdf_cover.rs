//! PDF 표지 렌더링과 거의 빈 표지 판정.
//!
//! MuPDF Rust worker로 텍스트·벡터·이미지 페이지를 렌더링한다. 출력은 제한된 크기의 PPM으로 받는다.

use image::ImageReader;
use image::imageops::FilterType;
use image::{DynamicImage, Rgb, RgbImage};
use std::io::Cursor;
use std::path::Path;
use sysdeps::SystemRunner;

/// 표지 가로:세로 비율.
pub const COVER_ASPECT_RATIO: f64 = 2.0 / 3.0;
/// 쪽 높이가 폭의 이 배 이상이면 위쪽만 표지로 자른다.
pub const COVER_TALL_RATIO: f64 = 2.0;
/// 가장 큰 색 덩어리가 이 비율 이상이고
pub const COVER_BACKGROUND_RATIO_THRESHOLD: f64 = 0.60;
/// 윤곽선 화소가 이 비율 이하이면 빈 표지로 본다.
pub const COVER_DETAIL_RATIO_THRESHOLD: f64 = 0.15;

const MAX_RENDER_DIMENSION: u32 = 1200;
fn extract_first_page_jpeg(path: &Path) -> Option<RgbImage> {
    let bytes = pdf_engine::pdf_worker_client::first_page_jpeg(&SystemRunner, path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.decode().ok().map(|image| image.to_rgb8())
}

fn crop_cover_image(rendered: &RgbImage, width: u32) -> Option<RgbImage> {
    let (render_width, render_height) = rendered.dimensions();
    let (crop_width, crop_height) = if f64::from(render_height) >= f64::from(render_width) * COVER_TALL_RATIO {
        (render_width.min(width), ((f64::from(width) / COVER_ASPECT_RATIO).round() as u32).min(render_height))
    } else if render_width > render_height {
        (render_width / 2, render_height)
    } else {
        (render_width, render_height)
    };
    if crop_width == 0 || crop_height == 0 {
        return None;
    }
    Some(image::imageops::crop_imm(rendered, 0, 0, crop_width, crop_height).to_image())
}

fn ppm_to_rgb(bytes: &[u8]) -> Option<RgbImage> {
    fn token<'a>(bytes: &'a [u8], offset: &mut usize) -> Option<&'a [u8]> {
        loop {
            while bytes.get(*offset).is_some_and(u8::is_ascii_whitespace) {
                *offset += 1;
            }
            if bytes.get(*offset) != Some(&b'#') {
                break;
            }
            while bytes.get(*offset).is_some_and(|byte| *byte != b'\n') {
                *offset += 1;
            }
        }
        let start = *offset;
        while bytes.get(*offset).is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'#') {
            *offset += 1;
        }
        (start < *offset).then_some(&bytes[start..*offset])
    }

    let mut offset = 0;
    if token(bytes, &mut offset)? != b"P6" {
        return None;
    }
    let width = std::str::from_utf8(token(bytes, &mut offset)?).ok()?.parse::<u32>().ok()?;
    let height = std::str::from_utf8(token(bytes, &mut offset)?).ok()?.parse::<u32>().ok()?;
    if token(bytes, &mut offset)? != b"255" || width == 0 || height == 0 || width > MAX_RENDER_DIMENSION || height > MAX_RENDER_DIMENSION {
        return None;
    }
    if bytes.get(offset).is_some_and(u8::is_ascii_whitespace) {
        let first = bytes[offset];
        offset += 1;
        if first == b'\r' && bytes.get(offset) == Some(&b'\n') {
            offset += 1;
        }
    } else {
        return None;
    }
    let expected = (width as usize).checked_mul(height as usize)?.checked_mul(3)?;
    if bytes.len().checked_sub(offset)? != expected {
        return None;
    }
    RgbImage::from_raw(width, height, bytes[offset..].to_vec())
}

/// Render a bounded page through the MuPDF Rust worker.
fn render_pdf_cover_page(path: &Path, page: usize, width: u32) -> Option<RgbImage> {
    let ppm = pdf_engine::pdf_worker_client::render_page(&SystemRunner, path, page, MAX_RENDER_DIMENSION).ok()?;
    crop_cover_image(&ppm_to_rgb(&ppm)?, width)
}

fn median_cut_palette(mut colors: Vec<([u8; 3], u32)>) -> Vec<[u8; 3]> {
    colors.sort_by_key(|(rgb, _)| *rgb);
    let mut boxes = vec![colors];
    while boxes.len() < 16 {
        let candidate = boxes.iter().enumerate().filter(|(_, colors)| colors.len() > 1).max_by_key(|(_, colors)| colors.iter().map(|(_, count)| *count).sum::<u32>()).map(|(index, _)| index);
        let Some(index) = candidate else { break };
        let colors = boxes.swap_remove(index);
        let ranges = (0..3)
            .map(|channel| {
                let low = colors.iter().map(|(rgb, _)| rgb[channel]).min().unwrap_or(0);
                let high = colors.iter().map(|(rgb, _)| rgb[channel]).max().unwrap_or(0);
                u32::from(high - low) * [77, 150, 29][channel]
            })
            .collect::<Vec<_>>();
        let axis = (0..3).max_by_key(|channel| ranges[*channel]).unwrap_or(0);
        let mut colors = colors;
        colors.sort_by_key(|(rgb, _)| (rgb[axis], *rgb));
        let population = colors.iter().map(|(_, count)| *count).sum::<u32>();
        let mut left_population = 0;
        let crossing = colors
            .iter()
            .position(|(_, count)| {
                left_population += *count;
                left_population * 2 > population
            })
            .unwrap_or(colors.len() - 2);
        let split_value = colors[crossing].0[axis];
        let mut split = crossing + 1;
        while split < colors.len() && colors[split].0[axis] == split_value {
            split += 1;
        }
        if split == colors.len() {
            split = crossing.max(1);
        }
        let right = colors.split_off(split);
        boxes.push(colors);
        boxes.push(right);
    }
    boxes
        .iter()
        .map(|colors| {
            let count = colors.iter().map(|(_, n)| *n).sum::<u32>();
            std::array::from_fn(|channel| {
                let sum = colors.iter().map(|(rgb, n)| u32::from(rgb[channel]) * n).sum::<u32>();
                ((sum * 2 + count) / (count * 2)) as u8
            })
        })
        .collect()
}

/// PDF 첫 쪽(거의 빈 쪽이면 둘째 쪽)을 표지로 그린다. 그리지 못하면 `None`이다.
pub fn render_pdf_cover(path: &Path, width: u32) -> Option<RgbImage> {
    static SLOT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _slot = SLOT.lock().unwrap_or_else(|error| error.into_inner());
    let first = extract_first_page_jpeg(path).and_then(|image| crop_cover_image(&image, width)).or_else(|| render_pdf_cover_page(path, 1, width))?;
    if is_mostly_blank_cover(&DynamicImage::ImageRgb8(first.clone()))
        && let Some(second) = render_pdf_cover_page(path, 2, width)
    {
        return Some(second);
    }
    Some(first)
}

/// 작은 단색 영역만 있는 이미지를 빈 표지로 판정한다. 64x96으로 줄여 가장 큰 색 덩어리의 비율과 윤곽선 화소 비율을 본다.
pub fn is_mostly_blank_cover(img: &DynamicImage) -> bool {
    if img.width() == 0 || img.height() == 0 {
        return false;
    }
    let sample = image::imageops::resize(&img.to_rgb8(), 64, 96, FilterType::Triangle);
    let pixel_count = f64::from(sample.width() * sample.height());
    // median cut: 가장 큰 색상 그룹을 반복 분할한다.
    let mut histogram = std::collections::HashMap::new();
    for pixel in sample.pixels() {
        *histogram.entry([pixel[0], pixel[1], pixel[2]]).or_insert(0u32) += 1;
    }
    let colors = histogram.iter().map(|(rgb, count)| (*rgb, *count)).collect();
    let palette = median_cut_palette(colors);
    let mut quantized_counts = vec![0u32; palette.len()];
    for (rgb, count) in &histogram {
        let closest = palette
            .iter()
            .enumerate()
            .min_by_key(|(_, color)| (0..3).map(|channel| (i32::from(rgb[channel]) - i32::from(color[channel])).pow(2) as u32).sum::<u32>())
            .map(|(index, _)| index)
            .unwrap_or(0);
        quantized_counts[closest] += count;
    }
    let largest = quantized_counts.into_iter().max().unwrap_or(0);
    let background_ratio = f64::from(largest) / pixel_count;
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
    background_ratio >= COVER_BACKGROUND_RATIO_THRESHOLD && detail_ratio <= COVER_DETAIL_RATIO_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::{extract_first_page_jpeg, median_cut_palette, ppm_to_rgb};
    use image::RgbImage;
    use lopdf::{Document, Object, Stream, dictionary};
    use std::path::Path;

    fn single_jpeg_pdf(path: &Path) {
        let image = RgbImage::from_fn(390, 559, |x, _| if x < 195 { image::Rgb([240, 20, 20]) } else { image::Rgb([20, 20, 240]) });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95).encode_image(&image).unwrap();
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let image_id = document.add_object(Stream::new(
            dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 390, "Height" => 559, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode"},
            jpeg,
        ));
        let content_id = document.add_object(Stream::new(dictionary! {}, b"q 390 0 0 559 0 0 cm /Im0 Do Q".to_vec()));
        let page_id = document.add_object(dictionary! {"Type" => "Page", "Parent" => Object::Reference(pages_id), "MediaBox" => Object::Array(vec![0, 0, 390, 559].into_iter().map(Object::Integer).collect()), "Resources" => dictionary! {"XObject" => dictionary! {"Im0" => Object::Reference(image_id)}}, "Contents" => Object::Reference(content_id)});
        document.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1}));
        let catalog_id = document.add_object(dictionary! {"Type" => "Catalog", "Pages" => Object::Reference(pages_id)});
        document.trailer.set("Root", Object::Reference(catalog_id));
        document.save(path).unwrap();
    }

    #[test]
    fn ppm_decoder_reads_poppler_pixels_and_rejects_truncated_or_oversized_output() {
        assert_eq!(ppm_to_rgb(b"P6\n1 1\n255\n\x10\x20\x30").unwrap().get_pixel(0, 0).0, [16, 32, 48]);
        assert!(ppm_to_rgb(b"P6\n2 1\n255\n\x00\x00\x00").is_none());
        assert!(ppm_to_rgb(b"P6\n1300 1300\n255\n").is_none());
    }

    #[test]
    fn median_cut_palette_is_independent_of_histogram_iteration_order() {
        let colors = vec![([40, 10, 10], 3), ([40, 10, 90], 4), ([40, 90, 10], 2), ([40, 90, 90], 5), ([90, 10, 10], 6), ([90, 10, 90], 2), ([90, 90, 10], 3), ([90, 90, 90], 7)];
        let mut reversed = colors.clone();
        reversed.reverse();

        assert_eq!(median_cut_palette(colors), median_cut_palette(reversed));
    }

    #[test]
    fn direct_extraction_accepts_only_one_bounded_first_page_jpeg() {
        use lopdf::Object;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.pdf");
        single_jpeg_pdf(&path);
        assert!(extract_first_page_jpeg(&path).is_some());
        for key in ["SMask", "Mask", "Width", "Height", "Filter"] {
            single_jpeg_pdf(&path);
            let mut doc = lopdf::Document::load(&path).unwrap();
            let stream = doc.objects.values_mut().find_map(|o| o.as_stream_mut().ok().filter(|s| s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image"))).unwrap();
            match key {
                "Width" => stream.dict.set("Width", 1_000_000),
                "Height" => stream.dict.set("Height", 1),
                "Filter" => stream.dict.set("Filter", Object::Name(b"FlateDecode".to_vec())),
                _ => stream.dict.set(key, Object::Null),
            }
            doc.save(&path).unwrap();
            assert!(extract_first_page_jpeg(&path).is_none(), "{key}");
        }
    }

    #[test]
    fn extracts_only_the_first_page_jpeg_without_rasterizing_the_pdf() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.pdf");
        single_jpeg_pdf(&path);

        let image = extract_first_page_jpeg(&path).expect("first page JPEG");

        assert_eq!(image.dimensions(), (390, 559));
        assert!(image.get_pixel(20, 20)[0] > 180);
        assert!(image.get_pixel(370, 20)[2] > 180);
    }
}
