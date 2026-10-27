//! Shared fixtures for the integration tests.
#![allow(dead_code)]

use ndarray::{ArrayD, IxDyn};
use pyferrite::prelude::*;

/// A scratch directory that cleans itself up.
pub struct Tmp(pub std::path::PathBuf);

impl Tmp {
    pub fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("pyferrite-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    pub fn join(&self, n: &str) -> std::path::PathBuf {
        self.0.join(n)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn f32_2x3() -> Array {
    Array::F32(
        ArrayD::from_shape_vec(IxDyn(&[2, 3]), vec![1.0, -2.5, 3.25, 4.0, 0.0, -0.125]).unwrap(),
    )
}

pub fn f64_3x2() -> Array {
    Array::F64(
        ArrayD::from_shape_vec(IxDyn(&[3, 2]), vec![0.5, -1.25, 3.0, 1e-300, 2.0, 9.75]).unwrap(),
    )
}

pub fn i64_4() -> Array {
    Array::I64(ArrayD::from_shape_vec(IxDyn(&[4]), vec![-1, 0, 7, 1 << 40]).unwrap())
}

pub fn u8_5() -> Array {
    Array::U8(ArrayD::from_shape_vec(IxDyn(&[5]), vec![0, 1, 128, 254, 255]).unwrap())
}

pub fn bool_3() -> Array {
    Array::Bool(ArrayD::from_shape_vec(IxDyn(&[3]), vec![true, false, true]).unwrap())
}

pub fn c128_2() -> Array {
    Array::C128(
        ArrayD::from_shape_vec(
            IxDyn(&[2]),
            vec![Complex64::new(1.0, 2.0), Complex64::new(-3.0, -4.0)],
        )
        .unwrap(),
    )
}

/// The standard mixed dict used across format round-trips.
pub fn sample_dict() -> Value {
    let mut d = Dict::new();
    d.insert(Value::Str("w".into()), Value::Array(f32_2x3()));
    d.insert(Value::Str("b".into()), Value::Array(i64_4()));
    d.insert(Value::Str("x".into()), Value::Array(f64_3x2()));
    d.insert(Value::Str("q".into()), Value::Array(u8_5()));
    Value::Dict(d)
}

/// Assert two arrays are bit-identical, with a useful message on failure.
pub fn assert_same(a: &Array, b: &Array, what: &str) {
    assert_eq!(a.dtype(), b.dtype(), "{what}: dtype");
    assert_eq!(a.shape(), b.shape(), "{what}: shape");
    let (x, y) = (
        pyferrite::value::to_bytes(a, Endian::Little).unwrap(),
        pyferrite::value::to_bytes(b, Endian::Little).unwrap(),
    );
    assert_eq!(x, y, "{what}: payload");
}

/// Every array in a value, keyed by its dotted path.
pub fn arrays(v: &Value) -> std::collections::BTreeMap<String, Array> {
    flatten_arrays(v).into_iter().map(|(k, a)| (k, a.clone())).collect()
}
