//! Stack, mark and memo bookkeeping for the pickle machine.

use super::machine::Machine;
use crate::error::{Error, Result};
use crate::value::Value;

impl<'a, 'b> Machine<'a, 'b> {
    /// Byte offset reached so far (joblib needs it for alignment padding).
    pub fn offset(&self) -> usize {
        self.cur.pos()
    }

    pub(crate) fn pop(&mut self) -> Result<Value> {
        self.stack.pop().ok_or_else(|| Error::pickle("stack underflow"))
    }

    pub(crate) fn pop_mark(&mut self) -> Result<Vec<Value>> {
        let m = self.marks.pop().ok_or_else(|| Error::pickle("no MARK on the stack"))?;
        if m > self.stack.len() {
            return Err(Error::pickle("corrupt MARK offset"));
        }
        Ok(self.stack.split_off(m))
    }

    pub(super) fn memo_put(&mut self, i: usize, v: Value) {
        if i >= self.memo.len() {
            self.memo.resize(i + 1, None);
        }
        self.memo[i] = Some(v);
    }

    pub(super) fn memo_get(&self, i: usize) -> Result<Value> {
        self.memo
            .get(i)
            .and_then(|x| x.clone())
            .ok_or_else(|| Error::pickle(format!("memo slot {i} is empty")))
    }
}
