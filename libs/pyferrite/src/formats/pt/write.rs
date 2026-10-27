//! Writing `torch.save`-compatible ZIP archives.

use crate::error::Result;
use crate::formats::pickle::Pickler;
use crate::options::WriteOptions;
use crate::value::Value;
use crate::zip::ZipWriter;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// Archive directory name used inside the ZIP (`torch.save` uses the stem of
/// the output file; any name works as long as it is consistent).
const ARCHIVE: &str = "archive";

/// `torch.serialization.DEFAULT_PROTOCOL`.
const TORCH_PICKLE_PROTOCOL: u8 = 2;

/// Serialise a value into a `.pt` archive held in memory.
///
/// The pickle stream always uses protocol 2, as `torch.save` does: the
/// `weights_only` unpickler that `torch.load` uses by default since PyTorch 2.6
/// rejects the opcodes protocols 4 and 5 add (`MEMOIZE`, `FRAME`, ...).
pub fn write_bytes(value: &Value, opts: &WriteOptions) -> Result<Vec<u8>> {
    let mut p = Pickler::new(TORCH_PICKLE_PROTOCOL);
    p.torch_mode = true;
    p.dump(value, opts)?;
    let storages = std::mem::take(&mut p.storages);
    let pkl = p.finish();

    let mut z = ZipWriter::new(Vec::new());
    z.add(&format!("{ARCHIVE}/data.pkl"), &pkl, opts.compression)?;
    z.add(&format!("{ARCHIVE}/version"), b"3\n", opts.compression)?;
    z.add(&format!("{ARCHIVE}/byteorder"), b"little", opts.compression)?;
    for (key, _dt, raw) in &storages {
        z.add(&format!("{ARCHIVE}/data/{key}"), raw, opts.compression)?;
    }
    z.finish()
}

/// Write a `.pt` file.
pub fn write_path(path: &Path, value: &Value, opts: &WriteOptions) -> Result<()> {
    let buf = write_bytes(value, opts)?;
    let mut f = BufWriter::new(File::create(path)?);
    std::io::Write::write_all(&mut f, &buf)?;
    std::io::Write::flush(&mut f)?;
    Ok(())
}
