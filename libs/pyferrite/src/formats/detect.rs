//! Format sniffing: magic bytes first, file extension as a fallback.

use super::Format;
use crate::error::{Error, Result};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Recognise a format from a prefix of the file.
///
/// Recognised magics:
/// * `\x93NUMPY`  -> [`Format::Npy`]
/// * `\x89HDF\r\n\x1a\n` -> [`Format::Hdf5`]
/// * `PK\x03\x04` -> npz or pt (disambiguated by the first member name)
/// * `\x80\x02..` / `\x80\x05..` and other pickle openers -> pickle-ish
pub fn detect_from_bytes(buf: &[u8]) -> Option<Format> {
    if buf.starts_with(b"\x93NUMPY") {
        return Some(Format::Npy);
    }
    if buf.starts_with(b"\x89HDF\r\n\x1a\n") {
        return Some(Format::Hdf5);
    }
    if buf.starts_with(b"PK\x03\x04") {
        // Peek at the first local file name: torch writes `<archive>/data.pkl`,
        // numpy writes `<name>.npy`.
        if buf.len() > 30 {
            let n = u16::from_le_bytes([buf[26], buf[27]]) as usize;
            let end = (30 + n).min(buf.len());
            let name = String::from_utf8_lossy(&buf[30..end]);
            if name.ends_with(".npy") {
                return Some(Format::Npz);
            }
            return Some(Format::Pt);
        }
        return Some(Format::Npz);
    }
    if buf.starts_with(b"ZF") || buf.starts_with(b"\x28\xb5\x2f\xfd") {
        return Some(Format::Joblib); // joblib compressed containers
    }
    if buf.first() == Some(&0x80) || buf.starts_with(b"(") || buf.starts_with(b"]") {
        return Some(Format::Pickle);
    }
    None
}

/// Sniff a file: read the first 64 bytes, fall back to the extension.
pub fn detect_from_path(path: &Path) -> Result<Format> {
    let mut head = [0u8; 64];
    let n = match File::open(path) {
        Ok(mut f) => f.read(&mut head).unwrap_or(0),
        Err(e) => return Err(Error::Io(e)),
    };
    if let Some(f) = detect_from_bytes(&head[..n]) {
        // The extension wins when the magic is ambiguous between pickle-based
        // containers, because `.joblib` and `.pkl` share their opening bytes.
        if f == Format::Pickle {
            if let Some(ext) = Format::from_path(path) {
                return Ok(ext);
            }
        }
        return Ok(f);
    }
    Format::from_path(path)
        .ok_or_else(|| Error::format(format!("cannot determine format of {}", path.display())))
}

#[cfg(test)]
#[path = "../../../../tests/rust/pyferrite/format_detect_unit.rs"]
mod tests;
