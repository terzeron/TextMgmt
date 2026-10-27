//! Dispatch table for `REDUCE` / `BUILD` over an allow-list of safe callables.

use super::np_rebuild as np;
use super::torch_rebuild as pt;
use crate::error::{Error, Result};
use crate::options::ReadOptions;
use crate::value::{Dict, PyObject, Value};

fn qual(v: &Value) -> Option<(String, String)> {
    match v {
        Value::Object(o) if o.args.is_empty() && o.state.is_none() => {
            Some((o.module.clone(), o.name.clone()))
        }
        Value::Object(o) => Some((o.module.clone(), o.name.clone())),
        _ => None,
    }
}

/// Interpret `callable(*args)` without running Python.
pub(crate) fn apply_reduce(callable: Value, args: Vec<Value>, opts: &ReadOptions) -> Result<Value> {
    let (module, name) = match qual(&callable) {
        Some(q) => q,
        None => return Err(Error::pickle("REDUCE on a non-callable")),
    };
    let m = module.as_str();
    let n = name.as_str();
    let numpy_core = matches!(
        m,
        "numpy.core.multiarray" | "numpy._core.multiarray" | "numpy.core._multiarray_umath"
    );
    Ok(match (m, n) {
        _ if numpy_core && n == "_reconstruct" => np::reconstruct_placeholder(),
        _ if numpy_core && n == "scalar" => np::scalar(&args, opts)?,
        ("numpy._core.numeric", "_frombuffer") | ("numpy.core.numeric", "_frombuffer") => {
            np::frombuffer(&args, opts)?
        }
        ("numpy", "dtype") | ("numpy", "_dtype") => np::dtype_placeholder(args),
        ("numpy", "ndarray") => np::reconstruct_placeholder(),
        ("_codecs", "encode") => codecs_encode(&args)?,
        ("collections", "OrderedDict") | ("collections", "defaultdict") => ordered_dict(&args),
        ("builtins", "bytearray") | ("__builtin__", "bytearray") => match args.first() {
            Some(Value::Bytes(b)) => Value::Bytes(b.clone()),
            Some(Value::Str(s)) => Value::Bytes(s.chars().map(|c| c as u8).collect()),
            _ => Value::Bytes(Vec::new()),
        },
        ("builtins", "set") | ("__builtin__", "set") | ("builtins", "frozenset") => {
            match args.into_iter().next() {
                Some(Value::List(v)) | Some(Value::Tuple(v)) | Some(Value::Set(v)) => Value::Set(v),
                _ => Value::Set(Vec::new()),
            }
        }
        ("builtins", "complex") | ("__builtin__", "complex") => {
            let re = args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
            let im = args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            Value::Complex(crate::num::Complex64::new(re, im))
        }
        ("torch._utils", "_rebuild_tensor_v2")
        | ("torch._utils", "_rebuild_tensor")
        | ("torch._utils", "_rebuild_tensor_v3") => pt::rebuild_tensor(&args, opts)?,
        ("torch._utils", "_rebuild_parameter")
        | ("torch._utils", "_rebuild_parameter_with_state") => {
            args.into_iter().next().unwrap_or(Value::None)
        }
        ("torch", "Size") => match args.into_iter().next() {
            Some(v) => v,
            None => Value::Tuple(Vec::new()),
        },
        ("torch", "device") | ("torch", "dtype") => {
            Value::Str(args.first().and_then(|v| v.as_str()).unwrap_or("").to_string())
        }
        _ => {
            if !opts.allow_unknown_globals && is_dangerous(m, n) {
                return Err(Error::pickle(format!(
                    "refusing to interpret `{m}.{n}`; it is not on the allow-list. \
                     Set ReadOptions::allow_unknown_globals if you trust this file."
                )));
            }
            let mut o = PyObject::new(module, name);
            o.args = args;
            Value::Object(o)
        }
    })
}

/// Callables that would be arbitrary code execution in a real unpickler.
fn is_dangerous(module: &str, name: &str) -> bool {
    matches!(
        (module, name),
        ("os", _)
            | ("subprocess", _)
            | ("sys", _)
            | ("shutil", _)
            | ("builtins", "eval")
            | ("builtins", "exec")
            | ("builtins", "compile")
            | ("builtins", "__import__")
            | ("builtins", "getattr")
            | ("__builtin__", "eval")
            | ("__builtin__", "exec")
            | ("posix", _)
            | ("nt", _)
            | ("webbrowser", _)
            | ("pty", _)
            | ("socket", _)
    )
}

fn codecs_encode(args: &[Value]) -> Result<Value> {
    let s = args
        .first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::pickle("_codecs.encode without a string"))?;
    Ok(Value::Bytes(s.chars().map(|c| c as u8).collect()))
}

fn ordered_dict(args: &[Value]) -> Value {
    let mut d = Dict::new();
    if let Some(Value::List(items)) | Some(Value::Tuple(items)) = args.first() {
        for it in items {
            if let Value::Tuple(kv) | Value::List(kv) = it {
                if kv.len() == 2 {
                    d.insert(kv[0].clone(), kv[1].clone());
                }
            }
        }
    }
    Value::Dict(d)
}

/// Interpret `obj.__setstate__(state)`.
pub(crate) fn apply_build(target: Value, state: Value, opts: &ReadOptions) -> Result<Value> {
    match target {
        Value::Object(o) if o.module == np::TAG && o.name == "ndarray" => match &state {
            Value::Tuple(t) | Value::List(t) => np::build_array(t, opts),
            _ => Err(Error::pickle("ndarray BUILD without a state tuple")),
        },
        Value::Object(mut o) if o.module == np::TAG && o.name == "dtype" => {
            o.state = Some(Box::new(state));
            Ok(Value::Object(o))
        }
        Value::Object(mut o) => {
            o.state = Some(Box::new(state));
            Ok(Value::Object(o))
        }
        Value::Dict(mut d) => {
            if let Value::Dict(s) = state {
                for (k, v) in s.into_entries() {
                    d.insert(k, v);
                }
            }
            Ok(Value::Dict(d))
        }
        other => Ok(other),
    }
}
