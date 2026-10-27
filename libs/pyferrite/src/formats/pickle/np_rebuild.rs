//! numpy reconstructors: `_reconstruct`, `dtype`, `scalar`.

use crate::dtype::{parse_descr, DType, Endian};
use crate::error::{Error, Result};
use crate::options::ReadOptions;
use crate::value::{from_bytes, Array, PyObject, Value};

/// Internal module tag for placeholders this crate creates itself.
pub(crate) const TAG: &str = "__pyferrite__";

/// Placeholder produced by `numpy.core.multiarray._reconstruct`.
pub(crate) fn reconstruct_placeholder() -> Value {
    Value::Object(PyObject::new(TAG, "ndarray"))
}

/// `numpy.dtype(name, align, copy)` — the name is kept until `BUILD` supplies
/// the byte order.
pub(crate) fn dtype_placeholder(args: Vec<Value>) -> Value {
    let mut o = PyObject::new(TAG, "dtype");
    o.args = args;
    Value::Object(o)
}

/// Extract `(DType, Endian)` from a value that should be a numpy dtype.
pub(crate) fn as_dtype(v: &Value) -> Result<(DType, Endian)> {
    match v {
        Value::Str(s) => parse_descr(s),
        Value::Object(o) if o.module == TAG && o.name == "dtype" => {
            let name = o
                .args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| Error::pickle("numpy dtype without a name"))?;
            let order = match o.state.as_deref() {
                Some(Value::Tuple(t)) => t.get(1).and_then(|x| x.as_str()).unwrap_or("|"),
                _ => "|",
            };
            // A structured dtype puts its field names in state[3] and
            // `{name: (dtype, offset)}` in state[4].
            if let Some(Value::Tuple(t)) = o.state.as_deref() {
                if let (Some(names), Some(Value::Dict(fields))) = (t.get(3), t.get(4)) {
                    if name.starts_with('V') {
                        return struct_dtype(names, fields, t.get(5).and_then(|v| v.as_i64()));
                    }
                }
            }
            parse_descr(&format!("{order}{name}"))
        }
        other => {
            Err(Error::pickle(format!("expected a numpy dtype, found `{}`", other.type_name())))
        }
    }
}

/// Rebuild a packed structured dtype from its pickled field table.
///
/// Fields must follow each other without padding, which is what numpy
/// produces unless `align=True` was requested; padded layouts are refused.
fn struct_dtype(
    names: &Value,
    fields: &crate::value::Dict,
    itemsize: Option<i64>,
) -> Result<(DType, Endian)> {
    let names = match names {
        Value::Tuple(n) | Value::List(n) => n,
        _ => return Err(Error::pickle("structured dtype without field names")),
    };
    let mut out = Vec::with_capacity(names.len());
    let mut endian = Endian::Little;
    let mut at = 0usize;
    for n in names {
        let key = n.as_str().ok_or_else(|| Error::pickle("structured dtype field name"))?;
        let (fdt, off) = match fields.get(key) {
            Some(Value::Tuple(t)) if t.len() >= 2 => (&t[0], t[1].as_i64().unwrap_or(-1)),
            _ => return Err(Error::pickle(format!("structured dtype lacks field `{key}`"))),
        };
        let (dt, e) = as_dtype(fdt)?;
        let size = dt
            .size()
            .ok_or_else(|| Error::unsupported("variable-size field in a structured dtype"))?;
        if off != at as i64 {
            return Err(Error::unsupported("structured numpy dtype with padding (align=True)"));
        }
        if e != Endian::Native {
            endian = e;
        }
        at += size;
        out.push(crate::dtype::Field { name: key.to_string(), dtype: dt, shape: Vec::new() });
    }
    if let Some(total) = itemsize {
        if total != at as i64 {
            return Err(Error::unsupported("structured numpy dtype with trailing padding"));
        }
    }
    Ok((DType::Struct(out), endian))
}

/// `numpy.core.multiarray.scalar(dtype, raw_bytes)`.
pub(crate) fn scalar(args: &[Value], _opts: &ReadOptions) -> Result<Value> {
    let (dt, endian) =
        as_dtype(args.first().ok_or_else(|| Error::pickle("scalar() needs a dtype"))?)?;
    let raw = match args.get(1) {
        Some(Value::Bytes(b)) => b.clone(),
        Some(Value::Str(s)) => s.chars().map(|c| c as u8).collect(),
        _ => return Err(Error::pickle("scalar() needs a bytes payload")),
    };
    let a = from_bytes(&dt, endian, &[1], &raw)?;
    Ok(array_first_as_value(&a))
}

