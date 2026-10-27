//! One slice of a large array.

use crate::dtype::DType;
use crate::value::Array;

/// A contiguous run of elements taken from one logical array.
///
/// Chunks arrive in C order, so `offset` is a flat index into the fully
/// materialised array: element `offset + i` of `full_shape` is `data[i]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    /// Name of the array inside its container (`""` for bare `.npy`).
    pub name: String,
    /// Element type after any requested cast.
    pub dtype: DType,
    /// Shape of the *whole* array this chunk belongs to.
    pub full_shape: Vec<usize>,
    /// Flat index of the first element of this chunk.
    pub offset: usize,
    /// The chunk payload, always 1-D and always **mutable**.
    pub data: Array,
}

impl Chunk {
    /// Number of elements carried by this chunk.
    pub fn len(&self) -> usize {
        self.data.len()
    }
    /// `true` when this is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Total number of elements in the logical array.
    pub fn total(&self) -> usize {
        self.full_shape.iter().product()
    }
    /// `true` when this chunk reaches the end of the logical array.
    pub fn is_last(&self) -> bool {
        self.offset + self.len() >= self.total()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::arr1;

    fn chunk(offset: usize, values: &[i32], shape: &[usize]) -> Chunk {
        Chunk {
            name: "weights".into(),
            dtype: DType::I32,
            full_shape: shape.to_vec(),
            offset,
            data: Array::I32(arr1(values).into_dyn()),
        }
    }

    #[test]
    fn reports_chunk_and_full_array_sizes() {
        let c = chunk(1, &[3, 4], &[2, 2]);
        assert_eq!(c.name, "weights");
        assert_eq!(c.dtype, DType::I32);
        assert_eq!(c.len(), 2);
        assert!(!c.is_empty());
        assert_eq!(c.total(), 4);
        assert!(!c.is_last());
    }

    #[test]
    fn identifies_empty_and_final_chunks() {
        assert!(chunk(0, &[], &[0]).is_empty());
        assert!(chunk(0, &[], &[0]).is_last());
        assert!(chunk(2, &[3, 4], &[2, 2]).is_last());
        assert!(chunk(5, &[6], &[2, 2]).is_last());
    }
}
