//! Chunk-at-a-time reading and writing for files that do not fit in RAM.

mod chunk;
mod reader;
mod writer;

pub use chunk::Chunk;
pub use reader::{ChunkSource, LazyReader};
pub use writer::{ChunkSink, LazyWriter};
