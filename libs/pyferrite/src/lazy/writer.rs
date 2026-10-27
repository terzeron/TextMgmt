//! The lazy writing front-end.

use crate::error::Result;
use crate::value::Array;

/// A sink that accepts array chunks and finalises a file on drop-free `finish`.
pub trait ChunkSink: Send {
    /// Append `chunk` (must be 1-D, C order) to the output.
    fn push(&mut self, chunk: &Array) -> Result<()>;
    /// Flush headers/footers and close the file.
    fn finish(self: Box<Self>) -> Result<()>;
    /// Elements still expected before the declared shape is filled.
    fn remaining(&self) -> usize;
}

/// Type-erased handle over any [`ChunkSink`].
///
/// ```no_run
/// # use pyferrite::{write_lazy, WriteOptions, dtype::DType};
/// let mut w = write_lazy("out.npy", DType::F32, &[1_000_000], &WriteOptions::new())?;
/// # let chunk = pyferrite::value::Array::F32(ndarray::ArrayD::zeros(ndarray::IxDyn(&[1000])));
/// for _ in 0..1000 { w.push(&chunk)?; }
/// w.finish()?;
/// # Ok::<(), pyferrite::Error>(())
/// ```
pub struct LazyWriter {
    inner: Box<dyn ChunkSink>,
}

impl LazyWriter {
    /// Create a new value.
    pub fn new(inner: Box<dyn ChunkSink>) -> Self {
        LazyWriter { inner }
    }
    /// Append one chunk.
    pub fn push(&mut self, chunk: &Array) -> Result<()> {
        self.inner.push(chunk)
    }
    /// Number of elements still missing before the declared shape is complete.
    pub fn remaining(&self) -> usize {
        self.inner.remaining()
    }
    /// Finish the file. Fails when fewer elements were pushed than declared.
    pub fn finish(self) -> Result<()> {
        self.inner.finish()
    }
}
