//! The Rust-side data model: dicts, frames, objects and exotic numerics.

mod common;
use common::*;
use ndarray::{ArrayD, IxDyn};
use pyferrite::prelude::*;

#[test]
fn dicts_preserve_insertion_order_like_python() {
    let mut d = Dict::new();
    for k in ["zeta", "alpha", "mu"] {
        d.insert(Value::Str(k.into()), Value::Int(1));
    }
    let keys: Vec<_> = d.keys();
    assert_eq!(keys, vec!["zeta", "alpha", "mu"]);
}

#[test]
fn dict_values_are_mutable_in_place() {
    let mut d = Dict::new();
    d.insert(Value::Str("n".into()), Value::Int(1));
    if let Some(Value::Int(n)) = d.get_mut("n") {
        *n = 42;
    }
    assert_eq!(d.get("n").and_then(|v| v.as_i64()), Some(42));
}

#[test]
fn arrays_mut_reaches_every_nested_array() {
    let mut inner = Dict::new();
    inner.insert(Value::Str("a".into()), Value::Array(f32_2x3()));
    let mut outer = Dict::new();
    outer.insert(Value::Str("g".into()), Value::Dict(inner));
    outer.insert(Value::Str("l".into()), Value::List(vec![Value::Array(f32_2x3())]));
    let mut v = Value::Dict(outer);
    let mut n = 0;
    for a in v.arrays_mut() {
        if let Some(f) = a.as_f32_mut() {
            f.iter_mut().for_each(|x| *x += 1.0);
            n += 1;
        }
    }
    assert_eq!(n, 2, "both the nested and the listed array must be reachable");
}

#[test]
fn value_views_and_type_names_cover_every_variant() {
    let values = vec![
        (Value::None, "None"),
        (Value::Bool(true), "bool"),
        (Value::Int(1), "int"),
        (Value::BigInt(vec![1]), "int"),
        (Value::Float(1.0), "float"),
        (Value::Complex(Complex64::new(1.0, 2.0)), "complex"),
        (Value::Str("text".into()), "str"),
        (Value::Bytes(vec![1]), "bytes"),
        (Value::List(vec![]), "list"),
        (Value::Tuple(vec![]), "tuple"),
        (Value::Set(vec![]), "set"),
        (Value::Dict(Dict::new()), "dict"),
        (Value::Array(f32_2x3()), "ndarray"),
        (Value::Frame(Frame::default()), "dataframe"),
        (Value::Object(PyObject::new("test", "Object")), "object"),
    ];

    for (value, expected_name) in &values {
        assert_eq!(value.type_name(), *expected_name);
    }

    assert_eq!(Value::Str("text".into()).as_str(), Some("text"));
    assert_eq!(Value::Int(7).as_i64(), Some(7));
    assert_eq!(Value::Bool(false).as_i64(), Some(0));
    assert_eq!(Value::Float(1.5).as_f64(), Some(1.5));
    assert_eq!(Value::Int(2).as_f64(), Some(2.0));
    assert_eq!(Value::Bool(true).as_f64(), Some(1.0));

    for value in &values {
        assert!(value.0.as_array().is_some() == matches!(value.0, Value::Array(_)));
        assert!(value.0.as_dict().is_some() == matches!(value.0, Value::Dict(_)));
        assert!(value.0.as_frame().is_some() == matches!(value.0, Value::Frame(_)));
        assert!(value.0.as_object().is_some() == matches!(value.0, Value::Object(_)));
    }

    let mut array = Value::Array(f32_2x3());
    assert!(array.as_array_mut().is_some());
    let mut dict = Value::Dict(Dict::new());
    assert!(dict.as_dict_mut().is_some());
    let mut frame = Value::Frame(Frame::default());
    assert!(frame.as_frame_mut().is_some());
}

#[test]
fn arrays_mut_walks_frames_and_all_object_fields() {
    let mut frame = Frame::default();
    frame.push_column(Series::new("frame", f32_2x3())).unwrap();

    let mut object = PyObject::new("test", "Object");
    object.state = Some(Box::new(Value::Array(f32_2x3())));
    object.args.push(Value::Array(f32_2x3()));
    object.list_items.push(Value::Array(f32_2x3()));
    object.dict_items.insert("array".into(), Value::Array(f32_2x3()));

    let mut value = Value::Tuple(vec![Value::Frame(frame), Value::Object(object)]);
    let arrays = value.arrays_mut();
    assert_eq!(arrays.len(), 5);
}

#[test]
fn reshaping_checks_the_element_count() {
    let mut a = f32_2x3();
    a.reshape(&[3, 2]).unwrap();
    assert_eq!(a.shape(), &[3, 2]);
    assert!(a.reshape(&[4, 4]).is_err());
}

