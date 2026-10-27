//! The five functions most users need: [`read()`], [`write()`], [`read_lazy()`],
//! [`write_lazy()`] and [`convert()`].

use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::formats::{detect_from_path, hdf5, joblib, npy, npz, pickle, pt, Format};
use crate::lazy::{LazyReader, LazyWriter};
use crate::options::{ReadOptions, WriteOptions};
use crate::value::{Array, Value};
use std::path::Path;

/// Read any supported file with default options.
///
/// ```no_run
/// let mut weights = pyferrite::read("model.pt")?;
/// if let Some(dict) = weights.as_dict_mut() {
///     for (_name, v) in dict.iter_mut() {
///         if let Some(a) = v.as_array_mut().and_then(|a| a.as_f32_mut()) {
///             a.iter_mut().for_each(|x| *x *= 0.5); // owned data, so mutable
///         }
///     }
/// }
/// # Ok::<(), pyferrite::Error>(())
/// ```
pub fn read<P: AsRef<Path>>(path: P) -> Result<Value> {
    read_with(path, &ReadOptions::default())
}

/// Read any supported file, choosing casts, laziness and safety limits.
pub fn read_with<P: AsRef<Path>>(path: P, opts: &ReadOptions) -> Result<Value> {
    let path = path.as_ref();
    let fmt = match opts.format {
        Some(f) => f,
        None => detect_from_path(path)?,
    };
    match fmt {
        Format::Npy => npy::read_path(path, opts),
        Format::Npz => npz::read_path(path, opts),
        Format::Pickle => pickle::read_path(path, opts),
        Format::Pt => pt::read_path(path, opts),
        Format::Joblib => joblib::read_path(path, opts),
        Format::Hdf5 => hdf5::read_path(path, opts),
    }
}

/// Write a value with default options; the format comes from the extension.
pub fn write<P: AsRef<Path>>(path: P, value: &Value) -> Result<()> {
    write_with(path, value, &WriteOptions::default())
}

/// Write a value, choosing the target dtype, shape, compression and format.
pub fn write_with<P: AsRef<Path>>(path: P, value: &Value, opts: &WriteOptions) -> Result<()> {
    let path = path.as_ref();
    let fmt = match opts.format {
        Some(f) => f,
        None => Format::from_path(path).ok_or_else(|| {
            Error::invalid(format!(
                "cannot infer a format from `{}`; set WriteOptions::format",
                path.display()
            ))
        })?,
    };
    validate(value, fmt)?;
    match fmt {
        Format::Npy => npy::write_path(path, value, opts),
        Format::Npz => npz::write_path(path, value, opts),
        Format::Pickle => pickle::write_path(path, value, opts),
        Format::Pt => pt::write_path(path, value, opts),
        Format::Joblib => joblib::write_path(path, value, opts),
        Format::Hdf5 => hdf5::write_path(path, value, opts),
    }
}

/// Reject values a format cannot represent *before* touching the disk.
///
/// This is the guard the design brief asked for: a `.npy` file must hold an
/// array, so writing a bare string into one fails with a clear message instead
/// of producing a file numpy cannot open.
fn validate(value: &Value, fmt: Format) -> Result<()> {
    if fmt == Format::Npy {
        match value {
            Value::Array(_)
            | Value::List(_)
            | Value::Tuple(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Bool(_)
            | Value::Frame(_) => Ok(()),
            other => Err(Error::invalid(format!(
                "`.npy` stores a single array; `{}` is not one. \
                 Use `.npz`, `.pkl` or `.h5` for containers.",
                other.type_name()
            ))),
        }
    } else {
        Ok(())
    }
}

/// Open a file for chunk-at-a-time reading.
pub fn read_lazy<P: AsRef<Path>>(path: P, opts: &ReadOptions) -> Result<LazyReader> {
    let path = path.as_ref();
    let fmt = match opts.format {
        Some(f) => f,
        None => detect_from_path(path)?,
    };
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    Ok(match fmt {
        Format::Npy => LazyReader::new(Box::new(npy::NpyChunkReader::open(path, &name, opts)?)),
        Format::Npz => LazyReader::new(Box::new(npz::NpzChunkReader::open(path, opts)?)),
        Format::Hdf5 => LazyReader::new(Box::new(hdf5::Hdf5ChunkReader::open(path, opts)?)),
        other => {
            return Err(Error::unsupported(format!(
                "lazy reading of {other:?} (its payload is interleaved with the object graph); \
                 read it eagerly instead"
            )))
        }
    })
}

/// Open a file for chunk-at-a-time writing.
///
/// The full `shape` and `dtype` must be declared upfront so the header can be
/// written before any data arrives.
pub fn write_lazy<P: AsRef<Path>>(
    path: P,
    dtype: DType,
    shape: &[usize],
    opts: &WriteOptions,
) -> Result<LazyWriter> {
    let path = path.as_ref();
    let fmt = match opts.format {
        Some(f) => f,
        None => Format::from_path(path)
            .ok_or_else(|| Error::invalid("cannot infer a format from the path"))?,
    };
    Ok(match fmt {
        Format::Npy => {
            LazyWriter::new(Box::new(npy::NpyChunkWriter::create(path, dtype, shape, opts)?))
        }
        Format::Hdf5 => {
            LazyWriter::new(Box::new(hdf5::Hdf5ChunkWriter::create(path, dtype, shape, opts)?))
        }
        other => {
            return Err(Error::unsupported(format!(
                "lazy writing of {other:?}; use `.npy` or `.h5`"
            )))
        }
    })
}

/// Read one file and write it back out in another format in a single call.
pub fn convert<P: AsRef<Path>, Q: AsRef<Path>>(
    src: P,
    dst: Q,
    r: &ReadOptions,
    w: &WriteOptions,
) -> Result<()> {
    let v = read_with(src, r)?;
    write_with(dst, &v, w)
}

/// Collect every array in a decoded value into `(path, array)` pairs.
///
/// Nested dicts join their keys with `.`, so a torch `state_dict` comes back as
/// `("layer1.weight", ...)`, matching what Python prints.
pub fn flatten_arrays(v: &Value) -> Vec<(String, &Array)> {
    let mut out = Vec::new();
    walk(v, String::new(), &mut out);
    out
}

fn walk<'a>(v: &'a Value, prefix: String, out: &mut Vec<(String, &'a Array)>) {
    let join = |p: &str, k: &str| if p.is_empty() { k.to_string() } else { format!("{p}.{k}") };
    match v {
        Value::Array(a) => out.push((prefix, a)),
        Value::Dict(d) => {
            for (k, val) in d.iter() {
                let key = k.as_str().map(|s| s.to_string()).unwrap_or_else(|| format!("{k:?}"));
                walk(val, join(&prefix, &key), out);
            }
        }
        Value::List(items) | Value::Tuple(items) => {
            for (i, val) in items.iter().enumerate() {
                walk(val, join(&prefix, &i.to_string()), out);
            }
        }
        Value::Frame(f) => {
            for c in &f.columns {
                out.push((join(&prefix, &c.name), &c.values));
            }
        }
        Value::Object(o) => {
            if let Some(s) = o.state.as_deref() {
                walk(s, prefix, out);
            }
        }
        _ => {}
    }
}
