//! Central-directory based ZIP reader.

use super::{METHOD_DEFLATE, METHOD_STORE};
use crate::error::{Error, Result};
use crate::util::{decompress_raw, Cursor};

/// One member of the archive.
#[derive(Clone, Debug)]
pub struct ZipEntry {
    /// Name of this item.
    pub name: String,
    /// ZIP compression method: 0 stored, 8 deflated.
    pub method: u16,
    /// Size of the member as stored.
    pub compressed_size: u64,
    /// Size of the member once inflated.
    pub uncompressed_size: u64,
    /// Offset of the *local* file header inside the archive.
    pub header_offset: u64,
    /// CRC-32 of the uncompressed member data.
    pub crc32: u32,
}

/// A parsed archive; the payload bytes stay borrowed so nothing is copied until
/// a member is actually requested.
pub struct ZipReader<'a> {
    data: &'a [u8],
    entries: Vec<ZipEntry>,
}

fn find_eocd(d: &[u8]) -> Option<usize> {
    let start = d.len().saturating_sub(66_000);
    (start..d.len().saturating_sub(21)).rev().find(|&i| &d[i..i + 4] == b"PK\x05\x06")
}

impl<'a> ZipReader<'a> {
    /// Parse the central directory of an in-memory archive.
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let eocd =
            find_eocd(data).ok_or_else(|| Error::format("no ZIP end-of-central-directory"))?;
        let mut c = Cursor::at(data, eocd + 8);
        let mut count = c.u16()? as u64;
        c.skip(2)?;
        let mut size = c.u32()? as u64;
        let mut offset = c.u32()? as u64;

        // ZIP64: the classic record saturates at 0xFFFF / 0xFFFFFFFF.
        if count == 0xFFFF || offset == 0xFFFF_FFFF || size == 0xFFFF_FFFF {
            let loc = data[..eocd]
                .windows(4)
                .rposition(|w| w == b"PK\x06\x07")
                .ok_or_else(|| Error::format("missing ZIP64 locator"))?;
            let mut l = Cursor::at(data, loc + 8);
            let z64 = l.u64()? as usize;
            let mut z = Cursor::at(data, z64 + 32);
            count = z.u64()?;
            z.skip(8)?;
            size = z.u64()?;
            offset = z.u64()?;
        }
        let _ = size;

        let mut entries = Vec::with_capacity(count as usize);
        let mut c = Cursor::at(data, offset as usize);
        for _ in 0..count {
            if c.take(4)? != b"PK\x01\x02" {
                break;
            }
            c.skip(6)?; // version made by / needed / flags
            let method = c.u16()?;
            c.skip(4)?; // mod time / date
            let crc32 = c.u32()?;
            let csize = c.u32()? as u64;
            let usize_ = c.u32()? as u64;
            let nlen = c.u16()? as usize;
            let elen = c.u16()? as usize;
            let clen = c.u16()? as usize;
            c.skip(8)?; // disk / attrs
            let hoff = c.u32()? as u64;
            let name = String::from_utf8_lossy(c.take(nlen)?).into_owned();
            let extra = c.take(elen)?.to_vec();
            c.skip(clen)?;
            let (usize_, csize, hoff) = patch_zip64(&extra, usize_, csize, hoff)?;
            entries.push(ZipEntry {
                name,
                method,
                compressed_size: csize,
                uncompressed_size: usize_,
                header_offset: hoff,
                crc32,
            });
        }
        Ok(ZipReader { data, entries })
    }

    /// All members, in central-directory order.
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Look up a member by exact name.
    pub fn entry(&self, name: &str) -> Option<&ZipEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Byte range of the member payload inside the archive (compressed form).
    pub fn raw_range(&self, e: &ZipEntry) -> Result<(usize, usize)> {
        let mut c = Cursor::at(self.data, e.header_offset as usize);
        if c.take(4)? != b"PK\x03\x04" {
            return Err(Error::format(format!("bad local header for `{}`", e.name)));
        }
        c.skip(22)?;
        let nlen = c.u16()? as usize;
        let elen = c.u16()? as usize;
        let start = c.pos() + nlen + elen;
        Ok((start, start + e.compressed_size as usize))
    }

    /// Decode one member into an owned buffer.
    pub fn read(&self, e: &ZipEntry) -> Result<Vec<u8>> {
        let (a, b) = self.raw_range(e)?;
        let raw = self
            .data
            .get(a..b)
            .ok_or_else(|| Error::format(format!("member `{}` is truncated", e.name)))?;
        match e.method {
            METHOD_STORE => Ok(raw.to_vec()),
            METHOD_DEFLATE => decompress_raw(raw, e.uncompressed_size as usize),
            m => Err(Error::unsupported(format!("zip compression method {m}"))),
        }
    }

    /// Borrow a *stored* member without copying (`None` when compressed).
    pub fn read_borrowed(&self, e: &ZipEntry) -> Result<Option<&'a [u8]>> {
        if e.method != METHOD_STORE {
            return Ok(None);
        }
        let (a, b) = self.raw_range(e)?;
        Ok(self.data.get(a..b))
    }
}

fn patch_zip64(extra: &[u8], u: u64, c: u64, h: u64) -> Result<(u64, u64, u64)> {
    let (mut u, mut c, mut h) = (u, c, h);
    let mut cur = Cursor::new(extra);
    while cur.remaining() >= 4 {
        let id = cur.u16()?;
        let len = cur.u16()? as usize;
        if id == 0x0001 {
            let mut f = Cursor::new(cur.take(len)?);
            if u == 0xFFFF_FFFF {
                u = f.u64()?;
            }
            if c == 0xFFFF_FFFF {
                c = f.u64()?;
            }
            if h == 0xFFFF_FFFF {
                h = f.u64()?;
            }
        } else {
            cur.skip(len)?;
        }
    }
    Ok((u, c, h))
}
