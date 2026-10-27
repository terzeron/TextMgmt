//! Materialising dataset bytes from contiguous, compact or chunked storage.

use super::chunks::{btree_chunks, scatter};
use super::msg::{self, Layout};
use super::objhdr::Message;
use super::superblock::Superblock;
use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};
use crate::util::decompress_zlib;

/// Everything needed to decode one dataset.
#[derive(Clone, Debug)]
pub struct DatasetInfo {
    pub dtype: DType,
    pub endian: Endian,
    pub shape: Vec<usize>,
    pub layout: Layout,
    pub filters: Vec<u16>,
}

/// Extract dataset metadata from a parsed object header.
pub fn info(msgs: &[Message], sb: &Superblock) -> Result<Option<DatasetInfo>> {
    let mut shape = None;
    let mut dt = None;
    let mut layout = None;
    let mut filters = Vec::new();
    for m in msgs {
        match m.kind {
            msg::MSG_DATASPACE => shape = Some(msg::dataspace(&m.data, sb)?),
            msg::MSG_DATATYPE => dt = Some(msg::datatype(&m.data)?),
            msg::MSG_LAYOUT => layout = Some(msg::layout(&m.data, sb)?),
            msg::MSG_FILTER => filters = msg::filters(&m.data)?,
            _ => {}
        }
    }
    match (shape, dt, layout) {
        (Some(shape), Some((dtype, endian)), Some(layout)) => {
            Ok(Some(DatasetInfo { dtype, endian, shape, layout, filters }))
        }
        _ => Ok(None),
    }
}

fn unfilter(mut buf: Vec<u8>, filters: &[u16], expect: usize, esz: usize) -> Result<Vec<u8>> {
    for f in filters.iter().rev() {
        buf = match f {
            1 => decompress_zlib(&buf, expect)?,
            2 => unshuffle(&buf, esz),
            3 => {
                buf.truncate(buf.len().saturating_sub(4)); // fletcher32 checksum
                buf
            }
            other => {
                return Err(Error::unsupported(format!(
                    "HDF5 filter id {other} (only deflate, shuffle and fletcher32 are built in)"
                )))
            }
        };
    }
    Ok(buf)
}

fn unshuffle(buf: &[u8], esz: usize) -> Vec<u8> {
    if esz <= 1 {
        return buf.to_vec();
    }
    let n = buf.len() / esz;
    let mut out = vec![0u8; buf.len()];
    for i in 0..esz {
        for j in 0..n {
            out[j * esz + i] = buf[i * n + j];
        }
    }
    out
}

