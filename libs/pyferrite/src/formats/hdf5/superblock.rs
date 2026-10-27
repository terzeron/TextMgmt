//! HDF5 superblock (format versions 0, 1, 2 and 3).

use crate::error::{Error, Result};
use crate::util::Cursor;

/// The signature every HDF5 file starts with.
pub const MAGIC: &[u8; 8] = b"\x89HDF\r\n\x1a\n";

/// The parts of the superblock the rest of the reader needs.
#[derive(Clone, Debug)]
pub struct Superblock {
    pub version: u8,
    /// Width in bytes of every file address.
    pub offset_size: usize,
    /// Width in bytes of every length field.
    pub length_size: usize,
    pub base_address: u64,
    /// Address of the root group's object header.
    pub root_address: u64,
    /// End-of-file address as recorded in the superblock.
    pub eof_address: u64,
}

fn addr(c: &mut Cursor<'_>, n: usize) -> Result<u64> {
    c.uint(n)
}

impl Superblock {
    /// Parse the superblock at the beginning of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self> {
        if !buf.starts_with(MAGIC) {
            return Err(Error::format("missing HDF5 signature"));
        }
        let mut c = Cursor::at(buf, 8);
        let version = c.u8()?;
        match version {
            0 | 1 => {
                c.skip(3)?; // free space / root group / reserved versions
                if version == 1 {
                    c.skip(1)?;
                }
                c.skip(1)?; // shared header message version
                let offset_size = c.u8()? as usize;
                let length_size = c.u8()? as usize;
                c.skip(1)?; // reserved
                c.skip(4)?; // group leaf/internal node K
                if version == 1 {
                    c.skip(4)?; // indexed storage internal node K + reserved
                }
                c.skip(4)?; // file consistency flags
                let base_address = addr(&mut c, offset_size)?;
                let _free = addr(&mut c, offset_size)?;
                let eof_address = addr(&mut c, offset_size)?;
                let _driver = addr(&mut c, offset_size)?;
                // Root group symbol table entry: link name offset, header address.
                let _name = addr(&mut c, offset_size)?;
                let root_address = addr(&mut c, offset_size)?;
                Ok(Superblock {
                    version,
                    offset_size,
                    length_size,
                    base_address,
                    root_address,
                    eof_address,
                })
            }
            2 | 3 => {
                let offset_size = c.u8()? as usize;
                let length_size = c.u8()? as usize;
                c.skip(1)?; // file consistency flags
                let base_address = addr(&mut c, offset_size)?;
                let _ext = addr(&mut c, offset_size)?;
                let eof_address = addr(&mut c, offset_size)?;
                let root_address = addr(&mut c, offset_size)?;
                Ok(Superblock {
                    version,
                    offset_size,
                    length_size,
                    base_address,
                    root_address,
                    eof_address,
                })
            }
            v => Err(Error::unsupported(format!("HDF5 superblock version {v}"))),
        }
    }

    /// Convert a file address into a slice index, honouring `base_address`.
    pub fn at(&self, a: u64) -> usize {
        (a.wrapping_add(self.base_address)) as usize
    }

    /// `true` when `a` points inside the file as the superblock describes it.
    ///
    /// The end-of-file address is authoritative: anything at or beyond it is a
    /// corrupt or truncated file, and catching that here keeps every later
    /// read from having to guess.
    pub fn in_bounds(&self, a: u64) -> bool {
        !self.undefined(a) && a.wrapping_add(self.base_address) < self.eof_address.max(1)
    }

    /// Which superblock format version this file uses (0 through 3).
    ///
    /// Only relevant when reporting on a file rather than reading it; the
    /// parser has already normalised the differences away.
    pub fn format_version(&self) -> u8 {
        self.version
    }

    /// `true` when an address is the "undefined" all-ones sentinel.
    pub fn undefined(&self, a: u64) -> bool {
        let mask =
            if self.offset_size >= 8 { u64::MAX } else { (1u64 << (8 * self.offset_size)) - 1 };
        a == mask
    }
}

#[cfg(test)]
#[path = "../../../../../tests/rust/pyferrite/superblock_unit.rs"]
mod tests;
