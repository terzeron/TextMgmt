//! Emitting numpy arrays and torch tensors from the pickler.
//!
//! numpy's own pickling is mimicked exactly — `_reconstruct` followed by a
//! `BUILD` carrying `(version, shape, dtype, is_fortran, rawdata)` — so
//! `pickle.load` yields a real `np.ndarray` rather than a surrogate.

use super::opcode::*;
use super::write::Pickler;
use crate::dtype::{to_descr, DType, Endian};
use crate::error::{Error, Result};
use crate::options::WriteOptions;
use crate::value::{to_bytes, to_bytes_x87, Array, Value};

impl Pickler {
    /// Emit an ndarray as a torch tensor backed by an out-of-band storage.
    fn dump_tensor(&mut self, a: &Array, shape: &[usize], dt: &DType) -> Result<()> {
        let key = self.storages.len().to_string();
        let raw = to_bytes(a, Endian::Little)?;
        let storage_name = torch_storage_name(dt)?;
        self.storages.push((key.clone(), dt.clone(), raw));
        self.global("torch._utils", "_rebuild_tensor_v2");
        self.out.push(MARK);
        // persistent id: ('storage', torch.<T>Storage, key, 'cpu', numel)
        self.out.push(MARK);
        self.text("storage");
        self.global("torch", storage_name);
        self.text(&key);
        self.text("cpu");
        self.int(a.len() as i64);
        self.out.push(TUPLE);
        self.out.push(BINPERSID);
        self.int(0);
        self.out.push(MARK);
        for d in shape {
            self.int(*d as i64);
        }
        self.out.push(TUPLE);
        self.out.push(MARK);
        for s in c_strides(shape) {
            self.int(s as i64);
        }
        self.out.push(TUPLE);
        self.out.push(NEWFALSE);
        self.global("collections", "OrderedDict");
        self.out.push(EMPTY_TUPLE);
        self.out.push(REDUCE);
        self.out.push(TUPLE);
        self.out.push(REDUCE);
        self.memoize();
        Ok(())
    }

    /// Emit an ndarray exactly the way numpy pickles one.
    pub fn dump_array(&mut self, a: &Array, opts: &WriteOptions) -> Result<()> {
        let target = crate::formats::resolve_target_dtype(&a.dtype(), opts);
        let cast;
        let a = if a.dtype() == target {
            a
        } else {
            cast = a.cast(&target, opts.cast_policy)?;
            &cast
        };
        let shape: Vec<usize> = match &opts.shape {
            Some(s) => s.clone(),
            None => a.shape().to_vec(),
        };
        if shape.iter().product::<usize>() != a.len() {
            return Err(Error::Shape { expected: shape, got: a.len() });
        }
        if self.torch_mode {
            return self.dump_tensor(a, &shape, &target);
        }
        self.global("numpy.core.multiarray", "_reconstruct");
        self.global("numpy", "ndarray");
        self.int(0);
        self.out.push(TUPLE1);
        self.bytes(b"b");
        self.out.push(TUPLE3);
        self.out.push(REDUCE);
        self.memoize();
        // state = (1, shape, dtype, is_fortran, rawdata)
        self.out.push(MARK);
        self.int(1);
        self.dump(&Value::Tuple(shape.iter().map(|d| Value::Int(*d as i64)).collect()), opts)?;
        self.dump_dtype(&target)?;
        self.out.push(NEWFALSE);
        let raw =
            if target == DType::X87 { to_bytes_x87(a)? } else { to_bytes(a, Endian::Little)? };
        self.bytes(&raw);
        self.out.push(TUPLE);
        self.out.push(BUILD);
        Ok(())
    }

    fn dump_dtype(&mut self, dt: &DType) -> Result<()> {
        let descr = to_descr(dt, Endian::Little)?;
        let (order, name) = descr.split_at(1);
        self.global("numpy", "dtype");
        self.out.push(MARK);
        self.text(name);
        self.out.push(NEWFALSE);
        self.out.push(NEWTRUE);
        self.out.push(TUPLE);
        self.out.push(REDUCE);
        self.memoize();
        // state = (3, byteorder, None, None, None, -1, -1, 0)
        self.out.push(MARK);
        self.int(3);
        self.text(if order == "|" { "|" } else { order });
        for _ in 0..3 {
            self.out.push(NONE);
        }
        self.out.push(BININT);
        self.out.extend_from_slice(&(-1i32).to_le_bytes());
        self.out.push(BININT);
        self.out.extend_from_slice(&(-1i32).to_le_bytes());
        self.int(0);
        self.out.push(TUPLE);
        self.out.push(BUILD);
        Ok(())
    }
}

fn c_strides(shape: &[usize]) -> Vec<usize> {
    let mut s = vec![1usize; shape.len()];
    for i in (0..shape.len().saturating_sub(1)).rev() {
        s[i] = s[i + 1] * shape[i + 1];
    }
    s
}

/// torch storage class name for an element type.
pub fn torch_storage_name(dt: &DType) -> Result<&'static str> {
    Ok(match dt {
        DType::F32 => "FloatStorage",
        DType::F64 => "DoubleStorage",
        DType::F16 => "HalfStorage",
        DType::BF16 => "BFloat16Storage",
        DType::I64 => "LongStorage",
        DType::I32 => "IntStorage",
        DType::I16 => "ShortStorage",
        DType::I8 => "CharStorage",
        DType::U8 => "ByteStorage",
        DType::Bool => "BoolStorage",
        DType::C64 => "ComplexFloatStorage",
        DType::C128 => "ComplexDoubleStorage",
        other => return Err(Error::unsupported(format!("torch storage for {other:?}"))),
    })
}