/// Read and de-filter the whole payload of a dataset.
pub fn raw_bytes(file: &[u8], sb: &Superblock, d: &DatasetInfo) -> Result<Vec<u8>> {
    let esz = d.dtype.size().ok_or_else(|| Error::unsupported("variable-size HDF5 datatype"))?;
    let total: usize = d.shape.iter().product::<usize>() * esz;
    match &d.layout {
        Layout::Compact(b) => Ok(b.clone()),
        Layout::Contiguous { address, size } => {
            if sb.undefined(*address) {
                return Ok(vec![0u8; total]);
            }
            // The layout message records the stored extent; if it disagrees
            // with what the dataspace implies, the file is inconsistent.
            if (*size as usize) < total {
                return Err(Error::format(format!(
                    "contiguous dataset stores {size} bytes but its shape needs {total}"
                )));
            }
            let p = sb.at(*address);
            Ok(file
                .get(p..p + total)
                .ok_or_else(|| Error::format("contiguous dataset runs past EOF"))?
                .to_vec())
        }
        Layout::Chunked { address, dims, elem_size } => {
            let mut out = vec![0u8; total];
            if sb.undefined(*address) {
                return Ok(out);
            }
            let mut chunks = Vec::new();
            btree_chunks(file, sb, *address, dims.len(), &mut chunks)?;
            for (offsets, addr, size, _mask) in chunks {
                let p = sb.at(addr);
                let raw = file
                    .get(p..p + size)
                    .ok_or_else(|| Error::format("chunk runs past EOF"))?
                    .to_vec();
                let chunk_elems: usize = dims.iter().product();
                let data = unfilter(raw, &d.filters, chunk_elems * *elem_size, *elem_size)?;
                scatter(&mut out, &data, &offsets, dims, &d.shape, esz);
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb() -> Superblock {
        Superblock {
            version: 3,
            offset_size: 4,
            length_size: 4,
            base_address: 2,
            root_address: 0,
            eof_address: 128,
        }
    }

    fn dataset(dtype: DType, shape: Vec<usize>, layout: Layout) -> DatasetInfo {
        DatasetInfo { dtype, endian: Endian::Little, shape, layout, filters: Vec::new() }
    }

    #[test]
    fn info_returns_none_until_all_required_messages_are_present() {
        let sb = sb();
        assert!(info(&[], &sb).unwrap().is_none());
        assert!(info(&[Message { kind: msg::MSG_DATATYPE, data: vec![] }], &sb).is_err());
        assert!(info(&[Message { kind: 0xffff, data: vec![] }], &sb).unwrap().is_none());

        let wide_sb = Superblock { offset_size: 8, length_size: 8, ..sb.clone() };
        let messages = [
            Message { kind: msg::MSG_DATASPACE, data: super::super::encode::dataspace(&[2]) },
            Message {
                kind: msg::MSG_DATATYPE,
                data: super::super::encode::datatype(&DType::U8).unwrap(),
            },
            Message { kind: msg::MSG_LAYOUT, data: super::super::encode::layout_contiguous(16, 2) },
            Message { kind: msg::MSG_FILTER, data: vec![2, 1, 1, 0, 0, 0, 0, 0] },
        ];
        let parsed = info(&messages, &wide_sb).unwrap().unwrap();
        assert_eq!(parsed.shape, [2]);
        assert_eq!(parsed.dtype, DType::U8);
        assert!(matches!(parsed.layout, Layout::Contiguous { address: 16, size: 2 }));
        assert_eq!(parsed.filters, [1]);
    }

    #[test]
    fn unshuffle_handles_single_byte_and_interleaved_element_bytes() {
        assert_eq!(unshuffle(&[1, 2, 3], 0), [1, 2, 3]);
        assert_eq!(unshuffle(&[1, 2, 3], 1), [1, 2, 3]);
        assert_eq!(unshuffle(&[1, 3, 2, 4], 2), [1, 2, 3, 4]);
    }

    #[test]
    fn unfilter_applies_reverse_pipeline_and_rejects_unknown_ids() {
        assert_eq!(unfilter(vec![1, 2, 3], &[], 3, 1).unwrap(), [1, 2, 3]);
        assert_eq!(unfilter(vec![1, 2, 3, 4, 5, 6], &[3], 2, 1).unwrap(), [1, 2]);
        assert_eq!(unfilter(vec![1, 3, 2, 4], &[2], 4, 2).unwrap(), [1, 2, 3, 4]);
        assert!(unfilter(vec![1], &[99], 1, 1).is_err());
    }

    #[cfg(feature = "deflate")]
    #[test]
    fn unfilter_decompresses_zlib_payloads() {
        let encoded = crate::util::compress_zlib(b"filtered data", 6).unwrap();
        assert_eq!(unfilter(encoded, &[1], 13, 1).unwrap(), b"filtered data");
    }

    #[test]
    fn raw_bytes_reads_compact_contiguous_and_undefined_layouts() {
        let sb = Superblock { offset_size: 8, length_size: 8, ..sb() };
        let compact = dataset(DType::U8, vec![2], Layout::Compact(vec![7, 8]));
        assert_eq!(raw_bytes(&[], &sb, &compact).unwrap(), [7, 8]);

        let contiguous = dataset(DType::U8, vec![4], Layout::Contiguous { address: 8, size: 4 });
        let mut file = vec![0; 16];
        file[10..14].copy_from_slice(&[9, 8, 7, 6]);
        assert_eq!(raw_bytes(&file, &sb, &contiguous).unwrap(), [9, 8, 7, 6]);

        let undefined = u64::MAX;
        let sparse =
            dataset(DType::U16, vec![2], Layout::Contiguous { address: undefined, size: 0 });
        assert_eq!(raw_bytes(&[], &sb, &sparse).unwrap(), [0; 4]);
        let chunked_sparse = dataset(
            DType::U8,
            vec![3],
            Layout::Chunked { address: undefined, dims: vec![2], elem_size: 1 },
        );
        assert_eq!(raw_bytes(&[], &sb, &chunked_sparse).unwrap(), [0; 3]);
    }

    #[test]
    fn raw_bytes_reads_chunked_storage_from_a_v1_btree() {
        let sb = Superblock { offset_size: 8, length_size: 8, ..sb() };
        let chunked = dataset(
            DType::U8,
            vec![2],
            Layout::Chunked { address: 0, dims: vec![2], elem_size: 1 },
        );
        let mut file = vec![0; 96];
        file[2..6].copy_from_slice(b"TREE");
        file[6] = 1; // v1 chunk B-tree
        file[7] = 0; // leaf node
        file[8..10].copy_from_slice(&1u16.to_le_bytes());
        file[10..26].fill(0); // sibling addresses
        file[26..30].copy_from_slice(&2u32.to_le_bytes());
        file[30..34].copy_from_slice(&0u32.to_le_bytes()); // filter mask
        file[34..42].copy_from_slice(&0u64.to_le_bytes()); // chunk offset
        file[42..50].copy_from_slice(&0u64.to_le_bytes()); // unused trailing offset
        file[50..58].copy_from_slice(&80u64.to_le_bytes()); // chunk address
        file[82..84].copy_from_slice(&[4, 5]); // sb.at(80) = 82

        assert_eq!(raw_bytes(&file, &sb, &chunked).unwrap(), [4, 5]);
    }

    #[test]
    fn raw_bytes_rejects_variable_types_short_extents_and_eof_overruns() {
        let sb = sb();
        assert!(
            raw_bytes(&[], &sb, &dataset(DType::Object, vec![1], Layout::Compact(vec![]))).is_err()
        );
        let too_short = dataset(DType::U32, vec![2], Layout::Contiguous { address: 8, size: 4 });
        assert!(raw_bytes(&[], &sb, &too_short).is_err());
        let past_eof = dataset(DType::U8, vec![8], Layout::Contiguous { address: 8, size: 8 });
        assert!(raw_bytes(&[0; 12], &sb, &past_eof).is_err());
    }
}
