//! Group traversal: local heaps, version-1 B-trees, symbol tables and links.

use super::msg::{MSG_LINK, MSG_SYMBOL_TABLE};
use super::objhdr::Message;
use super::superblock::Superblock;
use crate::error::{Error, Result};
use crate::util::Cursor;

/// A resolved child of a group.
#[derive(Clone, Debug)]
pub struct Link {
    pub name: String,
    pub address: u64,
}

/// List every child of the object header at `addr`.
pub fn children(file: &[u8], sb: &Superblock, msgs: &[Message]) -> Result<Vec<Link>> {
    let mut out = Vec::new();
    for m in msgs {
        match m.kind {
            MSG_SYMBOL_TABLE => {
                let mut c = Cursor::new(&m.data);
                let btree = c.uint(sb.offset_size)?;
                let heap = c.uint(sb.offset_size)?;
                let heap_data = local_heap(file, sb, heap)?;
                walk_btree(file, sb, btree, &heap_data, &mut out)?;
            }
            MSG_LINK => {
                if let Some(l) = parse_link(&m.data, sb)? {
                    out.push(l);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Read the data segment of a local heap.
fn local_heap(file: &[u8], sb: &Superblock, addr: u64) -> Result<Vec<u8>> {
    let p = sb.at(addr);
    let mut c = Cursor::at(file, p);
    if c.take(4)? != b"HEAP" {
        return Err(Error::format("bad local heap signature"));
    }
    c.skip(4)?; // version + reserved
    let size = c.uint(sb.length_size)? as usize;
    let _free = c.uint(sb.length_size)?;
    let data_addr = c.uint(sb.offset_size)?;
    let d = sb.at(data_addr);
    Ok(file.get(d..d + size).unwrap_or_default().to_vec())
}

fn heap_string(heap: &[u8], off: usize) -> String {
    let end = heap[off.min(heap.len())..]
        .iter()
        .position(|b| *b == 0)
        .map(|p| off + p)
        .unwrap_or(heap.len());
    String::from_utf8_lossy(&heap[off.min(heap.len())..end]).into_owned()
}

fn walk_btree(
    file: &[u8],
    sb: &Superblock,
    addr: u64,
    heap: &[u8],
    out: &mut Vec<Link>,
) -> Result<()> {
    if sb.undefined(addr) {
        return Ok(());
    }
    let p = sb.at(addr);
    let mut c = Cursor::at(file, p);
    if c.take(4)? != b"TREE" {
        return Err(Error::format("bad B-tree signature"));
    }
    let node_type = c.u8()?;
    let level = c.u8()?;
    let entries = c.u16()? as usize;
    c.skip(2 * sb.offset_size)?; // left / right siblings
    if node_type != 0 {
        return Ok(());
    }
    let mut kids = Vec::with_capacity(entries);
    for _ in 0..entries {
        c.skip(sb.length_size)?; // key
        kids.push(c.uint(sb.offset_size)?);
    }
    for k in kids {
        if level > 0 {
            walk_btree(file, sb, k, heap, out)?;
        } else {
            symbol_table_node(file, sb, k, heap, out)?;
        }
    }
    Ok(())
}

fn symbol_table_node(
    file: &[u8],
    sb: &Superblock,
    addr: u64,
    heap: &[u8],
    out: &mut Vec<Link>,
) -> Result<()> {
    let p = sb.at(addr);
    let mut c = Cursor::at(file, p);
    if c.take(4)? != b"SNOD" {
        return Err(Error::format("bad symbol table node signature"));
    }
    c.skip(2)?;
    let n = c.u16()? as usize;
    for _ in 0..n {
        let name_off = c.uint(sb.offset_size)? as usize;
        let header = c.uint(sb.offset_size)?;
        let _cache = c.u32()?;
        c.skip(4)?;
        c.skip(16)?; // scratch pad
        out.push(Link { name: heap_string(heap, name_off), address: header });
    }
    Ok(())
}

fn parse_link(data: &[u8], sb: &Superblock) -> Result<Option<Link>> {
    let mut c = Cursor::new(data);
    let _version = c.u8()?;
    let flags = c.u8()?;
    if flags & 0x08 != 0 {
        c.skip(1)?; // link type
    }
    if flags & 0x04 != 0 {
        c.skip(8)?; // creation order
    }
    if flags & 0x10 != 0 {
        c.skip(1)?; // charset
    }
    let len_size = 1usize << (flags & 0x03);
    let n = c.uint(len_size)? as usize;
    let name = String::from_utf8_lossy(c.take(n)?).into_owned();
    if flags & 0x08 != 0 {
        return Ok(None); // soft / external link
    }
    let address = c.uint(sb.offset_size)?;
    Ok(Some(Link { name, address }))
}

#[cfg(test)]
#[path = "../../../../../tests/rust/pyferrite/hdf5_group_unit.rs"]
mod tests;
