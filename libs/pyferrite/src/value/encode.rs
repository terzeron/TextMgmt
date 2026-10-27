//! [`Array`] -> raw buffer encoder (the mirror image of `codec.rs`).

use super::Array;
use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};
use crate::num::encode_x87;
use std::io::Write;

fn push<const N: usize>(out: &mut Vec<u8>, mut b: [u8; N], swap: bool) {
    if swap {
        b.reverse();
    }
    out.extend_from_slice(&b);
}

/// Append every element's bytes, choosing the byte order once rather than per
/// element so the native-order loop can vectorise.
macro_rules! put_all {
    ($out:ident, $x:ident, $swap:ident, |$v:ident| $bytes:expr) => {
        // A contiguous C-order array is walked as a flat slice, which avoids
        // the per-element index arithmetic of a dynamic-dimension iterator.
        match ($x.as_slice(), $swap) {
            (Some(s), false) => s.iter().for_each(|$v| $out.extend_from_slice(&$bytes)),
            (Some(s), true) => s.iter().for_each(|$v| {
                let mut b = $bytes;
                b.reverse();
                $out.extend_from_slice(&b);
            }),
            (None, _) => $x.iter().for_each(|$v| push(&mut $out, $bytes, $swap)),
        }
    };
}

/// Serialise an array into a densely packed C-order buffer.
///
/// The array **must** already carry the requested element type; call
/// [`Array::cast`] first if it does not.
pub fn to_bytes(a: &Array, endian: Endian) -> Result<Vec<u8>> {
    let swap = endian.needs_swap();
    let dt = a.dtype();
    let mut out = Vec::with_capacity(a.len() * dt.size().unwrap_or(8));
    match a {
        Array::Bool(x) => out.extend(x.iter().map(|v| *v as u8)),
        Array::I8(x) => out.extend(x.iter().map(|v| *v as u8)),
        Array::U8(x) => out.extend(x.iter().copied()),
        Array::F8E4M3(x) => out.extend(x.iter().map(|v| v.0)),
        Array::F8E5M2(x) => out.extend(x.iter().map(|v| v.0)),
        Array::I16(x) => put_all!(out, x, swap, |v| v.to_le_bytes()),
        Array::U16(x) => put_all!(out, x, swap, |v| v.to_le_bytes()),
        Array::I32(x) => put_all!(out, x, swap, |v| v.to_le_bytes()),
        Array::U32(x) => put_all!(out, x, swap, |v| v.to_le_bytes()),
        Array::I64(x) => put_all!(out, x, swap, |v| v.to_le_bytes()),
        Array::U64(x) => put_all!(out, x, swap, |v| v.to_le_bytes()),
        Array::F16(x) => put_all!(out, x, swap, |v| v.0.to_le_bytes()),
        Array::BF16(x) => put_all!(out, x, swap, |v| v.0.to_le_bytes()),
        Array::F32(x) => put_all!(out, x, swap, |v| v.to_le_bytes()),
        Array::F64(x) => put_all!(out, x, swap, |v| v.to_le_bytes()),
        Array::F128(x) => put_all!(out, x, swap, |v| v.0.to_le_bytes()),
        Array::C64(x) => x.iter().for_each(|v| {
            push(&mut out, v.re.to_le_bytes(), swap);
            push(&mut out, v.im.to_le_bytes(), swap);
        }),
        Array::C128(x) => x.iter().for_each(|v| {
            push(&mut out, v.re.to_le_bytes(), swap);
            push(&mut out, v.im.to_le_bytes(), swap);
        }),
        Array::Str(x) => {
            let width = match dt {
                DType::Str(n) => n.max(1),
                _ => 1,
            };
            for s in x.iter() {
                let mut n = 0;
                for c in s.chars().take(width) {
                    push(&mut out, (c as u32).to_le_bytes(), swap);
                    n += 1;
                }
                out.extend(std::iter::repeat(0u8).take((width - n) * 4));
            }
        }
        Array::Bytes(x) => {
            let width = match dt {
                DType::Bytes(n) => n,
                _ => 0,
            };
            for b in x.iter() {
                let take = b.len().min(width);
                out.extend_from_slice(&b[..take]);
                out.extend(std::iter::repeat(0u8).take(width - take));
            }
        }
    }
    Ok(out)
}

/// Serialise as x87 80-bit extended (numpy `longdouble`), 16 bytes per element.
pub fn to_bytes_x87(a: &Array) -> Result<Vec<u8>> {
    let q = match a {
        Array::F128(x) => x.clone(),
        other => other
            .cast(&DType::F128, crate::options::CastPolicy::Saturate)?
            .into_f128()
            .ok_or_else(|| Error::cast("cannot view as f128"))?,
    };
    let mut out = Vec::with_capacity(q.len() * 16);
    for v in q.iter() {
        out.extend_from_slice(&encode_x87(*v));
        out.extend_from_slice(&[0u8; 6]);
    }
    Ok(out)
}

/// Elements encoded per block by [`write_bytes_to`].
const BLOCK: usize = 1 << 16;

macro_rules! stream_slice {
    ($x:ident, $w:ident, $swap:ident, $n:expr, |$v:ident| $bytes:expr) => {{
        if let Some(s) = $x.as_slice() {
            let mut buf = vec![0u8; BLOCK.min(s.len()) * $n];
            for chunk in s.chunks(BLOCK) {
                let out = &mut buf[..chunk.len() * $n];
                for (dst, $v) in out.chunks_exact_mut($n).zip(chunk) {
                    let mut b = $bytes;
                    if $swap {
                        b.reverse();
                    }
                    dst.copy_from_slice(&b);
                }
                $w.write_all(out)?;
            }
            return Ok(());
        }
    }};
}

/// Stream an array's bytes into `w` block by block, without materialising
/// the whole encoded buffer. Equivalent to `w.write_all(&to_bytes(a, endian)?)`.
pub fn write_bytes_to<W: Write>(a: &Array, endian: Endian, w: &mut W) -> Result<()> {
    let swap = endian.needs_swap();
    match a {
        Array::I16(x) => stream_slice!(x, w, swap, 2, |v| v.to_le_bytes()),
        Array::U16(x) => stream_slice!(x, w, swap, 2, |v| v.to_le_bytes()),
        Array::I32(x) => stream_slice!(x, w, swap, 4, |v| v.to_le_bytes()),
        Array::U32(x) => stream_slice!(x, w, swap, 4, |v| v.to_le_bytes()),
        Array::I64(x) => stream_slice!(x, w, swap, 8, |v| v.to_le_bytes()),
        Array::U64(x) => stream_slice!(x, w, swap, 8, |v| v.to_le_bytes()),
        Array::F32(x) => stream_slice!(x, w, swap, 4, |v| v.to_le_bytes()),
        Array::F64(x) => stream_slice!(x, w, swap, 8, |v| v.to_le_bytes()),
        Array::U8(x) => {
            if let Some(s) = x.as_slice() {
                w.write_all(s)?;
                return Ok(());
            }
        }
        _ => {}
    }
    w.write_all(&to_bytes(a, endian)?)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/rust/pyferrite/value_encode_unit.rs"]
mod tests;
