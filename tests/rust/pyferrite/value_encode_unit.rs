use super::*;
use crate::num::{Complex32, Complex64, BF16, F128, F16, F8E4M3, F8E5M2};
use ndarray::arr1;

#[test]
fn to_bytes_encodes_scalar_integer_and_float_variants() {
    assert_eq!(
        to_bytes(&Array::Bool(arr1(&[true, false]).into_dyn()), Endian::Little).unwrap(),
        [1, 0]
    );
    assert_eq!(to_bytes(&Array::I8(arr1(&[-1, 2]).into_dyn()), Endian::Little).unwrap(), [255, 2]);
    assert_eq!(to_bytes(&Array::U8(arr1(&[1, 255]).into_dyn()), Endian::Big).unwrap(), [1, 255]);
    assert_eq!(
        to_bytes(&Array::F8E4M3(arr1(&[F8E4M3(0x38)]).into_dyn()), Endian::Big).unwrap(),
        [0x38]
    );
    assert_eq!(
        to_bytes(&Array::F8E5M2(arr1(&[F8E5M2(0x3C)]).into_dyn()), Endian::Big).unwrap(),
        [0x3C]
    );

    assert_eq!(
        to_bytes(&Array::I16(arr1(&[0x1234]).into_dyn()), Endian::Big).unwrap(),
        [0x12, 0x34]
    );
    assert_eq!(
        to_bytes(&Array::U16(arr1(&[0x1234]).into_dyn()), Endian::Little).unwrap(),
        [0x34, 0x12]
    );
    assert_eq!(
        to_bytes(&Array::I32(arr1(&[0x12345678]).into_dyn()), Endian::Little).unwrap(),
        [0x78, 0x56, 0x34, 0x12]
    );
    assert_eq!(
        to_bytes(&Array::U32(arr1(&[0x12345678]).into_dyn()), Endian::Big).unwrap(),
        [0x12, 0x34, 0x56, 0x78]
    );
    assert_eq!(
        to_bytes(&Array::I64(arr1(&[0x0102030405060708]).into_dyn()), Endian::Little).unwrap(),
        [8, 7, 6, 5, 4, 3, 2, 1]
    );
    assert_eq!(
        to_bytes(&Array::U64(arr1(&[0x0102030405060708]).into_dyn()), Endian::Big).unwrap(),
        [1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(
        to_bytes(&Array::F16(arr1(&[F16(0x1234)]).into_dyn()), Endian::Big).unwrap(),
        [0x12, 0x34]
    );
    assert_eq!(
        to_bytes(&Array::BF16(arr1(&[BF16(0x1234)]).into_dyn()), Endian::Little).unwrap(),
        [0x34, 0x12]
    );
    assert_eq!(
        to_bytes(&Array::F32(arr1(&[1.0]).into_dyn()), Endian::Little).unwrap(),
        1.0f32.to_le_bytes()
    );
    assert_eq!(
        to_bytes(&Array::F64(arr1(&[1.0]).into_dyn()), Endian::Little).unwrap(),
        1.0f64.to_le_bytes()
    );
    assert_eq!(
        to_bytes(&Array::F128(arr1(&[F128::ONE]).into_dyn()), Endian::Little).unwrap(),
        F128::ONE.0.to_le_bytes()
    );
}

#[test]
fn to_bytes_encodes_complex_and_padded_string_arrays() {
    let c32 = Array::C64(arr1(&[Complex32::new(1.0, -2.0)]).into_dyn());
    let mut expected32 = 1.0f32.to_le_bytes().to_vec();
    expected32.extend((-2.0f32).to_le_bytes());
    assert_eq!(to_bytes(&c32, Endian::Little).unwrap(), expected32);
    let mut expected32_be = 1.0f32.to_be_bytes().to_vec();
    expected32_be.extend((-2.0f32).to_be_bytes());
    assert_eq!(to_bytes(&c32, Endian::Big).unwrap(), expected32_be);

    let c64 = Array::C128(arr1(&[Complex64::new(3.0, 4.0)]).into_dyn());
    let mut expected64 = 3.0f64.to_le_bytes().to_vec();
    expected64.extend(4.0f64.to_le_bytes());
    assert_eq!(to_bytes(&c64, Endian::Little).unwrap(), expected64);

    let strings = Array::Str(arr1(&["éa".to_owned(), "b".to_owned()]).into_dyn());
    assert_eq!(
        to_bytes(&strings, Endian::Little).unwrap(),
        [0xE9, 0, 0, 0, b'a', 0, 0, 0, b'b', 0, 0, 0, 0, 0, 0, 0]
    );
    let bytes = Array::Bytes(arr1(&[b"ab".to_vec(), b"x".to_vec()]).into_dyn());
    assert_eq!(to_bytes(&bytes, Endian::Little).unwrap(), b"abx\0");

    let strided = Array::I16(
        ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&[2, 2]), vec![1, 2, 3, 4])
            .unwrap()
            .reversed_axes(),
    );
    assert_eq!(to_bytes(&strided, Endian::Little).unwrap(), [1, 0, 3, 0, 2, 0, 4, 0]);
}

