//! `polars::DataFrame` -> [`Frame`].

use crate::error::{Error, Result};
use crate::value::{Array, Frame, Series};
use polars::prelude::*;

/// Convert a `polars::DataFrame` back into a [`Frame`].
///
/// Nulls become entries in the validity bitmap; the underlying slot holds the
/// type's default value, so the numeric buffer stays dense and writable to
/// numpy.
pub fn from_polars(df: &DataFrame) -> Result<Frame> {
    let mut cols = Vec::with_capacity(df.width());
    for s in df.get_columns() {
        cols.push(series_from_polars(s)?);
    }
    Frame::from_columns(cols)
}

macro_rules! gather {
    ($ca:expr, $variant:ident, $name:expr) => {{
        let n = $ca.len();
        let mut vals = Vec::with_capacity(n);
        let mut mask = Vec::with_capacity(n);
        let mut any_null = false;
        for v in $ca.into_iter() {
            match v {
                Some(x) => {
                    vals.push(x);
                    mask.push(true);
                }
                None => {
                    vals.push(Default::default());
                    mask.push(false);
                    any_null = true;
                }
            }
        }
        let arr = Array::$variant(
            ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&[n]), vals)
                .map_err(|e| Error::invalid(format!("{e}")))?,
        );
        let mut s = Series::new($name, arr);
        if any_null {
            s.validity = Some(mask);
        }
        s
    }};
}

fn series_from_polars(s: &polars::prelude::Series) -> Result<Series> {
    let name = s.name();
    Ok(match s.dtype() {
        DataType::Boolean => gather!(s.bool().unwrap(), Bool, name),
        DataType::Int8 => gather!(s.i8().unwrap(), I8, name),
        DataType::Int16 => gather!(s.i16().unwrap(), I16, name),
        DataType::Int32 => gather!(s.i32().unwrap(), I32, name),
        DataType::Int64 => gather!(s.i64().unwrap(), I64, name),
        DataType::UInt8 => gather!(s.u8().unwrap(), U8, name),
        DataType::UInt16 => gather!(s.u16().unwrap(), U16, name),
        DataType::UInt32 => gather!(s.u32().unwrap(), U32, name),
        DataType::UInt64 => gather!(s.u64().unwrap(), U64, name),
        DataType::Float32 => gather!(s.f32().unwrap(), F32, name),
        DataType::Float64 => gather!(s.f64().unwrap(), F64, name),
        DataType::Utf8 => {
            let ca = s.utf8().unwrap();
            let n = ca.len();
            let mut vals = Vec::with_capacity(n);
            let mut mask = Vec::with_capacity(n);
            let mut any_null = false;
            for v in ca.into_iter() {
                match v {
                    Some(x) => {
                        vals.push(x.to_string());
                        mask.push(true);
                    }
                    None => {
                        vals.push(String::new());
                        mask.push(false);
                        any_null = true;
                    }
                }
            }
            let arr = Array::Str(
                ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&[n]), vals)
                    .map_err(|e| Error::invalid(format!("{e}")))?,
            );
            let mut out = Series::new(name, arr);
            if any_null {
                out.validity = Some(mask);
            }
            out
        }
        other => {
            return Err(Error::unsupported(format!(
                "no pyferrite element type for the polars dtype {other}"
            )))
        }
    })
}
