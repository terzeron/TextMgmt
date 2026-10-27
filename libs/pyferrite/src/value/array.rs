//! The dynamically typed n-dimensional array that every reader produces.

use crate::dtype::DType;
use crate::num::{Complex32, Complex64, BF16, F128, F16, F8E4M3, F8E5M2};
use ndarray::ArrayD;

/// An owned, **mutable** n-dimensional array tagged with its element type.
///
/// Because the value is owned, `let mut a = ...` is all you need in order to
/// scale, normalise or quantise weights in place before feeding a model.
#[derive(Clone, Debug, PartialEq)]
pub enum Array {
    /// Booleans, one byte each, as in numpy `bool_`.
    Bool(ArrayD<bool>),
    /// Signed 8-bit integers (`int8`).
    I8(ArrayD<i8>),
    /// Signed 16-bit integers (`int16`).
    I16(ArrayD<i16>),
    /// Signed 32-bit integers (`int32`).
    I32(ArrayD<i32>),
    /// Signed 64-bit integers (`int64`).
    I64(ArrayD<i64>),
    /// Unsigned 8-bit integers (`uint8`).
    U8(ArrayD<u8>),
    /// Unsigned 16-bit integers (`uint16`).
    U16(ArrayD<u16>),
    /// Unsigned 32-bit integers (`uint32`).
    U32(ArrayD<u32>),
    /// Unsigned 64-bit integers (`uint64`).
    U64(ArrayD<u64>),
    /// 8-bit float, 4 exponent and 3 mantissa bits; no infinities, maximum 448.
    F8E4M3(ArrayD<F8E4M3>),
    /// 8-bit float, 5 exponent and 2 mantissa bits; IEEE-shaped, maximum 57344.
    F8E5M2(ArrayD<F8E5M2>),
    /// IEEE binary16 half precision (`float16`).
    F16(ArrayD<F16>),
    /// bfloat16: the top 16 bits of an `f32`, keeping its exponent range.
    BF16(ArrayD<BF16>),
    /// IEEE binary32 single precision (`float32`).
    F32(ArrayD<f32>),
    /// IEEE binary64 double precision (`float64`).
    F64(ArrayD<f64>),
    /// IEEE binary128 quadruple precision, with software arithmetic.
    F128(ArrayD<F128>),
    /// Complex pair of `f32` (`complex64`).
    C64(ArrayD<Complex32>),
    /// Complex pair of `f64` (`complex128`).
    C128(ArrayD<Complex64>),
    /// Fixed-width UTF-32 text, the inner value being the character count.
    Str(ArrayD<String>),
    /// Fixed-width byte string, the inner value being the length.
    Bytes(ArrayD<Vec<u8>>),
}

macro_rules! for_each {
    ($self:expr, $a:ident => $body:expr) => {
        match $self {
            Array::Bool($a) => $body,
            Array::I8($a) => $body,
            Array::I16($a) => $body,
            Array::I32($a) => $body,
            Array::I64($a) => $body,
            Array::U8($a) => $body,
            Array::U16($a) => $body,
            Array::U32($a) => $body,
            Array::U64($a) => $body,
            Array::F8E4M3($a) => $body,
            Array::F8E5M2($a) => $body,
            Array::F16($a) => $body,
            Array::BF16($a) => $body,
            Array::F32($a) => $body,
            Array::F64($a) => $body,
            Array::F128($a) => $body,
            Array::C64($a) => $body,
            Array::C128($a) => $body,
            Array::Str($a) => $body,
            Array::Bytes($a) => $body,
        }
    };
}

