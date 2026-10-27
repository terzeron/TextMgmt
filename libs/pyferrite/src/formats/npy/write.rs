//! Encoding a [`Value`] into a `.npy` image.

use super::header::render;
use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};
use crate::options::WriteOptions;
use crate::value::{to_bytes, to_bytes_x87, write_bytes_to, Array, Value};
use std::borrow::Cow;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Resolve the array, target dtype and target shape implied by `opts`.
pub fn prepare(value: &Value, opts: &WriteOptions) -> Result<(Array, DType, Vec<usize>)> {
    let (arr, target, shape) = prepare_ref(value, opts)?;
    Ok((arr.into_owned(), target, shape))
}

/// As [`prepare`], but borrows the caller's array when no cast or reshape is
/// needed, so writing a large array does not copy it first.
fn prepare_ref<'a>(
    value: &'a Value,
    opts: &WriteOptions,
) -> Result<(Cow<'a, Array>, DType, Vec<usize>)> {
    let arr = match value {
        Value::Array(a) => Cow::Borrowed(a),
        other => Cow::Owned(crate::formats::value_to_array(other)?),
    };
    let target = crate::formats::resolve_target_dtype(&arr.dtype(), opts);
    let mut arr =
        if arr.dtype() == target { arr } else { Cow::Owned(arr.cast(&target, opts.cast_policy)?) };
    let shape = match &opts.shape {
        Some(s) => {
            let want: usize = s.iter().product();
            if want != arr.len() {
                return Err(Error::Shape { expected: s.clone(), got: arr.len() });
            }
            if arr.shape() != s.as_slice() {
                arr.to_mut().reshape(s)?;
            }
            s.clone()
        }
        None => arr.shape().to_vec(),
    };
    Ok((arr, target, shape))
}

/// Serialise a value as a complete `.npy` image.
pub fn write_bytes(value: &Value, opts: &WriteOptions) -> Result<Vec<u8>> {
    // A frame becomes a record array, which is what numpy reads it back as.
    if let Value::Frame(f) = value {
        let (dtype, shape, payload) = super::record::encode(f)?;
        let mut out = render(&dtype, Endian::Little, &shape, false)?;
        out.extend_from_slice(&payload);
        return Ok(out);
    }
    let (arr, target, shape) = prepare_ref(value, opts)?;
    let endian = Endian::Little;
    let mut out = render(&target, endian, &shape, opts.fortran_order)?;
    // Column-major storage is a property of the *bytes*, not of the array the
    // caller handed us. Reversing the axes before serialising means numpy
    // reads back the same logical array, just stored the other way round —
    // flagging the header alone would transpose it.
    let arr = if opts.fortran_order && arr.ndim() > 1 {
        let axes: Vec<usize> = (0..arr.ndim()).rev().collect();
        Cow::Owned(crate::formats::permute(arr.into_owned(), &axes))
    } else {
        arr
    };
    let payload = if target == DType::X87 { to_bytes_x87(&arr)? } else { to_bytes(&arr, endian)? };
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Write a `.npy` file. Every check (container fit, cast, shape) runs before
/// the file is created, then the payload is streamed block by block so the
/// encoded array is never held in memory twice.
pub(super) fn write_file(path: &Path, value: &Value, opts: &WriteOptions) -> Result<()> {
    if let Value::Frame(_) = value {
        std::fs::write(path, write_bytes(value, opts)?)?;
        return Ok(());
    }
    let (arr, target, shape) = prepare_ref(value, opts)?;
    let endian = Endian::Little;
    let header = render(&target, endian, &shape, opts.fortran_order)?;
    let arr = if opts.fortran_order && arr.ndim() > 1 {
        let axes: Vec<usize> = (0..arr.ndim()).rev().collect();
        Cow::Owned(crate::formats::permute(arr.into_owned(), &axes))
    } else {
        arr
    };
    let x87 = if target == DType::X87 { Some(to_bytes_x87(&arr)?) } else { None };
    let mut f = BufWriter::new(File::create(path)?);
    f.write_all(&header)?;
    match x87 {
        Some(b) => f.write_all(&b)?,
        None => write_bytes_to(&arr, endian, &mut f)?,
    }
    f.flush()?;
    Ok(())
}
