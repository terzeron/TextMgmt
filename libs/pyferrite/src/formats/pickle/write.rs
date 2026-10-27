//! The pickler: [`Value`] -> a pickle stream Python can load unmodified.

use super::opcode::*;
use crate::dtype::DType;
use crate::error::Result;
use crate::options::WriteOptions;
use crate::value::Value;

/// Incremental pickle emitter.
pub struct Pickler {
    pub(crate) out: Vec<u8>,
    proto: u8,
    memo: usize,
    /// When set, arrays are emitted as torch persistent-id storages instead of
    /// numpy `_reconstruct` calls, and their bytes are collected here.
    pub torch_mode: bool,
    /// `(key, dtype, raw bytes)` for every storage emitted in torch mode.
    pub storages: Vec<(String, DType, Vec<u8>)>,
}

impl Pickler {
    /// Start a stream with the given protocol (clamped to 2..=5).
    pub fn new(proto: u8) -> Self {
        let proto = proto.clamp(2, 5);
        let mut out = Vec::with_capacity(256);
        out.push(PROTO);
        out.push(proto);
        Pickler { out, proto, memo: 0, torch_mode: false, storages: Vec::new() }
    }

    /// Finish the stream and return the bytes.
    pub fn finish(mut self) -> Vec<u8> {
        self.out.push(STOP);
        self.out
    }

    pub(crate) fn memoize(&mut self) {
        if self.proto >= 4 {
            self.out.push(MEMOIZE);
        } else if self.memo < 256 {
            self.out.push(BINPUT);
            self.out.push(self.memo as u8);
        } else {
            self.out.push(LONG_BINPUT);
            self.out.extend_from_slice(&(self.memo as u32).to_le_bytes());
        }
        self.memo += 1;
    }

    pub(crate) fn int(&mut self, v: i64) {
        if (0..256).contains(&v) {
            self.out.push(BININT1);
            self.out.push(v as u8);
        } else if (0..65536).contains(&v) {
            self.out.push(BININT2);
            self.out.extend_from_slice(&(v as u16).to_le_bytes());
        } else if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
            self.out.push(BININT);
            self.out.extend_from_slice(&(v as i32).to_le_bytes());
        } else {
            let b = v.to_le_bytes();
            self.out.push(LONG1);
            self.out.push(8);
            self.out.extend_from_slice(&b);
        }
    }

    pub(crate) fn text(&mut self, s: &str) {
        let b = s.as_bytes();
        if self.proto >= 4 && b.len() < 256 {
            self.out.push(SHORT_BINUNICODE);
            self.out.push(b.len() as u8);
        } else {
            self.out.push(BINUNICODE);
            self.out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        }
        self.out.extend_from_slice(b);
    }

    pub(crate) fn bytes(&mut self, b: &[u8]) {
        if b.len() < 256 {
            self.out.push(SHORT_BINBYTES);
            self.out.push(b.len() as u8);
        } else if self.proto >= 4 {
            self.out.push(BINBYTES8);
            self.out.extend_from_slice(&(b.len() as u64).to_le_bytes());
        } else {
            self.out.push(BINBYTES);
            self.out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        }
        self.out.extend_from_slice(b);
    }

    pub(crate) fn global(&mut self, module: &str, name: &str) {
        if self.proto >= 4 {
            self.text(module);
            self.text(name);
            self.out.push(STACK_GLOBAL);
        } else {
            self.out.push(GLOBAL);
            self.out.extend_from_slice(module.as_bytes());
            self.out.push(b'\n');
            self.out.extend_from_slice(name.as_bytes());
            self.out.push(b'\n');
        }
        self.memoize();
    }

    /// Emit any [`Value`].
    pub fn dump(&mut self, v: &Value, opts: &WriteOptions) -> Result<()> {
        match v {
            Value::None => self.out.push(NONE),
            Value::Bool(b) => self.out.push(if *b { NEWTRUE } else { NEWFALSE }),
            Value::Int(i) => self.int(*i),
            Value::BigInt(b) => {
                self.out.push(LONG4);
                self.out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                self.out.extend_from_slice(b);
            }
            Value::Float(f) => {
                self.out.push(BINFLOAT);
                self.out.extend_from_slice(&f.to_be_bytes());
            }
            Value::Complex(c) => {
                self.global("builtins", "complex");
                self.out.push(MARK);
                self.dump(&Value::Float(c.re), opts)?;
                self.dump(&Value::Float(c.im), opts)?;
                self.out.push(TUPLE);
                self.out.push(REDUCE);
            }
            Value::Str(s) => self.text(s),
            Value::Bytes(b) => self.bytes(b),
            Value::Tuple(items) => {
                self.out.push(MARK);
                for i in items {
                    self.dump(i, opts)?;
                }
                self.out.push(TUPLE);
            }
            Value::List(items) | Value::Set(items) => {
                self.out.push(EMPTY_LIST);
                self.memoize();
                self.out.push(MARK);
                for i in items {
                    self.dump(i, opts)?;
                }
                self.out.push(APPENDS);
            }
            Value::Dict(d) => {
                self.out.push(EMPTY_DICT);
                self.memoize();
                self.out.push(MARK);
                for (k, val) in d.iter() {
                    self.dump(k, opts)?;
                    self.dump(val, opts)?;
                }
                self.out.push(SETITEMS);
            }
            Value::Array(a) => self.dump_array(a, opts)?,
            Value::Frame(f) => {
                // Frames are emitted as a plain dict of columns, which both
                // pandas and polars can ingest with one constructor call.
                let mut d = crate::value::Dict::new();
                for c in &f.columns {
                    d.insert(Value::Str(c.name.clone()), Value::Array(c.values.clone()));
                }
                self.dump(&Value::Dict(d), opts)?;
            }
            Value::Object(o) => {
                self.global(&o.module, &o.name);
                self.out.push(MARK);
                for a in &o.args {
                    self.dump(a, opts)?;
                }
                self.out.push(TUPLE);
                self.out.push(REDUCE);
                self.memoize();
                if let Some(state) = o.state.as_deref() {
                    self.dump(state, opts)?;
                    self.out.push(BUILD);
                }
            }
        }
        Ok(())
    }
}

/// Serialise a value as a standalone pickle stream.
pub fn write_bytes(value: &Value, opts: &WriteOptions) -> Result<Vec<u8>> {
    let mut p = Pickler::new(opts.pickle_protocol);
    p.dump(value, opts)?;
    Ok(p.finish())
}