impl Array {
    /// Element type tag.
    pub fn dtype(&self) -> DType {
        match self {
            Array::Bool(_) => DType::Bool,
            Array::I8(_) => DType::I8,
            Array::I16(_) => DType::I16,
            Array::I32(_) => DType::I32,
            Array::I64(_) => DType::I64,
            Array::U8(_) => DType::U8,
            Array::U16(_) => DType::U16,
            Array::U32(_) => DType::U32,
            Array::U64(_) => DType::U64,
            Array::F8E4M3(_) => DType::F8E4M3,
            Array::F8E5M2(_) => DType::F8E5M2,
            Array::F16(_) => DType::F16,
            Array::BF16(_) => DType::BF16,
            Array::F32(_) => DType::F32,
            Array::F64(_) => DType::F64,
            Array::F128(_) => DType::F128,
            Array::C64(_) => DType::C64,
            Array::C128(_) => DType::C128,
            Array::Str(a) => DType::Str(a.iter().map(|s| s.chars().count()).max().unwrap_or(0)),
            Array::Bytes(a) => DType::Bytes(a.iter().map(|b| b.len()).max().unwrap_or(0)),
        }
    }

    /// Shape as a slice, exactly as numpy would report it.
    pub fn shape(&self) -> &[usize] {
        for_each!(self, a => a.shape())
    }

    /// Total number of elements.
    pub fn len(&self) -> usize {
        for_each!(self, a => a.len())
    }

    /// `true` when the array holds no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of dimensions.
    pub fn ndim(&self) -> usize {
        self.shape().len()
    }

    /// Reshape in place; fails when the element count would change.
    pub fn reshape(&mut self, shape: &[usize]) -> crate::Result<()> {
        let want: usize = shape.iter().product();
        if want != self.len() {
            return Err(crate::Error::Shape { expected: shape.to_vec(), got: self.len() });
        }
        let d = ndarray::IxDyn(shape);
        for_each!(self, a => {
            let tmp = std::mem::replace(a, ArrayD::from_shape_vec(ndarray::IxDyn(&[0]), vec![]).unwrap());
            *a = tmp.into_shape(d).map_err(|e| crate::Error::format(e.to_string()))?;
        });
        Ok(())
    }
}

macro_rules! accessors {
    ($($variant:ident, $ty:ty, $get:ident, $get_mut:ident, $into:ident);* $(;)?) => {
        impl Array {
            $(
                #[doc = concat!("Borrow as `ArrayD<", stringify!($ty), ">` when the tag matches.")]
                pub fn $get(&self) -> Option<&ArrayD<$ty>> {
                    match self { Array::$variant(a) => Some(a), _ => None }
                }
                #[doc = concat!("Mutably borrow as `ArrayD<", stringify!($ty), ">` when the tag matches.")]
                pub fn $get_mut(&mut self) -> Option<&mut ArrayD<$ty>> {
                    match self { Array::$variant(a) => Some(a), _ => None }
                }
                #[doc = concat!("Consume into `ArrayD<", stringify!($ty), ">` when the tag matches.")]
                pub fn $into(self) -> Option<ArrayD<$ty>> {
                    match self { Array::$variant(a) => Some(a), _ => None }
                }
            )*
        }
    };
}

accessors! {
    Bool, bool, as_bool, as_bool_mut, into_bool;
    I8, i8, as_i8, as_i8_mut, into_i8;
    I16, i16, as_i16, as_i16_mut, into_i16;
    I32, i32, as_i32, as_i32_mut, into_i32;
    I64, i64, as_i64, as_i64_mut, into_i64;
    U8, u8, as_u8, as_u8_mut, into_u8;
    U16, u16, as_u16, as_u16_mut, into_u16;
    U32, u32, as_u32, as_u32_mut, into_u32;
    U64, u64, as_u64, as_u64_mut, into_u64;
    F16, F16, as_f16, as_f16_mut, into_f16;
    BF16, BF16, as_bf16, as_bf16_mut, into_bf16;
    F32, f32, as_f32, as_f32_mut, into_f32;
    F64, f64, as_f64, as_f64_mut, into_f64;
    F128, F128, as_f128, as_f128_mut, into_f128;
    C64, Complex32, as_c64, as_c64_mut, into_c64;
    C128, Complex64, as_c128, as_c128_mut, into_c128;
    Str, String, as_str_array, as_str_array_mut, into_str_array;
    Bytes, Vec<u8>, as_bytes_array, as_bytes_array_mut, into_bytes_array;
}
