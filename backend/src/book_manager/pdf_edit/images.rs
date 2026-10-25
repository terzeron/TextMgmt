use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use base64::Engine;
use image::{DynamicImage, ImageDecoder, ImageFormat};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use serde_json::Value;

use crate::direct_cover_url::ImageFetcher;
use crate::epub_cover::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS};
use crate::image_edit::{Rect, crop_pil};

pub(super) struct Edits {
    remove: BTreeSet<i64>,
    order: Vec<i64>,
    pub crops: BTreeMap<i64, Rect>,
    pub splits: BTreeMap<i64, (Rect, Rect)>,
    pub inserts: BTreeMap<i64, Source>,
}

pub(super) enum Source {
    Upload(Vec<u8>),
    Url(String),
}

fn rect(value: &Value) -> Result<Rect, String> {
    let number = |key| value.get(key).and_then(Value::as_f64).ok_or_else(|| "크롭 영역이 쪽 밖이거나 비어 있음".to_string());
    Ok((number("x0")?, number("y0")?, number("x1")?, number("y1")?, value.get("angle").and_then(Value::as_f64).unwrap_or(0.0)))
}

impl Edits {
    pub fn parse(pages: &[i64], crops: Option<&[Value]>, splits: Option<&[Value]>, inserts: Option<&[Value]>, order: Option<&[i64]>) -> Result<Self, String> {
        let mut result = Self { remove: pages.iter().copied().collect(), order: order.unwrap_or_default().to_vec(), crops: BTreeMap::new(), splits: BTreeMap::new(), inserts: BTreeMap::new() };
        let page = |v: &Value| v.get("page").and_then(Value::as_i64).ok_or_else(|| "페이지 번호가 잘못되었습니다".to_string());
        for insert in inserts.unwrap_or_default() {
            let n = page(insert)?;
            if result.inserts.contains_key(&n) {
                return Err(format!("{n}쪽 앞에 이미지가 둘 이상 지정되었습니다"));
            }
            let source = if let Some(data) = insert.get("image_data").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                    return Err("이미지가 비어 있거나 너무 큽니다".into());
                }
                Source::Upload(base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| format!("{n}쪽 앞에 넣을 이미지 데이터를 읽을 수 없습니다"))?)
            } else {
                let url = insert.get("url").and_then(Value::as_str).unwrap_or("").trim();
                if url.is_empty() {
                    return Err(format!("{n}쪽 앞에 넣을 이미지 URL이 비어 있습니다"));
                }
                Source::Url(url.into())
            };
            result.inserts.insert(n, source);
        }
        for split in splits.unwrap_or_default() {
            let n = page(split)?;
            if result.splits.insert(n, (rect(&split["first"])?, rect(&split["second"])?)).is_some() {
                return Err(format!("{n}쪽에 분할이 둘 이상 지정되었습니다"));
            }
        }
        for crop in crops.unwrap_or_default() {
            let n = page(crop)?;
            if result.crops.insert(n, rect(crop)?).is_some() {
                return Err(format!("{n}쪽에 크롭 영역이 둘 이상 지정되었습니다"));
            }
        }
        Ok(result)
    }

    pub fn validate(&self, total: usize) -> Result<(), String> {
        if self.remove.is_empty() && self.order.is_empty() && self.crops.is_empty() && self.splits.is_empty() && self.inserts.is_empty() {
            return Err("변경할 쪽을 선택하세요".into());
        }
        if self.crops.keys().any(|n| self.remove.contains(n) || self.splits.contains_key(n)) || self.splits.keys().any(|n| self.remove.contains(n)) {
            return Err("같은 쪽에 삭제, 크롭, 분할을 함께 지정할 수 없습니다".into());
        }
        if self.remove.iter().chain(self.crops.keys()).chain(self.splits.keys()).chain(self.inserts.keys()).any(|n| *n < 1 || *n > total as i64) {
            return Err(format!("페이지 범위가 잘못되었습니다 (1~{total})"));
        }
        if self.remove.len() >= total {
            return Err("모든 페이지를 삭제할 수 없습니다".into());
        }
        let mut sorted = self.order.clone();
        sorted.sort_unstable();
        if sorted.len() > total || sorted.iter().enumerate().any(|(i, &n)| n != i as i64 + 1) {
            return Err("쪽 순서가 잘못되었습니다".into());
        }
        Ok(())
    }

    pub fn apply(&self, pdf: &mut Document, fetcher: &dyn ImageFetcher) -> Result<Vec<ObjectId>, String> {
        let original = pdf.get_pages();
        let mut groups = BTreeMap::new();
        for (&n, &id) in &original {
            let n = i64::from(n);
            let mut group = Vec::new();
            if let Some(source) = self.inserts.get(&n) {
                let jpeg = match source {
                    Source::Upload(data) => normalize_upload(data),
                    Source::Url(url) => download_image(fetcher, url),
                }
                .map_err(|e| format!("{n}쪽 앞에 넣을 이미지를 가져올 수 없습니다: {e}"))?;
                let bounds = page_box(pdf, id)?;
                group.push(insert_page(pdf, &jpeg, bounds[2] - bounds[0])?);
            }
            if !self.remove.contains(&n) {
                if let Some(&(first, second)) = self.splits.get(&n) {
                    let copy = pdf.add_object(pdf.get_dictionary(id).map_err(|e| e.to_string())?.clone());
                    crop_page(pdf, id, n, first)?;
                    crop_page(pdf, copy, n, second)?;
                    group.extend([id, copy]);
                } else {
                    if let Some(&rect) = self.crops.get(&n) {
                        crop_page(pdf, id, n, rect)?;
                    }
                    group.push(id);
                }
            }
            groups.insert(n, group);
        }
        Ok(self.order.iter().copied().chain(self.order.len() as i64 + 1..=original.len() as i64).flat_map(|n| groups.remove(&n).unwrap_or_default()).collect())
    }
}