#[test]
fn x87_encoder_accepts_quad_and_casts_other_numeric_arrays() {
    let quad = Array::F128(arr1(&[F128::ONE]).into_dyn());
    let direct = to_bytes_x87(&quad).unwrap();
    assert_eq!(direct.len(), 16);
    assert_eq!(&direct[10..], &[0; 6]);

    let single = Array::F32(arr1(&[1.0]).into_dyn());
    let converted = to_bytes_x87(&single).unwrap();
    assert_eq!(converted.len(), 16);
    assert_eq!(&converted[10..], &[0; 6]);
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("closed"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn streaming_writer_uses_block_and_fallback_paths_and_propagates_io_errors() {
    let values = vec![0x1234u16; BLOCK + 1];
    let array = Array::U16(ndarray::Array1::from(values).into_dyn());
    let expected = to_bytes(&array, Endian::Big).unwrap();
    let mut out = Vec::new();
    write_bytes_to(&array, Endian::Big, &mut out).unwrap();
    assert_eq!(out, expected);

    for array in [
        Array::I16(arr1(&[0x1234i16]).into_dyn()),
        Array::I32(arr1(&[0x12345678i32]).into_dyn()),
        Array::U32(arr1(&[0x12345678u32]).into_dyn()),
        Array::I64(arr1(&[0x0102030405060708i64]).into_dyn()),
        Array::U64(arr1(&[0x0102030405060708u64]).into_dyn()),
        Array::F32(arr1(&[1.25f32]).into_dyn()),
        Array::F64(arr1(&[1.25f64]).into_dyn()),
    ] {
        let mut streamed = Vec::new();
        write_bytes_to(&array, Endian::Big, &mut streamed).unwrap();
        assert_eq!(streamed, to_bytes(&array, Endian::Big).unwrap());
    }

    let bytes = Array::U8(arr1(&[1, 2, 3]).into_dyn());
    out.clear();
    write_bytes_to(&bytes, Endian::Little, &mut out).unwrap();
    assert_eq!(out, [1, 2, 3]);

    let fallback = Array::F16(arr1(&[F16(0x1234)]).into_dyn());
    out.clear();
    write_bytes_to(&fallback, Endian::Little, &mut out).unwrap();
    assert_eq!(out, to_bytes(&fallback, Endian::Little).unwrap());

    let strided_u8 = Array::U8(
        ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&[2, 2]), vec![1, 2, 3, 4])
            .unwrap()
            .reversed_axes(),
    );
    out.clear();
    write_bytes_to(&strided_u8, Endian::Little, &mut out).unwrap();
    assert_eq!(out, [1, 3, 2, 4]);
    assert!(write_bytes_to(&bytes, Endian::Little, &mut FailingWriter).is_err());
}
