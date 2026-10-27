//! Decoding a `.npy` payload into a [`Value`].

use super::header::{parse, NpyHeader};
use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::options::ReadOptions;
use crate::value::{from_bytes, Array, Frame, Series, Value};
use ndarray::{ArrayD, IxDyn};
use std::io::Read;
use std::path::Path;

/// Bytes read from disk per block by [`read_file`].
const BLOCK_BYTES: usize = 1 << 20;

macro_rules! read_prim {
    ($r:ident, $count:ident, $swap:ident, $shape:ident, $ty:ty, $n:expr, $variant:ident) => {{
        let mut out: Vec<$ty> = Vec::with_capacity($count);
        let mut buf = vec![0u8; (BLOCK_BYTES / $n * $n).min($count * $n)];
        let mut left = $count * $n;
        while left > 0 {
            let take = left.min(buf.len());
            $r.read_exact(&mut buf[..take]).map_err(truncated)?;
            let bytes = buf[..take].chunks_exact($n);
            if $swap {
                out.extend(bytes.map(|c| <$ty>::from_be_bytes(c.try_into().unwrap())));
            } else {
                out.extend(bytes.map(|c| <$ty>::from_le_bytes(c.try_into().unwrap())));
            }
            left -= take;
        }
        Array::$variant(
            ArrayD::from_shape_vec(IxDyn($shape), out).map_err(|e| Error::format(e.to_string()))?,
        )
    }};
}

fn truncated(e: std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof => Error::format("npy payload is truncated"),
        _ => Error::Io(e),
    }
}

/// Decode a plain numeric payload block by block from `r`; `None` for element
/// types that need the whole buffer (strings, records, extended floats, ...).
fn stream_numeric<R: Read>(r: &mut R, h: &NpyHeader) -> Result<Option<Array>> {
    let count: usize = h.shape.iter().product();
    let swap = h.endian.needs_swap();
    let shape = h.shape.as_slice();
    Ok(Some(match h.dtype {
        DType::I16 => read_prim!(r, count, swap, shape, i16, 2, I16),
        DType::U16 => read_prim!(r, count, swap, shape, u16, 2, U16),
        DType::I32 => read_prim!(r, count, swap, shape, i32, 4, I32),
        DType::U32 => read_prim!(r, count, swap, shape, u32, 4, U32),
        DType::I64 => read_prim!(r, count, swap, shape, i64, 8, I64),
        DType::U64 => read_prim!(r, count, swap, shape, u64, 8, U64),
        DType::F32 => read_prim!(r, count, swap, shape, f32, 4, F32),
        DType::F64 => read_prim!(r, count, swap, shape, f64, 8, F64),
        _ => return Ok(None),
    }))
}

/// Read a `.npy` file from disk.
///
/// Plain numeric arrays are decoded block by block straight from the file, so
/// the file image is never held in memory next to the decoded array. Other
/// element types go through [`read_bytes`].
pub fn read_file(path: &Path, opts: &ReadOptions) -> Result<Value> {
    let mut f = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut head = vec![0u8; 12];
    f.read_exact(&mut head[..10]).map_err(|_| Error::format("npy file is truncated"))?;
    let (have, hlen) = match head[6] {
        1 => (10, u16::from_le_bytes([head[8], head[9]]) as usize + 10),
        2 | 3 => {
            f.read_exact(&mut head[10..12]).map_err(|_| Error::format("npy file is truncated"))?;
            (12, u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as usize + 12)
        }
        _ => return read_bytes(&std::fs::read(path)?, opts),
    };
    if hlen > 1 << 24 {
        return Err(Error::format("npy header is implausibly large"));
    }
    head.resize(hlen, 0);
    f.read_exact(&mut head[have..]).map_err(|_| Error::format("npy header is truncated"))?;
    let h = parse(&head)?;
    if let Some(n) = h.nbytes() {
        if n > opts.max_alloc {
            return Err(Error::invalid(format!(
                "array needs {n} bytes, over the {} byte max_alloc guard",
                opts.max_alloc
            )));
        }
    }
    // Never trust the header's size over the file's: check that the payload
    // is really there before allocating for it.
    if let Some(n) = h.nbytes() {
        let len = f.get_ref().metadata()?.len() as usize;
        if len < hlen + n {
            return Err(Error::format(format!(
                "npy payload is truncated: header promises {n} bytes, file holds {}",
                len.saturating_sub(hlen)
            )));
        }
    }
    match stream_numeric(&mut f, &h)? {
        Some(mut arr) => {
            if h.fortran_order && h.shape.len() > 1 {
                arr = transpose_from_fortran(arr, &h.shape)?;
            }
            Ok(Value::Array(crate::formats::apply_cast(arr, opts)?))
        }
        None => read_bytes(&std::fs::read(path)?, opts),
    }
}

