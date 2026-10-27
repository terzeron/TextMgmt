//! Address assignment for the writer.
//!
//! HDF5 puts every structure at an absolute file address, and object headers
//! reference addresses that come *later* in the file. So the writer runs two
//! passes: this module lays the whole file out on paper, then `write.rs` emits
//! bytes into the reserved slots.

use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::value::Array;

/// Fixed structure sizes for an 8-byte-offset, 8-byte-length file.
pub const SUPERBLOCK: usize = 96;
pub const GROUP_HEADER: usize = 40;
pub const HEAP_HEADER: usize = 32;
pub const SNOD_PREFIX: usize = 8;
pub const SNOD_ENTRY: usize = 40;
pub const BTREE_INTERNAL_K: usize = 16;

/// Round `n` up to the next multiple of 8.
pub fn align8(n: usize) -> usize {
    (n + 7) & !7
}

/// Size of a B-tree node given the internal node arity.
pub fn btree_size(k: usize) -> usize {
    24 + (2 * k + 1) * 8 + (2 * k) * 8
}

/// The planned tree, mirroring the `Value` being written.
pub enum Node<'a> {
    Group(Group<'a>),
    Dataset(Dataset<'a>),
}

/// A group and the addresses of the structures that implement it.
pub struct Group<'a> {
    pub children: Vec<(String, Node<'a>)>,
    pub leaf_k: usize,
    pub header: u64,
    pub heap: u64,
    pub heap_data: u64,
    pub heap_size: usize,
    pub btree: u64,
    pub snod: u64,
    /// Heap offset of each child name, in sorted order.
    pub name_offsets: Vec<usize>,
    /// Heap offset of the 0xFF sentinel that terminates the B-tree key range.
    pub sentinel: usize,
}

/// A dataset and where its header and payload live.
pub struct Dataset<'a> {
    pub array: &'a Array,
    pub dtype: DType,
    pub shape: Vec<usize>,
    pub header: u64,
    pub header_size: usize,
    pub data: u64,
    pub data_size: usize,
}

/// Byte length of a datatype message body for `dt`.
pub fn datatype_size(dt: &DType) -> Result<usize> {
    Ok(match dt {
        DType::Bool
        | DType::I8
        | DType::I16
        | DType::I32
        | DType::I64
        | DType::U8
        | DType::U16
        | DType::U32
        | DType::U64 => 12,
        DType::F16 | DType::F32 | DType::F64 => 20,
        DType::Bytes(_) | DType::Str(_) => 8,
        other => {
            return Err(Error::unsupported(format!(
                "writing `{other:?}` to HDF5; cast to an integer, float or byte-string dtype first"
            )))
        }
    })
}

/// Byte length of a dataset object header, messages included.
pub fn dataset_header_size(dt: &DType, rank: usize) -> Result<usize> {
    let esz = dt.size().ok_or_else(|| Error::unsupported("variable-size HDF5 dtype"))?;
    let dataspace = 8 + 8 * rank;
    let datatype = datatype_size(dt)?;
    let fill = 2 + 4 + esz;
    let layout = 18;
    Ok(16 + [dataspace, datatype, fill, layout].iter().map(|s| 8 + align8(*s)).sum::<usize>())
}

/// Heap data-segment size for a set of child names, plus each name's offset.
///
/// Offset 0 always holds the empty string, which the root group's symbol table
/// entry points at and which doubles as the B-tree's lower bound.
pub fn plan_heap(names: &[String]) -> (usize, Vec<usize>, usize) {
    let mut offsets = Vec::with_capacity(names.len());
    let mut cursor = 8usize;
    for n in names {
        offsets.push(cursor);
        cursor += align8(n.len() + 1);
    }
    let sentinel = cursor;
    cursor += 8; // seven 0xFF bytes and a NUL: greater than any UTF-8 name
    (cursor, offsets, sentinel)
}
