//! Decoding pickled Apache Arrow arrays without pyarrow.
//!
//! pandas 3 keeps strings in `ArrowStringArray`, whose state holds a pyarrow
//! array pickled as `pyarrow.lib._restore_array(type, length, null_count,
//! offset, buffers, children, dictionary)`, each buffer being
//! `pyarrow.lib.py_buffer(bytes)`. Strings, large strings and the common
//! fixed-width numeric types are decoded here; anything else is left alone.

use crate::value::{Array, PyObject, Value};
use ndarray::{ArrayD, IxDyn};

fn alias(t: &Value) -> Option<String> {
    match t {
        // `pyarrow.lib.type_for_alias("large_string")`
        Value::Object(o) if o.name == "type_for_alias" => {
            o.args.first()?.as_str().map(String::from)
        }
        // `pyarrow.lib.string()` and friends
        Value::Object(o) => Some(o.name.clone()),
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn buffer(v: &Value) -> Option<&[u8]> {
    match v {
        Value::Bytes(b) => Some(b),
        Value::Object(o) if o.name == "py_buffer" => buffer(o.args.first()?),
        _ => None,
    }
}

fn is_valid(bitmap: Option<&[u8]>, i: usize) -> bool {
    bitmap.map(|b| b.get(i / 8).map(|x| x >> (i % 8) & 1 == 1).unwrap_or(false)).unwrap_or(true)
}

/// Decode `_restore_array(...)` into an [`Array`], if its type is supported.
pub fn restore_array(o: &PyObject) -> Option<Array> {
    let args = match o.args.as_slice() {
        [Value::Tuple(t)] | [Value::List(t)] => t.as_slice(),
        other => other,
    };
    let ty = alias(args.first()?)?;
    let len = args.get(1)?.as_i64()? as usize;
    let offset = args.get(3).and_then(|v| v.as_i64()).unwrap_or(0) as usize;
    let bufs = match args.get(4)? {
        Value::List(b) | Value::Tuple(b) => b,
        _ => return None,
    };
    let validity = bufs.first().and_then(buffer);
    let shape = IxDyn(&[len]);
    macro_rules! fixed {
        ($t:ty, $n:expr, $variant:ident, $null:expr) => {{
            let data = buffer(bufs.get(1)?)?;
            let vals: Vec<$t> = (0..len)
                .map(|i| {
                    let k = (offset + i) * $n;
                    match data.get(k..k + $n) {
                        Some(b) if is_valid(validity, offset + i) => {
                            <$t>::from_le_bytes(b.try_into().unwrap())
                        }
                        _ => $null,
                    }
                })
                .collect();
            ArrayD::from_shape_vec(shape, vals).ok().map(Array::$variant)
        }};
    }
    match ty.as_str() {
        "string" | "utf8" | "large_string" | "large_utf8" => {
            let wide = ty.starts_with("large");
            let offs = buffer(bufs.get(1)?)?;
            let data = buffer(bufs.get(2)?).unwrap_or(&[]);
            let at = |i: usize| -> Option<usize> {
                if wide {
                    Some(i64::from_le_bytes(offs.get(i * 8..i * 8 + 8)?.try_into().ok()?) as usize)
                } else {
                    Some(i32::from_le_bytes(offs.get(i * 4..i * 4 + 4)?.try_into().ok()?) as usize)
                }
            };
            let mut out = Vec::with_capacity(len);
            for i in offset..offset + len {
                let (a, b) = (at(i)?, at(i + 1)?);
                out.push(if is_valid(validity, i) {
                    String::from_utf8_lossy(data.get(a..b)?).into_owned()
                } else {
                    String::new()
                });
            }
            ArrayD::from_shape_vec(shape, out).ok().map(Array::Str)
        }
        "int64" => fixed!(i64, 8, I64, 0),
        "int32" => fixed!(i32, 4, I32, 0),
        "double" | "float64" => fixed!(f64, 8, F64, f64::NAN),
        "float" | "float32" => fixed!(f32, 4, F32, f32::NAN),
        _ => None,
    }
}

/// `pyarrow.lib.chunked_array([chunks], type)`: concatenate decoded chunks.
pub fn chunked_array(o: &PyObject) -> Option<Array> {
    let chunks = match o.args.first()? {
        Value::List(c) | Value::Tuple(c) => c,
        _ => return None,
    };
    let parts: Vec<Array> = chunks
        .iter()
        .filter_map(|c| match c {
            Value::Object(p) if p.name == "_restore_array" => restore_array(p),
            _ => None,
        })
        .collect();
    if parts.len() != chunks.len() || parts.is_empty() {
        return None;
    }
    if parts.len() == 1 {
        return parts.into_iter().next();
    }
    match &parts[0] {
        Array::Str(_) => {
            let mut all: Vec<String> = Vec::new();
            for p in &parts {
                match p {
                    Array::Str(x) => all.extend(x.iter().cloned()),
                    _ => return None,
                }
            }
            let n = all.len();
            ArrayD::from_shape_vec(IxDyn(&[n]), all).ok().map(Array::Str)
        }
        _ => None,
    }
}
