//! A lossless intermediate scalar used as the pivot of every numeric cast.
//!
//! Converting `N` source types to `M` target types directly would need `N * M`
//! code paths; pivoting through [`Wide`] needs only `N + M`.

use crate::num::{Complex64, F128};
use crate::value::Array;

/// The widest representation of any scalar this crate understands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wide {
    /// Booleans, one byte each, as in numpy `bool_`.
    Bool(bool),
    /// A Python `int` that fits in 64 bits.
    Int(i128),
    /// An unsigned integer widened to `u128`.
    UInt(u128),
    /// A Python `float`.
    Float(F128),
    /// A Python `complex`.
    Complex(Complex64),
}

impl Wide {
    /// Best-effort numeric view as `f64` (used for float targets and reports).
    pub fn as_f64(self) -> f64 {
        match self {
            Wide::Bool(b) => b as u8 as f64,
            Wide::Int(i) => i as f64,
            Wide::UInt(u) => u as f64,
            Wide::Float(f) => f.to_f64(),
            Wide::Complex(c) => c.re,
        }
    }
    /// Exact quad-precision view.
    pub fn as_f128(self) -> F128 {
        match self {
            Wide::Bool(b) => F128::from_f64(b as u8 as f64),
            Wide::Int(i) => F128::from_f64(i as f64),
            Wide::UInt(u) => F128::from_f64(u as f64),
            Wide::Float(f) => f,
            Wide::Complex(c) => F128::from_f64(c.re),
        }
    }
    /// Integer view; `None` when the value has a fractional part or is NaN.
    pub fn as_i128(self) -> Option<i128> {
        match self {
            Wide::Bool(b) => Some(b as i128),
            Wide::Int(i) => Some(i),
            Wide::UInt(u) => i128::try_from(u).ok(),
            Wide::Float(f) => {
                let v = f.to_f64();
                if v.is_finite() && v.fract() == 0.0 && v.abs() < 1.7e38 {
                    Some(v as i128)
                } else {
                    None
                }
            }
            Wide::Complex(c) => {
                if c.im == 0.0 && c.re.fract() == 0.0 {
                    Some(c.re as i128)
                } else {
                    None
                }
            }
        }
    }
    /// Complex view (real types get a zero imaginary part).
    pub fn as_complex(self) -> Complex64 {
        match self {
            Wide::Complex(c) => c,
            other => Complex64::new(other.as_f64(), 0.0),
        }
    }
    /// Truthiness following Python's rules for numbers.
    pub fn as_bool(self) -> bool {
        match self {
            Wide::Bool(b) => b,
            Wide::Int(i) => i != 0,
            Wide::UInt(u) => u != 0,
            Wide::Float(f) => f.to_f64() != 0.0,
            Wide::Complex(c) => c.re != 0.0 || c.im != 0.0,
        }
    }
}

