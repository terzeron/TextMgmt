//! Version-1 chunk B-trees and the scatter of chunks into a dense buffer.

use super::superblock::Superblock;
use crate::error::{Error, Result};
use crate::util::Cursor;

/// Copy one chunk into the right place of the dense output buffer.
pub(super) fn scatter(
    out: &mut [u8],
    chunk: &[u8],
    offsets: &[usize],
    dims: &[usize],
    shape: &[usize],
    esz: usize,
) {
    let rank = shape.len();
    if rank == 0 {
        let n = esz.min(chunk.len()).min(out.len());
        out[..n].copy_from_slice(&chunk[..n]);
        return;
    }
    let row = dims[rank - 1];
    let rows: usize = dims[..rank - 1].iter().product();
    let mut idx = vec![0usize; rank.saturating_sub(1)];
    for r in 0..rows {
        let mut flat = 0usize;
        let mut stride = 1usize;
        // C-order flat index of (offsets + idx, 0)
        for d in (0..rank).rev() {
            let coord = if d == rank - 1 { offsets[d] } else { offsets[d] + idx[d] };
            if coord >= shape[d] {
                flat = usize::MAX;
                break;
            }
            flat = flat.saturating_add(coord * stride);
            stride *= shape[d];
        }
        if flat != usize::MAX {
            let avail = shape[rank - 1].saturating_sub(offsets[rank - 1]).min(row);
            let src = r * row * esz;
            let dst = flat * esz;
            if src + avail * esz <= chunk.len() && dst + avail * esz <= out.len() {
                out[dst..dst + avail * esz].copy_from_slice(&chunk[src..src + avail * esz]);
            }
        }
        for d in (0..idx.len()).rev() {
            idx[d] += 1;
            if idx[d] < dims[d] {
                break;
            }
            idx[d] = 0;
        }
    }
}

type ChunkRec = (Vec<usize>, u64, usize, u32);

/// Walk a version-1 chunk B-tree, collecting every leaf chunk.
pub(super) fn btree_chunks(
    file: &[u8],
    sb: &Superblock,
    addr: u64,
    rank: usize,
    out: &mut Vec<ChunkRec>,
) -> Result<()> {
    let p = sb.at(addr);
    let mut c = Cursor::at(file, p);
    if c.take(4)? != b"TREE" {
        return Err(Error::format("bad chunk B-tree signature"));
    }
    let node_type = c.u8()?;
    let level = c.u8()?;
    let entries = c.u16()? as usize;
    c.skip(2 * sb.offset_size)?;
    if node_type != 1 {
        return Err(Error::format("expected a chunk B-tree"));
    }
    let mut keys = Vec::with_capacity(entries);
    let mut kids = Vec::with_capacity(entries);
    for _ in 0..entries {
        let size = c.u32()? as usize;
        let mask = c.u32()?;
        let mut offs = Vec::with_capacity(rank + 1);
        for _ in 0..=rank {
            offs.push(c.u64()? as usize);
        }
        offs.pop();
        keys.push((offs, size, mask));
        kids.push(c.uint(sb.offset_size)?);
    }
    for (i, k) in kids.into_iter().enumerate() {
        if level > 0 {
            btree_chunks(file, sb, k, rank, out)?;
        } else {
            let (offs, size, mask) = keys[i].clone();
            out.push((offs, k, size, mask));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn superblock() -> Superblock {
        Superblock {
            version: 0,
            offset_size: 8,
            length_size: 8,
            base_address: 0,
            root_address: 0,
            eof_address: 4096,
        }
    }

    fn node(node_type: u8, level: u8, child: u64) -> Vec<u8> {
        let mut bytes = b"TREE".to_vec();
        bytes.extend([node_type, level]);
        bytes.extend(1u16.to_le_bytes());
        bytes.extend([0; 16]); // left and right sibling addresses
        bytes.extend(4u32.to_le_bytes()); // chunk size
        bytes.extend(0u32.to_le_bytes()); // filter mask
        bytes.extend(7u64.to_le_bytes()); // first chunk offset
        bytes.extend(0u64.to_le_bytes()); // unused trailing dimension
        bytes.extend(child.to_le_bytes());
        bytes
    }

    #[test]
    fn scatter_copies_scalars_and_partial_rows() {
        let mut scalar = [0; 3];
        scatter(&mut scalar, &[1, 2], &[], &[], &[], 3);
        assert_eq!(scalar, [1, 2, 0]);

        let mut row = [0; 4];
        scatter(&mut row, &[9, 8, 7], &[1], &[3], &[4], 1);
        assert_eq!(row, [0, 9, 8, 7]);
    }

    #[test]
    fn scatter_places_multidimensional_chunks_and_skips_out_of_bounds_data() {
        let mut out = [0; 9];
        scatter(&mut out, &[10, 11, 12, 13], &[1, 1], &[2, 2], &[3, 3], 1);
        assert_eq!(out, [0, 0, 0, 0, 10, 11, 0, 12, 13]);

        let mut too_short = [0; 4];
        scatter(&mut too_short, &[1, 2, 3, 4], &[2, 2], &[2, 2], &[3, 3], 1);
        assert_eq!(too_short, [0; 4]); // destination range is outside the output
        scatter(&mut too_short, &[1, 2], &[0], &[4], &[4], 1);
        assert_eq!(too_short, [0; 4]); // source range is outside the chunk
        scatter(&mut out, &[1], &[4, 4], &[1, 1], &[3, 3], 1);
        assert_eq!(out, [0, 0, 0, 0, 10, 11, 0, 12, 13]);
    }

    #[test]
    fn btree_reads_leaf_chunks_and_recurses_through_internal_nodes() {
        let sb = superblock();
        let mut leaf_out = Vec::new();
        btree_chunks(&node(1, 0, 123), &sb, 0, 1, &mut leaf_out).unwrap();
        assert_eq!(leaf_out, vec![(vec![7], 123, 4, 0)]);

        let mut file = node(1, 1, 64);
        file.resize(64, 0);
        file.extend(node(1, 0, 512));
        let mut nested = Vec::new();
        btree_chunks(&file, &sb, 0, 1, &mut nested).unwrap();
        assert_eq!(nested, vec![(vec![7], 512, 4, 0)]);
    }

    #[test]
    fn btree_rejects_bad_signatures_node_types_and_truncated_nodes() {
        let sb = superblock();
        let mut chunks = Vec::new();
        assert!(btree_chunks(b"NOPE", &sb, 0, 1, &mut chunks).is_err());
        assert!(btree_chunks(&node(0, 0, 10), &sb, 0, 1, &mut chunks).is_err());
        assert!(btree_chunks(b"TREE", &sb, 0, 1, &mut chunks).is_err());
    }
}
