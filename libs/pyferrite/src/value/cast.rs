//! Numeric casting between [`Array`] element types under a [`CastPolicy`].

use super::wide::{to_wide, Wide};
use super::Array;
use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::num::{Complex32, Complex64, BF16, F16, F8E4M3, F8E5M2};
use crate::options::CastPolicy;
use ndarray::{ArrayD, IxDyn};

fn build<T: Clone>(shape: &[usize], data: Vec<T>) -> Result<ArrayD<T>> {
    ArrayD::from_shape_vec(IxDyn(shape), data).map_err(|e| Error::format(e.to_string()))
}

/// Convert one `Wide` into a bounded integer, honouring `policy`.
fn to_int<T>(w: Wide, policy: CastPolicy, lo: i128, hi: i128, f: impl Fn(i128) -> T) -> Result<T> {
    let v = match w.as_i128() {
        Some(v) => v,
        None => match policy {
            CastPolicy::Strict => {
                return Err(Error::cast(format!("{:?} is not an exact integer", w)))
            }
            _ => w.as_f64().round() as i128,
        },
    };
    if v < lo || v > hi {
        match policy {
            CastPolicy::Strict => {
                return Err(Error::cast(format!("{v} out of range [{lo}, {hi}]")))
            }
            CastPolicy::Saturate => return Ok(f(v.clamp(lo, hi))),
            CastPolicy::Wrap => {
                let span = hi.wrapping_sub(lo).wrapping_add(1);
                let m = v.wrapping_sub(lo).rem_euclid(span).wrapping_add(lo);
                return Ok(f(m));
            }
        }
    }
    Ok(f(v))
}

fn float_overflow(v: f64, max: f64, policy: CastPolicy) -> Result<f64> {
    if v.is_finite() && v.abs() > max {
        match policy {
            CastPolicy::Strict => Err(Error::cast(format!("{v} overflows target float range"))),
            CastPolicy::Saturate => Ok(v.signum() * max),
            CastPolicy::Wrap => Ok(v.signum() * f64::INFINITY),
        }
    } else {
        Ok(v)
    }
}

impl Array {
    /// Cast every element to `target`, allocating a new array.
    ///
    /// Returns [`Error::Cast`] under [`CastPolicy::Strict`] whenever a value
    /// would not survive the round trip.
    pub fn cast(&self, target: &DType, policy: CastPolicy) -> Result<Array> {
        if &self.dtype() == target {
            return Ok(self.clone());
        }
        let shape = self.shape().to_vec();
        if let DType::Str(_) = target {
            let data: Vec<String> = match self {
                Array::Str(x) => x.iter().cloned().collect(),
                Array::Bytes(x) => {
                    x.iter().map(|b| String::from_utf8_lossy(b).into_owned()).collect()
                }
                other => to_wide(other).into_iter().map(|w| format!("{}", w.as_f64())).collect(),
            };
            return Ok(Array::Str(build(&shape, data)?));
        }
        if let DType::Bytes(_) = target {
            let data: Vec<Vec<u8>> = match self {
                Array::Bytes(x) => x.iter().cloned().collect(),
                Array::Str(x) => x.iter().map(|s| s.as_bytes().to_vec()).collect(),
                other => to_wide(other)
                    .into_iter()
                    .map(|w| format!("{}", w.as_f64()).into_bytes())
                    .collect(),
            };
            return Ok(Array::Bytes(build(&shape, data)?));
        }
        let w = to_wide(self);
        Ok(match target {
            DType::Bool => Array::Bool(build(&shape, w.iter().map(|x| x.as_bool()).collect())?),
            DType::I8 => Array::I8(build(
                &shape,
                ints(&w, policy, i8::MIN as i128, i8::MAX as i128, |v| v as i8)?,
            )?),
            DType::I16 => Array::I16(build(
                &shape,
                ints(&w, policy, i16::MIN as i128, i16::MAX as i128, |v| v as i16)?,
            )?),
            DType::I32 => Array::I32(build(
                &shape,
                ints(&w, policy, i32::MIN as i128, i32::MAX as i128, |v| v as i32)?,
            )?),
            DType::I64 => Array::I64(build(
                &shape,
                ints(&w, policy, i64::MIN as i128, i64::MAX as i128, |v| v as i64)?,
            )?),
            DType::U8 => {
                Array::U8(build(&shape, ints(&w, policy, 0, u8::MAX as i128, |v| v as u8)?)?)
            }
            DType::U16 => {
                Array::U16(build(&shape, ints(&w, policy, 0, u16::MAX as i128, |v| v as u16)?)?)
            }
            DType::U32 => {
                Array::U32(build(&shape, ints(&w, policy, 0, u32::MAX as i128, |v| v as u32)?)?)
            }
            DType::U64 => {
                Array::U64(build(&shape, ints(&w, policy, 0, u64::MAX as i128, |v| v as u64)?)?)
            }
            DType::F8E4M3 => {
                Array::F8E4M3(build(&shape, floats(&w, policy, 448.0, F8E4M3::from_f64)?)?)
            }
            DType::F8E5M2 => {
                Array::F8E5M2(build(&shape, floats(&w, policy, 57344.0, F8E5M2::from_f64)?)?)
            }
            DType::F16 => Array::F16(build(&shape, floats(&w, policy, 65504.0, F16::from_f64)?)?),
            DType::BF16 => {
                Array::BF16(build(&shape, floats(&w, policy, 3.3895314e38, BF16::from_f64)?)?)
            }
            DType::F32 => {
                Array::F32(build(&shape, floats(&w, policy, f32::MAX as f64, |v| v as f32)?)?)
            }
            DType::F64 => {
                Array::F64(build(&shape, w.iter().map(|x| x.as_f128().to_f64()).collect())?)
            }
            DType::F128 | DType::X87 => {
                Array::F128(build(&shape, w.iter().map(|x| x.as_f128()).collect())?)
            }
            DType::C64 => Array::C64(build(
                &shape,
                w.iter()
                    .map(|x| {
                        let c = x.as_complex();
                        Complex32::new(c.re as f32, c.im as f32)
                    })
                    .collect(),
            )?),
            DType::C128 | DType::C256 => Array::C128(build(
                &shape,
                w.iter().map(|x| x.as_complex()).collect::<Vec<Complex64>>(),
            )?),
            DType::Object => return Err(Error::unsupported("cast to object dtype")),
            DType::Struct(_) => return Err(Error::unsupported("cast to structured dtype")),
            DType::Str(_) | DType::Bytes(_) => unreachable!(),
        })
    }
}

fn ints<T>(
    w: &[Wide],
    policy: CastPolicy,
    lo: i128,
    hi: i128,
    f: impl Fn(i128) -> T + Copy,
) -> Result<Vec<T>> {
    w.iter().map(|x| to_int(*x, policy, lo, hi, f)).collect()
}

fn floats<T>(
    w: &[Wide],
    policy: CastPolicy,
    max: f64,
    f: impl Fn(f64) -> T + Copy,
) -> Result<Vec<T>> {
    w.iter()
        .map(|x| {
            let v = x.as_f128().to_f64();
            if v.is_nan() {
                return Ok(f(v));
            }
            Ok(f(float_overflow(v, max, policy)?))
        })
        .collect()
}
