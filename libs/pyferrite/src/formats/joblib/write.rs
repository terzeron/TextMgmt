//! Writing `joblib.dump`-compatible files.
//!
//! Arrays are emitted as `joblib.numpy_pickle.NumpyArrayWrapper` objects
//! followed by their raw buffer, which is exactly what joblib's own
//! `NumpyPickler` produces, so `joblib.load` reads these files back.

use crate::dtype::{to_descr, DType, Endian};
use crate::error::Result;
use crate::formats::pickle::Pickler;
use crate::options::{Compression, WriteOptions};
use crate::util::compress_zlib;
use crate::value::{to_bytes, Array, Dict, PyObject, Value};
use std::path::Path;

fn wrapper(a: &Array, dt: &DType) -> Result<Value> {
    let mut state = Dict::new();
    state.insert(Value::Str("subclass".into()), Value::Tuple(vec![]));
    state.insert(
        Value::Str("shape".into()),
        Value::Tuple(a.shape().iter().map(|d| Value::Int(*d as i64)).collect()),
    );
    state.insert(Value::Str("order".into()), Value::Str("C".into()));
    state.insert(Value::Str("dtype".into()), Value::Str(to_descr(dt, Endian::Little)?));
    state.insert(Value::Str("allow_mmap".into()), Value::Bool(true));
    state.insert(Value::Str("numpy_array_alignment_bytes".into()), Value::None);
    let mut o = PyObject::new("joblib.numpy_pickle", "NumpyArrayWrapper");
    o.args = vec![];
    o.state = Some(Box::new(Value::Dict(state)));
    Ok(Value::Object(o))
}

fn emit(p: &mut Pickler, v: &Value, opts: &WriteOptions) -> Result<()> {
    match v {
        Value::Array(a) => {
            let target = crate::formats::resolve_target_dtype(&a.dtype(), opts);
            let cast;
            let a = if a.dtype() == target {
                a
            } else {
                cast = a.cast(&target, opts.cast_policy)?;
                &cast
            };
            p.dump(&wrapper(a, &target)?, opts)?;
            let raw = to_bytes(a, Endian::Little)?;
            p.out.extend_from_slice(&raw);
        }
        other => p.dump(other, opts)?,
    }
    Ok(())
}

/// Serialise a value into a `.joblib` image.
///
/// Only top-level values and one level of dict/list nesting keep the
/// out-of-band layout; deeper arrays are pickled inline as numpy arrays, which
/// joblib also accepts.
pub fn write_bytes(value: &Value, opts: &WriteOptions) -> Result<Vec<u8>> {
    let mut p = Pickler::new(opts.pickle_protocol);
    emit(&mut p, value, opts)?;
    let raw = p.finish();
    Ok(match opts.compression {
        Compression::None => raw,
        Compression::Deflate(l) | Compression::Zlib(l) => compress_zlib(&raw, l)?,
    })
}

/// Write a `.joblib` file.
pub fn write_path(path: &Path, value: &Value, opts: &WriteOptions) -> Result<()> {
    std::fs::write(path, write_bytes(value, opts)?)?;
    Ok(())
}
