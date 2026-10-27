//! Pickle protocols 0-5: a stack machine that never executes Python.

mod joblib_hook;
mod machine;
mod machine_state;
mod np_rebuild;
mod opcode;
mod ops_data;
mod ops_obj;
mod reduce;
mod text;
mod torch_rebuild;
mod write;
mod write_np;

pub use machine::{Machine, PersistentLoad};
pub use torch_rebuild::{storage_dtype, storage_value};
pub use write::{write_bytes, Pickler};
pub use write_np::torch_storage_name;

use crate::error::Result;
use crate::options::{ReadOptions, WriteOptions};
use crate::value::Value;
use std::path::Path;

/// Decode a pickle stream held in memory.
pub fn read_bytes(buf: &[u8], opts: &ReadOptions) -> Result<Value> {
    machine::Machine::new(buf, opts).run()
}

/// Decode a pickle stream, resolving persistent ids through `f`.
///
/// `torch.save` and `joblib.dump` both park their array payloads out of band
/// and reference them by persistent id; this is the hook that fetches them.
pub fn read_bytes_with(
    buf: &[u8],
    opts: &ReadOptions,
    f: &mut dyn FnMut(&Value) -> Result<Value>,
) -> Result<Value> {
    machine::Machine::new(buf, opts).with_persistent(f).run()
}

/// Read a `.pkl` file.
pub fn read_path(path: &Path, opts: &ReadOptions) -> Result<Value> {
    let buf = std::fs::read(path)?;
    read_bytes(&buf, opts)
}

/// Write a `.pkl` file.
pub fn write_path(path: &Path, value: &Value, opts: &WriteOptions) -> Result<()> {
    std::fs::write(path, write_bytes(value, opts)?)?;
    Ok(())
}
