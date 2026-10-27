//! `.h5` / `.hdf5` — a pure-Rust subset of the HDF5 format.
//!
//! # What is supported
//!
//! *Reading*: superblock versions 0–3; object headers version 1 and 2 with
//! continuation blocks; groups indexed either by a symbol table B-tree (the
//! classic layout) or by link messages (the "new style" layout); contiguous,
//! compact and chunked datasets; the deflate, shuffle and fletcher32 filters;
//! integer, floating-point, bitfield, enumeration and fixed-length string
//! datatypes.
//!
//! *Writing*: superblock version 0 with version-1 object headers and
//! contiguous little-endian datasets, nested to any depth.
//!
//! # What is not
//!
//! Variable-length strings, compound and array datatypes, references, virtual
//! and external datasets, attributes, and any filter beyond the three above.
//! Reading such a file yields a clear [`Error::Unsupported`] rather than
//! silently wrong numbers.
//!
//! [`Error::Unsupported`]: crate::Error::Unsupported

mod chunks;
mod dataset;
mod encode;
mod group;
mod layout_plan;
mod lazy;
mod msg;
mod objhdr;
mod plan;
mod read;
mod superblock;
mod write;

pub use lazy::{Hdf5ChunkReader, Hdf5ChunkWriter};
pub use read::{format_version, index, read_bytes, read_path};
pub use write::{write_bytes, write_path};
