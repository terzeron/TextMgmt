//! Configurable numeric types on read and on write.

mod common;
use common::*;
use ndarray::{ArrayD, IxDyn};
use pyferrite::prelude::*;

#[test]
fn float_downcast_on_read() {
    let t = Tmp::new("cast-read");
    let p = t.join("x.npy");
    write(&p, &Value::Array(f64_3x2())).unwrap();
    let v = read_with(&p, &ReadOptions::new().float_as(FloatKind::F32)).unwrap();
    assert_eq!(v.as_array().unwrap().dtype(), DType::F32);
}

#[test]
fn float_downcast_on_write() {
    let t = Tmp::new("cast-write");
    let p = t.join("x.npy");
    write_with(&p, &Value::Array(f64_3x2()), &WriteOptions::new().float_as(FloatKind::F32))
        .unwrap();
    assert_eq!(read(&p).unwrap().as_array().unwrap().dtype(), DType::F32);
}

#[test]
fn integer_width_is_configurable_both_ways() {
    let t = Tmp::new("cast-int");
    let p = t.join("x.npy");
    let small = Array::I64(ArrayD::from_shape_vec(IxDyn(&[3]), vec![-5, 0, 100]).unwrap());
    write_with(&p, &Value::Array(small), &WriteOptions::new().int_as(IntKind::I16)).unwrap();
    assert_eq!(read(&p).unwrap().as_array().unwrap().dtype(), DType::I16);
    let widened = read_with(&p, &ReadOptions::new().int_as(IntKind::I64)).unwrap();
    assert_eq!(widened.as_array().unwrap().dtype(), DType::I64);
}

#[test]
fn eight_bit_floats_are_available_as_a_target() {
    // `f8` here means an 8-*bit* float, not numpy's 8-*byte* naming.
    let a = Array::F32(ArrayD::from_shape_vec(IxDyn(&[4]), vec![0.5, -1.0, 2.0, 448.0]).unwrap());
    let small = a.cast(&DType::F8E4M3, CastPolicy::Saturate).unwrap();
    assert_eq!(small.dtype(), DType::F8E4M3);
    let back = small.cast(&DType::F32, CastPolicy::Strict).unwrap();
    let v = back.as_f32().unwrap();
    assert_eq!(v.as_slice().unwrap(), &[0.5, -1.0, 2.0, 448.0]);
}

#[test]
fn strict_policy_refuses_to_lose_information() {
    let big = Array::I64(ArrayD::from_shape_vec(IxDyn(&[2]), vec![0, 100_000]).unwrap());
    let err = big.cast(&DType::I16, CastPolicy::Strict).unwrap_err();
    assert!(matches!(err, Error::Cast(_)), "got {err}");
}

#[test]
fn saturate_clamps_and_wrap_truncates() {
    let big = Array::I64(ArrayD::from_shape_vec(IxDyn(&[2]), vec![-99_999, 99_999]).unwrap());
    let sat = big.cast(&DType::I16, CastPolicy::Saturate).unwrap();
    assert_eq!(sat.as_i16().unwrap().as_slice().unwrap(), &[i16::MIN, i16::MAX]);
    let wrapped = big.cast(&DType::I16, CastPolicy::Wrap).unwrap();
    assert_eq!(
        wrapped.as_i16().unwrap().as_slice().unwrap(),
        &[(-99_999i64) as i16, 99_999i64 as i16]
    );
}

#[test]
fn half_precision_survives_a_narrowing_round_trip() {
    let a = Array::F64(ArrayD::from_shape_vec(IxDyn(&[4]), vec![1.0, 0.5, -0.25, 1024.0]).unwrap());
    let half = a.cast(&DType::F16, CastPolicy::Strict).unwrap();
    let back = half.cast(&DType::F64, CastPolicy::Strict).unwrap();
    assert_eq!(back.as_f64().unwrap().as_slice().unwrap(), &[1.0, 0.5, -0.25, 1024.0]);
}

#[test]
fn bfloat16_keeps_the_exponent_range_of_f32() {
    let a = Array::F32(ArrayD::from_shape_vec(IxDyn(&[2]), vec![3.0e38, 1.0e-38]).unwrap());
    let bf = a.cast(&DType::BF16, CastPolicy::Strict).unwrap();
    let back = bf.cast(&DType::F32, CastPolicy::Strict).unwrap();
    let v = back.as_f32().unwrap();
    assert!(v.as_slice().unwrap()[0].is_finite(), "bf16 should not overflow at 3e38");
    assert!(v.as_slice().unwrap()[1] > 0.0, "bf16 should not flush 1e-38 to zero");
}

#[test]
fn casts_apply_through_a_whole_container() {
    let t = Tmp::new("cast-container");
    let p = t.join("m.npz");
    write(&p, &sample_dict()).unwrap();
    // `b` holds 2^40, which does not fit in an i32, so Strict must refuse.
    let strict = ReadOptions::new().float_as(FloatKind::F32).int_as(IntKind::I32);
    assert!(matches!(read_with(&p, &strict), Err(Error::Cast(_))));
    let v = read_with(&p, &strict.clone().cast_policy(CastPolicy::Saturate)).unwrap();
    for (name, a) in flatten_arrays(&v) {
        match name.as_str() {
            "w" | "x" => assert_eq!(a.dtype(), DType::F32, "{name}"),
            "b" => {
                assert_eq!(a.dtype(), DType::I32, "{name}");
                assert_eq!(a.as_i32().unwrap().as_slice().unwrap()[3], i32::MAX);
            }
            _ => {}
        }
    }
}

