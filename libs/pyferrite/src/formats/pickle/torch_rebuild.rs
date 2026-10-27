//! torch reconstructors: storages, `_rebuild_tensor_v2`, `_rebuild_parameter`.

use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};
use crate::options::ReadOptions;
use crate::value::{from_bytes, PyObject, Value};

pub(crate) const TAG: &str = "__pyferrite__";

/// Map a torch storage class name onto an element type.
pub fn storage_dtype(name: &str) -> Option<DType> {
    Some(match name.trim_end_matches("Storage") {
        "Float" | "TypedFloat" => DType::F32,
        "Double" => DType::F64,
        "Half" => DType::F16,
        "BFloat16" => DType::BF16,
        "Long" => DType::I64,
        "Int" => DType::I32,
        "Short" => DType::I16,
        "Char" => DType::I8,
        "Byte" => DType::U8,
        "Bool" => DType::Bool,
        "ComplexFloat" => DType::C64,
        "ComplexDouble" => DType::C128,
        "Float8_e4m3fn" => DType::F8E4M3,
        "Float8_e5m2" => DType::F8E5M2,
        _ => return None,
    })
}

/// Wrap raw storage bytes so `_rebuild_tensor_v2` can slice them later.
pub fn storage_value(dtype: DType, bytes: Vec<u8>) -> Value {
    let mut o = PyObject::new(TAG, "storage");
    o.args = vec![Value::Str(format!("{dtype:?}")), Value::Bytes(bytes)];
    Value::Object(o)
}

fn storage_parts(v: &Value) -> Option<(DType, &Vec<u8>)> {
    match v {
        Value::Object(o) if o.module == TAG && o.name == "storage" => {
            let name = o.args.first()?.as_str()?;
            let dt = parse_dtype_debug(name)?;
            match o.args.get(1)? {
                Value::Bytes(b) => Some((dt, b)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_dtype_debug(s: &str) -> Option<DType> {
    Some(match s {
        "Bool" => DType::Bool,
        "I8" => DType::I8,
        "I16" => DType::I16,
        "I32" => DType::I32,
        "I64" => DType::I64,
        "U8" => DType::U8,
        "U16" => DType::U16,
        "U32" => DType::U32,
        "U64" => DType::U64,
        "F16" => DType::F16,
        "BF16" => DType::BF16,
        "F32" => DType::F32,
        "F64" => DType::F64,
        "F8E4M3" => DType::F8E4M3,
        "F8E5M2" => DType::F8E5M2,
        "C64" => DType::C64,
        "C128" => DType::C128,
        _ => return None,
    })
}

fn usizes(v: Option<&Value>) -> Vec<usize> {
    match v {
        Some(Value::Tuple(t)) | Some(Value::List(t)) => {
            t.iter().map(|x| x.as_i64().unwrap_or(0).max(0) as usize).collect()
        }
        _ => Vec::new(),
    }
}

/// `torch._utils._rebuild_tensor_v2(storage, offset, size, stride, ...)`.
pub fn rebuild_tensor(args: &[Value], opts: &ReadOptions) -> Result<Value> {
    let (dt, bytes) =
        storage_parts(args.first().ok_or_else(|| Error::pickle("tensor without storage"))?)
            .ok_or_else(|| Error::pickle("tensor storage was not resolved"))?;
    let esz = dt.size().ok_or_else(|| Error::unsupported("variable-size tensor dtype"))?;
    let offset = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0).max(0) as usize;
    let shape = usizes(args.get(2));
    let stride = usizes(args.get(3));
    let n: usize = shape.iter().product();
    if n * esz > opts.max_alloc {
        return Err(Error::invalid("tensor exceeds max_alloc"));
    }
    let contiguous = c_strides(&shape) == stride || stride.is_empty();
    let raw: Vec<u8> = if contiguous {
        let start = offset * esz;
        bytes
            .get(start..start + n * esz)
            .ok_or_else(|| Error::format("tensor storage slice is out of range"))?
            .to_vec()
    } else {
        gather(bytes, esz, offset, &shape, &stride)?
    };
    let arr = from_bytes(&dt, Endian::Little, &shape, &raw)?;
    Ok(Value::Array(crate::formats::apply_cast(arr, opts)?))
}

fn c_strides(shape: &[usize]) -> Vec<usize> {
    let mut s = vec![1usize; shape.len()];
    for i in (0..shape.len().saturating_sub(1)).rev() {
        s[i] = s[i + 1] * shape[i + 1];
    }
    s
}

/// Copy a strided (non-contiguous) view into a dense C-order buffer.
fn gather(
    bytes: &[u8],
    esz: usize,
    offset: usize,
    shape: &[usize],
    stride: &[usize],
) -> Result<Vec<u8>> {
    let n: usize = shape.iter().product();
    let mut out = Vec::with_capacity(n * esz);
    let mut idx = vec![0usize; shape.len()];
    for _ in 0..n {
        let mut flat = offset;
        for (d, i) in idx.iter().enumerate() {
            flat += i * stride.get(d).copied().unwrap_or(0);
        }
        let start = flat * esz;
        out.extend_from_slice(
            bytes
                .get(start..start + esz)
                .ok_or_else(|| Error::format("strided tensor read out of range"))?,
        );
        for d in (0..shape.len()).rev() {
            idx[d] += 1;
            if idx[d] < shape[d] {
                break;
            }
            idx[d] = 0;
        }
    }
    Ok(out)
}
