//! joblib's out-of-band array payloads.
//!
//! `joblib.dump` pickles a `NumpyArrayWrapper` describing the array and then
//! writes the raw buffer straight into the same stream, so the reader has to
//! consume those bytes at exactly the right moment.

use super::machine::Machine;
use super::np_rebuild::as_dtype;
use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::value::Value;

fn is_wrapper(v: &Value) -> bool {
    matches!(v, Value::Object(o)
        if o.module.starts_with("joblib") && o.name.ends_with("ArrayWrapper"))
}

impl<'a, 'b> Machine<'a, 'b> {
    /// If `built` is a joblib array wrapper, read its payload and return the
    /// resulting array; otherwise pass the value through untouched.
    pub(crate) fn joblib_payload(&mut self, built: Value) -> Result<Value> {
        if !is_wrapper(&built) {
            return Ok(built);
        }
        let o = match &built {
            Value::Object(o) => o,
            _ => unreachable!(),
        };
        let state = o
            .state
            .as_deref()
            .and_then(|s| s.as_dict())
            .ok_or_else(|| Error::pickle("joblib wrapper without a state dict"))?;
        let shape: Vec<usize> = match state.get("shape") {
            Some(Value::Tuple(t)) | Some(Value::List(t)) => {
                t.iter().map(|x| x.as_i64().unwrap_or(0) as usize).collect()
            }
            _ => return Err(Error::pickle("joblib wrapper without a shape")),
        };
        let (dt, endian) = as_dtype(
            state.get("dtype").ok_or_else(|| Error::pickle("joblib wrapper without a dtype"))?,
        )?;
        let fortran = state.get("order").and_then(|v| v.as_str()) == Some("F");

        // Optional alignment padding: one length byte followed by filler.
        if let Some(v) = state.get("numpy_array_alignment_bytes") {
            if !matches!(v, Value::None) {
                let pad = self.cur.u8()? as usize;
                self.cur.skip(pad)?;
            }
        }
        if dt == DType::Object {
            // Object arrays are stored as a nested pickle.
            let rest = &self.cur.buffer()[self.cur.pos()..];
            let mut sub = Machine::new(rest, self.opts);
            let v = sub.run()?;
            let used = sub.offset();
            self.cur.skip(used)?;
            return Ok(v);
        }
        let n: usize = shape.iter().product();
        let esz = dt.size().ok_or_else(|| Error::unsupported("variable-size joblib dtype"))?;
        if n * esz > self.opts.max_alloc {
            return Err(Error::invalid("joblib array exceeds max_alloc"));
        }
        let raw = self.cur.take(n * esz)?;
        super::np_rebuild::decode_raw(dt, endian, shape, fortran, raw, self.opts)
    }
}
