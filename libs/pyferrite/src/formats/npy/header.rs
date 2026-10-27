//! The `.npy` header: magic, version, and a Python `dict` repr.

use crate::dtype::{parse_descr, to_descr, DType, Endian, Field};
use crate::error::{Error, Result};
use crate::util::parse_python_literal;
use crate::value::Value;

/// Everything the header tells us about the payload that follows it.
#[derive(Clone, Debug, PartialEq)]
pub struct NpyHeader {
    /// Element type.
    pub dtype: DType,
    /// Byte order of the stored payload.
    pub endian: Endian,
    /// `true` when the payload is column-major.
    pub fortran_order: bool,
    /// Dimensions, outermost first.
    pub shape: Vec<usize>,
    /// Byte offset of the first payload byte.
    pub data_offset: usize,
    /// `.npy` format version, `(1, 0)` unless a big header forced `(2, 0)`.
    pub version: (u8, u8),
}

impl NpyHeader {
    /// Number of elements in the payload.
    pub fn count(&self) -> usize {
        self.shape.iter().product()
    }
    /// Payload size in bytes, or `None` for object arrays.
    pub fn nbytes(&self) -> Option<usize> {
        Some(self.count() * self.dtype.size()?)
    }
}

fn descr_to_dtype(v: &Value) -> Result<(DType, Endian)> {
    match v {
        Value::Str(s) => parse_descr(s),
        Value::List(items) => {
            let mut fields = Vec::with_capacity(items.len());
            for it in items {
                let t = match it {
                    Value::Tuple(t) | Value::List(t) => t,
                    _ => return Err(Error::format("bad field in structured descr")),
                };
                let name = t
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::format("structured field without a name"))?
                    .to_string();
                let (dtype, _) = descr_to_dtype(
                    t.get(1).ok_or_else(|| Error::format("structured field without a dtype"))?,
                )?;
                let shape = match t.get(2) {
                    Some(Value::Tuple(s)) | Some(Value::List(s)) => {
                        s.iter().filter_map(|x| x.as_i64()).map(|x| x as usize).collect()
                    }
                    _ => Vec::new(),
                };
                fields.push(Field { name, dtype, shape });
            }
            Ok((DType::Struct(fields), Endian::Native))
        }
        other => Err(Error::format(format!("unsupported descr node: {}", other.type_name()))),
    }
}

/// Parse the header at the beginning of `buf`.
pub fn parse(buf: &[u8]) -> Result<NpyHeader> {
    if buf.len() < 10 || &buf[..6] != b"\x93NUMPY" {
        return Err(Error::format("missing \\x93NUMPY magic"));
    }
    let (major, minor) = (buf[6], buf[7]);
    let (hlen, start) = match major {
        1 => (u16::from_le_bytes([buf[8], buf[9]]) as usize, 10usize),
        2 | 3 => (u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize, 12usize),
        v => return Err(Error::unsupported(format!("npy format version {v}"))),
    };
    let text = std::str::from_utf8(
        buf.get(start..start + hlen).ok_or_else(|| Error::format("npy header is truncated"))?,
    )
    .map_err(|_| Error::format("npy header is not valid utf-8"))?;
    let dict = parse_python_literal(text)?;
    let d = dict.as_dict().ok_or_else(|| Error::format("npy header is not a dict"))?;
    let descr = d.get("descr").ok_or_else(|| Error::format("npy header lacks `descr`"))?;
    let (dtype, endian) = descr_to_dtype(descr)?;
    let fortran_order = matches!(d.get("fortran_order"), Some(Value::Bool(true)));
    let shape = match d.get("shape") {
        Some(Value::Tuple(t)) | Some(Value::List(t)) => {
            t.iter().map(|x| x.as_i64().unwrap_or(0) as usize).collect()
        }
        _ => return Err(Error::format("npy header lacks `shape`")),
    };
    Ok(NpyHeader {
        dtype,
        endian,
        fortran_order,
        shape,
        data_offset: start + hlen,
        version: (major, minor),
    })
}

/// Render a structured dtype as numpy's `[('name', 'descr'), ...]` literal.
fn struct_descr(fields: &[Field], endian: Endian) -> Result<String> {
    let mut parts = Vec::with_capacity(fields.len());
    for f in fields {
        let inner = to_descr(&f.dtype, endian)?;
        if f.shape.is_empty() {
            parts.push(format!("('{}', '{inner}')", f.name));
        } else {
            let dims: String = f.shape.iter().map(|d| format!("{d}, ")).collect();
            parts.push(format!("('{}', '{inner}', ({dims}))", f.name));
        }
    }
    Ok(format!("[{}]", parts.join(", ")))
}

/// Render a header, padding it so the payload starts on a 64-byte boundary.
pub fn render(
    dtype: &DType,
    endian: Endian,
    shape: &[usize],
    fortran_order: bool,
) -> Result<Vec<u8>> {
    // A structured dtype is written as numpy's list-of-pairs form rather than
    // a scalar type string.
    let descr = match dtype {
        DType::Struct(fields) => struct_descr(fields, endian)?,
        scalar => format!("'{}'", to_descr(scalar, endian)?),
    };
    let mut dims: String = shape.iter().map(|d| format!("{d}, ")).collect();
    if shape.len() == 1 {
        dims = format!("{}, ", shape[0]);
    }
    let body = format!(
        "{{'descr': {descr}, 'fortran_order': {}, 'shape': ({dims}), }}",
        if fortran_order { "True" } else { "False" }
    );
    let big = body.len() + 12 > u16::MAX as usize;
    let prelude = if big { 12 } else { 10 };
    let mut total = prelude + body.len() + 1;
    let pad = (64 - total % 64) % 64;
    total += pad;
    let hlen = total - prelude;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"\x93NUMPY");
    if big {
        out.extend_from_slice(&[2, 0]);
        out.extend_from_slice(&(hlen as u32).to_le_bytes());
    } else {
        out.extend_from_slice(&[1, 0]);
        out.extend_from_slice(&(hlen as u16).to_le_bytes());
    }
    out.extend_from_slice(body.as_bytes());
    out.extend(std::iter::repeat(b' ').take(pad));
    out.push(b'\n');
    Ok(out)
}