#[test]
fn frames_carry_named_columns() {
    let mut f = Frame::default();
    f.push_column(Series::new("x", u8_5())).unwrap();
    f.push_column(Series::new(
        "y",
        Array::F32(ArrayD::from_shape_vec(IxDyn(&[5]), vec![1.0, 2.0, 3.0, 4.0, 5.0]).unwrap()),
    ))
    .unwrap();
    // A column of the wrong height must be rejected.
    assert!(f.push_column(Series::new("bad", i64_4())).is_err());
    assert_eq!(f.width(), 2);
    assert_eq!(f.height(), 5);
    assert_eq!(f.names(), vec!["x", "y"]);
    assert!(f.column("y").is_some());
    assert!(f.column("nope").is_none());
}

#[test]
fn array_encoders_cover_numeric_variants_and_endianness() {
    use pyferrite::{dtype::Endian, value::to_bytes};

    let cases = [
        (Array::Bool(ndarray::arr1(&[true, false]).into_dyn()), vec![1, 0]),
        (Array::I8(ndarray::arr1(&[-1, 2]).into_dyn()), vec![255, 2]),
        (Array::U8(ndarray::arr1(&[1, 2]).into_dyn()), vec![1, 2]),
        (Array::I16(ndarray::arr1(&[0x0102]).into_dyn()), vec![2, 1]),
        (Array::U16(ndarray::arr1(&[0x0102]).into_dyn()), vec![2, 1]),
        (Array::I32(ndarray::arr1(&[0x01020304]).into_dyn()), vec![4, 3, 2, 1]),
        (Array::U32(ndarray::arr1(&[0x01020304]).into_dyn()), vec![4, 3, 2, 1]),
        (Array::I64(ndarray::arr1(&[0x0102030405060708]).into_dyn()), vec![8, 7, 6, 5, 4, 3, 2, 1]),
        (Array::U64(ndarray::arr1(&[0x0102030405060708]).into_dyn()), vec![8, 7, 6, 5, 4, 3, 2, 1]),
        (Array::F8E4M3(ndarray::arr1(&[F8E4M3(0x38)]).into_dyn()), vec![0x38]),
        (Array::F8E5M2(ndarray::arr1(&[F8E5M2(0x3c)]).into_dyn()), vec![0x3c]),
        (Array::F16(ndarray::arr1(&[F16(0x0102)]).into_dyn()), vec![2, 1]),
        (Array::BF16(ndarray::arr1(&[BF16(0x0102)]).into_dyn()), vec![2, 1]),
        (Array::F32(ndarray::arr1(&[f32::from_bits(0x01020304)]).into_dyn()), vec![4, 3, 2, 1]),
        (
            Array::F64(ndarray::arr1(&[f64::from_bits(0x0102030405060708)]).into_dyn()),
            vec![8, 7, 6, 5, 4, 3, 2, 1],
        ),
    ];
    for (array, expected) in cases {
        assert_eq!(to_bytes(&array, Endian::Little).unwrap(), expected);
    }
    let array = Array::U32(ndarray::arr1(&[0x01020304]).into_dyn());
    assert_eq!(to_bytes(&array, Endian::Big).unwrap(), [1, 2, 3, 4]);
}

#[test]
fn array_encoders_cover_quad_complex_and_fixed_width_variants() {
    use pyferrite::{
        dtype::Endian,
        value::{to_bytes, to_bytes_x87},
    };

    let quad = Array::F128(ndarray::arr1(&[F128::from_f64(1.0)]).into_dyn());
    assert_eq!(to_bytes(&quad, Endian::Little).unwrap().len(), 16);
    let c64 = Array::C64(ndarray::arr1(&[Complex32::new(1.0, -2.0)]).into_dyn());
    assert_eq!(
        to_bytes(&c64, Endian::Little).unwrap(),
        [1.0f32.to_le_bytes(), (-2.0f32).to_le_bytes()].concat()
    );
    let c128 = Array::C128(ndarray::arr1(&[Complex64::new(1.0, -2.0)]).into_dyn());
    assert_eq!(
        to_bytes(&c128, Endian::Big).unwrap(),
        [1.0f64.to_be_bytes(), (-2.0f64).to_be_bytes()].concat()
    );

    let strings = Array::Str(ndarray::arr1(&["가a".to_owned(), "나".to_owned()]).into_dyn());
    let bytes = to_bytes(&strings, Endian::Little).unwrap();
    assert_eq!(bytes.len(), 16);
    assert_eq!(&bytes[8..], &[0x98, 0xb0, 0, 0, 0, 0, 0, 0]);
    let fixed = Array::Bytes(ndarray::arr1(&[b"abc".to_vec(), b"x".to_vec()]).into_dyn());
    assert_eq!(to_bytes(&fixed, Endian::Little).unwrap(), b"abcx\0\0");

    for array in [&quad, &Array::I32(ndarray::arr1(&[1]).into_dyn())] {
        let bytes = to_bytes_x87(array).unwrap();
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[10..], &[0; 6]);
    }
}

