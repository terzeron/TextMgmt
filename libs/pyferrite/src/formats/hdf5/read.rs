//! Turning an HDF5 file into a nested [`Value`].

use super::dataset;
use super::group;
use super::objhdr;
use super::superblock::Superblock;
use crate::error::Result;
use crate::options::ReadOptions;
use crate::value::{from_bytes, Dict, Value};
use std::path::Path;

/// Decode an in-memory HDF5 image.
pub fn read_bytes(buf: &[u8], opts: &ReadOptions) -> Result<Value> {
    let sb = Superblock::parse(buf)?;
    node(buf, &sb, sb.root_address, opts, 0)
}

/// Read an `.h5` file.
pub fn read_path(path: &Path, opts: &ReadOptions) -> Result<Value> {
    read_bytes(&std::fs::read(path)?, opts)
}

/// `true` when the header describes a group, even an empty one.
fn is_group(msgs: &[objhdr::Message]) -> bool {
    use super::msg::{MSG_LINK, MSG_LINK_INFO, MSG_SYMBOL_TABLE};
    msgs.iter().any(|m| matches!(m.kind, MSG_LINK_INFO | MSG_LINK | MSG_SYMBOL_TABLE))
}

fn node(buf: &[u8], sb: &Superblock, addr: u64, opts: &ReadOptions, depth: usize) -> Result<Value> {
    if depth > 64 {
        return Ok(Value::None);
    }
    let msgs = objhdr::parse(buf, sb, addr)?;
    if let Some(d) = dataset::info(&msgs, sb)? {
        let esz = d.dtype.size().unwrap_or(1);
        let n: usize = d.shape.iter().product();
        if n * esz > opts.max_alloc {
            return Err(crate::Error::invalid("HDF5 dataset exceeds max_alloc"));
        }
        let raw = dataset::raw_bytes(buf, sb, &d)?;
        let arr = from_bytes(&d.dtype, d.endian, &d.shape, &raw)?;
        return Ok(Value::Array(crate::formats::apply_cast(arr, opts)?));
    }
    let links = group::children(buf, sb, &msgs)?;
    if links.is_empty() && !msgs.is_empty() && !is_group(&msgs) {
        return Err(crate::Error::unsupported(format!(
            "HDF5 object at address {addr} is neither a dataset nor a group \
             (header messages: {:?})",
            msgs.iter().map(|m| m.kind).collect::<Vec<_>>()
        )));
    }
    let mut out = Dict::new();
    for link in links {
        if sb.undefined(link.address) {
            continue;
        }
        let v = node(buf, sb, link.address, opts, depth + 1)?;
        out.insert(Value::Str(link.name), v);
    }
    Ok(Value::Dict(out))
}

/// Report which superblock version an HDF5 image uses, without decoding it.
///
/// Useful when a file fails to read and you want to know what you are dealing
/// with: versions 0 and 1 are the classic layout, 2 and 3 the newer one.
///
/// ```no_run
/// let bytes = std::fs::read("model.h5")?;
/// println!("superblock v{}", pyferrite::formats::hdf5::format_version(&bytes)?);
/// # Ok::<(), pyferrite::Error>(())
/// ```
pub fn format_version(buf: &[u8]) -> Result<u8> {
    Ok(Superblock::parse(buf)?.format_version())
}

/// One [`index`] entry: dataset path, shape, element type and data address.
pub type DatasetEntry = (String, Vec<usize>, crate::dtype::DType, u64);

/// List `(path, shape, dtype, address)` for every dataset without loading any payload.
pub fn index(buf: &[u8]) -> Result<Vec<DatasetEntry>> {
    let sb = Superblock::parse(buf)?;
    let mut out = Vec::new();
    collect(buf, &sb, sb.root_address, String::new(), &mut out, 0)?;
    Ok(out)
}

fn collect(
    buf: &[u8],
    sb: &Superblock,
    addr: u64,
    prefix: String,
    out: &mut Vec<(String, Vec<usize>, crate::dtype::DType, u64)>,
    depth: usize,
) -> Result<()> {
    if depth > 64 {
        return Ok(());
    }
    let msgs = objhdr::parse(buf, sb, addr)?;
    if let Some(d) = dataset::info(&msgs, sb)? {
        out.push((prefix, d.shape, d.dtype, addr));
        return Ok(());
    }
    for link in group::children(buf, sb, &msgs)? {
        if sb.undefined(link.address) {
            continue;
        }
        let p =
            if prefix.is_empty() { link.name.clone() } else { format!("{prefix}/{}", link.name) };
        collect(buf, sb, link.address, p, out, depth + 1)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../../tests/rust/pyferrite/hdf5_read_unit.rs"]
mod tests;
