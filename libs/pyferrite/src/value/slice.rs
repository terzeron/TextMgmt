//! Extracting sub-arrays and scalars from an [`Array`].
//!
//! These helpers exist mainly for the pandas bridge, which has to split
//! two-dimensional `BlockManager` blocks back into individual columns.

use super::Array;

impl Array {
    /// Row `r` of a two-dimensional array, as a standalone one-dimensional one.
    ///
    /// Returns `None` if the array is not two-dimensional or `r` is out of
    /// range. Used by the pandas bridge to split blocks into columns.
    pub fn row(&self, r: usize) -> Option<Array> {
        macro_rules! pick {
            ($($v:ident),* $(,)?) => {
                match self {
                    $(Array::$v(a) => {
                        let s = a.shape();
                        if s.len() != 2 || r >= s[0] {
                            return None;
                        }
                        let row: Vec<_> = (0..s[1]).map(|c| a[[r, c]].clone()).collect();
                        Some(Array::$v(
                            ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&[s[1]]), row).ok()?,
                        ))
                    })*
                }
            };
        }
        pick!(
            Bool, I8, I16, I32, I64, U8, U16, U32, U64, F8E4M3, F8E5M2, F16, BF16, F32, F64, F128,
            C64, C128, Str, Bytes,
        )
    }

    /// Element `i` rendered as a string, when the element type is textual.
    ///
    /// Useful for reading column labels out of an object or string array.
    pub fn as_str_at(&self, i: usize) -> Option<String> {
        match self {
            Array::Str(a) => a.iter().nth(i).cloned(),
            Array::Bytes(a) => a.iter().nth(i).map(|b| String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }
}
