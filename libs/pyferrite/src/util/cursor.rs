//! A bounds-checked little-endian byte cursor used by every binary parser.

use crate::error::{Error, Result};

/// Read-only cursor over a byte slice.
#[derive(Clone, Debug)]
pub struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

macro_rules! rd {
    ($name:ident, $be:ident, $ty:ty, $n:expr) => {
        #[doc = concat!("Read a little-endian `", stringify!($ty), "`.")]
        pub fn $name(&mut self) -> Result<$ty> {
            let b = self.take($n)?;
            let mut a = [0u8; $n];
            a.copy_from_slice(b);
            Ok(<$ty>::from_le_bytes(a))
        }
        #[doc = concat!("Read a big-endian `", stringify!($ty), "`.")]
        pub fn $be(&mut self) -> Result<$ty> {
            let b = self.take($n)?;
            let mut a = [0u8; $n];
            a.copy_from_slice(b);
            Ok(<$ty>::from_be_bytes(a))
        }
    };
}

impl<'a> Cursor<'a> {
    /// Create a new value.
    pub fn new(buf: &'a [u8]) -> Self {
        Cursor { buf, pos: 0 }
    }
    /// At.
    pub fn at(buf: &'a [u8], pos: usize) -> Self {
        Cursor { buf, pos }
    }
    /// Pos.
    pub fn pos(&self) -> usize {
        self.pos
    }
    /// Set pos.
    pub fn set_pos(&mut self, p: usize) {
        self.pos = p;
    }
    /// Remaining.
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }
    /// `true` when this is empty.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }
    /// Advance by `n` bytes and return them.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return Err(Error::format(format!(
                "unexpected end of data: need {n} bytes at offset {}, have {}",
                self.pos,
                self.remaining()
            )));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    /// Skip `n` bytes.
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    /// Peek at the next byte without consuming it.
    pub fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }
    /// U8.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    /// I8.
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.take(1)?[0] as i8)
    }
    rd!(u16, u16_be, u16, 2);
    rd!(u32, u32_be, u32, 4);
    rd!(u64, u64_be, u64, 8);
    rd!(i16, i16_be, i16, 2);
    rd!(i32, i32_be, i32, 4);
    rd!(i64, i64_be, i64, 8);

    /// Read an unsigned integer of `n` bytes (1, 2, 4 or 8), little-endian.
    pub fn uint(&mut self, n: usize) -> Result<u64> {
        let b = self.take(n)?;
        let mut v = 0u64;
        for (i, x) in b.iter().enumerate() {
            v |= (*x as u64) << (8 * i);
        }
        Ok(v)
    }
    /// Read a NUL-terminated string, consuming the terminator.
    pub fn cstr(&mut self) -> Result<String> {
        let start = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != 0 {
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.buf[start..self.pos]).into_owned();
        if self.pos < self.buf.len() {
            self.pos += 1;
        }
        Ok(s)
    }
    /// Align the position up to a multiple of `n`.
    pub fn align(&mut self, n: usize) {
        let r = self.pos % n;
        if r != 0 {
            self.pos += n - r;
        }
    }
    /// The whole underlying buffer.
    pub fn buffer(&self) -> &'a [u8] {
        self.buf
    }
}
