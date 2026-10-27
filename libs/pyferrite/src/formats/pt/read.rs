//! Reading `torch.save` archives (ZIP layout and the pre-1.6 legacy layout).

use crate::error::{Error, Result};
use crate::formats::pickle;
use crate::options::ReadOptions;
use crate::value::{PyObject, Value};
use crate::zip::ZipReader;
use std::path::Path;

fn storage_key(id: &Value) -> Option<(String, String)> {
    let items = match id {
        Value::Tuple(t) | Value::List(t) => t,
        _ => return None,
    };
    if items.first().and_then(|v| v.as_str()) != Some("storage") {
        return None;
    }
    let cls = match items.get(1)? {
        Value::Object(o) => o.name.clone(),
        Value::Str(s) => s.clone(),
        _ => return None,
    };
    let key = match items.get(2)? {
        Value::Str(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        _ => return None,
    };
    Some((cls, key))
}

/// Decode an in-memory `.pt` archive.
pub fn read_bytes(buf: &[u8], opts: &ReadOptions) -> Result<Value> {
    if !buf.starts_with(b"PK\x03\x04") {
        return legacy(buf, opts);
    }
    let z = ZipReader::new(buf)?;
    let pkl_entry = z
        .entries()
        .iter()
        .find(|e| e.name.ends_with("/data.pkl") || e.name == "data.pkl")
        .ok_or_else(|| Error::format("no data.pkl inside the torch archive"))?
        .clone();
    let prefix = pkl_entry.name.trim_end_matches("data.pkl").to_string();
    let pkl = z.read(&pkl_entry)?;

    let mut load = |id: &Value| -> Result<Value> {
        let (cls, key) =
            storage_key(id).ok_or_else(|| Error::pickle("unrecognised torch persistent id"))?;
        let dt = pickle::storage_dtype(&cls)
            .ok_or_else(|| Error::unsupported(format!("torch storage class `{cls}`")))?;
        let name = format!("{prefix}data/{key}");
        let e = z
            .entry(&name)
            .ok_or_else(|| Error::format(format!("missing storage member `{name}`")))?;
        Ok(pickle::storage_value(dt, z.read(e)?))
    };
    pickle::read_bytes_with(&pkl, opts, &mut load)
}

/// Read a `.pt` file.
pub fn read_path(path: &Path, opts: &ReadOptions) -> Result<Value> {
    read_bytes(&std::fs::read(path)?, opts)
}

/// Pre-1.6 `torch.save`: five concatenated pickles followed by raw storages.
fn legacy(buf: &[u8], opts: &ReadOptions) -> Result<Value> {
    // magic, protocol version, sys_info, then the object pickle.
    let mut off = 0usize;
    let mut skipped = 0;
    while skipped < 3 {
        let end = find_stop(&buf[off..]).ok_or_else(|| Error::format("truncated legacy .pt"))?;
        off += end + 1;
        skipped += 1;
    }
    let mut deferred: Vec<Value> = Vec::new();
    let mut load = |id: &Value| -> Result<Value> {
        deferred.push(id.clone());
        Ok(Value::Object(PyObject {
            module: "__pyferrite__".into(),
            name: "legacy_storage".into(),
            args: vec![id.clone()],
            ..Default::default()
        }))
    };
    let v = pickle::read_bytes_with(&buf[off..], opts, &mut load)?;
    Err(Error::unsupported(format!(
        "legacy (pre-1.6) torch checkpoints are read-only up to the object graph; \
         {} storages were referenced but their payload layout is not supported. \
         Re-save with `torch.save(obj, path, _use_new_zipfile_serialization=True)`. \
         Top-level object: {}",
        deferred.len(),
        v.type_name()
    )))
}

fn find_stop(buf: &[u8]) -> Option<usize> {
    // A pickle always ends with `.`; scanning is safe here because the leading
    // headers torch writes are tiny and contain no framed payloads.
    buf.iter().position(|b| *b == b'.')
}
