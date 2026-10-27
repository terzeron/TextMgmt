//! Small, dependency-free helpers: byte cursors, CRC-32 and deflate glue.

mod crc32;
mod cursor;
mod deflate;
mod pylit;

pub use crc32::{crc32, Crc32};
pub use cursor::Cursor;
pub use deflate::{compress_raw, compress_zlib, decompress_raw, decompress_zlib};
pub use pylit::parse_python_literal;
