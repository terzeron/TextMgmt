//! Reading `joblib.dump` output, compressed or not.

use super::gzip;
use crate::error::{Error, Result};
use crate::formats::pickle::Machine;
use crate::options::ReadOptions;
use crate::util::decompress_zlib;
use crate::value::Value;
use std::path::Path;

/// Decode an in-memory `.joblib` file.
pub fn read_bytes(buf: &[u8], opts: &ReadOptions) -> Result<Value> {
    let owned;
    let data: &[u8] = match gzip::kind(buf) {
        Some("gzip") => {
            owned = gzip::decode(buf)?;
            &owned
        }
        Some("zlib") => {
            owned = decompress_zlib(buf, buf.len() * 4)?;
            &owned
        }
        Some(other) => {
            return Err(Error::unsupported(format!(
                "joblib `{other}` compression needs a non-Rust codec; \
                 re-dump with compress=0, 'zlib' or 'gzip'"
            )))
        }
        None => buf,
    };
    Machine::new(data, opts).with_joblib().run()
}

/// Read a `.joblib` file.
pub fn read_path(path: &Path, opts: &ReadOptions) -> Result<Value> {
    read_bytes(&std::fs::read(path)?, opts)
}
