//! Raw buffer <-> [`Array`] codec shared by npy, HDF5, torch storages and joblib.

use super::Array;
use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};
use crate::num::{decode_x87, Complex32, Complex64, BF16, F128, F16, F8E4M3, F8E5M2};
use ndarray::{ArrayD, IxDyn};

macro_rules! decode_prim {
    ($buf:ident, $shape:ident, $swap:ident, $ty:ty, $n:expr, $variant:ident) => {{
        let count: usize = $shape.iter().product();
        let bytes = $buf[..count * $n].chunks_exact($n);
        // Two separate loops so the common, native-order one vectorises.
        let out: Vec<$ty> = if $swap {
            bytes.map(|c| <$ty>::from_be_bytes(c.try_into().unwrap())).collect()
        } else {
            bytes.map(|c| <$ty>::from_le_bytes(c.try_into().unwrap())).collect()
        };
        Array::$variant(
            ArrayD::from_shape_vec(IxDyn($shape), out).map_err(|e| Error::format(e.to_string()))?,
        )
    }};
}

/// Decode a densely packed C-order buffer into an [`Array`].
pub fn from_bytes(dt: &DType, endian: Endian, shape: &[usize], buf: &[u8]) -> Result<Array> {
    let count = shape
        .iter()
        .try_fold(1usize, |count, dim| count.checked_mul(*dim))
        .ok_or_else(|| Error::format("array shape element count overflows usize"))?;
    let isize_ = dt.size().ok_or_else(|| Error::unsupported("object dtype has no fixed size"))?;
    let required = count
        .checked_mul(isize_)
        .ok_or_else(|| Error::format("array byte size overflows usize"))?;
    if buf.len() < required {
        return Err(Error::format(format!("buffer holds {} bytes, need {}", buf.len(), required)));
    }
    let swap = endian.needs_swap();
    Ok(match dt {
        DType::Bool => Array::Bool(
            ArrayD::from_shape_vec(IxDyn(shape), buf[..count].iter().map(|b| *b != 0).collect())
                .map_err(|e| Error::format(e.to_string()))?,
        ),
        DType::I8 => decode_prim!(buf, shape, swap, i8, 1, I8),
        DType::U8 => decode_prim!(buf, shape, swap, u8, 1, U8),
        DType::I16 => decode_prim!(buf, shape, swap, i16, 2, I16),
        DType::U16 => decode_prim!(buf, shape, swap, u16, 2, U16),
        DType::I32 => decode_prim!(buf, shape, swap, i32, 4, I32),
        DType::U32 => decode_prim!(buf, shape, swap, u32, 4, U32),
        DType::I64 => decode_prim!(buf, shape, swap, i64, 8, I64),
        DType::U64 => decode_prim!(buf, shape, swap, u64, 8, U64),
        DType::F32 => decode_prim!(buf, shape, swap, f32, 4, F32),
        DType::F64 => decode_prim!(buf, shape, swap, f64, 8, F64),
        DType::F8E4M3 => Array::F8E4M3(shaped(shape, buf[..count].iter().map(|b| F8E4M3(*b)))?),
        DType::F8E5M2 => Array::F8E5M2(shaped(shape, buf[..count].iter().map(|b| F8E5M2(*b)))?),
        DType::F16 => Array::F16(shaped(shape, (0..count).map(|i| F16(rd16(buf, i, swap))))?),
        DType::BF16 => Array::BF16(shaped(shape, (0..count).map(|i| BF16(rd16(buf, i, swap))))?),
        DType::F128 => Array::F128(shaped(
            shape,
            (0..count).map(|i| {
                let mut b = [0u8; 16];
                b.copy_from_slice(&buf[i * 16..i * 16 + 16]);
                if swap {
                    b.reverse();
                }
                F128(u128::from_le_bytes(b))
            }),
        )?),
        DType::X87 => Array::F128(shaped(shape, (0..count).map(|i| decode_x87(&buf[i * 16..])))?),
        DType::C64 => Array::C64(shaped(
            shape,
            (0..count)
                .map(|i| Complex32::new(rdf32(buf, 2 * i, swap), rdf32(buf, 2 * i + 1, swap))),
        )?),
        DType::C128 => Array::C128(shaped(
            shape,
            (0..count)
                .map(|i| Complex64::new(rdf64(buf, 2 * i, swap), rdf64(buf, 2 * i + 1, swap))),
        )?),
        DType::C256 => Array::C128(shaped(
            shape,
            (0..count).map(|i| {
                Complex64::new(
                    decode_x87(&buf[i * 32..]).to_f64(),
                    decode_x87(&buf[i * 32 + 16..]).to_f64(),
                )
            }),
        )?),
        DType::Str(n) => Array::Str(shaped(
            shape,
            (0..count).map(|i| {
                let base = i * 4 * n;
                let mut s = String::with_capacity(*n);
                for k in 0..*n {
                    let mut b = [0u8; 4];
                    b.copy_from_slice(&buf[base + k * 4..base + k * 4 + 4]);
                    if swap {
                        b.reverse();
                    }
                    let cp = u32::from_le_bytes(b);
                    if cp == 0 {
                        break;
                    }
                    s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                }
                s
            }),
        )?),
        DType::Bytes(n) => Array::Bytes(shaped(
            shape,
            (0..count).map(|i| {
                let sl = &buf[i * n..i * n + n];
                let end = sl.iter().rposition(|b| *b != 0).map(|p| p + 1).unwrap_or(0);
                sl[..end].to_vec()
            }),
        )?),
        DType::Object | DType::Struct(_) => {
            return Err(Error::unsupported("object/structured dtype in raw buffer decode"))
        }
    })
}

