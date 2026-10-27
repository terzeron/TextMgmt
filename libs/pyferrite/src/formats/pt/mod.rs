//! `.pt` / `.pth` — `torch.save` archives.
//!
//! Since torch 1.6 the file is a ZIP holding `archive/data.pkl` plus one raw
//! blob per storage under `archive/data/<key>`. The pickle references those
//! blobs through persistent ids, which this module resolves.

mod read;
mod write;

pub use read::{read_bytes, read_path};
pub use write::{write_bytes, write_path};
