//! Byte-level emission helpers for the HDF5 writer.

use super::layout_plan::align8;
use crate::dtype::DType;
use crate::error::{Error, Result};

/// Undefined-address sentinel for an 8-byte offset field.
pub const UNDEF: u64 = u64::MAX;

/// A fixed-size file image written through absolute addresses.
pub struct Image {
    pub buf: Vec<u8>,
}

impl Image {
    pub fn new(size: usize) -> Self {
        Image { buf: vec![0u8; size] }
    }
    pub fn put(&mut self, at: u64, bytes: &[u8]) {
        let a = at as usize;
        self.buf[a..a + bytes.len()].copy_from_slice(bytes);
    }
    pub fn u8_at(&mut self, at: u64, v: u8) {
        self.buf[at as usize] = v;
    }
    pub fn u16_at(&mut self, at: u64, v: u16) {
        self.put(at, &v.to_le_bytes());
    }
    pub fn u32_at(&mut self, at: u64, v: u32) {
        self.put(at, &v.to_le_bytes());
    }
    pub fn u64_at(&mut self, at: u64, v: u64) {
        self.put(at, &v.to_le_bytes());
    }
}

/// Encode a datatype message body.
pub fn datatype(dt: &DType) -> Result<Vec<u8>> {
    let esz = dt.size().ok_or_else(|| Error::unsupported("variable-size HDF5 dtype"))? as u32;
    let mut v = Vec::with_capacity(20);
    let signed = matches!(dt, DType::I8 | DType::I16 | DType::I32 | DType::I64);
    match dt {
        DType::Bool
        | DType::I8
        | DType::I16
        | DType::I32
        | DType::I64
        | DType::U8
        | DType::U16
        | DType::U32
        | DType::U64 => {
            v.push(0x10); // version 1, class 0 (fixed point)
            v.push(if signed { 0x08 } else { 0x00 }); // little endian, sign bit
            v.extend_from_slice(&[0, 0]);
            v.extend_from_slice(&esz.to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes()); // bit offset
            v.extend_from_slice(&((esz * 8) as u16).to_le_bytes()); // precision
        }
        DType::F16 | DType::F32 | DType::F64 => {
            let (prec, exp_loc, exp_size, mant_size, bias) = match dt {
                DType::F16 => (16u16, 10u8, 5u8, 10u8, 15u32),
                DType::F32 => (32, 23, 8, 23, 127),
                _ => (64, 52, 11, 52, 1023),
            };
            v.push(0x11); // version 1, class 1 (floating point)
            v.push(0x20); // little endian, mantissa normalisation = implied MSB
            v.push((prec - 1) as u8); // sign bit location
            v.push(0);
            v.extend_from_slice(&esz.to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes()); // bit offset
            v.extend_from_slice(&prec.to_le_bytes());
            v.push(exp_loc);
            v.push(exp_size);
            v.push(0); // mantissa location
            v.push(mant_size);
            v.extend_from_slice(&bias.to_le_bytes());
        }
        DType::Bytes(_) | DType::Str(_) => {
            v.push(0x13); // version 1, class 3 (string)
            v.push(0x00); // null-terminated, ASCII
            v.extend_from_slice(&[0, 0]);
            v.extend_from_slice(&esz.to_le_bytes());
        }
        other => return Err(Error::unsupported(format!("HDF5 datatype for {other:?}"))),
    }
    Ok(v)
}

/// Encode a version-1 dataspace message body.
pub fn dataspace(shape: &[usize]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + 8 * shape.len());
    v.push(1); // version
    v.push(shape.len() as u8);
    v.push(0); // flags: no maximum dimensions, no permutation index
    v.extend_from_slice(&[0; 5]);
    for d in shape {
        v.extend_from_slice(&(*d as u64).to_le_bytes());
    }
    v
}

/// Encode a version-3 fill value message body: all-zero bytes.
pub fn fill_value(esz: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(6 + esz);
    v.push(3); // version
    v.push(0x22); // late allocation, fill value defined
    v.extend_from_slice(&(esz as u32).to_le_bytes());
    v.extend(std::iter::repeat(0u8).take(esz));
    v
}

/// Encode a version-3 contiguous data layout message body.
pub fn layout_contiguous(address: u64, size: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(18);
    v.push(3); // version
    v.push(1); // class: contiguous
    v.extend_from_slice(&address.to_le_bytes());
    v.extend_from_slice(&size.to_le_bytes());
    v
}

/// Encode a symbol table message body.
pub fn symbol_table(btree: u64, heap: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(16);
    v.extend_from_slice(&btree.to_le_bytes());
    v.extend_from_slice(&heap.to_le_bytes());
    v
}

/// Write a version-1 object header at `addr`.
///
/// Returns the number of bytes consumed, which must match what the planner
/// reserved — the caller asserts this so layout drift fails loudly.
pub fn object_header(img: &mut Image, addr: u64, msgs: &[(u16, Vec<u8>)]) -> usize {
    let body: usize = msgs.iter().map(|(_, d)| 8 + align8(d.len())).sum();
    img.u8_at(addr, 1); // version
    img.u8_at(addr + 1, 0);
    img.u16_at(addr + 2, msgs.len() as u16);
    img.u32_at(addr + 4, 1); // reference count
    img.u32_at(addr + 8, body as u32);
    // bytes 12..16 are padding that aligns the message block to 8
    let mut p = addr + 16;
    for (kind, data) in msgs {
        img.u16_at(p, *kind);
        img.u16_at(p + 2, align8(data.len()) as u16);
        img.u8_at(p + 4, 0); // flags
        img.put(p + 8, data);
        p += 8 + align8(data.len()) as u64;
    }
    16 + body
}

#[cfg(test)]
#[path = "../../../../../tests/rust/pyferrite/hdf5_encode_unit.rs"]
mod tests;