fn shaped<T, I: Iterator<Item = T>>(shape: &[usize], it: I) -> Result<ArrayD<T>> {
    ArrayD::from_shape_vec(IxDyn(shape), it.collect()).map_err(|e| Error::format(e.to_string()))
}

fn rd16(buf: &[u8], i: usize, swap: bool) -> u16 {
    let mut b = [buf[i * 2], buf[i * 2 + 1]];
    if swap {
        b.reverse();
    }
    u16::from_le_bytes(b)
}
fn rdf32(buf: &[u8], i: usize, swap: bool) -> f32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&buf[i * 4..i * 4 + 4]);
    if swap {
        b.reverse();
    }
    f32::from_le_bytes(b)
}
fn rdf64(buf: &[u8], i: usize, swap: bool) -> f64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[i * 8..i * 8 + 8]);
    if swap {
        b.reverse();
    }
    f64::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opposite_endian() -> Endian {
        if cfg!(target_endian = "little") {
            Endian::Big
        } else {
            Endian::Little
        }
    }

    #[test]
    fn decodes_boolean_integer_and_float_variants() {
        assert_eq!(from_bytes(&DType::Bool, Endian::Native, &[3], &[0, 1, 2]).unwrap().len(), 3);
        for dt in [DType::I8, DType::U8, DType::F8E4M3, DType::F8E5M2] {
            assert_eq!(from_bytes(&dt, Endian::Native, &[1], &[1]).unwrap().shape(), &[1]);
        }
        for dt in [DType::I16, DType::U16, DType::F16, DType::BF16] {
            assert_eq!(from_bytes(&dt, Endian::Native, &[1], &[0, 0x3c]).unwrap().len(), 1);
            assert_eq!(from_bytes(&dt, opposite_endian(), &[1], &[0x3c, 0]).unwrap().len(), 1);
        }
        for dt in [DType::I32, DType::U32, DType::F32] {
            let a = from_bytes(&dt, Endian::Native, &[1], &[0, 0, 0x80, 0x3f]).unwrap();
            assert_eq!(a.len(), 1);
            assert_eq!(
                from_bytes(&dt, opposite_endian(), &[1], &[0x3f, 0x80, 0, 0]).unwrap().len(),
                1
            );
        }
        for dt in [DType::I64, DType::U64, DType::F64] {
            let mut le = vec![0; 8];
            le[0] = 1;
            assert_eq!(from_bytes(&dt, Endian::Native, &[1], &le).unwrap().len(), 1);
            le.reverse();
            assert_eq!(from_bytes(&dt, opposite_endian(), &[1], &le).unwrap().len(), 1);
        }
    }

    #[test]
    fn decodes_wide_and_complex_numeric_variants() {
        let mut wide = vec![0; 16];
        wide[0] = 1;
        assert!(matches!(
            from_bytes(&DType::F128, Endian::Native, &[1], &wide).unwrap(),
            Array::F128(_)
        ));
        wide.reverse();
        assert!(matches!(
            from_bytes(&DType::F128, opposite_endian(), &[1], &wide).unwrap(),
            Array::F128(_)
        ));
        assert!(matches!(
            from_bytes(&DType::X87, Endian::Native, &[1], &[0; 16]).unwrap(),
            Array::F128(_)
        ));
        assert!(matches!(
            from_bytes(&DType::C64, Endian::Native, &[1], &[0; 8]).unwrap(),
            Array::C64(_)
        ));
        assert!(matches!(
            from_bytes(&DType::C64, opposite_endian(), &[1], &[0; 8]).unwrap(),
            Array::C64(_)
        ));
        assert!(matches!(
            from_bytes(&DType::C128, Endian::Native, &[1], &[0; 16]).unwrap(),
            Array::C128(_)
        ));
        assert!(matches!(
            from_bytes(&DType::C128, opposite_endian(), &[1], &[0; 16]).unwrap(),
            Array::C128(_)
        ));
        assert!(matches!(
            from_bytes(&DType::C256, Endian::Native, &[1], &[0; 32]).unwrap(),
            Array::C128(_)
        ));
    }

    #[test]
    fn decodes_fixed_width_strings_and_replaces_invalid_unicode() {
        let mut buf = Vec::new();
        for ch in ['A' as u32, 0, 0x11_0000] {
            buf.extend_from_slice(&ch.to_le_bytes());
        }
        let values = from_bytes(&DType::Str(3), Endian::Native, &[1], &buf).unwrap();
        assert_eq!(values, Array::Str(ndarray::arr1(&["A".to_owned()]).into_dyn()));
        let mut be = Vec::new();
        for ch in ['Z' as u32, 0] {
            be.extend_from_slice(&ch.to_be_bytes());
        }
        assert_eq!(
            from_bytes(&DType::Str(2), opposite_endian(), &[1], &be).unwrap(),
            Array::Str(ndarray::arr1(&["Z".to_owned()]).into_dyn())
        );
        assert_eq!(
            from_bytes(&DType::Str(1), Endian::Native, &[1], &[0, 0, 0x11, 0]).unwrap(),
            Array::Str(ndarray::arr1(&["�".to_owned()]).into_dyn())
        );
    }

    #[test]
    fn decodes_bytes_and_empty_arrays() {
        let decoded =
            from_bytes(&DType::Bytes(4), Endian::Native, &[2], b"a\0\0\0\0\0\0\0").unwrap();
        assert_eq!(decoded, Array::Bytes(ndarray::arr1(&[b"a".to_vec(), Vec::new()]).into_dyn()));
        assert_eq!(from_bytes(&DType::Bytes(0), Endian::Native, &[0], &[]).unwrap().len(), 0);
    }

    #[test]
    fn rejects_objects_structures_short_buffers_and_invalid_shapes() {
        assert!(from_bytes(&DType::Object, Endian::Native, &[1], &[]).is_err());
        assert!(from_bytes(&DType::Struct(Vec::new()), Endian::Native, &[1], &[]).is_err());
        assert!(from_bytes(&DType::I32, Endian::Native, &[2], &[0, 0, 0, 0]).is_err());
        assert!(matches!(
            from_bytes(&DType::I8, Endian::Native, &[2], &[1, 2]).unwrap(),
            Array::I8(_)
        ));
        let overflowing_shape = [usize::MAX, 2, 0];
        assert!(from_bytes(&DType::Bool, Endian::Native, &overflowing_shape, &[]).is_err());
        assert!(from_bytes(&DType::I8, Endian::Native, &overflowing_shape, &[]).is_err());
    }
}
