//! `.npy` — a single numpy array with a self-describing text header.
//!
//! Reading, writing, and chunk-at-a-time streaming in both directions.

mod header;
mod lazy;
mod read;
mod record;
mod write;

pub use header::{parse, render, NpyHeader};
pub use lazy::{NpyChunkReader, NpyChunkWriter};
pub use read::{decode_payload, read_bytes};
pub use write::{prepare, write_bytes};

use crate::error::Result;
use crate::options::{ReadOptions, WriteOptions};
use crate::value::Value;
use std::path::Path;

/// Read a `.npy` file from disk.
pub fn read_path(path: &Path, opts: &ReadOptions) -> Result<Value> {
    read::read_file(path, opts)
}

/// Write a value to a `.npy` file.
pub fn write_path(path: &Path, value: &Value, opts: &WriteOptions) -> Result<()> {
    write::write_file(path, value, opts)
}
