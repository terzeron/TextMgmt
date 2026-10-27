use super::*;

#[test]
fn endian_helpers_and_descriptor_prefixes_follow_numpy_rules() {
    assert_eq!(Endian::Little.as_char(), '<');
    assert_eq!(Endian::Big.as_char(), '>');
    assert_eq!(Endian::Native.as_char(), '|');
    assert_eq!(Endian::Little.needs_swap(), cfg!(target_endian = "big"));
    assert_eq!(Endian::Big.needs_swap(), cfg!(target_endian = "little"));
    assert!(!Endian::Native.needs_swap());

    assert_eq!(parse_descr(" <i2 ").unwrap(), (DType::I16, Endian::Little));
    assert_eq!(parse_descr(">u4").unwrap(), (DType::U32, Endian::Big));
    assert_eq!(parse_descr("|S3").unwrap(), (DType::Bytes(3), Endian::Native));
    assert_eq!(
        parse_descr("=f8").unwrap().1,
        if cfg!(target_endian = "little") { Endian::Little } else { Endian::Big }
    );
    assert_eq!(parse_descr("f4").unwrap(), (DType::F32, Endian::Native));
}

#[test]
fn descriptor_parser_covers_supported_scalar_kinds_and_aliases() {
    for (text, dtype) in [
        ("b1", DType::Bool),
        ("?", DType::Bool),
        ("i1", DType::I8),
        ("i2", DType::I16),
        ("i4", DType::I32),
        ("i8", DType::I64),
        ("u1", DType::U8),
        ("u2", DType::U16),
        ("u4", DType::U32),
        ("u8", DType::U64),
        ("f2", DType::F16),
        ("f4", DType::F32),
        ("f8", DType::F64),
        ("f12", DType::X87),
        ("f16", DType::X87),
        ("g", DType::X87),
        ("c8", DType::C64),
        ("c16", DType::C128),
        ("c24", DType::C256),
        ("c32", DType::C256),
        ("U4", DType::Str(4)),
        ("S5", DType::Bytes(5)),
        ("a6", DType::Bytes(6)),
        ("V7", DType::Bytes(7)),
        ("O", DType::Object),
        ("M8", DType::I64),
        ("m8", DType::I64),
    ] {
        assert_eq!(parse_descr(text).unwrap().0, dtype, "{text}");
    }
    assert!(parse_descr("").is_err());
    assert!(parse_descr(">").is_err());
    assert!(parse_descr("i3").is_err());
    assert!(parse_descr("x1").is_err());
}

#[test]
fn descriptor_renderer_covers_all_scalar_types_and_rejects_structures() {
    for (dtype, expected) in [
        (DType::Bool, "|b1"),
        (DType::I8, "|i1"),
        (DType::U8, "|u1"),
        (DType::F8E4M3, "|u1"),
        (DType::F8E5M2, "|u1"),
        (DType::I16, ">i2"),
        (DType::I32, ">i4"),
        (DType::I64, ">i8"),
        (DType::U16, ">u2"),
        (DType::U32, ">u4"),
        (DType::U64, ">u8"),
        (DType::F16, ">f2"),
        (DType::BF16, ">f2"),
        (DType::F32, ">f4"),
        (DType::F64, ">f8"),
        (DType::F128, ">f16"),
        (DType::X87, ">f16"),
        (DType::C64, ">c8"),
        (DType::C128, ">c16"),
        (DType::C256, ">c32"),
        (DType::Str(3), ">U3"),
        (DType::Bytes(5), "|S5"),
        (DType::Object, "|O"),
    ] {
        assert_eq!(to_descr(&dtype, Endian::Big).unwrap(), expected, "{dtype:?}");
    }
    assert!(to_descr(&DType::Struct(Vec::new()), Endian::Little).is_err());
}

#[test]
fn struct_dtype_builder_preserves_fields_and_propagates_parse_errors() {
    let dtype = struct_from_fields(vec![
        ("x".into(), "<i4".into(), vec![2, 3]),
        ("name".into(), "|S8".into(), vec![]),
    ])
    .unwrap();
    assert_eq!(
        dtype,
        DType::Struct(vec![
            crate::dtype::Field { name: "x".into(), dtype: DType::I32, shape: vec![2, 3] },
            crate::dtype::Field { name: "name".into(), dtype: DType::Bytes(8), shape: vec![] },
        ])
    );
    assert_eq!(struct_from_fields(vec![]).unwrap(), DType::Struct(vec![]));
    assert!(struct_from_fields(vec![("bad".into(), "i3".into(), vec![])]).is_err());
}