/// Flatten any numeric [`Array`] into `Wide` scalars in C order.
pub fn to_wide(a: &Array) -> Vec<Wide> {
    match a {
        Array::Bool(x) => x.iter().map(|v| Wide::Bool(*v)).collect(),
        Array::I8(x) => x.iter().map(|v| Wide::Int(*v as i128)).collect(),
        Array::I16(x) => x.iter().map(|v| Wide::Int(*v as i128)).collect(),
        Array::I32(x) => x.iter().map(|v| Wide::Int(*v as i128)).collect(),
        Array::I64(x) => x.iter().map(|v| Wide::Int(*v as i128)).collect(),
        Array::U8(x) => x.iter().map(|v| Wide::UInt(*v as u128)).collect(),
        Array::U16(x) => x.iter().map(|v| Wide::UInt(*v as u128)).collect(),
        Array::U32(x) => x.iter().map(|v| Wide::UInt(*v as u128)).collect(),
        Array::U64(x) => x.iter().map(|v| Wide::UInt(*v as u128)).collect(),
        Array::F8E4M3(x) => x.iter().map(|v| Wide::Float(F128::from_f32(v.to_f32()))).collect(),
        Array::F8E5M2(x) => x.iter().map(|v| Wide::Float(F128::from_f32(v.to_f32()))).collect(),
        Array::F16(x) => x.iter().map(|v| Wide::Float(F128::from_f32(v.to_f32()))).collect(),
        Array::BF16(x) => x.iter().map(|v| Wide::Float(F128::from_f32(v.to_f32()))).collect(),
        Array::F32(x) => x.iter().map(|v| Wide::Float(F128::from_f32(*v))).collect(),
        Array::F64(x) => x.iter().map(|v| Wide::Float(F128::from_f64(*v))).collect(),
        Array::F128(x) => x.iter().map(|v| Wide::Float(*v)).collect(),
        Array::C64(x) => {
            x.iter().map(|v| Wide::Complex(Complex64::new(v.re as f64, v.im as f64))).collect()
        }
        Array::C128(x) => x.iter().map(|v| Wide::Complex(*v)).collect(),
        Array::Str(x) => x
            .iter()
            .map(|s| {
                s.trim()
                    .parse::<f64>()
                    .map(|f| Wide::Float(F128::from_f64(f)))
                    .unwrap_or(Wide::Float(F128::NAN))
            })
            .collect(),
        Array::Bytes(x) => {
            x.iter().map(|b| Wide::UInt(b.first().copied().unwrap_or(0) as u128)).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::num::{BF16, F16, F8E4M3, F8E5M2};
    use ndarray::arr1;

    fn array<T>(items: &[T]) -> ndarray::ArrayD<T>
    where
        T: Clone,
    {
        arr1(items).into_dyn()
    }

    #[test]
    fn every_array_variant_converts_to_wide_values() {
        let cases = vec![
            (Array::Bool(array(&[true, false])), vec![Wide::Bool(true), Wide::Bool(false)]),
            (Array::I8(array(&[-2i8])), vec![Wide::Int(-2)]),
            (Array::I16(array(&[-3i16])), vec![Wide::Int(-3)]),
            (Array::I32(array(&[-4i32])), vec![Wide::Int(-4)]),
            (Array::I64(array(&[-5i64])), vec![Wide::Int(-5)]),
            (Array::U8(array(&[6u8])), vec![Wide::UInt(6)]),
            (Array::U16(array(&[7u16])), vec![Wide::UInt(7)]),
            (Array::U32(array(&[8u32])), vec![Wide::UInt(8)]),
            (Array::U64(array(&[9u64])), vec![Wide::UInt(9)]),
            (Array::F8E4M3(array(&[F8E4M3(0x38)])), vec![Wide::Float(F128::from_f32(1.0))]),
            (Array::F8E5M2(array(&[F8E5M2(0x3c)])), vec![Wide::Float(F128::from_f32(1.0))]),
            (Array::F16(array(&[F16::from_f32(1.0)])), vec![Wide::Float(F128::from_f32(1.0))]),
            (Array::BF16(array(&[BF16::from_f32(1.0)])), vec![Wide::Float(F128::from_f32(1.0))]),
            (Array::F32(array(&[1.0f32])), vec![Wide::Float(F128::from_f32(1.0))]),
            (Array::F64(array(&[1.0f64])), vec![Wide::Float(F128::from_f64(1.0))]),
            (Array::F128(array(&[F128::from_f64(1.0)])), vec![Wide::Float(F128::from_f64(1.0))]),
            (
                Array::C64(array(&[crate::num::Complex32::new(2.0f32, 3.0f32)])),
                vec![Wide::Complex(Complex64::new(2.0, 3.0))],
            ),
            (
                Array::C128(array(&[Complex64::new(2.0, 3.0)])),
                vec![Wide::Complex(Complex64::new(2.0, 3.0))],
            ),
            (
                Array::Str(array(&["1.25".to_owned(), "bad".to_owned()])),
                vec![Wide::Float(F128::from_f64(1.25)), Wide::Float(F128::NAN)],
            ),
            (
                Array::Bytes(array(&[b"xy".to_vec(), Vec::new()])),
                vec![Wide::UInt(b'x' as u128), Wide::UInt(0)],
            ),
        ];
        for (input, expected) in cases {
            let actual = to_wide(&input);
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.into_iter().zip(expected) {
                match (actual, expected) {
                    (Wide::Float(a), Wide::Float(b)) if a.is_nan() && b.is_nan() => {}
                    (a, b) => assert_eq!(a, b),
                }
            }
        }
    }

    #[test]
    fn wide_views_follow_python_numeric_rules() {
        assert_eq!(Wide::Bool(true).as_f64(), 1.0);
        assert_eq!(Wide::Int(-2).as_f64(), -2.0);
        assert_eq!(Wide::UInt(3).as_f64(), 3.0);
        assert_eq!(Wide::Float(F128::from_f64(4.0)).as_f64(), 4.0);
        assert_eq!(Wide::Complex(Complex64::new(5.0, 6.0)).as_f64(), 5.0);
        assert_eq!(Wide::Bool(false).as_f128(), F128::from_f64(0.0));
        assert_eq!(Wide::Int(-2).as_i128(), Some(-2));
        assert_eq!(Wide::UInt(u128::MAX).as_i128(), None);
        assert_eq!(Wide::Float(F128::from_f64(2.0)).as_i128(), Some(2));
        assert_eq!(Wide::Float(F128::NAN).as_i128(), None);
        assert_eq!(Wide::Complex(Complex64::new(3.0, 0.0)).as_i128(), Some(3));
        assert_eq!(Wide::Complex(Complex64::new(3.5, 0.0)).as_i128(), None);
        assert_eq!(Wide::Complex(Complex64::new(3.0, 1.0)).as_i128(), None);
        assert!(!Wide::Int(0).as_bool());
        assert!(Wide::UInt(1).as_bool());
        assert!(!Wide::Float(F128::from_f64(0.0)).as_bool());
        assert!(Wide::Complex(Complex64::new(0.0, 1.0)).as_bool());
        assert_eq!(Wide::Int(1).as_complex(), Complex64::new(1.0, 0.0));
    }
}