pub(super) fn download_image(fetcher: &dyn ImageFetcher, url: &str) -> Result<Vec<u8>, String> {
    use crate::direct_cover_url::FetchError;
    let bytes = fetcher.fetch_url_image(url).map_err(|error| match error {
        FetchError::Value(message) => message,
        FetchError::Os(_) => "이미지 서버 주소를 확인하지 못했습니다".into(),
        FetchError::Other(error) => {
            log::error!("download image: {error}");
            "이미지를 가져오지 못했습니다".into()
        }
    })?;
    normalize_upload(&bytes)
}

pub(super) fn normalize_upload(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.is_empty() || data.len() > MAX_IMAGE_BYTES {
        return Err("이미지가 비어 있거나 너무 큽니다".into());
    }
    let unreadable = |_| "이미지 형식을 읽을 수 없습니다".to_string();
    let reader = image::ImageReader::new(Cursor::new(data)).with_guessed_format().map_err(|_| "이미지 형식을 읽을 수 없습니다".to_string())?;
    let format = reader.format();
    let mut decoder = reader.into_decoder().map_err(unreadable)?;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS as u64 {
        return Err("이미지 해상도가 너무 큽니다".into());
    }
    let orientation = decoder.orientation().map_err(unreadable)?;
    let color = decoder.color_type();
    let mut image = DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    if format == Some(ImageFormat::Jpeg) && color == image::ColorType::Rgb8 && orientation == image::metadata::Orientation::NoTransforms && w <= 2000 && h <= 3000 {
        return Ok(data.to_vec());
    }
    image.apply_orientation(orientation);
    let image = if image.width() > 2000 || image.height() > 3000 { image.resize(2000, 3000, image::imageops::FilterType::Lanczos3) } else { image };
    let mut output = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 92).encode_image(&image.to_rgb8()).map_err(unreadable)?;
    Ok(output)
}

fn page_box(pdf: &Document, id: ObjectId) -> Result<[f64; 4], String> {
    let page = pdf.get_dictionary(id).map_err(|e| e.to_string())?;
    let value = page.get(b"CropBox").or_else(|_| page.get(b"MediaBox")).map_err(|e| e.to_string())?;
    let (_, value) = pdf.dereference(value).map_err(|e| e.to_string())?;
    let values = value.as_array().map_err(|e| e.to_string())?;
    if values.len() != 4 {
        return Err("페이지 크기가 잘못되었습니다".into());
    }
    let mut out = [0.0; 4];
    for (i, value) in values.iter().enumerate() {
        out[i] = f64::from(value.as_float().map_err(|e| e.to_string())?);
    }
    if out.iter().any(|v| !v.is_finite()) || out[2] <= out[0] || out[3] <= out[1] {
        return Err("페이지 크기가 잘못되었습니다".into());
    }
    Ok(out)
}