#[test]
fn casts_cover_integer_rounding_float_overflow_and_special_values() {
    let floats =
        Array::F64(ArrayD::from_shape_vec(IxDyn(&[4]), vec![1.0, 1.5, f64::NAN, 300.0]).unwrap());
    assert!(matches!(floats.cast(&DType::I8, CastPolicy::Strict), Err(Error::Cast(_))));
    let rounded = floats.cast(&DType::I8, CastPolicy::Saturate).unwrap();
    assert_eq!(rounded.as_i8().unwrap().as_slice().unwrap(), &[1, 2, 0, 127]);
    let wrapped = floats.cast(&DType::I8, CastPolicy::Wrap).unwrap();
    assert_eq!(wrapped.as_i8().unwrap().as_slice().unwrap(), &[1, 2, 0, 44]);

    let huge = Array::F64(ArrayD::from_shape_vec(IxDyn(&[2]), vec![1e10, -1e10]).unwrap());
    assert!(matches!(huge.cast(&DType::F16, CastPolicy::Strict), Err(Error::Cast(_))));
    let saturated = huge.cast(&DType::F16, CastPolicy::Saturate).unwrap();
    assert_eq!(
        saturated
            .as_f16()
            .unwrap()
            .as_slice()
            .unwrap()
            .iter()
            .map(|v| v.to_f64())
            .collect::<Vec<_>>(),
        &[65504.0, -65504.0]
    );
    let wrapped = huge.cast(&DType::F16, CastPolicy::Wrap).unwrap();
    assert!(wrapped.as_f16().unwrap().as_slice().unwrap().iter().all(|v| v.to_f64().is_infinite()));
}

#[test]
fn casts_cover_string_bytes_boolean_complex_and_unsupported_targets() {
    let numeric = Array::F64(ArrayD::from_shape_vec(IxDyn(&[2]), vec![1.5, 0.0]).unwrap());
    let strings = numeric.cast(&DType::Str(4), CastPolicy::Strict).unwrap();
    let Array::Str(string_values) = &strings else { panic!("expected strings") };
    assert_eq!(string_values.as_slice().unwrap(), &["1.5", "0"]);
    let bytes = numeric.cast(&DType::Bytes(4), CastPolicy::Strict).unwrap();
    let Array::Bytes(byte_values) = &bytes else { panic!("expected bytes") };
    assert_eq!(byte_values.as_slice().unwrap(), &[b"1.5".to_vec(), b"0".to_vec()]);
    assert_eq!(strings.cast(&DType::Bytes(4), CastPolicy::Strict).unwrap().len(), 2);
    assert_eq!(bytes.cast(&DType::Str(4), CastPolicy::Strict).unwrap().len(), 2);
    assert!(bytes.cast(&DType::Bool, CastPolicy::Strict).is_ok());
    assert!(numeric.cast(&DType::C64, CastPolicy::Strict).is_ok());
    assert!(numeric.cast(&DType::C128, CastPolicy::Strict).is_ok());
    assert!(matches!(numeric.cast(&DType::Object, CastPolicy::Strict), Err(Error::Unsupported(_))));
    assert!(matches!(
        numeric.cast(&DType::Struct(Vec::new()), CastPolicy::Strict),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn casts_every_integer_width_and_wide_float_target() {
    let source = Array::F64(ArrayD::from_shape_vec(IxDyn(&[2]), vec![0.0, 1.0]).unwrap());
    for dtype in [
        DType::I8,
        DType::I16,
        DType::I32,
        DType::I64,
        DType::U8,
        DType::U16,
        DType::U32,
        DType::U64,
        DType::F8E5M2,
        DType::BF16,
        DType::F32,
        DType::F64,
        DType::F128,
        DType::X87,
        DType::C256,
    ] {
        assert_eq!(source.cast(&dtype, CastPolicy::Strict).unwrap().len(), 2, "{dtype:?}");
    }
    let already_i64 = source.cast(&DType::I64, CastPolicy::Strict).unwrap();
    assert_eq!(already_i64.dtype(), DType::I64);
    let bytes = Array::Bytes(ArrayD::from_shape_vec(IxDyn(&[1]), vec![vec![0xff, b'a']]).unwrap());
    let lossy = bytes.cast(&DType::Str(2), CastPolicy::Strict).unwrap();
    let Array::Str(lossy) = lossy else { panic!("expected strings") };
    assert_eq!(lossy[[0]], "�a");
}

#[test]
fn casts_cover_same_family_strings_bytes_and_nan_float_targets() {
    let strings =
        Array::Str(ArrayD::from_shape_vec(IxDyn(&[1]), vec!["hello".to_owned()]).unwrap());
    let shortened = strings.cast(&DType::Str(2), CastPolicy::Strict).unwrap();
    let Array::Str(shortened) = shortened else { panic!("expected strings") };
    assert_eq!(shortened[[0]], "hello");

    let bytes = Array::Bytes(ArrayD::from_shape_vec(IxDyn(&[1]), vec![b"hello".to_vec()]).unwrap());
    let widened = bytes.cast(&DType::Bytes(10), CastPolicy::Strict).unwrap();
    let Array::Bytes(widened) = widened else { panic!("expected bytes") };
    assert_eq!(widened[[0]], b"hello");

    let nan = Array::F64(ArrayD::from_shape_vec(IxDyn(&[1]), vec![f64::NAN]).unwrap());
    let converted = nan.cast(&DType::F32, CastPolicy::Strict).unwrap();
    assert!(converted.as_f32().unwrap()[[0]].is_nan());
}
