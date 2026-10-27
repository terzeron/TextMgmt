//! The pickle stack machine.
//!
//! No Python is ever executed: `REDUCE` and `BUILD` are interpreted for a
//! curated allow-list (numpy, torch, `collections`, `_codecs`) and every other
//! callable is captured structurally as a [`PyObject`].

use super::opcode::*;
use crate::error::{Error, Result};
use crate::options::ReadOptions;
use crate::util::Cursor;
use crate::value::{Dict, PyObject, Value};

/// Resolves torch/joblib persistent ids to real payloads.
pub type PersistentLoad<'a> = &'a mut dyn FnMut(&Value) -> Result<Value>;

/// The pickle virtual machine: a stack, a mark stack and a memo.
///
/// It reads opcodes and builds [`Value`]s. It has no mechanism for calling
/// anything, so a pickle can never execute code through it.
pub struct Machine<'a, 'b> {
    /// Cursor over the pickle stream.
    pub cur: Cursor<'a>,
    /// Operand stack.
    pub stack: Vec<Value>,
    /// The marks of this value.
    pub marks: Vec<usize>,
    /// The memo of this value.
    pub memo: Vec<Option<Value>>,
    /// The opts of this value.
    pub opts: &'b ReadOptions,
    /// The persistent of this value.
    pub persistent: Option<PersistentLoad<'b>>,
    /// The proto of this value.
    pub proto: u8,
    /// Enables joblib's inline out-of-band array payloads.
    pub joblib: bool,
}

impl<'a, 'b> Machine<'a, 'b> {
    /// Create a new value.
    pub fn new(buf: &'a [u8], opts: &'b ReadOptions) -> Self {
        Machine {
            cur: Cursor::new(buf),
            stack: Vec::with_capacity(64),
            marks: Vec::new(),
            memo: Vec::new(),
            opts,
            persistent: None,
            proto: 2,
            joblib: false,
        }
    }

    /// With persistent.
    pub fn with_persistent(mut self, f: PersistentLoad<'b>) -> Self {
        self.persistent = Some(f);
        self
    }

    /// Turn on joblib mode: `NumpyArrayWrapper` objects consume the raw array
    /// bytes that follow them in the stream.
    pub fn with_joblib(mut self) -> Self {
        self.joblib = true;
        self
    }

    /// Run until `STOP`, returning the value left on the stack.
    pub fn run(&mut self) -> Result<Value> {
        loop {
            let op = self.cur.u8()?;
            if op == STOP {
                return self.pop();
            }
            self.step(op)?;
        }
    }

    fn step(&mut self, op: u8) -> Result<()> {
        match op {
            PROTO => self.proto = self.cur.u8()?,
            FRAME => {
                self.cur.skip(8)?;
            }
            MARK => self.marks.push(self.stack.len()),
            NONE => self.stack.push(Value::None),
            NEWTRUE => self.stack.push(Value::Bool(true)),
            NEWFALSE => self.stack.push(Value::Bool(false)),
            POP => {
                if self.marks.last() == Some(&self.stack.len()) {
                    self.marks.pop();
                } else {
                    self.pop()?;
                }
            }
            POP_MARK => {
                self.pop_mark()?;
            }
            DUP => {
                let v = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::pickle("DUP on empty stack"))?;
                self.stack.push(v);
            }
            MEMOIZE => {
                let v = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::pickle("MEMOIZE on empty stack"))?;
                let i = self.memo.len();
                self.memo_put(i, v);
            }
            PUT => {
                let i: usize =
                    self.line()?.trim().parse().map_err(|_| Error::pickle("bad PUT index"))?;
                let v = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::pickle("PUT on empty stack"))?;
                self.memo_put(i, v);
            }
            BINPUT => {
                let i = self.cur.u8()? as usize;
                let v = self.stack.last().cloned().unwrap_or(Value::None);
                self.memo_put(i, v);
            }
            LONG_BINPUT => {
                let i = self.cur.u32()? as usize;
                let v = self.stack.last().cloned().unwrap_or(Value::None);
                self.memo_put(i, v);
            }
            GET => {
                let i: usize =
                    self.line()?.trim().parse().map_err(|_| Error::pickle("bad GET index"))?;
                let v = self.memo_get(i)?;
                self.stack.push(v);
            }
            BINGET => {
                let i = self.cur.u8()? as usize;
                let v = self.memo_get(i)?;
                self.stack.push(v);
            }
            LONG_BINGET => {
                let i = self.cur.u32()? as usize;
                let v = self.memo_get(i)?;
                self.stack.push(v);
            }
            _ => return self.step_data(op),
        }
        Ok(())
    }

    /// Read a newline-terminated token (protocol 0 opcodes).
    pub(crate) fn line(&mut self) -> Result<String> {
        let start = self.cur.pos();
        let buf = self.cur.buffer();
        let mut i = start;
        while i < buf.len() && buf[i] != b'\n' {
            i += 1;
        }
        let s = String::from_utf8_lossy(&buf[start..i]).into_owned();
        self.cur.set_pos((i + 1).min(buf.len()));
        Ok(s)
    }

    pub(crate) fn build_object(&mut self, module: String, name: String) -> Value {
        Value::Object(PyObject::new(module, name))
    }

    pub(crate) fn dict_from_flat(items: Vec<Value>) -> Result<Dict> {
        if items.len() % 2 != 0 {
            return Err(Error::pickle("odd number of items for a dict"));
        }
        let mut d = Dict::with_capacity(items.len() / 2);
        let mut it = items.into_iter();
        while let (Some(k), Some(v)) = (it.next(), it.next()) {
            d.insert(k, v);
        }
        Ok(d)
    }
}
