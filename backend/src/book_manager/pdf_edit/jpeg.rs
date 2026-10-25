//! JPEG 크롭의 인코딩 설정. 원본 DQT/SOF를 읽고 같은 양자화·샘플링·progressive로 저장한다.
use image::DynamicImage;
use jpeg_encoder::{ColorType, Encoder, QuantizationTableType, SamplingFactor};
use std::collections::BTreeMap;

// JPEG DQT의 zigzag 순서 → 인코더가 받는 행 우선 순서.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52,
    45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

struct Settings {
    tables: BTreeMap<u8, [u16; 64]>,
    components: Vec<(u8, u8)>, // (sampling factors, quantization table ID)
    progressive: bool,
}

fn invalid() -> String {
    "JPEG 인코딩 설정을 읽을 수 없음".into()
}

impl Settings {
    fn read(bytes: &[u8]) -> Result<Self, String> {
        if !bytes.starts_with(&[0xff, 0xd8]) {
            return Err(invalid());
        }
        let mut result = Self { tables: BTreeMap::new(), components: Vec::new(), progressive: false };
        let mut pos = 2;
        while pos < bytes.len() {
            if bytes.get(pos) != Some(&0xff) {
                return Err(invalid());
            }
            while bytes.get(pos) == Some(&0xff) {
                pos += 1;
            }
            let marker = *bytes.get(pos).ok_or_else(invalid)?;
            pos += 1;
            if marker == 0xda {
                if result.components.is_empty() {
                    return Err(invalid());
                }
                return Ok(result);
            }
            if marker == 0xd9 {
                break;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let size = bytes.get(pos..pos + 2).ok_or_else(invalid)?;
            let len = u16::from_be_bytes([size[0], size[1]]) as usize;
            if len < 2 {
                return Err(invalid());
            }
            let body = bytes.get(pos + 2..pos + len).ok_or_else(invalid)?;
            match marker {
                0xdb => result.read_tables(body)?,
                0xc0..=0xc2 => {
                    let count = *body.get(5).ok_or_else(invalid)? as usize;
                    if body[0] != 8 || ![1, 3].contains(&count) || body.len() != 6 + count * 3 || !result.components.is_empty() {
                        return Err(invalid());
                    }
                    result.components = body[6..].chunks_exact(3).map(|c| (c[1], c[2])).collect();
                    result.progressive = marker == 0xc2;
                }
                _ => {}
            }
            pos += len;
        }
        Err(invalid())
    }

    fn read_tables(&mut self, mut data: &[u8]) -> Result<(), String> {
        while let Some((&header, rest)) = data.split_first() {
            let bytes_per_value = match header >> 4 {
                0 => 1,
                1 => 2,
                _ => return Err(invalid()),
            };
            let count = 64 * bytes_per_value;
            let values = rest.get(..count).ok_or_else(invalid)?;
            let mut table = [0u16; 64];
            for (i, value) in values.chunks_exact(bytes_per_value).enumerate() {
                let value = if bytes_per_value == 1 { u16::from(value[0]) } else { u16::from_be_bytes([value[0], value[1]]) };
                // jpeg-encoder 출력 DQT는 8비트다. 잘라내거나 clamp해서 화질 설정을 바꾸지 않는다.
                if value == 0 || value > 255 {
                    return Err("지원하지 않는 JPEG 양자화 값".into());
                }
                table[ZIGZAG[i]] = value;
            }
            if header & 0x0f > 3 {
                return Err(invalid());
            }
            self.tables.insert(header & 0x0f, table);
            data = &rest[count..];
        }
        Ok(())
    }
}

pub(super) fn encode_crop(original: &[u8], cropped: &DynamicImage) -> Result<Vec<u8>, String> {
    let settings = Settings::read(original)?;
    let first = settings.components[0];
    let gray = settings.components.len() == 1;
    let luma = *settings.tables.get(&first.1).ok_or_else(invalid)?;
    let chroma = if gray {
        settings.tables.get(&1).copied().unwrap_or(luma)
    } else {
        let second = settings.components[1];
        let third = settings.components[2];
        if second.0 != 0x11 || third.0 != 0x11 || settings.tables.get(&second.1) != settings.tables.get(&third.1) {
            return Err("지원하지 않는 JPEG 색상 샘플링".into());
        }
        *settings.tables.get(&second.1).ok_or_else(invalid)?
    };
    let sampling = SamplingFactor::from_factors(first.0 >> 4, first.0 & 0x0f).ok_or_else(invalid)?;
    let width = u16::try_from(cropped.width()).map_err(|_| invalid())?;
    let height = u16::try_from(cropped.height()).map_err(|_| invalid())?;
    let mut output = Vec::new();
    let mut encoder = Encoder::new(&mut output, 100);
    encoder.set_quantization_tables(QuantizationTableType::Custom(Box::new(luma)), QuantizationTableType::Custom(Box::new(chroma)));
    encoder.set_sampling_factor(sampling);
    encoder.set_progressive(settings.progressive);
    if gray { encoder.encode(cropped.to_luma8().as_raw(), width, height, ColorType::Luma) } else { encoder.encode(cropped.to_rgb8().as_raw(), width, height, ColorType::Rgb) }
        .map_err(|e| e.to_string())?;
    Ok(output)
}
