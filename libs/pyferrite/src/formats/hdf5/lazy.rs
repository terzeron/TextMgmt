//! Chunk-at-a-time HDF5 reading and writing.

use super::read;
use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::lazy::{Chunk, ChunkSink, ChunkSource};
use crate::options::{ReadOptions, WriteOptions};
use crate::value::{from_bytes, Array};
use std::path::{Path, PathBuf};

/// Streams every dataset of an HDF5 file in `chunk_elements` slices.
pub struct Hdf5ChunkReader {
    data: Vec<u8>,
    index: Vec<(String, Vec<usize>, DType, u64)>,
    opts: ReadOptions,
    ds: usize,
    cursor: usize,
    payload: Option<Vec<u8>>,
}

impl Hdf5ChunkReader {
    /// Open a file for streaming.
    pub fn open(path: &Path, opts: &ReadOptions) -> Result<Self> {
        let data = std::fs::read(path)?;
        let index = read::index(&data)?;
        Ok(Hdf5ChunkReader { data, index, opts: opts.clone(), ds: 0, cursor: 0, payload: None })
    }
}

impl ChunkSource for Hdf5ChunkReader {
    fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        loop {
            if self.ds >= self.index.len() {
                return Ok(None);
            }
            let (name, shape, dt, addr) = self.index[self.ds].clone();
            let total: usize = shape.iter().product();
            if self.payload.is_none() {
                let sb = super::superblock::Superblock::parse(&self.data)?;
                let msgs = super::objhdr::parse(&self.data, &sb, addr)?;
                let info = super::dataset::info(&msgs, &sb)?
                    .ok_or_else(|| Error::format("dataset vanished between passes"))?;
                self.payload = Some(super::dataset::raw_bytes(&self.data, &sb, &info)?);
                self.cursor = 0;
            }
            if self.cursor >= total {
                self.payload = None;
                self.ds += 1;
                continue;
            }
            let esz = dt.size().ok_or_else(|| Error::unsupported("variable-size dtype"))?;
            let n = self.opts.chunk_elements.min(total - self.cursor);
            let buf = self.payload.as_ref().unwrap();
            let start = self.cursor * esz;
            let arr =
                from_bytes(&dt, crate::dtype::Endian::Little, &[n], &buf[start..start + n * esz])?;
            let arr = crate::formats::apply_cast(arr, &self.opts)?;
            let chunk = Chunk {
                name,
                dtype: arr.dtype(),
                full_shape: shape,
                offset: self.cursor,
                data: arr,
            };
            self.cursor += n;
            return Ok(Some(chunk));
        }
    }
    fn names(&self) -> Vec<String> {
        self.index.iter().map(|(n, _, _, _)| n.clone()).collect()
    }
}

/// Buffers chunks and emits a single-dataset HDF5 file on `finish`.
///
/// HDF5 stores its object headers ahead of the payload, so a strictly
/// single-pass writer would have to know every address in advance. Buffering
/// keeps the API identical to the `.npy` sink while staying correct.
pub struct Hdf5ChunkWriter {
    path: PathBuf,
    dtype: DType,
    shape: Vec<usize>,
    total: usize,
    buf: Vec<u8>,
    written: usize,
    opts: WriteOptions,
}

impl Hdf5ChunkWriter {
    /// Declare the dataset upfront.
    pub fn create(path: &Path, dtype: DType, shape: &[usize], opts: &WriteOptions) -> Result<Self> {
        Ok(Hdf5ChunkWriter {
            path: path.to_path_buf(),
            dtype,
            shape: shape.to_vec(),
            total: shape.iter().product(),
            buf: Vec::new(),
            written: 0,
            opts: opts.clone(),
        })
    }
}

impl ChunkSink for Hdf5ChunkWriter {
    fn push(&mut self, chunk: &Array) -> Result<()> {
        if self.written + chunk.len() > self.total {
            return Err(Error::invalid("chunk overflows the declared shape"));
        }
        let cast;
        let a = if chunk.dtype() == self.dtype {
            chunk
        } else {
            cast = chunk.cast(&self.dtype, self.opts.cast_policy)?;
            &cast
        };
        self.buf.extend_from_slice(&crate::value::to_bytes(a, crate::dtype::Endian::Little)?);
        self.written += a.len();
        Ok(())
    }
    fn finish(self: Box<Self>) -> Result<()> {
        if self.written != self.total {
            return Err(Error::invalid("declared more elements than were written"));
        }
        let arr = from_bytes(&self.dtype, crate::dtype::Endian::Little, &self.shape, &self.buf)?;
        let mut d = crate::value::Dict::new();
        d.insert(crate::value::Value::Str("data".into()), crate::value::Value::Array(arr));
        super::write::write_path(&self.path, &crate::value::Value::Dict(d), &self.opts)
    }
    fn remaining(&self) -> usize {
        self.total - self.written
    }
}
