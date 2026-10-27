//! Class-reference, `REDUCE`, `BUILD` and persistent-id opcodes.

use super::machine::Machine;
use super::opcode::*;
use super::reduce::apply_reduce;
use crate::error::{Error, Result};
use crate::value::{PyObject, Value};

impl<'a, 'b> Machine<'a, 'b> {
    pub(crate) fn step_obj(&mut self, op: u8) -> Result<()> {
        match op {
            GLOBAL => {
                let module = self.line()?;
                let name = self.line()?;
                let v = self.build_object(module, name);
                self.stack.push(v);
            }
            STACK_GLOBAL => {
                let name = self.pop()?;
                let module = self.pop()?;
                let (m, n) = (
                    module.as_str().unwrap_or_default().to_string(),
                    name.as_str().unwrap_or_default().to_string(),
                );
                let v = self.build_object(m, n);
                self.stack.push(v);
            }
            REDUCE => {
                let args = self.pop()?;
                let callable = self.pop()?;
                let args = match args {
                    Value::Tuple(t) | Value::List(t) => t,
                    other => vec![other],
                };
                let v = apply_reduce(callable, args, self.opts)?;
                self.stack.push(v);
            }
            NEWOBJ => {
                let args = self.pop()?;
                let cls = self.pop()?;
                let args = match args {
                    Value::Tuple(t) | Value::List(t) => t,
                    other => vec![other],
                };
                let v = apply_reduce(cls, args, self.opts)?;
                self.stack.push(v);
            }
            NEWOBJ_EX => {
                let kwargs = self.pop()?;
                let args = self.pop()?;
                let cls = self.pop()?;
                let args = match args {
                    Value::Tuple(t) | Value::List(t) => t,
                    other => vec![other],
                };
                let mut v = apply_reduce(cls, args, self.opts)?;
                if let (Value::Object(o), Value::Dict(kw)) = (&mut v, kwargs) {
                    o.kwargs = kw;
                }
                self.stack.push(v);
            }
            INST | OBJ => {
                let (cls, args) = if op == INST {
                    let module = self.line()?;
                    let name = self.line()?;
                    (self.build_object(module, name), self.pop_mark()?)
                } else {
                    let mut items = self.pop_mark()?;
                    if items.is_empty() {
                        return Err(Error::pickle("OBJ without a class"));
                    }
                    let cls = items.remove(0);
                    (cls, items)
                };
                let v = apply_reduce(cls, args, self.opts)?;
                self.stack.push(v);
            }
            BUILD => {
                let state = self.pop()?;
                let target = self.pop()?;
                let built = super::reduce::apply_build(target, state, self.opts)?;
                let built = if self.joblib { self.joblib_payload(built)? } else { built };
                self.stack.push(built);
            }
            PERSID => {
                let id = Value::Str(self.line()?);
                let v = self.resolve_persistent(id)?;
                self.stack.push(v);
            }
            BINPERSID => {
                let id = self.pop()?;
                let v = self.resolve_persistent(id)?;
                self.stack.push(v);
            }
            EXT1 | EXT2 | EXT4 => {
                let code = match op {
                    EXT1 => self.cur.u8()? as u32,
                    EXT2 => self.cur.u16()? as u32,
                    _ => self.cur.u32()?,
                };
                self.stack.push(Value::Object(PyObject::new("copyreg", format!("ext{code}"))));
            }
            other => {
                return Err(Error::pickle(format!(
                    "unknown opcode 0x{other:02x} (`{}`) at offset {}",
                    other as char,
                    self.cur.pos() - 1
                )))
            }
        }
        Ok(())
    }

    fn resolve_persistent(&mut self, id: Value) -> Result<Value> {
        match self.persistent.as_mut() {
            Some(f) => f(&id),
            None => Ok(Value::Object(PyObject {
                module: "__persistent__".into(),
                name: "id".into(),
                args: vec![id],
                ..Default::default()
            })),
        }
    }
}
