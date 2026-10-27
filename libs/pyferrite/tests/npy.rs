//! `.npy` round-trips across every element type the format can hold.

mod common;
use common::*;
use ndarray::{ArrayD, IxDyn};
use pyferrite::prelude::*;

fn roundtrip(a: Array, tag: &str) {
    let t = Tmp::new(&format!("npy-{tag}"));
    let p = t.join("x.npy");
    write(&p, &Value::Array(a.clone())).unwrap();
    let back = read(&p).unwrap();
    assert_same(&a, back.as_array().unwrap(), tag);
}

#[test]
fn every_dtype_roundtrips() {
    roundtrip(bool_3(), "bool");
    roundtrip(u8_5(), "u8");
    roundtrip(i64_4(), "i64");
    roundtrip(f32_2x3(), "f32");
    roundtrip(f64_3x2(), "f64");
    roundtrip(c128_2(), "c128");
    roundtrip(
        Array::I16(ArrayD::from_shape_vec(IxDyn(&[3]), vec![-32768i16, 0, 32767]).unwrap()),
        "i16",
    );
    roundtrip(
        Array::U32(ArrayD::from_shape_vec(IxDyn(&[2, 2]), vec![0u32, 1, 4294967295, 7]).unwrap()),
        "u32",
    );
    roundtrip(
        Array::F16(
            ArrayD::from_shape_vec(
                IxDyn(&[3]),
                vec![F16::from_f64(1.5), F16::from_f64(-2.25), F16::from_f64(65504.0)],
            )
            .unwrap(),
        ),
        "f16",
    );
}

#[test]
fn zero_length_and_scalar_shapes_survive() {
    roundtrip(Array::F32(ArrayD::from_shape_vec(IxDyn(&[0]), vec![]).unwrap()), "empty");
    roundtrip(Array::F64(ArrayD::from_shape_vec(IxDyn(&[]), vec![42.0]).unwrap()), "scalar");
    roundtrip(
        Array::F32(ArrayD::from_shape_vec(IxDyn(&[1, 1, 1, 1]), vec![1.0]).unwrap()),
        "unit-4d",
    );
}

#[test]
fn high_rank_arrays_keep_their_element_order() {
    let n: usize = 2 * 3 * 4 * 5;
    let a = Array::F64(
        ArrayD::from_shape_vec(IxDyn(&[2, 3, 4, 5]), (0..n).map(|i| i as f64).collect()).unwrap(),
    );
    roundtrip(a, "4d");
}

#[test]
fn shape_and_dtype_are_declared_on_write() {
    // The brief calls for shape control: numpy tells `2` and `[1,2]` apart.
    let t = Tmp::new("npy-shape");
    let p = t.join("x.npy");
    let flat = Array::F64(
        ArrayD::from_shape_vec(IxDyn(&[6]), (0..6).map(|i| i as f64).collect()).unwrap(),
    );
    let opts = WriteOptions::new().shape([1, 6]);
    write_with(&p, &Value::Array(flat), &opts).unwrap();
    assert_eq!(read(&p).unwrap().as_array().unwrap().shape(), &[1, 6]);
}

#[test]
fn wrong_declared_shape_is_rejected() {
    let t = Tmp::new("npy-badshape");
    let opts = WriteOptions::new().shape([4, 4]);
    let err = write_with(t.join("x.npy"), &Value::Array(f32_2x3()), &opts).unwrap_err();
    assert!(matches!(err, Error::Shape { .. }), "got {err}");
}

#[test]
fn fortran_order_changes_the_bytes_not_the_array() {
    // numpy must read back the same logical array; only the storage order
    // differs. Flagging the header without reordering would transpose it.
    let t = Tmp::new("npy-fortran");
    let c = t.join("c.npy");
    let f = t.join("f.npy");
    let a = Array::F64(
        ArrayD::from_shape_vec(IxDyn(&[3, 4]), (0..12).map(|i| i as f64).collect()).unwrap(),
    );
    write(&c, &Value::Array(a.clone())).unwrap();
    write_with(&f, &Value::Array(a.clone()), &WriteOptions::new().fortran_order(true)).unwrap();

    let back_c = read(&c).unwrap();
    let back_f = read(&f).unwrap();
    assert_same(back_c.as_array().unwrap(), back_f.as_array().unwrap(), "fortran vs c");
    assert_same(&a, back_f.as_array().unwrap(), "fortran round trip");
    // The files differ on disk even though the arrays agree.
    assert_ne!(std::fs::read(&c).unwrap(), std::fs::read(&f).unwrap());
}
