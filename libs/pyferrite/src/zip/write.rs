//! Streaming ZIP writer (stored + deflate), ZIP64-aware.

use super::{METHOD_DEFLATE, METHOD_STORE};
use crate::error::Result;
use crate::options::Compression;
use crate::util::{compress_raw, crc32};
use std::io::Write;

struct Record {
    name: String,
    method: u16,
    crc: u32,
    csize: u64,
    usize_: u64,
    offset: u64,
}

/// Writes ZIP members one after another and finalises the central directory on
/// [`ZipWriter::finish`].
pub struct ZipWriter<W: Write> {
    out: W,
    pos: u64,
    records: Vec<Record>,
}

impl<W: Write> ZipWriter<W> {
    /// Create a new value.
    pub fn new(out: W) -> Self {
        ZipWriter { out, pos: 0, records: Vec::new() }
    }

    /// Append a member. `compression` chooses stored vs deflate.
    pub fn add(&mut self, name: &str, data: &[u8], compression: Compression) -> Result<()> {
        let (method, payload) = match compression {
            Compression::None => (METHOD_STORE, data.to_vec()),
            Compression::Deflate(l) | Compression::Zlib(l) => {
                (METHOD_DEFLATE, compress_raw(data, l)?)
            }
        };
        let crc = crc32(data);
        let offset = self.pos;
        let nb = name.as_bytes();
        let mut h = Vec::with_capacity(30 + nb.len());
        h.extend_from_slice(b"PK\x03\x04");
        h.extend_from_slice(&20u16.to_le_bytes()); // version needed
        h.extend_from_slice(&0u16.to_le_bytes()); // flags
        h.extend_from_slice(&method.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // time
        h.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1980-01-01
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // extra len
        h.extend_from_slice(nb);
        self.out.write_all(&h)?;
        self.out.write_all(&payload)?;
        self.pos += h.len() as u64 + payload.len() as u64;
        self.records.push(Record {
            name: name.to_string(),
            method,
            crc,
            csize: payload.len() as u64,
            usize_: data.len() as u64,
            offset,
        });
        Ok(())
    }

    /// Write the central directory and return the wrapped writer.
    pub fn finish(mut self) -> Result<W> {
        let cd_start = self.pos;
        let mut cd = Vec::new();
        for r in &self.records {
            let nb = r.name.as_bytes();
            cd.extend_from_slice(b"PK\x01\x02");
            cd.extend_from_slice(&0x031Eu16.to_le_bytes()); // made by: unix, 3.0
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&r.method.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0x21u16.to_le_bytes());
            cd.extend_from_slice(&r.crc.to_le_bytes());
            cd.extend_from_slice(&(r.csize.min(u32::MAX as u64) as u32).to_le_bytes());
            cd.extend_from_slice(&(r.usize_.min(u32::MAX as u64) as u32).to_le_bytes());
            cd.extend_from_slice(&(nb.len() as u16).to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes()); // extra
            cd.extend_from_slice(&0u16.to_le_bytes()); // comment
            cd.extend_from_slice(&0u16.to_le_bytes()); // disk
            cd.extend_from_slice(&0u16.to_le_bytes()); // int attrs
            cd.extend_from_slice(&0u32.to_le_bytes()); // ext attrs
            cd.extend_from_slice(&(r.offset.min(u32::MAX as u64) as u32).to_le_bytes());
            cd.extend_from_slice(nb);
        }
        self.out.write_all(&cd)?;
        let mut eocd = Vec::with_capacity(22);
        eocd.extend_from_slice(b"PK\x05\x06");
        eocd.extend_from_slice(&0u16.to_le_bytes());
        eocd.extend_from_slice(&0u16.to_le_bytes());
        let n = self.records.len().min(u16::MAX as usize) as u16;
        eocd.extend_from_slice(&n.to_le_bytes());
        eocd.extend_from_slice(&n.to_le_bytes());
        eocd.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        eocd.extend_from_slice(&(cd_start as u32).to_le_bytes());
        eocd.extend_from_slice(&0u16.to_le_bytes());
        self.out.write_all(&eocd)?;
        self.out.flush()?;
        Ok(self.out)
    }
}