#[test]
fn streaming_encoder_handles_strided_and_empty_arrays() {
    use pyferrite::{
        dtype::Endian,
        value::{to_bytes, write_bytes_to},
    };

    let arrays = [
        Array::U16(ndarray::arr1(&[1, 2]).into_dyn()),
        Array::I32(ndarray::arr1(&[1, 2]).into_dyn()),
        Array::U32(ndarray::arr1(&[1, 2]).into_dyn()),
        Array::I64(ndarray::arr1(&[1, 2]).into_dyn()),
        Array::U64(ndarray::arr1(&[1, 2]).into_dyn()),
        Array::F32(ndarray::arr1(&[1.0, 2.0]).into_dyn()),
        Array::F64(ndarray::arr1(&[1.0, 2.0]).into_dyn()),
        Array::U8(ndarray::arr1(&[1, 2]).into_dyn()),
    ];
    for array in arrays {
        let expected = to_bytes(&array, Endian::Big).unwrap();
        let mut output = Vec::new();
        write_bytes_to(&array, Endian::Big, &mut output).unwrap();
        assert_eq!(output, expected);
    }

    let non_contiguous = Array::U32(ndarray::arr2(&[[1u32, 2], [3, 4]]).reversed_axes().into_dyn());
    let expected = to_bytes(&non_contiguous, Endian::Big).unwrap();
    let mut output = Vec::new();
    write_bytes_to(&non_contiguous, Endian::Big, &mut output).unwrap();
    assert_eq!(output, expected);

    let empty = Array::U32(ndarray::arr1(&[]).into_dyn());
    write_bytes_to(&empty, Endian::Little, &mut output).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn f128_arithmetic_beats_f64_precision() {
    let third = F128::from_f64(1.0) / F128::from_f64(3.0);
    let back = third * F128::from_f64(3.0);
    assert_eq!(back.to_f64(), 1.0, "1/3*3 must round-trip exactly in binary128");
}

#[test]
fn f128_survives_the_extremes_of_f64() {
    for v in [1e-300, 1e300, -2.5e-310, f64::MIN_POSITIVE] {
        assert_eq!(F128::from_f64(v).to_f64(), v, "round trip of {v}");
    }
}

#[test]
fn complex_arrays_round_trip_through_every_container() {
    for ext in ["npy", "npz", "pkl"] {
        let t = Tmp::new(&format!("cplx-{ext}"));
        let p = t.join(&format!("c.{ext}"));
        let src = if ext == "npy" {
            Value::Array(c128_2())
        } else {
            let mut d = Dict::new();
            d.insert(Value::Str("z".into()), Value::Array(c128_2()));
            Value::Dict(d)
        };
        write(&p, &src).unwrap();
        let back = read(&p).unwrap();
        let got = flatten_arrays(&back);
        assert_eq!(got.len(), 1, "{ext}");
        assert_same(&c128_2(), got[0].1, ext);
    }
}

#[test]
fn flatten_uses_dotted_paths_like_a_state_dict() {
    let mut inner = Dict::new();
    inner.insert(Value::Str("weight".into()), Value::Array(f32_2x3()));
    let mut outer = Dict::new();
    outer.insert(Value::Str("layer1".into()), Value::Dict(inner));
    let names: Vec<_> = flatten_arrays(&Value::Dict(outer)).into_iter().map(|(k, _)| k).collect();
    assert_eq!(names, vec!["layer1.weight"]);
}

#[test]
fn dtype_sizes_match_numpy() {
    for (dt, size) in [
        (DType::Bool, 1),
        (DType::I8, 1),
        (DType::I16, 2),
        (DType::I32, 4),
        (DType::I64, 8),
        (DType::U64, 8),
        (DType::F16, 2),
        (DType::BF16, 2),
        (DType::F32, 4),
        (DType::F64, 8),
        (DType::F128, 16),
        (DType::C64, 8),
        (DType::C128, 16),
        (DType::C256, 32),
        (DType::F8E4M3, 1),
        (DType::F8E5M2, 1),
    ] {
        assert_eq!(dt.size(), Some(size), "{dt:?}");
    }
}
