//! HDF5 header messages: dataspace, datatype, layout, filters.

use super::superblock::Superblock;
use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};
use crate::util::Cursor;

pub const MSG_DATASPACE: u16 = 0x0001;
pub const MSG_LINK_INFO: u16 = 0x0002;
pub const MSG_DATATYPE: u16 = 0x0003;
pub const MSG_FILL_VALUE: u16 = 0x0005;
pub const MSG_LAYOUT: u16 = 0x0008;
pub const MSG_FILTER: u16 = 0x000B;
pub const MSG_LINK: u16 = 0x0006;
pub const MSG_CONTINUATION: u16 = 0x0010;
pub const MSG_SYMBOL_TABLE: u16 = 0x0011;

/// Where a dataset's raw bytes live.
#[derive(Clone, Debug)]
pub enum Layout {
    /// Data is stored inside the header message itself.
    Compact(Vec<u8>),
    /// One contiguous run at `address`.
    Contiguous { address: u64, size: u64 },
    /// Chunked, indexed by a version-1 B-tree at `address`.
    Chunked { address: u64, dims: Vec<usize>, elem_size: usize },
}

/// Parse a dataspace message into its dimension list.
pub fn dataspace(data: &[u8], sb: &Superblock) -> Result<Vec<usize>> {
    let mut c = Cursor::new(data);
    let version = c.u8()?;
    let rank = c.u8()? as usize;
    let flags = c.u8()?;
    if version == 1 {
        c.skip(5)?;
    } else {
        c.skip(1)?; // type
    }
    let mut dims = Vec::with_capacity(rank);
    for _ in 0..rank {
        dims.push(c.uint(sb.length_size)? as usize);
    }
    let _ = flags;
    Ok(dims)
}

/// Parse a datatype message into `(DType, Endian)`.
pub fn datatype(data: &[u8]) -> Result<(DType, Endian)> {
    let mut c = Cursor::new(data);
    let b0 = c.u8()?;
    let class = b0 & 0x0F;
    let f0 = c.u8()?;
    let f1 = c.u8()?;
    let _f2 = c.u8()?;
    let size = c.u32()? as usize;
    let endian = if f0 & 1 == 1 { Endian::Big } else { Endian::Little };
    Ok(match class {
        0 => {
            // fixed point
            let signed = f0 & 0x08 != 0;
            let dt = match (size, signed) {
                (1, true) => DType::I8,
                (2, true) => DType::I16,
                (4, true) => DType::I32,
                (8, true) => DType::I64,
                (1, false) => DType::U8,
                (2, false) => DType::U16,
                (4, false) => DType::U32,
                (8, false) => DType::U64,
                (s, _) => return Err(Error::unsupported(format!("{s}-byte integer"))),
            };
            (dt, endian)
        }
        1 => {
            let dt = match size {
                2 => DType::F16,
                4 => DType::F32,
                8 => DType::F64,
                16 => DType::F128,
                s => return Err(Error::unsupported(format!("{s}-byte float"))),
            };
            (dt, endian)
        }
        3 => (DType::Bytes(size), Endian::Native),
        4 => (DType::U8, Endian::Native), // bitfield
        8 => {
            // Enumeration. h5py stores numpy booleans as a one-byte enum with
            // the two members FALSE = 0 and TRUE = 1; read that back as bool.
            // Any other enum is read as its base integer type.
            let members = u16::from_le_bytes([f0, f1]);
            let is_bool = size == 1
                && members == 2
                && contains(data, b"FALSE\0")
                && contains(data, b"TRUE\0");
            if is_bool {
                (DType::Bool, Endian::Native)
            } else {
                (
                    match size {
                        1 => DType::U8,
                        2 => DType::U16,
                        4 => DType::U32,
                        _ => DType::U64,
                    },
                    endian,
                )
            }
        }
        9 => (DType::Bytes(size), Endian::Native), // variable-length
        c => return Err(Error::unsupported(format!("HDF5 datatype class {c}"))),
    })
}

/// Parse a data layout message.
pub fn layout(data: &[u8], sb: &Superblock) -> Result<Layout> {
    let mut c = Cursor::new(data);
    let version = c.u8()?;
    match version {
        1 | 2 => {
            let rank = c.u8()? as usize;
            let class = c.u8()?;
            c.skip(5)?;
            let address = if class == 0 { 0 } else { c.uint(sb.offset_size)? };
            let mut dims = Vec::with_capacity(rank);
            for _ in 0..rank {
                dims.push(c.u32()? as usize);
            }
            match class {
                1 => {
                    let size: usize = dims.iter().product();
                    Ok(Layout::Contiguous { address, size: size as u64 })
                }
                2 => {
                    let elem = dims.pop().unwrap_or(1);
                    Ok(Layout::Chunked { address, dims, elem_size: elem })
                }
                _ => {
                    let n = c.u32()? as usize;
                    Ok(Layout::Compact(c.take(n)?.to_vec()))
                }
            }
        }
        3 | 4 => {
            let class = c.u8()?;
            match class {
                0 => {
                    let n = c.u16()? as usize;
                    Ok(Layout::Compact(c.take(n)?.to_vec()))
                }
                1 => {
                    let address = c.uint(sb.offset_size)?;
                    let size = c.uint(sb.length_size)?;
                    Ok(Layout::Contiguous { address, size })
                }
                2 => {
                    let rank = c.u8()? as usize;
                    let address = c.uint(sb.offset_size)?;
                    let mut dims = Vec::with_capacity(rank);
                    for _ in 0..rank {
                        dims.push(c.u32()? as usize);
                    }
                    let elem = dims.pop().unwrap_or(1);
                    Ok(Layout::Chunked { address, dims, elem_size: elem })
                }
                c => Err(Error::unsupported(format!("HDF5 layout class {c}"))),
            }
        }
        v => Err(Error::unsupported(format!("HDF5 layout message version {v}"))),
    }
}

/// Identifiers of the filters in a pipeline, in application order.
pub fn filters(data: &[u8]) -> Result<Vec<u16>> {
    let mut c = Cursor::new(data);
    let version = c.u8()?;
    let n = c.u8()? as usize;
    if version == 1 {
        c.skip(6)?;
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let id = c.u16()?;
        let name_len = if version == 1 || id >= 256 { c.u16()? as usize } else { 0 };
        let _flags = c.u16()?;
        let nvals = c.u16()? as usize;
        if name_len > 0 {
            c.skip(name_len)?;
        }
        c.skip(4 * nvals)?;
        if version == 1 && (4 * nvals) % 8 != 0 {
            c.skip(8 - (4 * nvals) % 8)?;
        }
        out.push(id);
    }
    Ok(out)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
#[path = "../../../../../tests/rust/pyferrite/hdf5_msg_unit.rs"]
mod tests;
