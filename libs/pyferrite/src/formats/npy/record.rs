//! Writing a [`Frame`] as a numpy record (structured) array.
//!
//! This is the inverse of the read path, which turns a structured `.npy` into a
//! `Frame`. numpy stores records row-major with the fields of one row adjacent,
//! so the columns have to be interleaved.

use crate::dtype::{DType, Endian, Field};
use crate::error::{Error, Result};
use crate::value::{to_bytes, Frame};

/// Build the structured dtype and interleaved payload for a frame.
pub fn encode(frame: &Frame) -> Result<(DType, Vec<usize>, Vec<u8>)> {
    if frame.columns.is_empty() {
        return Err(Error::invalid("cannot write a frame with no columns"));
    }
    let rows = frame.height();

    let mut fields = Vec::with_capacity(frame.columns.len());
    let mut widths = Vec::with_capacity(frame.columns.len());
    let mut buffers = Vec::with_capacity(frame.columns.len());
    for c in &frame.columns {
        let dt = c.values.dtype();
        let w = dt.size().ok_or_else(|| {
            Error::unsupported(format!(
                "column `{}` has the variable-size type {dt:?}, which a record array cannot hold",
                c.name
            ))
        })?;
        if c.values.len() != rows {
            return Err(Error::Shape { expected: vec![rows], got: c.values.len() });
        }
        fields.push(Field { name: c.name.clone(), dtype: dt, shape: Vec::new() });
        widths.push(w);
        buffers.push(to_bytes(&c.values, Endian::Little)?);
    }

    let stride: usize = widths.iter().sum();
    let mut out = vec![0u8; stride * rows];
    for r in 0..rows {
        let mut at = r * stride;
        for (col, w) in widths.iter().enumerate() {
            out[at..at + w].copy_from_slice(&buffers[col][r * w..(r + 1) * w]);
            at += w;
        }
    }
    Ok((DType::Struct(fields), vec![rows], out))
}
