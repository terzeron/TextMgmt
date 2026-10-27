//! The lazy reading front-end.

use super::Chunk;
use crate::error::Result;

/// A source that yields [`Chunk`]s until it is exhausted.
///
/// Implemented by every format that supports streaming; users normally hold a
/// [`LazyReader`] instead of naming the concrete type.
pub trait ChunkSource: Send {
    /// Produce the next chunk, or `Ok(None)` at end of stream.
    fn next_chunk(&mut self) -> Result<Option<Chunk>>;
    /// Names of the logical arrays this source will emit, when known upfront.
    fn names(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Type-erased, iterator-shaped handle over any [`ChunkSource`].
///
/// ```no_run
/// # use pyferrite::{read_lazy, ReadOptions};
/// let mut r = read_lazy("weights.npy", &ReadOptions::new().chunk_elements(65_536))?;
/// while let Some(mut chunk) = r.next_chunk()? {
///     // `chunk.data` is owned and mutable: rescale in place, then forward it.
///     if let Some(v) = chunk.data.as_f32_mut() { v.iter_mut().for_each(|x| *x *= 0.5); }
/// }
/// # Ok::<(), pyferrite::Error>(())
/// ```
pub struct LazyReader {
    inner: Box<dyn ChunkSource>,
    done: bool,
}

impl LazyReader {
    /// Wrap a concrete source.
    pub fn new(inner: Box<dyn ChunkSource>) -> Self {
        LazyReader { inner, done: false }
    }
    /// Names of the arrays that will be streamed, when the container knows them.
    pub fn names(&self) -> Vec<String> {
        self.inner.names()
    }
    /// Pull the next chunk explicitly.
    pub fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        if self.done {
            return Ok(None);
        }
        let c = self.inner.next_chunk()?;
        if c.is_none() {
            self.done = true;
        }
        Ok(c)
    }
    /// Drain every remaining chunk into a vector (defeats the point, but handy
    /// in tests).
    pub fn collect_chunks(&mut self) -> Result<Vec<Chunk>> {
        let mut out = Vec::new();
        while let Some(c) = self.next_chunk()? {
            out.push(c);
        }
        Ok(out)
    }
}

impl Iterator for LazyReader {
    type Item = Result<Chunk>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.next_chunk() {
            Ok(Some(c)) => Some(Ok(c)),
            Ok(None) => None,
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtype::DType;
    use crate::error::Error;
    use crate::value::Array;
    use ndarray::arr1;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct Source {
        chunks: Vec<Option<Chunk>>,
        names: Vec<String>,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl ChunkSource for Source {
        fn next_chunk(&mut self) -> Result<Option<Chunk>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                self.fail = false;
                return Err(Error::format("source failed"));
            }
            Ok(if self.chunks.is_empty() { None } else { self.chunks.remove(0) })
        }

        fn names(&self) -> Vec<String> {
            self.names.clone()
        }
    }

    fn chunk(offset: usize) -> Chunk {
        Chunk {
            name: "weights".into(),
            dtype: DType::I32,
            full_shape: vec![2],
            offset,
            data: Array::I32(arr1(&[offset as i32]).into_dyn()),
        }
    }

    fn source(chunks: Vec<Option<Chunk>>, calls: Arc<AtomicUsize>) -> Source {
        Source { chunks, names: vec!["weights".into()], calls, fail: false }
    }

    #[test]
    fn forwards_names_chunks_and_stops_polling_after_end() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut reader = LazyReader::new(Box::new(source(
            vec![Some(chunk(0)), None, Some(chunk(1))],
            calls.clone(),
        )));
        assert_eq!(reader.names(), vec!["weights"]);
        assert_eq!(reader.next_chunk().unwrap(), Some(chunk(0)));
        assert!(reader.next_chunk().unwrap().is_none());
        assert!(reader.next_chunk().unwrap().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn collect_chunks_drains_the_source_and_iterator_stops_cleanly() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut reader =
            LazyReader::new(Box::new(source(vec![Some(chunk(0)), Some(chunk(1))], calls.clone())));
        assert_eq!(reader.collect_chunks().unwrap(), vec![chunk(0), chunk(1)]);
        assert!(reader.next().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn iterator_yields_source_error_once_then_marks_reader_done() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut failing = source(vec![Some(chunk(0))], calls.clone());
        failing.fail = true;
        let mut reader = LazyReader::new(Box::new(failing));
        assert!(reader.next().unwrap().is_err());
        assert!(reader.next().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn default_source_names_are_empty() {
        struct Nameless;
        impl ChunkSource for Nameless {
            fn next_chunk(&mut self) -> Result<Option<Chunk>> {
                Ok(None)
            }
        }
        assert!(Nameless.names().is_empty());
    }
}