fn insert_page(pdf: &mut Document, jpeg: &[u8], width: f64) -> Result<ObjectId, String> {
    let (w, h) = image::ImageReader::with_format(Cursor::new(jpeg), ImageFormat::Jpeg).into_dimensions().map_err(|e| e.to_string())?;
    let height = width * f64::from(h) / f64::from(w);
    let image = pdf.add_object(Stream::new(
        dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>w as i64,"Height"=>h as i64,"BitsPerComponent"=>8,"ColorSpace"=>"DeviceRGB","Filter"=>"DCTDecode"},
        jpeg.to_vec(),
    ));
    let content = pdf.add_object(Stream::new(lopdf::Dictionary::new(), format!("q {width:.4} 0 0 {height:.4} 0 0 cm /Im0 Do Q").into_bytes()));
    Ok(pdf.add_object(dictionary! {"Type"=>"Page","MediaBox"=>vec![0.into(),0.into(),Object::Real(width as f32),Object::Real(height as f32)],"Resources"=>dictionary! {"XObject"=>dictionary! {"Im0"=>image}},"Contents"=>content}))
}

fn crop_page(pdf: &mut Document, id: ObjectId, n: i64, rect: Rect) -> Result<(), String> {
    let fail = |reason: &str| format!("{n}쪽: 크롭할 수 없습니다 ({reason})");
    let page = pdf.get_dictionary(id).map_err(|_| fail("리소스 없음"))?.clone();
    if page.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0) % 360 != 0 {
        return Err(fail("회전된 쪽"));
    }
    if !page.has(b"Contents") || !page.has(b"Resources") {
        return Err(fail("리소스 없음"));
    }
    let bytes = pdf.get_page_content_with_limit(id, 4096).map_err(|_| fail("이미지 한 장으로 된 쪽이 아님"))?;
    let ops = lopdf::content::Content::decode(&bytes).map_err(|_| fail("이미지 배치 행렬을 읽을 수 없음"))?.operations;
    if ops.len() != 4 || ops[0].operator != "q" || ops[1].operator != "cm" || ops[2].operator != "Do" || ops[3].operator != "Q" || ops[1].operands.len() != 6 || ops[2].operands.len() != 1 {
        return Err(fail("이미지 한 장으로 된 쪽이 아님"));
    }
    let m: Vec<f64> = ops[1].operands.iter().map(|v| v.as_float().map(f64::from)).collect::<lopdf::Result<_>>().map_err(|_| fail("이미지 배치 행렬을 읽을 수 없음"))?;
    let bounds = page_box(pdf, id)?;
    let (bw, bh) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    if m.iter().any(|v| !v.is_finite())
        || m[1] != 0.0
        || m[2] != 0.0
        || (m[0] - bw).abs() > 2.0_f64.max(bw * 0.01)
        || (m[3] - bh).abs() > 2.0_f64.max(bh * 0.01)
        || (m[4] - bounds[0]).abs() > 2.0_f64.max(bw * 0.01)
        || (m[5] - bounds[1]).abs() > 2.0_f64.max(bh * 0.01)
    {
        return Err(fail("이미지가 쪽 전체를 덮지 않음"));
    }
    let name = ops[2].operands[0].as_name().map_err(|_| fail("이미지를 찾을 수 없음"))?;
    let (_, resources) = pdf.dereference(page.get(b"Resources").map_err(|_| fail("리소스 없음"))?).map_err(|_| fail("리소스 없음"))?;
    let mut resources = resources.as_dict().map_err(|_| fail("리소스 없음"))?.clone();
    let (_, xobjects) = pdf.dereference(resources.get(b"XObject").map_err(|_| fail("이미지를 찾을 수 없음"))?).map_err(|_| fail("이미지를 찾을 수 없음"))?;
    let mut xobjects = xobjects.as_dict().map_err(|_| fail("이미지를 찾을 수 없음"))?.clone();
    let (_, object) = pdf.dereference(xobjects.get(name).map_err(|_| fail("이미지를 찾을 수 없음"))?).map_err(|_| fail("이미지를 찾을 수 없음"))?;
    let stream = object.as_stream().map_err(|_| fail("이미지를 찾을 수 없음"))?;
    let source = decode_pixels(pdf, stream).map_err(|reason| fail(&reason))?;
    let original_space = stream.dict.get(b"ColorSpace").ok().cloned();
    let one_bit = stream.dict.get(b"BitsPerComponent").and_then(Object::as_i64).ok() == Some(1) && rect.4.abs() < 0.01 && matches!(source, DynamicImage::ImageLuma8(_));
    let (w, h) = (source.width(), source.height());
    let cropped = crop_pil(&source, rect).map_err(|e| fail(&e.to_string()))?;
    let (ow, oh) = (cropped.width(), cropped.height());
    // 원본 픽셀을 다시 손실 압축하지 않는다. JPEG 양자화 보존은 별도 이식 대상이다.
    let grayscale = matches!(cropped, DynamicImage::ImageLuma8(_));
    let mut pixels = if grayscale { cropped.to_luma8().into_raw() } else { cropped.to_rgb8().into_raw() };
    if one_bit {
        let stride = (ow as usize).div_ceil(8);
        let mut packed = vec![0u8; stride * oh as usize];
        for (i, &pixel) in pixels.iter().enumerate() {
            if pixel != 0 {
                packed[i / ow as usize * stride + (i % ow as usize) / 8] |= 0x80 >> (i % ow as usize % 8);
            }
        }
        pixels = packed;
    }
    let mut new_image = Stream::new(
        dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>ow as i64,"Height"=>oh as i64,"BitsPerComponent"=>if one_bit {1} else {8},"ColorSpace"=>if grayscale {"DeviceGray"} else {"DeviceRGB"}},
        pixels,
    );
    if let Some(space) = original_space
        && color_components(pdf, &space) == Some(if grayscale { 1 } else { 3 })
    {
        new_image.dict.set("ColorSpace", space);
    }
    new_image.compress().map_err(|e| fail(&e.to_string()))?;
    let image = pdf.add_object(new_image);
    xobjects.set(name, image);
    resources.set("XObject", xobjects);
    let (nw, nh) = (bw * f64::from(ow) / f64::from(w), bh * f64::from(oh) / f64::from(h));
    let content = pdf.add_object(Stream::new(lopdf::Dictionary::new(), format!("q {nw:.4} 0 0 {nh:.4} 0 0 cm /{} Do Q", String::from_utf8_lossy(name)).into_bytes()));
    let page = pdf.get_object_mut(id).and_then(Object::as_dict_mut).map_err(|e| e.to_string())?;
    page.set("Resources", resources);
    page.set("Contents", content);
    page.set("MediaBox", vec![0.into(), 0.into(), Object::Real(nw as f32), Object::Real(nh as f32)]);
    for key in [b"CropBox".as_slice(), b"TrimBox", b"BleedBox", b"ArtBox"] {
        page.remove(key);
    }
    Ok(())
}