/// Decode a complete in-memory `.npy` image.
pub fn read_bytes(buf: &[u8], opts: &ReadOptions) -> Result<Value> {
    let h = parse(buf)?;
    decode_payload(&h, &buf[h.data_offset..], opts)
}

/// Decode the payload that follows an already parsed header.
pub fn decode_payload(h: &NpyHeader, payload: &[u8], opts: &ReadOptions) -> Result<Value> {
    if let DType::Struct(fields) = &h.dtype {
        let frame = decode_record(fields, h, payload, opts)?;
        return Ok(Value::Frame(frame));
    }
    if h.dtype == DType::Object {
        // Object arrays are stored as a nested pickle right after the header.
        let v = crate::formats::pickle::read_bytes(payload, opts)?;
        return Ok(v);
    }
    if let Some(n) = h.nbytes() {
        if n > opts.max_alloc {
            return Err(Error::invalid(format!(
                "array needs {n} bytes, over the {} byte max_alloc guard",
                opts.max_alloc
            )));
        }
    }
    let mut arr = from_bytes(&h.dtype, h.endian, &h.shape, payload)?;
    if h.fortran_order && h.shape.len() > 1 {
        arr = transpose_from_fortran(arr, &h.shape)?;
    }
    Ok(Value::Array(crate::formats::apply_cast(arr, opts)?))
}

/// Fortran-ordered payloads are read as the reversed shape, then transposed.
fn transpose_from_fortran(arr: Array, shape: &[usize]) -> Result<Array> {
    let rev: Vec<usize> = shape.iter().rev().copied().collect();
    let mut a = arr;
    a.reshape(&rev)?;
    let axes: Vec<usize> = (0..rev.len()).rev().collect();
    Ok(crate::formats::permute(a, &axes))
}

fn decode_record(
    fields: &[crate::dtype::Field],
    h: &NpyHeader,
    payload: &[u8],
    opts: &ReadOptions,
) -> Result<Frame> {
    let rows: usize = h.shape.iter().product();
    let stride =
        h.dtype.size().ok_or_else(|| Error::unsupported("variable-size structured dtype"))?;
    let mut cols = Vec::with_capacity(fields.len());
    let mut off = 0usize;
    for f in fields {
        let esz =
            f.dtype.size().ok_or_else(|| Error::unsupported("object field in structured dtype"))?;
        let n: usize = f.shape.iter().product::<usize>().max(1);
        let mut buf = Vec::with_capacity(rows * esz * n);
        for r in 0..rows {
            let start = r * stride + off;
            buf.extend_from_slice(
                payload
                    .get(start..start + esz * n)
                    .ok_or_else(|| Error::format("record array is truncated"))?,
            );
        }
        let mut shape = vec![rows];
        shape.extend_from_slice(&f.shape);
        let arr = from_bytes(&f.dtype, h.endian, &shape, &buf)?;
        cols.push(Series::new(f.name.clone(), crate::formats::apply_cast(arr, opts)?));
        off += esz * n;
    }
    Frame::from_columns(cols)
}
