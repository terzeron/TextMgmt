use super::*;
use crate::dtype::IntKind;
use crate::num::{BF16, F128, F16};
use crate::value::{Dict, PyObject};

fn structured_dtype(
    names: Value,
    field: Option<Value>,
    itemsize: Option<i64>,
) -> Result<(DType, Endian)> {
    let mut fields = Dict::new();
    if let Some(field) = field {
        fields.insert(Value::Str("id".into()), field);
    }
    struct_dtype(&names, &fields, itemsize)
}

#[test]
fn dtype_rejects_missing_names_and_non_dtype_values() {
    let mut unnamed = PyObject::new(TAG, "dtype");
    assert!(matches!(as_dtype(&Value::Object(unnamed.clone())), Err(Error::Pickle(_))));

    unnamed.args.push(Value::Int(4));
    assert!(matches!(as_dtype(&Value::Object(unnamed)), Err(Error::Pickle(_))));
    assert!(matches!(as_dtype(&Value::Int(4)), Err(Error::Pickle(_))));

    let mut fields = Dict::new();
    fields.insert(
        Value::Str("id".into()),
        Value::Tuple(vec![Value::Str("<i4".into()), Value::Int(0)]),
    );
    let mut primitive = PyObject::new(TAG, "dtype");
    primitive.args.push(Value::Str("i4".into()));
    primitive.state = Some(Box::new(Value::Tuple(vec![
        Value::Int(3),
        Value::None,
        Value::None,
        Value::Tuple(vec![Value::Str("id".into())]),
        Value::Dict(fields),
        Value::Int(4),
    ])));
    assert_eq!(as_dtype(&Value::Object(primitive)).unwrap().0, DType::I32);

    let mut non_tuple_state = PyObject::new(TAG, "dtype");
    non_tuple_state.args.push(Value::Str("i4".into()));
    non_tuple_state.state = Some(Box::new(Value::Int(1)));
    assert_eq!(as_dtype(&Value::Object(non_tuple_state)).unwrap().0, DType::I32);

    let mut incomplete_state = PyObject::new(TAG, "dtype");
    incomplete_state.args.push(Value::Str("i4".into()));
    incomplete_state.state = Some(Box::new(Value::Tuple(vec![
        Value::None,
        Value::None,
        Value::None,
        Value::Int(1),
        Value::Int(2),
    ])));
    assert_eq!(as_dtype(&Value::Object(incomplete_state)).unwrap().0, DType::I32);
}

#[test]
fn dtype_reads_structured_fields_from_build_state() {
    let mut fields = Dict::new();
    fields.insert(
        Value::Str("id".into()),
        Value::Tuple(vec![Value::Str("<i4".into()), Value::Int(0)]),
    );
    let mut dtype = PyObject::new(TAG, "dtype");
    dtype.args.push(Value::Str("V4".into()));
    dtype.state = Some(Box::new(Value::Tuple(vec![
        Value::Int(3),
        Value::None,
        Value::None,
        Value::Tuple(vec![Value::Str("id".into())]),
        Value::Dict(fields),
        Value::Int(4),
    ])));

    let (decoded, _) = as_dtype(&Value::Object(dtype)).unwrap();

    assert!(
        matches!(decoded, DType::Struct(fields) if fields[0].name == "id" && fields[0].dtype == DType::I32)
    );
}

