//! Pass two of the writer: emit the planned file.
//!
//! The subset produced here is deliberately narrow but strictly conformant:
//! superblock version 0, version-1 object headers, symbol-table groups indexed
//! by a version-1 B-tree, and contiguous little-endian datasets. `h5py`,
//! `h5ls` and the reference C library all read it.

use super::encode::{self, Image, UNDEF};
use super::layout_plan::*;
use super::msg::{MSG_DATASPACE, MSG_DATATYPE, MSG_FILL_VALUE, MSG_LAYOUT, MSG_SYMBOL_TABLE};
use super::plan::plan;
use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};
use crate::options::WriteOptions;
use crate::value::{to_bytes, Value};
use std::path::Path;

/// Element type actually written for a source dtype, rejecting what HDF5
/// cannot express in this subset.
pub fn target_dtype(src: &DType, opts: &WriteOptions) -> Result<DType> {
    let t = crate::formats::resolve_target_dtype(src, opts);
    datatype_size(&t)?; // validates
    Ok(t)
}

/// Serialise a value into an in-memory HDF5 image.
pub fn write_bytes(value: &Value, opts: &WriteOptions) -> Result<Vec<u8>> {
    let (root, eof) = plan(value, opts)?;
    let mut img = Image::new(eof as usize);
    superblock(&mut img, &root, eof)?;
    emit(&mut img, &root, opts)?;
    Ok(img.buf)
}

/// Write an `.h5` file.
pub fn write_path(path: &Path, value: &Value, opts: &WriteOptions) -> Result<()> {
    std::fs::write(path, write_bytes(value, opts)?)?;
    Ok(())
}

fn superblock(img: &mut Image, root: &Node<'_>, eof: u64) -> Result<()> {
    let g = match root {
        Node::Group(g) => g,
        Node::Dataset(_) => {
            return Err(Error::invalid(
                "an HDF5 file's root must be a group; wrap the array in a dict",
            ))
        }
    };
    img.put(0, super::superblock::MAGIC);
    img.u8_at(8, 0); // superblock version
    img.u8_at(9, 0); // free space storage version
    img.u8_at(10, 0); // root group symbol table entry version
    img.u8_at(11, 0);
    img.u8_at(12, 0); // shared header message format version
    img.u8_at(13, 8); // size of offsets
    img.u8_at(14, 8); // size of lengths
    img.u8_at(15, 0);
    img.u16_at(16, g.leaf_k as u16); // group leaf node K
    img.u16_at(18, BTREE_INTERNAL_K as u16); // group internal node K
    img.u32_at(20, 0); // file consistency flags
    img.u64_at(24, 0); // base address
    img.u64_at(32, UNDEF); // free space info
    img.u64_at(40, eof); // end of file address
    img.u64_at(48, UNDEF); // driver information block
                           // Root group symbol table entry
    img.u64_at(56, 0); // link name offset: the empty string at heap offset 0
    img.u64_at(64, g.header);
    img.u32_at(72, 1); // cache type: symbol table metadata cached below
    img.u32_at(76, 0);
    img.u64_at(80, g.btree);
    img.u64_at(88, g.heap);
    Ok(())
}

fn emit(img: &mut Image, node: &Node<'_>, opts: &WriteOptions) -> Result<()> {
    match node {
        Node::Group(g) => emit_group(img, g, opts),
        Node::Dataset(d) => emit_dataset(img, d, opts),
    }
}

fn emit_group(img: &mut Image, g: &Group<'_>, opts: &WriteOptions) -> Result<()> {
    let used = encode::object_header(
        img,
        g.header,
        &[(MSG_SYMBOL_TABLE, encode::symbol_table(g.btree, g.heap))],
    );
    debug_assert_eq!(used, GROUP_HEADER);

    // Local heap header, then its data segment.
    img.put(g.heap, b"HEAP");
    img.u8_at(g.heap + 4, 0); // version
    img.u64_at(g.heap + 8, g.heap_size as u64);
    // The library's sentinel for "no free blocks" is 1, not the undefined
    // address: H5HL_FREE_NULL. Writing UNDEF here trips "bad heap free list".
    img.u64_at(g.heap + 16, 1);
    img.u64_at(g.heap + 24, g.heap_data);
    for (i, (name, _)) in g.children.iter().enumerate() {
        img.put(g.heap_data + g.name_offsets[i] as u64, name.as_bytes());
    }
    img.put(g.heap_data + g.sentinel as u64, &[0xFF; 7]);

    // A single-child B-tree pointing at one symbol table node. The keys bracket
    // every name: the empty string at offset 0 sorts below all of them, and the
    // 0xFF sentinel sorts above.
    img.put(g.btree, b"TREE");
    img.u8_at(g.btree + 4, 0); // node type: group
    img.u8_at(g.btree + 5, 0); // level: leaf
    img.u16_at(g.btree + 6, 1); // entries used
    img.u64_at(g.btree + 8, UNDEF); // left sibling
    img.u64_at(g.btree + 16, UNDEF); // right sibling
    img.u64_at(g.btree + 24, 0); // key 0
    img.u64_at(g.btree + 32, g.snod); // child 0
    img.u64_at(g.btree + 40, g.sentinel as u64); // key 1

    img.put(g.snod, b"SNOD");
    img.u8_at(g.snod + 4, 1); // version
    img.u16_at(g.snod + 6, g.children.len() as u16);
    for (i, (_, child)) in g.children.iter().enumerate() {
        let e = g.snod + SNOD_PREFIX as u64 + (i * SNOD_ENTRY) as u64;
        img.u64_at(e, g.name_offsets[i] as u64);
        img.u64_at(e + 8, child.header_address());
        img.u32_at(e + 16, 0); // cache type: nothing cached
    }
    for (_, child) in &g.children {
        emit(img, child, opts)?;
    }
    Ok(())
}

fn emit_dataset(img: &mut Image, d: &Dataset<'_>, opts: &WriteOptions) -> Result<()> {
    let esz = d.dtype.size().unwrap_or(1);
    let msgs = vec![
        (MSG_DATASPACE, encode::dataspace(&d.shape)),
        (MSG_DATATYPE, encode::datatype(&d.dtype)?),
        (MSG_FILL_VALUE, encode::fill_value(esz)),
        (MSG_LAYOUT, encode::layout_contiguous(d.data, d.data_size as u64)),
    ];
    let used = encode::object_header(img, d.header, &msgs);
    debug_assert_eq!(used, d.header_size);

    let cast;
    let arr = if d.array.dtype() == d.dtype {
        d.array
    } else {
        cast = d.array.cast(&d.dtype, opts.cast_policy)?;
        &cast
    };
    let raw = to_bytes(arr, Endian::Little)?;
    if raw.len() != d.data_size {
        return Err(Error::Shape { expected: d.shape.clone(), got: raw.len() / esz.max(1) });
    }
    img.put(d.data, &raw);
    Ok(())
}
