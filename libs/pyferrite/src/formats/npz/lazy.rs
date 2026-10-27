//! Chunked reading of `.npz` members.
//!
//! The archive index is tiny, so it is parsed eagerly; payloads are inflated
//! one member at a time and then handed out in `chunk_elements` slices.

use crate::error::Result;
use crate::formats::npy;
use crate::lazy::{Chunk, ChunkSource};
use crate::options::ReadOptions;
use crate::value::from_bytes;
use std::path::Path;

/// Streams every array of a `.npz` archive, member by member.
pub struct NpzChunkReader {
    data: Vec<u8>,
    members: Vec<(String, u64)>,
    opts: ReadOptions,
    current: Option<Current>,
    next_member: usize,
}

struct Current {
    name: String,
    header: npy::NpyHeader,
    payload: Vec<u8>,
    cursor: usize,
}

impl NpzChunkReader {
    /// Open an archive for streaming.
    pub fn open(path: &Path, opts: &ReadOptions) -> Result<Self> {
        let data = std::fs::read(path)?;
        let members = {
            let z = crate::zip::ZipReader::new(&data)?;
            z.entries()
                .iter()
                .filter(|e| e.name.ends_with(".npy"))
                .map(|e| (e.name.clone(), e.header_offset))
                .collect::<Vec<_>>()
        };
        Ok(NpzChunkReader { data, members, opts: opts.clone(), current: None, next_member: 0 })
    }

    fn load_next(&mut self) -> Result<bool> {
        if self.next_member >= self.members.len() {
            return Ok(false);
        }
        let (name, _) = self.members[self.next_member].clone();
        self.next_member += 1;
        let z = crate::zip::ZipReader::new(&self.data)?;
        let e = z.entry(&name).unwrap();
        let raw = z.read(e)?;
        let header = npy::parse(&raw)?;
        let payload = raw[header.data_offset..].to_vec();
        self.current = Some(Current {
            name: name.trim_end_matches(".npy").to_string(),
            header,
            payload,
            cursor: 0,
        });
        Ok(true)
    }
}

impl ChunkSource for NpzChunkReader {
    fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        loop {
            if self.current.is_none() && !self.load_next()? {
                return Ok(None);
            }
            let c = self.current.as_mut().unwrap();
            let total: usize = c.header.shape.iter().product();
            if c.cursor >= total {
                self.current = None;
                continue;
            }
            let esz = match c.header.dtype.size() {
                Some(s) => s,
                None => {
                    self.current = None;
                    continue;
                }
            };
            let n = self.opts.chunk_elements.min(total - c.cursor);
            let start = c.cursor * esz;
            let arr = from_bytes(
                &c.header.dtype,
                c.header.endian,
                &[n],
                &c.payload[start..start + n * esz],
            )?;
            let arr = crate::formats::apply_cast(arr, &self.opts)?;
            let chunk = Chunk {
                name: c.name.clone(),
                dtype: arr.dtype(),
                full_shape: c.header.shape.clone(),
                offset: c.cursor,
                data: arr,
            };
            c.cursor += n;
            return Ok(Some(chunk));
        }
    }
    fn names(&self) -> Vec<String> {
        self.members.iter().map(|(n, _)| n.trim_end_matches(".npy").to_string()).collect()
    }
}