fn array_first_as_value(a: &Array) -> Value {
    match a {
        Array::Bool(x) => Value::Bool(x.iter().next().copied().unwrap_or(false)),
        Array::I8(x) => Value::Int(x.iter().next().copied().unwrap_or(0) as i64),
        Array::I16(x) => Value::Int(x.iter().next().copied().unwrap_or(0) as i64),
        Array::I32(x) => Value::Int(x.iter().next().copied().unwrap_or(0) as i64),
        Array::I64(x) => Value::Int(x.iter().next().copied().unwrap_or(0)),
        Array::U8(x) => Value::Int(x.iter().next().copied().unwrap_or(0) as i64),
        Array::U16(x) => Value::Int(x.iter().next().copied().unwrap_or(0) as i64),
        Array::U32(x) => Value::Int(x.iter().next().copied().unwrap_or(0) as i64),
        Array::U64(x) => Value::Int(x.iter().next().copied().unwrap_or(0) as i64),
        Array::F32(x) => Value::Float(x.iter().next().copied().unwrap_or(0.0) as f64),
        Array::F64(x) => Value::Float(x.iter().next().copied().unwrap_or(0.0)),
        Array::F16(x) => Value::Float(x.iter().next().map(|v| v.to_f64()).unwrap_or(0.0)),
        Array::BF16(x) => Value::Float(x.iter().next().map(|v| v.to_f64()).unwrap_or(0.0)),
        Array::F128(x) => Value::Float(x.iter().next().map(|v| v.to_f64()).unwrap_or(0.0)),
        Array::C64(x) => Value::Complex(
            x.iter()
                .next()
                .map(|c| crate::num::Complex64::new(c.re as f64, c.im as f64))
                .unwrap_or_default(),
        ),
        Array::C128(x) => Value::Complex(x.iter().next().copied().unwrap_or_default()),
        Array::Str(x) => Value::Str(x.iter().next().cloned().unwrap_or_default()),
        _ => Value::None,
    }
}

/// Finish an `ndarray` placeholder from its `__setstate__` tuple
/// `(version, shape, dtype, is_fortran, rawdata)`.
pub(crate) fn build_array(state: &[Value], opts: &ReadOptions) -> Result<Value> {
    let shape: Vec<usize> = match state.get(1) {
        Some(Value::Tuple(t)) | Some(Value::List(t)) => {
            t.iter().map(|x| x.as_i64().unwrap_or(0) as usize).collect()
        }
        _ => return Err(Error::pickle("ndarray state without a shape")),
    };
    let (dt, endian) =
        as_dtype(state.get(2).ok_or_else(|| Error::pickle("ndarray state without a dtype"))?)?;
    let fortran = matches!(state.get(3), Some(Value::Bool(true)));
    let raw = match state.get(4) {
        Some(Value::Bytes(b)) => b.clone(),
        Some(Value::Str(s)) => s.chars().map(|c| c as u8).collect(),
        Some(Value::List(items)) => {
            // Object arrays keep their elements as a plain Python list.
            return Ok(Value::List(items.clone()));
        }
        // A zero-element array carries no buffer at all, which is not the same
        // thing as a missing one.
        _ if shape.contains(&0) => Vec::new(),
        _ => return Err(Error::pickle("ndarray state without raw data")),
    };
    let n: usize = shape.iter().product();
    if let Some(sz) = dt.size() {
        if n * sz > opts.max_alloc {
            return Err(Error::invalid("pickled array exceeds max_alloc"));
        }
    }
    decode_raw(dt, endian, shape, fortran, &raw, opts)
}

/// Turn a raw numpy buffer into a value: a record array becomes a [`Frame`]
/// exactly as a structured `.npy` payload does, anything else an [`Array`].
///
/// [`Frame`]: crate::value::Frame
pub(crate) fn decode_raw(
    dt: DType,
    endian: Endian,
    shape: Vec<usize>,
    fortran: bool,
    raw: &[u8],
    opts: &ReadOptions,
) -> Result<Value> {
    if let DType::Struct(_) = dt {
        let h = crate::formats::npy::NpyHeader {
            dtype: dt,
            endian,
            fortran_order: fortran,
            shape,
            data_offset: 0,
            version: (1, 0),
        };
        return crate::formats::npy::decode_payload(&h, raw, opts);
    }
    let read_shape: Vec<usize> =
        if fortran { shape.iter().rev().copied().collect() } else { shape.clone() };
    let mut arr = from_bytes(&dt, endian, &read_shape, raw)?;
    if fortran && shape.len() > 1 {
        let axes: Vec<usize> = (0..shape.len()).rev().collect();
        arr = crate::formats::permute(arr, &axes);
    }
    Ok(Value::Array(crate::formats::apply_cast(arr, opts)?))
}

/// `numpy._core.numeric._frombuffer(buffer, dtype, shape, order)`.
///
/// Protocol 5 pickles produced by numpy 2.x take this route instead of
/// `_reconstruct`, handing the raw buffer over directly.
pub fn frombuffer(args: &[Value], opts: &ReadOptions) -> Result<Value> {
    let raw = match args.first() {
        Some(Value::Bytes(b)) => b,
        _ => return Err(Error::pickle("_frombuffer without a byte buffer")),
    };
    let (dt, endian) =
        as_dtype(args.get(1).ok_or_else(|| Error::pickle("_frombuffer without a dtype"))?)?;
    let shape: Vec<usize> = match args.get(2) {
        Some(Value::Tuple(t)) | Some(Value::List(t)) => {
            t.iter().map(|v| v.as_i64().unwrap_or(0) as usize).collect()
        }
        Some(Value::Int(n)) => vec![*n as usize],
        _ => return Err(Error::pickle("_frombuffer without a shape")),
    };
    let fortran = matches!(args.get(3), Some(Value::Str(s)) if s == "F");
    decode_raw(dt, endian, shape, fortran, raw, opts)
}

#[cfg(test)]
#[path = "../../../../../tests/rust/pyferrite/np_rebuild_unit.rs"]
mod tests;
