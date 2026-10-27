//! `Frame` ⇄ `polars::DataFrame`.
//!
//! Enabled by the `polars-interop` feature. Conversion is by column: every
//! [`Series`] becomes a polars series of the matching
//! logical type, with the validity bitmap carried across as nulls.

use crate::error::{Error, Result};
use crate::value::{Array, Frame, Series};
use polars::prelude::*;

/// Convert a [`Frame`] into a `polars::DataFrame`.
///
/// Element types map to their obvious polars counterparts. The narrow float
/// types (`f8`, `f16`, `bf16`) and `f128` have no polars equivalent and are
/// widened to `f64`; complex numbers have none either and are rejected.
///
/// ```no_run
/// # use pyferrite::prelude::*;
/// let v = read("table.pkl")?;
/// if let Some(frame) = v.as_frame() {
///     let df = pyferrite::interop::polars::to_polars(frame)?;
///     println!("{df}");
/// }
/// # Ok::<(), pyferrite::Error>(())
/// ```
pub fn to_polars(frame: &Frame) -> Result<DataFrame> {
    let mut cols: Vec<polars::prelude::Series> = Vec::with_capacity(frame.columns.len());
    for s in &frame.columns {
        cols.push(series_to_polars(s)?);
    }
    DataFrame::new(cols).map_err(|e| Error::invalid(format!("polars rejected the frame: {e}")))
}

macro_rules! numeric {
    ($name:expr, $vals:expr, $valid:expr) => {{
        match $valid {
            None => polars::prelude::Series::from_iter($vals.iter().copied()).with_name($name),
            Some(mask) => {
                polars::prelude::Series::from_iter($vals.iter().enumerate().map(|(i, v)| {
                    if mask.get(i).copied().unwrap_or(true) {
                        Some(*v)
                    } else {
                        None
                    }
                }))
                .with_name($name)
            }
        }
    }};
}

fn series_to_polars(s: &Series) -> Result<polars::prelude::Series> {
    let name = s.name.as_str();
    let valid = s.validity.as_ref();
    Ok(match &s.values {
        Array::Bool(a) => numeric!(name, a, valid),
        Array::I8(a) => numeric!(name, a, valid),
        Array::I16(a) => numeric!(name, a, valid),
        Array::I32(a) => numeric!(name, a, valid),
        Array::I64(a) => numeric!(name, a, valid),
        Array::U8(a) => numeric!(name, a, valid),
        Array::U16(a) => numeric!(name, a, valid),
        Array::U32(a) => numeric!(name, a, valid),
        Array::U64(a) => numeric!(name, a, valid),
        Array::F32(a) => numeric!(name, a, valid),
        Array::F64(a) => numeric!(name, a, valid),
        // Types polars has no slot for are widened rather than dropped.
        Array::F16(a) => {
            let v: Vec<f64> = a.iter().map(|x| x.to_f64()).collect();
            numeric!(name, &v, valid)
        }
        Array::BF16(a) => {
            let v: Vec<f64> = a.iter().map(|x| x.to_f32() as f64).collect();
            numeric!(name, &v, valid)
        }
        Array::F8E4M3(a) => {
            let v: Vec<f64> = a.iter().map(|x| x.to_f32() as f64).collect();
            numeric!(name, &v, valid)
        }
        Array::F8E5M2(a) => {
            let v: Vec<f64> = a.iter().map(|x| x.to_f32() as f64).collect();
            numeric!(name, &v, valid)
        }
        Array::F128(a) => {
            let v: Vec<f64> = a.iter().map(|x| x.to_f64()).collect();
            numeric!(name, &v, valid)
        }
        Array::Str(a) => {
            let v: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
            match valid {
                None => polars::prelude::Series::new(name, v),
                Some(mask) => polars::prelude::Series::new(
                    name,
                    v.iter()
                        .enumerate()
                        .map(
                            |(i, s)| {
                                if mask.get(i).copied().unwrap_or(true) {
                                    Some(*s)
                                } else {
                                    None
                                }
                            },
                        )
                        .collect::<Vec<_>>(),
                ),
            }
        }
        Array::Bytes(a) => {
            let v: Vec<&[u8]> = a.iter().map(|b| b.as_slice()).collect();
            polars::prelude::Series::new(name, v)
        }
        other => {
            return Err(Error::unsupported(format!(
                "polars has no column type for {:?}; cast it first",
                other.dtype()
            )))
        }
    })
}
