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

#[cfg(test)]
mod tests {
    use super::Settings;

    fn marker(id: u8, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xff, id];
        bytes.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        bytes.extend_from_slice(body);
        bytes
    }

    fn frame(marker_id: u8, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        bytes.extend(marker(marker_id, body));
        bytes.extend(marker(0xda, &[]));
        bytes
    }

    fn valid_frame() -> Vec<u8> {
        let table = [std::iter::once(0).chain(std::iter::repeat_n(10, 64)).collect::<Vec<_>>()];
        let mut bytes = vec![0xff, 0xd8];
        bytes.extend(marker(0xdb, &table[0]));
        bytes.extend(marker(0xc0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]));
        bytes.extend(marker(0xda, &[]));
        bytes
    }

    #[test]
    fn settings_reader_rejects_malformed_marker_streams() {
        for bytes in [
            b"not jpeg".as_slice(),
            &[0xff, 0xd8, 0xff],
            &[0xff, 0xd8, 0x00],
            &[0xff, 0xd8, 0xff, 0xdb, 0x00],
            &[0xff, 0xd8, 0xff, 0xdb, 0x00, 0x01],
            &[0xff, 0xd8, 0xff, 0xdb, 0x00, 0x03, 0x00],
            &[0xff, 0xd8, 0xff, 0xd9],
            &[0xff, 0xd8, 0xff, 0xda, 0x00, 0x02],
        ] {
            assert!(Settings::read(bytes).is_err(), "accepted {bytes:?}");
        }
    }

    #[test]
    fn settings_reader_validates_quantization_tables() {
        for table in [
            vec![0x20],
            vec![0x00],
            std::iter::once(0).chain(std::iter::repeat_n(0, 64)).collect(),
            std::iter::once(0x10).chain(std::iter::repeat_n(0, 128)).collect(),
            std::iter::once(0x10).chain([1, 0]).chain(std::iter::repeat_n(0, 126)).collect(),
            std::iter::once(0x04).chain(std::iter::repeat_n(10, 64)).collect(),
        ] {
            let bytes = frame(0xdb, &table);
            assert!(Settings::read(&bytes).is_err(), "accepted DQT {table:?}");
        }
    }

    #[test]
    fn settings_reader_validates_frame_shape_and_table_presence() {
        for bytes in [frame(0xc0, &[12, 0, 1, 0, 1, 1, 1, 0x11, 0]), frame(0xc0, &[8, 0, 1, 0, 1, 2, 1, 0x11, 0, 2, 0x11, 0]), frame(0xc0, &[8, 0, 1, 0, 1, 1, 1])] {
            assert!(Settings::read(&bytes).is_err());
        }

        let mut duplicate_frame = valid_frame();
        let second_sof = marker(0xc0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
        let sos = duplicate_frame.split_off(duplicate_frame.len() - 4);
        duplicate_frame.extend(second_sof);
        duplicate_frame.extend(sos);
        assert!(Settings::read(&duplicate_frame).is_err());

        let missing_table = frame(0xc0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 2]);
        assert!(Settings::read(&missing_table).is_ok());
    }

    #[test]
    fn crop_encoder_rejects_missing_tables_unsupported_sampling_and_large_dimensions() {
        let cropped = image::DynamicImage::new_luma8(1, 1);
        let missing_table = frame(0xc0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 2]);
        assert!(super::encode_crop(&missing_table, &cropped).is_err());

        let mut color = vec![0xff, 0xd8];
        let table = std::iter::once(0).chain(std::iter::repeat_n(10, 64)).collect::<Vec<_>>();
        color.extend(marker(0xdb, &table));
        color.extend(marker(0xc0, &[8, 0, 1, 0, 1, 3, 1, 0x11, 0, 2, 0x22, 0, 3, 0x11, 0]));
        color.extend(marker(0xda, &[]));
        let color_crop = image::DynamicImage::new_rgb8(1, 1);
        assert!(super::encode_crop(&color, &color_crop).is_err());

        let invalid_sampling = frame(0xc0, &[8, 0, 1, 0, 1, 1, 1, 0x00, 0]);
        let with_table = valid_frame();
        let mut invalid_sampling = invalid_sampling;
        invalid_sampling.splice(2..2, marker(0xdb, &table));
        assert!(super::encode_crop(&invalid_sampling, &cropped).is_err());

        let too_wide = image::DynamicImage::new_luma8(65_536, 1);
        assert!(super::encode_crop(&with_table, &too_wide).is_err());
    }
}
