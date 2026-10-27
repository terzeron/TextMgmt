//! Minimal, pure-Rust ZIP reader/writer for `.npz` and `.pt` containers.
//!
//! Only the two methods those producers use are implemented — *stored* (0) and
//! *deflate* (8) — plus ZIP64 extents, which `torch.save` emits for large
//! checkpoints. This avoids pulling in the `zip` crate and its transitive
//! C-backed compressors.

mod read;
mod write;

pub use read::{ZipEntry, ZipReader};
pub use write::ZipWriter;

/// Compression method identifiers as stored in the ZIP headers.
pub const METHOD_STORE: u16 = 0;
/// `METHOD_DEFLATE`.
pub const METHOD_DEFLATE: u16 = 8;
