//! A tiny parser for the Python literal subset used by `.npy` headers.
//!
//! `{'descr': '<f8', 'fortran_order': False, 'shape': (2, 3), }` is a Python
//! `dict` repr, so decoding it needs a (very small) literal parser. Only the
//! constructs numpy actually emits are accepted: dicts, lists, tuples,
//! strings, integers, floats, `True`, `False`, `None`.

use crate::error::{Error, Result};
use crate::value::{Dict, Value};

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<()> {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(Error::format(format!("expected `{}` at offset {}", c as char, self.i)))
        }
    }
    fn string(&mut self) -> Result<String> {
        let q = self.b[self.i];
        self.i += 1;
        let mut s = String::new();
        while self.i < self.b.len() && self.b[self.i] != q {
            if self.b[self.i] == b'\\' && self.i + 1 < self.b.len() {
                self.i += 1;
                s.push(match self.b[self.i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    o => o as char,
                });
            } else {
                s.push(self.b[self.i] as char);
            }
            self.i += 1;
        }
        self.i += 1;
        Ok(s)
    }
    fn value(&mut self) -> Result<Value> {
        self.ws();
        let c = *self.b.get(self.i).ok_or_else(|| Error::format("truncated literal"))?;
        match c {
            b'\'' | b'"' => Ok(Value::Str(self.string()?)),
            b'{' => {
                self.i += 1;
                let mut d = Dict::new();
                loop {
                    self.ws();
                    if self.b.get(self.i) == Some(&b'}') {
                        self.i += 1;
                        break;
                    }
                    let k = self.value()?;
                    self.eat(b':')?;
                    let v = self.value()?;
                    d.insert(k, v);
                    self.ws();
                    if self.b.get(self.i) == Some(&b',') {
                        self.i += 1;
                    }
                }
                Ok(Value::Dict(d))
            }
            b'[' | b'(' => {
                let close = if c == b'[' { b']' } else { b')' };
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.b.get(self.i) == Some(&close) {
                        self.i += 1;
                        break;
                    }
                    items.push(self.value()?);
                    self.ws();
                    if self.b.get(self.i) == Some(&b',') {
                        self.i += 1;
                    }
                }
                Ok(if c == b'[' { Value::List(items) } else { Value::Tuple(items) })
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len()
                    && !matches!(self.b[self.i], b',' | b'}' | b']' | b')' | b':')
                    && !(self.b[self.i] as char).is_whitespace()
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.b[start..self.i]).unwrap_or("").trim();
                Ok(match t {
                    "True" => Value::Bool(true),
                    "False" => Value::Bool(false),
                    "None" => Value::None,
                    _ => {
                        if let Ok(i) = t.parse::<i64>() {
                            Value::Int(i)
                        } else if let Ok(f) = t.parse::<f64>() {
                            Value::Float(f)
                        } else {
                            Value::Str(t.to_string())
                        }
                    }
                })
            }
        }
    }
}

/// Parse a Python literal expression into a [`Value`].
pub fn parse_python_literal(s: &str) -> Result<Value> {
    let mut p = P { b: s.as_bytes(), i: 0 };
    p.value()
}
