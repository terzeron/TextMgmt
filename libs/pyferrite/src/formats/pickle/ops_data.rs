//! Data-pushing and container-building opcodes.

use super::machine::Machine;
use super::opcode::*;
use super::text::{bigint_from_le, unescape};
use crate::error::{Error, Result};
use crate::value::Value;

impl<'a, 'b> Machine<'a, 'b> {
    pub(crate) fn step_data(&mut self, op: u8) -> Result<()> {
        match op {
            INT => {
                let t = self.line()?;
                let t = t.trim();
                self.stack.push(match t {
                    "01" => Value::Bool(true),
                    "00" => Value::Bool(false),
                    _ => Value::Int(t.parse().map_err(|_| Error::pickle("bad INT"))?),
                });
            }
            BININT => {
                let v = self.cur.i32()?;
                self.stack.push(Value::Int(v as i64));
            }
            BININT1 => {
                let v = self.cur.u8()?;
                self.stack.push(Value::Int(v as i64));
            }
            BININT2 => {
                let v = self.cur.u16()?;
                self.stack.push(Value::Int(v as i64));
            }
            LONG => {
                let t = self.line()?;
                let t = t.trim().trim_end_matches('L');
                self.stack.push(Value::Int(t.parse().unwrap_or(0)));
            }
            LONG1 => {
                let n = self.cur.u8()? as usize;
                let b = self.cur.take(n)?;
                self.stack.push(bigint_from_le(b));
            }
            LONG4 => {
                let n = self.cur.u32()? as usize;
                let b = self.cur.take(n)?;
                self.stack.push(bigint_from_le(b));
            }
            FLOAT => {
                let t = self.line()?;
                self.stack.push(Value::Float(t.trim().parse().unwrap_or(f64::NAN)));
            }
            BINFLOAT => {
                let b = self.cur.take(8)?;
                let mut a = [0u8; 8];
                a.copy_from_slice(b);
                self.stack.push(Value::Float(f64::from_be_bytes(a)));
            }
            STRING => {
                let t = self.line()?;
                let t = t.trim();
                let inner = t.trim_matches(|c| c == '\'' || c == '"');
                self.stack.push(Value::Str(unescape(inner)));
            }
            UNICODE => {
                let t = self.line()?;
                self.stack.push(Value::Str(unescape(&t)));
            }
            BINSTRING | BINUNICODE => {
                let n = self.cur.u32()? as usize;
                self.take_text(n, op == BINSTRING)?;
            }
            SHORT_BINSTRING | SHORT_BINUNICODE => {
                let n = self.cur.u8()? as usize;
                self.take_text(n, op == SHORT_BINSTRING)?;
            }
            BINUNICODE8 => {
                let n = self.cur.u64()? as usize;
                self.take_text(n, false)?;
            }
            BINBYTES => {
                let n = self.cur.u32()? as usize;
                let b = self.cur.take(n)?.to_vec();
                self.stack.push(Value::Bytes(b));
            }
            SHORT_BINBYTES => {
                let n = self.cur.u8()? as usize;
                let b = self.cur.take(n)?.to_vec();
                self.stack.push(Value::Bytes(b));
            }
            BINBYTES8 | BYTEARRAY8 => {
                let n = self.cur.u64()? as usize;
                let b = self.cur.take(n)?.to_vec();
                self.stack.push(Value::Bytes(b));
            }
            EMPTY_TUPLE => self.stack.push(Value::Tuple(Vec::new())),
            EMPTY_LIST => self.stack.push(Value::List(Vec::new())),
            EMPTY_DICT => self.stack.push(Value::Dict(Default::default())),
            EMPTY_SET => self.stack.push(Value::Set(Vec::new())),
            TUPLE1 | TUPLE2 | TUPLE3 => {
                let n = (op - TUPLE1 + 1) as usize;
                let at =
                    self.stack.len().checked_sub(n).ok_or_else(|| Error::pickle("short TUPLE"))?;
                let items = self.stack.split_off(at);
                self.stack.push(Value::Tuple(items));
            }
            TUPLE => {
                let items = self.pop_mark()?;
                self.stack.push(Value::Tuple(items));
            }
            LIST => {
                let items = self.pop_mark()?;
                self.stack.push(Value::List(items));
            }
            DICT => {
                let items = self.pop_mark()?;
                self.stack.push(Value::Dict(Self::dict_from_flat(items)?));
            }
            FROZENSET => {
                let items = self.pop_mark()?;
                self.stack.push(Value::Set(items));
            }
            APPEND => {
                let v = self.pop()?;
                self.append_to_last(vec![v])?;
            }
            APPENDS | ADDITEMS => {
                let items = self.pop_mark()?;
                self.append_to_last(items)?;
            }
            SETITEM => {
                let v = self.pop()?;
                let k = self.pop()?;
                self.setitems(vec![k, v])?;
            }
            SETITEMS => {
                let items = self.pop_mark()?;
                self.setitems(items)?;
            }
            NEXT_BUFFER | READONLY_BUFFER => {
                return Err(Error::unsupported("out-of-band pickle buffers (protocol 5)"))
            }
            _ => return self.step_obj(op),
        }
        Ok(())
    }

    fn take_text(&mut self, n: usize, latin1: bool) -> Result<()> {
        let b = self.cur.take(n)?;
        let s = if latin1 {
            b.iter().map(|c| *c as char).collect()
        } else {
            String::from_utf8_lossy(b).into_owned()
        };
        self.stack.push(Value::Str(s));
        Ok(())
    }

    fn append_to_last(&mut self, items: Vec<Value>) -> Result<()> {
        match self.stack.last_mut() {
            Some(Value::List(l)) | Some(Value::Set(l)) => l.extend(items),
            Some(Value::Object(o)) => o.list_items.extend(items),
            _ => return Err(Error::pickle("APPEND target is not a list")),
        }
        Ok(())
    }

    fn setitems(&mut self, items: Vec<Value>) -> Result<()> {
        let pairs = Self::dict_from_flat(items)?;
        match self.stack.last_mut() {
            Some(Value::Dict(d)) => {
                for (k, v) in pairs.into_entries() {
                    d.insert(k, v);
                }
            }
            Some(Value::Object(o)) => {
                for (k, v) in pairs.into_entries() {
                    o.dict_items.insert(k, v);
                }
            }
            _ => return Err(Error::pickle("SETITEM target is not a dict")),
        }
        Ok(())
    }
}
