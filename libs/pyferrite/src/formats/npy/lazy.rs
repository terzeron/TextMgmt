//! Chunked reading and writing of `.npy` payloads.

use super::header::{parse, render, NpyHeader};
use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};
use crate::lazy::{Chunk, ChunkSink, ChunkSource};
use crate::options::{ReadOptions, WriteOptions};
use crate::value::{from_bytes, to_bytes, Array};
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Streams a `.npy` payload `chunk_elements` at a time.
pub struct NpyChunkReader {
    file: File,
    header: NpyHeader,
    target: DType,
    opts: ReadOptions,
    name: String,
    cursor: usize,
    elem_size: usize,
}

impl NpyChunkReader {
    /// Open a `.npy` file for streaming.
    pub fn open(path: &Path, name: &str, opts: &ReadOptions) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut head = vec![0u8; 4096];
        let n = file.read(&mut head)?;
        head.truncate(n);
        let header = parse(&head)?;
        if header.dtype == DType::Object || matches!(header.dtype, DType::Struct(_)) {
            return Err(Error::unsupported("lazy reading of object/record arrays"));
        }
        if header.fortran_order {
            return Err(Error::unsupported(
                "lazy reading of Fortran-ordered arrays (transposition needs the whole payload)",
            ));
        }
        let elem_size = header.dtype.size().unwrap();
        file.seek(SeekFrom::Start(header.data_offset as u64))?;
        let target = crate::formats::resolve_target_dtype_read(&header.dtype, opts);
        Ok(NpyChunkReader {
            file,
            header,
            target,
            opts: opts.clone(),
            name: name.to_string(),
            cursor: 0,
            elem_size,
        })
    }
}

impl ChunkSource for NpyChunkReader {
    fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        let total: usize = self.header.shape.iter().product();
        if self.cursor >= total {
            return Ok(None);
        }
        let n = self.opts.chunk_elements.min(total - self.cursor);
        let mut buf = vec![0u8; n * self.elem_size];
        self.file.read_exact(&mut buf)?;
        let arr = from_bytes(&self.header.dtype, self.header.endian, &[n], &buf)?;
        let arr = crate::formats::apply_cast(arr, &self.opts)?;
        let offset = self.cursor;
        self.cursor += n;
        Ok(Some(Chunk {
            name: self.name.clone(),
            dtype: self.target.clone(),
            full_shape: self.header.shape.clone(),
            offset,
            data: arr,
        }))
    }
    fn names(&self) -> Vec<String> {
        vec![self.name.clone()]
    }
}

/// Writes a `.npy` file incrementally: header first, then chunks.
pub struct NpyChunkWriter {
    out: BufWriter<File>,
    dtype: DType,
    total: usize,
    written: usize,
    opts: WriteOptions,
}

impl NpyChunkWriter {
    /// Create the file and emit the header for `shape` / `dtype` upfront.
    pub fn create(path: &Path, dtype: DType, shape: &[usize], opts: &WriteOptions) -> Result<Self> {
        if !dtype.is_numeric() {
            return Err(Error::invalid(format!(
                "lazy npy writing needs a numeric dtype, got {dtype:?}"
            )));
        }
        let file = File::create(path)?;
        let mut out = BufWriter::new(file);
        out.write_all(&render(&dtype, Endian::Little, shape, false)?)?;
        Ok(NpyChunkWriter {
            out,
            dtype,
            total: shape.iter().product(),
            written: 0,
            opts: opts.clone(),
        })
    }
}

impl ChunkSink for NpyChunkWriter {
    fn push(&mut self, chunk: &Array) -> Result<()> {
        if self.written + chunk.len() > self.total {
            return Err(Error::invalid(format!(
                "chunk overflows the declared shape: {} + {} > {}",
                self.written,
                chunk.len(),
                self.total
            )));
        }
        let cast;
        let a = if chunk.dtype() == self.dtype {
            chunk
        } else {
            cast = chunk.cast(&self.dtype, self.opts.cast_policy)?;
            &cast
        };
        self.out.write_all(&to_bytes(a, Endian::Little)?)?;
        self.written += a.len();
        Ok(())
    }
    fn finish(mut self: Box<Self>) -> Result<()> {
        if self.written != self.total {
            return Err(Error::invalid(format!(
                "declared {} elements but only {} were written",
                self.total, self.written
            )));
        }
        self.out.flush()?;
        Ok(())
    }
    fn remaining(&self) -> usize {
        self.total - self.written
    }
}
