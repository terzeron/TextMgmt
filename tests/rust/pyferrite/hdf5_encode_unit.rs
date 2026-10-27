use super::*;

#[test]
fn image_writes_little_endian_values_at_absolute_offsets() {
    let mut image = Image::new(32);
    image.u8_at(0, 0xA5);
    image.u16_at(1, 0x1234);
    image.u32_at(3, 0x12345678);
    image.u64_at(7, 0x0102030405060708);
    image.put(15, b"ok");
    assert_eq!(
        &image.buf[..17],
        &[0xA5, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 8, 7, 6, 5, 4, 3, 2, 1, b'o', b'k']
    );
}

#[test]
fn datatype_encodes_integer_float_and_string_families() {
    for dtype in [
        DType::Bool,
        DType::I8,
        DType::I16,
        DType::I32,
        DType::I64,
        DType::U8,
        DType::U16,
        DType::U32,
        DType::U64,
    ] {
        let encoded = datatype(&dtype).unwrap();
        assert_eq!(encoded[0] & 0x0F, 0);
        assert_eq!(
            u32::from_le_bytes(encoded[4..8].try_into().unwrap()) as usize,
            dtype.size().unwrap()
        );
    }
    for dtype in [DType::F16, DType::F32, DType::F64] {
        assert_eq!(datatype(&dtype).unwrap()[0] & 0x0F, 1);
    }
    for dtype in [DType::Bytes(4), DType::Str(2)] {
        assert_eq!(datatype(&dtype).unwrap()[0] & 0x0F, 3);
    }
    assert_eq!(UNDEF, u64::MAX);
    assert!(datatype(&DType::Object).is_err());
    assert!(datatype(&DType::C64).is_err());
    assert!(datatype(&DType::Struct(Vec::new())).is_err());
}

#[test]
fn dataspace_fill_layout_and_symbol_table_encode_expected_fields() {
    let shape = dataspace(&[2, 3]);
    assert_eq!(&shape[..8], &[1, 2, 0, 0, 0, 0, 0, 0]);
    assert_eq!(u64::from_le_bytes(shape[8..16].try_into().unwrap()), 2);
    assert_eq!(u64::from_le_bytes(shape[16..24].try_into().unwrap()), 3);

    assert_eq!(fill_value(3), [3, 0x22, 3, 0, 0, 0, 0, 0, 0]);
    let layout = layout_contiguous(0x1234, 0x5678);
    assert_eq!(&layout[..2], &[3, 1]);
    assert_eq!(u64::from_le_bytes(layout[2..10].try_into().unwrap()), 0x1234);
    assert_eq!(u64::from_le_bytes(layout[10..18].try_into().unwrap()), 0x5678);
    assert_eq!(symbol_table(7, 9), [7, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn object_header_writes_message_headers_payloads_and_alignment() {
    let mut image = Image::new(64);
    let consumed = object_header(&mut image, 8, &[(3, b"x".to_vec()), (4, b"12345678".to_vec())]);
    assert_eq!(consumed, 48);
    assert_eq!(image.buf[8], 1);
    assert_eq!(u16::from_le_bytes(image.buf[10..12].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(image.buf[16..20].try_into().unwrap()), 32);
    assert_eq!(u16::from_le_bytes(image.buf[24..26].try_into().unwrap()), 3);
    assert_eq!(u16::from_le_bytes(image.buf[26..28].try_into().unwrap()), 8);
    assert_eq!(image.buf[32], b'x');
    assert_eq!(u16::from_le_bytes(image.buf[40..42].try_into().unwrap()), 4);
    assert_eq!(&image.buf[48..56], b"12345678");
}