fn color_components(pdf: &Document, space: &Object) -> Option<usize> {
    let (_, space) = pdf.dereference(space).ok()?;
    match space.as_name().ok() {
        Some(b"DeviceRGB") => Some(3),
        Some(b"DeviceGray") => Some(1),
        _ => {
            let values = space.as_array().ok()?;
            if values.len() != 2 || values[0].as_name().ok() != Some(b"ICCBased") {
                return None;
            }
            let (_, profile) = pdf.dereference(&values[1]).ok()?;
            let channels = profile.as_stream().ok()?.dict.get(b"N").ok()?.as_i64().ok()?;
            match channels {
                1 => Some(1),
                3 => Some(3),
                _ => None,
            }
        }
    }
}

fn decode_pixels(pdf: &Document, stream: &Stream) -> Result<DynamicImage, String> {
    let err = || "이미지를 해독할 수 없음".to_string();
    let d = &stream.dict;
    if d.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image") {
        return Err("이미지를 찾을 수 없음".into());
    }
    if [b"SMask".as_slice(), b"Mask", b"Decode"].iter().any(|key| d.has(key)) || d.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false) {
        return Err("지원하지 않는 이미지 색상 구성".into());
    }
    let w = u32::try_from(d.get(b"Width").and_then(Object::as_i64).map_err(|_| err())?).map_err(|_| err())?;
    let h = u32::try_from(d.get(b"Height").and_then(Object::as_i64).map_err(|_| err())?).map_err(|_| err())?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS as u64 {
        return Err("이미지 해상도가 너무 큽니다".into());
    }
    let (_, space) = pdf.dereference(d.get(b"ColorSpace").map_err(|_| err())?).map_err(|_| err())?;
    let indexed = space.as_array().ok().filter(|v| v.len() == 4 && v[0].as_name().ok() == Some(b"Indexed"));
    let channels = if indexed.is_some() { 1 } else { color_components(pdf, space).ok_or_else(|| "지원하지 않는 이미지 색상 구성".to_string())? };
    let bits = d.get(b"BitsPerComponent").and_then(Object::as_i64).unwrap_or(8);
    if ![1, 2, 4, 8].contains(&bits) || channels != 1 && bits != 8 {
        return Err("지원하지 않는 이미지 색상 구성".into());
    }
    let filters = if d.has(b"Filter") { stream.filters().map_err(|_| "지원하지 않는 이미지 형식".to_string())? } else { Vec::new() };
    if filters == [b"DCTDecode".as_slice()] {
        if bits != 8 || space.as_name().is_err() {
            return Err("지원하지 않는 이미지 색상 구성".into());
        }
        let reader = image::ImageReader::with_format(Cursor::new(&stream.content), ImageFormat::Jpeg);
        let size = reader.into_dimensions().map_err(|_| err())?;
        if size != (w, h) {
            return Err("이미지 크기 불일치".into());
        }
        return image::load_from_memory_with_format(&stream.content, ImageFormat::Jpeg).map_err(|_| err());
    }
    if filters.len() > 1 || filters.iter().any(|f| ![b"FlateDecode".as_slice(), b"LZWDecode", b"RunLengthDecode"].contains(f)) {
        return Err("지원하지 않는 이미지 형식".into());
    }
    let row_bytes = (w as usize * channels * bits as usize).div_ceil(8);
    let expected = row_bytes * h as usize;
    let mut data = stream.get_plain_content_with_limit(expected + h as usize).map_err(|_| err())?;
    if data.len() != expected {
        return Err("이미지 크기 불일치".into());
    }
    if bits < 8 {
        let mask = (1u16 << bits) - 1;
        data = (0..w as usize * h as usize)
            .map(|i| {
                let (y, x) = (i / w as usize, i % w as usize);
                let bit = x * bits as usize;
                let sample = (data[y * row_bytes + bit / 8] >> (8 - bits as usize - bit % 8)) & mask as u8;
                if indexed.is_some() { sample } else { (u16::from(sample) * 255 / mask) as u8 }
            })
            .collect();
    }
    if let Some(space) = indexed {
        let channels = color_components(pdf, &space[1]).ok_or_else(err)?;
        let high = usize::try_from(space[2].as_i64().map_err(|_| err())?).map_err(|_| err())?;
        if high > 255 {
            return Err(err());
        }
        let (_, lookup) = pdf.dereference(&space[3]).map_err(|_| err())?;
        let palette = match lookup {
            Object::String(bytes, _) => bytes.clone(),
            Object::Stream(stream) => stream.get_plain_content_with_limit((high + 1) * channels).map_err(|_| err())?,
            _ => return Err(err()),
        };
        if palette.len() < (high + 1) * channels || data.iter().any(|&i| i as usize > high) {
            return Err(err());
        }
        let rgb = data
            .into_iter()
            .flat_map(|i| {
                let i = i as usize * channels;
                if channels == 3 { [palette[i], palette[i + 1], palette[i + 2]] } else { [palette[i]; 3] }
            })
            .collect();
        return Ok(DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, rgb).ok_or_else(err)?));
    }
    if channels == 3 {
        Ok(DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, data).ok_or_else(err)?))
    } else {
        Ok(DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, data).ok_or_else(err)?))
    }
}
