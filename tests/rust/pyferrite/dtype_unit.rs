use super::*;

#[test]
fn element_sizes_cover_every_scalar_family_and_variable_objects() {
    for dtype in [DType::Bool, DType::I8, DType::U8, DType::F8E4M3, DType::F8E5M2] {
        assert_eq!(dtype.size(), Some(1), "{dtype:?}");
    }
    for dtype in [DType::I16, DType::U16, DType::F16, DType::BF16] {
        assert_eq!(dtype.size(), Some(2), "{dtype:?}");
    }
    for dtype in [DType::I32, DType::U32, DType::F32] {
        assert_eq!(dtype.size(), Some(4), "{dtype:?}");
    }
    for dtype in [DType::I64, DType::U64, DType::F64, DType::C64] {
        assert_eq!(dtype.size(), Some(8), "{dtype:?}");
    }
    for dtype in [DType::F128, DType::X87, DType::C128] {
        assert_eq!(dtype.size(), Some(16), "{dtype:?}");
    }
    assert_eq!(DType::C256.size(), Some(32));
    assert_eq!(DType::Str(3).size(), Some(12));
    assert_eq!(DType::Bytes(7).size(), Some(7));
    assert_eq!(DType::Object.size(), None);
}

#[test]
fn structured_sizes_respect_field_shapes_and_variable_field_types() {
    let record = DType::Struct(vec![
        Field { name: "matrix".into(), dtype: DType::I16, shape: vec![2, 3] },
        Field { name: "scalar".into(), dtype: DType::F32, shape: vec![] },
        Field { name: "empty".into(), dtype: DType::U64, shape: vec![4, 0] },
    ]);
    assert_eq!(record.size(), Some(24));
    assert_eq!(
        DType::Struct(vec![Field { name: "object".into(), dtype: DType::Object, shape: vec![] }])
            .size(),
        None
    );
}

#[test]
fn numeric_classifiers_include_only_their_documented_families() {
    for dtype in [
        DType::I8,
        DType::I16,
        DType::I32,
        DType::I64,
        DType::U8,
        DType::U16,
        DType::U32,
        DType::U64,
    ] {
        assert!(dtype.is_integer());
        assert!(dtype.is_numeric());
    }
    for dtype in [
        DType::F8E4M3,
        DType::F8E5M2,
        DType::F16,
        DType::BF16,
        DType::F32,
        DType::F64,
        DType::F128,
        DType::X87,
    ] {
        assert!(dtype.is_float());
        assert!(dtype.is_numeric());
        assert!(!dtype.is_integer());
    }
    for dtype in [DType::C64, DType::C128, DType::C256] {
        assert!(dtype.is_complex());
        assert!(dtype.is_numeric());
        assert!(!dtype.is_float());
    }
    assert!(DType::Bool.is_numeric());
    for dtype in [DType::Str(1), DType::Bytes(1), DType::Object, DType::Struct(vec![])] {
        assert!(!dtype.is_numeric());
        assert!(!dtype.is_integer());
        assert!(!dtype.is_float());
        assert!(!dtype.is_complex());
    }
}
