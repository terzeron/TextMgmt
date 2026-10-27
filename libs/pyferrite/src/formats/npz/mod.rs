//! `.npz` — a ZIP archive whose members are `.npy` images.

mod lazy;

pub use lazy::NpzChunkReader;

use crate::error::{Error, Result};
use crate::formats::npy;
use crate::options::{ReadOptions, WriteOptions};
use crate::value::{Dict, Value};
use crate::zip::{ZipReader, ZipWriter};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Decode an in-memory `.npz` archive into a dict of arrays.
pub fn read_bytes(buf: &[u8], opts: &ReadOptions) -> Result<Value> {
    let z = ZipReader::new(buf)?;
    let mut d = Dict::new();
    for e in z.entries() {
        if !e.name.ends_with(".npy") {
            continue;
        }
        let name = e.name.trim_end_matches(".npy").to_string();
        let raw = z.read(e)?;
        nest(&mut d, &name, npy::read_bytes(&raw, opts)?);
    }
    Ok(Value::Dict(d))
}

/// Insert `v` at a `/`-separated path, creating intermediate dicts.
fn nest(root: &mut Dict, path: &str, v: Value) {
    match path.split_once('/') {
        None => {
            root.insert(Value::Str(path.to_string()), v);
        }
        Some((head, tail)) => {
            let key = Value::Str(head.to_string());
            if !matches!(root.get(head), Some(Value::Dict(_))) {
                root.insert(key.clone(), Value::Dict(Dict::new()));
            }
            if let Some(Value::Dict(child)) = root.get_mut(head) {
                nest(child, tail, v);
            }
        }
    }
}

/// Read a `.npz` file.
pub fn read_path(path: &Path, opts: &ReadOptions) -> Result<Value> {
    read_bytes(&std::fs::read(path)?, opts)
}

/// Write a dict (or a single array) as a `.npz` archive.
///
/// `numpy.savez` names anonymous arrays `arr_0`, `arr_1`, ...; so does this.
/// The archive is assembled in memory first, so a member that fails to
/// encode leaves no partial file behind.
pub fn write_path(path: &Path, value: &Value, opts: &WriteOptions) -> Result<()> {
    let mut members = Vec::new();
    flatten(value, String::new(), &mut members)?;
    let mut z = ZipWriter::new(Vec::new());
    for (name, v) in members {
        z.add(&format!("{name}.npy"), &npy::write_bytes(v, opts)?, opts.compression)?;
    }
    let image = z.finish()?;
    let mut file = BufWriter::new(File::create(path)?);
    file.write_all(&image)?;
    file.flush()?;
    Ok(())
}

/// Flatten a value into `(member name, array)` pairs.
///
/// `numpy.savez` has no notion of nesting, so nested dicts are joined with `/`
/// — `np.load(...)["grp/inner"]` reads them straight back, and `read_path`
/// rebuilds the nesting on the way in.
fn flatten<'a>(v: &'a Value, prefix: String, out: &mut Vec<(String, &'a Value)>) -> Result<()> {
    let join = |p: &str, k: &str| if p.is_empty() { k.to_string() } else { format!("{p}/{k}") };
    match v {
        Value::Dict(d) => {
            for (k, val) in d.iter() {
                let name =
                    k.as_str().ok_or_else(|| Error::invalid("npz member names must be strings"))?;
                if name.contains('/') {
                    return Err(Error::invalid(format!(
                        "`/` is the nesting separator and cannot appear in the key `{name}`"
                    )));
                }
                flatten(val, join(&prefix, name), out)?;
            }
            Ok(())
        }
        Value::List(items) | Value::Tuple(items) if prefix.is_empty() => {
            for (i, val) in items.iter().enumerate() {
                flatten(val, format!("arr_{i}"), out)?;
            }
            Ok(())
        }
        single => {
            out.push((if prefix.is_empty() { "arr_0".into() } else { prefix }, single));
            Ok(())
        }
    }
}