#[test]
fn structured_dtype_rejects_malformed_names_fields_and_padding() {
    assert!(matches!(structured_dtype(Value::Str("id".into()), None, None), Err(Error::Pickle(_))));
    assert!(matches!(
        structured_dtype(Value::Tuple(vec![Value::Int(1)]), None, None),
        Err(Error::Pickle(_))
    ));
    assert!(matches!(
        structured_dtype(Value::Tuple(vec![Value::Str("id".into())]), None, None),
        Err(Error::Pickle(_))
    ));
    assert!(matches!(
        structured_dtype(
            Value::Tuple(vec![Value::Str("id".into())]),
            Some(Value::Tuple(vec![Value::Str("<i4".into())])),
            None
        ),
        Err(Error::Pickle(_))
    ));
    assert!(matches!(
        structured_dtype(
            Value::Tuple(vec![Value::Str("id".into())]),
            Some(Value::Tuple(vec![Value::Str("<i4".into()), Value::Int(1)])),
            None
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        structured_dtype(
            Value::Tuple(vec![Value::Str("id".into())]),
            Some(Value::Tuple(vec![Value::Str("|O".into()), Value::Int(0)])),
            None
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        structured_dtype(
            Value::Tuple(vec![Value::Str("id".into())]),
            Some(Value::Tuple(vec![Value::Str("<i4".into()), Value::Int(0)])),
            Some(8)
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        structured_dtype(
            Value::Tuple(vec![Value::Str("id".into())]),
            Some(Value::Tuple(vec![Value::Str("<i4".into()), Value::Int(0)])),
            Some(4)
        ),
        Ok((DType::Struct(_), _))
    ));
    assert!(matches!(
        structured_dtype(
            Value::List(vec![Value::Str("id".into())]),
            Some(Value::Tuple(vec![Value::Str("<i4".into()), Value::Int(0)])),
            None
        ),
        Ok((DType::Struct(_), _))
    ));
    assert!(matches!(
        structured_dtype(
            Value::Tuple(vec![Value::Str("id".into())]),
            Some(Value::Tuple(vec![Value::Int(4), Value::Int(0)])),
            None
        ),
        Err(Error::Pickle(_))
    ));
    assert!(matches!(
        structured_dtype(
            Value::Tuple(vec![Value::Str("id".into())]),
            Some(Value::Tuple(vec![Value::Str("<i4".into()), Value::Int(0)])),
            None
        ),
        Ok((DType::Struct(_), _))
    ));
}

#[test]
fn scalar_validates_arguments_and_accepts_string_bytes() {
    let opts = ReadOptions::new();
    assert!(matches!(scalar(&[], &opts), Err(Error::Pickle(_))));
    assert!(matches!(scalar(&[Value::Str("<i4".into())], &opts), Err(Error::Pickle(_))));
    assert!(matches!(
        scalar(&[Value::Str("<i4".into()), Value::Int(4)], &opts),
        Err(Error::Pickle(_))
    ));
    assert!(scalar(&[Value::Str("<i4".into()), Value::Bytes(vec![4])], &opts).is_err());
    assert_eq!(
        scalar(&[Value::Str("<i4".into()), Value::Str("\u{4}\0\0\0".into())], &opts).unwrap(),
        Value::Int(4)
    );
    assert_eq!(
        scalar(&[Value::Str("<i4".into()), Value::Bytes(4i32.to_le_bytes().to_vec())], &opts)
            .unwrap(),
        Value::Int(4)
    );
}

#[test]
fn scalar_conversion_covers_bfloat_quad_complex_and_fallback_values() {
    use crate::num::{Complex32, Complex64};

    let empty = ndarray::ArrayD::from_shape_vec(vec![0], Vec::<Vec<u8>>::new()).unwrap();
    let bf16 = ndarray::arr0(BF16::from_f32(1.5)).into_dyn();
    let f128 = ndarray::arr0(F128::from_f64(2.5)).into_dyn();
    let c64 = ndarray::arr0(Complex32::new(1.0, -2.0)).into_dyn();
    let c128 = ndarray::arr0(Complex64::new(3.0, -4.0)).into_dyn();

    assert_eq!(array_first_as_value(&Array::BF16(bf16)), Value::Float(1.5));
    assert_eq!(array_first_as_value(&Array::F128(f128)), Value::Float(2.5));
    assert_eq!(array_first_as_value(&Array::C64(c64)), Value::Complex(Complex64::new(1.0, -2.0)));
    assert_eq!(array_first_as_value(&Array::C128(c128)), Value::Complex(Complex64::new(3.0, -4.0)));
    assert_eq!(array_first_as_value(&Array::Bytes(empty)), Value::None);
}

#[test]
fn scalar_conversion_covers_remaining_array_variants() {
    assert_eq!(
        array_first_as_value(&Array::Bool(ndarray::arr0(true).into_dyn())),
        Value::Bool(true)
    );
    assert_eq!(array_first_as_value(&Array::I8(ndarray::arr0(-1).into_dyn())), Value::Int(-1));
    assert_eq!(array_first_as_value(&Array::I16(ndarray::arr0(-2).into_dyn())), Value::Int(-2));
    assert_eq!(array_first_as_value(&Array::I32(ndarray::arr0(-3).into_dyn())), Value::Int(-3));
    assert_eq!(array_first_as_value(&Array::I64(ndarray::arr0(-4).into_dyn())), Value::Int(-4));
    assert_eq!(array_first_as_value(&Array::U8(ndarray::arr0(1).into_dyn())), Value::Int(1));
    assert_eq!(array_first_as_value(&Array::U16(ndarray::arr0(2).into_dyn())), Value::Int(2));
    assert_eq!(array_first_as_value(&Array::U32(ndarray::arr0(3).into_dyn())), Value::Int(3));
    assert_eq!(array_first_as_value(&Array::U64(ndarray::arr0(4).into_dyn())), Value::Int(4));
    assert_eq!(
        array_first_as_value(&Array::F32(ndarray::arr0(1.25).into_dyn())),
        Value::Float(1.25)
    );
    assert_eq!(array_first_as_value(&Array::F64(ndarray::arr0(2.5).into_dyn())), Value::Float(2.5));
    assert_eq!(
        array_first_as_value(&Array::F16(ndarray::arr0(F16::from_f32(3.5)).into_dyn())),
        Value::Float(3.5)
    );
    assert_eq!(
        array_first_as_value(&Array::Str(ndarray::arr0("txt".to_owned()).into_dyn())),
        Value::Str("txt".into())
    );
}

#[test]
fn raw_decoder_handles_record_fortran_and_strict_cast_paths() {
    let record_dtype = DType::Struct(vec![crate::dtype::Field {
        name: "id".into(),
        dtype: DType::I32,
        shape: Vec::new(),
    }]);
    let record = decode_raw(
        record_dtype,
        Endian::Little,
        vec![1],
        false,
        &7i32.to_le_bytes(),
        &ReadOptions::new(),
    )
    .unwrap();
    assert!(matches!(record, Value::Frame(_)));

    let fortran = decode_raw(
        DType::I16,
        Endian::Little,
        vec![2, 2],
        true,
        &[1i16, 2, 3, 4].into_iter().flat_map(i16::to_le_bytes).collect::<Vec<_>>(),
        &ReadOptions::new(),
    )
    .unwrap();
    assert!(
        matches!(fortran, Value::Array(Array::I16(values)) if values.iter().copied().collect::<Vec<_>>() == [1, 3, 2, 4])
    );

    let cast_error = decode_raw(
        DType::U8,
        Endian::Native,
        vec![1],
        false,
        &[u8::MAX],
        &ReadOptions::new().int_as(IntKind::I8),
    );
    assert!(matches!(cast_error, Err(Error::Cast(_))));
}

#[test]
fn ndarray_state_validates_shape_dtype_buffer_and_allocation_limit() {
    let opts = ReadOptions::new();
    assert!(matches!(build_array(&[], &opts), Err(Error::Pickle(_))));
    assert!(matches!(
        build_array(&[Value::None, Value::Tuple(vec![Value::Int(1)])], &opts),
        Err(Error::Pickle(_))
    ));
    assert!(matches!(
        build_array(&[Value::None, Value::Tuple(vec![Value::Int(1)]), Value::Int(4)], &opts),
        Err(Error::Pickle(_))
    ));
    let object_values = Value::List(vec![Value::Int(1), Value::Str("x".into())]);
    assert_eq!(
        build_array(
            &[
                Value::None,
                Value::Tuple(vec![Value::Int(2)]),
                Value::Str("<i4".into()),
                Value::Bool(false),
                object_values.clone()
            ],
            &opts
        )
        .unwrap(),
        object_values
    );
    assert!(matches!(
        build_array(
            &[
                Value::None,
                Value::Tuple(vec![Value::Int(1)]),
                Value::Str("<i4".into()),
                Value::Bool(false)
            ],
            &opts
        ),
        Err(Error::Pickle(_))
    ));
    assert!(matches!(
        build_array(
            &[
                Value::None,
                Value::Tuple(vec![Value::Int(2)]),
                Value::Str("<i4".into()),
                Value::Bool(false),
                Value::Bytes(vec![0; 8])
            ],
            &ReadOptions { max_alloc: 8, ..opts.clone() }
        ),
        Ok(Value::Array(_))
    ));
    assert!(matches!(
        build_array(
            &[
                Value::None,
                Value::Tuple(vec![Value::Int(2)]),
                Value::Str("<i4".into()),
                Value::Bool(false),
                Value::Bytes(vec![0; 8])
            ],
            &ReadOptions { max_alloc: 7, ..opts }
        ),
        Err(Error::InvalidArgument(_))
    ));
    let string_bytes = build_array(
        &[
            Value::None,
            Value::Tuple(vec![Value::Int(2)]),
            Value::Str("|u1".into()),
            Value::Bool(false),
            Value::Str("\u{1}\u{2}".into()),
        ],
        &ReadOptions::new(),
    )
    .unwrap();
    assert!(
        matches!(string_bytes, Value::Array(Array::U8(values)) if values.iter().copied().collect::<Vec<_>>() == [1, 2])
    );
    assert!(matches!(
        build_array(
            &[
                Value::None,
                Value::Tuple(vec![Value::Int(1)]),
                Value::Str("|O".into()),
                Value::Bool(false),
                Value::Bytes(Vec::new()),
            ],
            &ReadOptions::new(),
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        build_array(
            &[
                Value::None,
                Value::List(vec![Value::Int(2)]),
                Value::Str("<i4".into()),
                Value::Bool(false),
                Value::Bytes(vec![0; 8]),
            ],
            &ReadOptions::new(),
        ),
        Ok(Value::Array(Array::I32(_)))
    ));
    assert!(matches!(
        build_array(
            &[
                Value::None,
                Value::Tuple(vec![Value::Int(0)]),
                Value::Str("<i4".into()),
                Value::Bool(false),
            ],
            &ReadOptions::new(),
        ),
        Ok(Value::Array(Array::I32(_)))
    ));
    assert!(matches!(
        build_array(
            &[
                Value::None,
                Value::Tuple(vec![Value::Int(1)]),
                Value::Str("<i4".into()),
                Value::Bool(false),
                Value::Bytes(vec![0; 4]),
            ],
            &ReadOptions::new().int_as(IntKind::I8),
        ),
        Ok(Value::Array(Array::I8(_)))
    ));
}

#[test]
fn frombuffer_accepts_integer_shapes_and_rejects_missing_parts() {
    let opts = ReadOptions::new();
    assert!(matches!(frombuffer(&[], &opts), Err(Error::Pickle(_))));
    assert!(matches!(frombuffer(&[Value::Bytes(vec![1])], &opts), Err(Error::Pickle(_))));
    assert!(matches!(
        frombuffer(&[Value::Bytes(vec![1]), Value::Str("|u1".into())], &opts),
        Err(Error::Pickle(_))
    ));
    let result = frombuffer(
        &[
            Value::Bytes(vec![1, 2]),
            Value::Str("|u1".into()),
            Value::Int(2),
            Value::Str("C".into()),
        ],
        &opts,
    )
    .unwrap();
    assert!(matches!(result, Value::Array(Array::U8(_))));
    let list_shape = frombuffer(
        &[Value::Bytes(vec![1, 2]), Value::Str("|u1".into()), Value::List(vec![Value::Int(2)])],
        &opts,
    )
    .unwrap();
    assert!(matches!(list_shape, Value::Array(Array::U8(_))));
}
