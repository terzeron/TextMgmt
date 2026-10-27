//! Description of element types shared by numpy, torch, HDF5 and this crate.

mod descr;
mod kind;

pub use descr::{parse_descr, struct_from_fields, to_descr, Endian};
pub use kind::{FloatKind, IntKind};

/// A concrete element type.
///
/// Every reader maps its native type tags onto this enum, and every writer maps
/// it back, so the conversion matrix is `N + M` instead of `N * M`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DType {
    /// Booleans, one byte each, as in numpy `bool_`.
    Bool,
    /// Signed 8-bit integers (`int8`).
    I8,
    /// Signed 16-bit integers (`int16`).
    I16,
    /// Signed 32-bit integers (`int32`).
    I32,
    /// Signed 64-bit integers (`int64`).
    I64,
    /// Unsigned 8-bit integers (`uint8`).
    U8,
    /// Unsigned 16-bit integers (`uint16`).
    U16,
    /// Unsigned 32-bit integers (`uint32`).
    U32,
    /// Unsigned 64-bit integers (`uint64`).
    U64,
    /// `float8_e4m3fn`
    F8E4M3,
    /// `float8_e5m2`
    F8E5M2,
    /// IEEE `binary16`
    F16,
    /// `bfloat16`
    BF16,
    /// IEEE binary32 single precision (`float32`).
    F32,
    /// IEEE binary64 double precision (`float64`).
    F64,
    /// IEEE `binary128`
    F128,
    /// x87 80-bit extended (numpy `longdouble` on x86)
    X87,
    /// `complex64` = 2 x f32
    C64,
    /// `complex128` = 2 x f64
    C128,
    /// `complex256` = 2 x longdouble / binary128
    C256,
    /// Fixed-width UTF-32 string, `n` code points (numpy `<U{n}`).
    Str(usize),
    /// Fixed-width byte string, `n` bytes (numpy `|S{n}`).
    Bytes(usize),
    /// Opaque Python object (numpy `O`) — stored as a nested pickle.
    Object,
    /// Structured / record dtype: an ordered list of named fields.
    Struct(Vec<Field>),
}

/// One field of a structured dtype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Name of this item.
    pub name: String,
    /// Element type.
    pub dtype: DType,
    /// Sub-array shape, e.g. `('a', '<f8', (3, 2))`.
    pub shape: Vec<usize>,
}

impl DType {
    /// Size of one element in bytes, or `None` for variable-size objects.
    pub fn size(&self) -> Option<usize> {
        Some(match self {
            DType::Bool | DType::I8 | DType::U8 | DType::F8E4M3 | DType::F8E5M2 => 1,
            DType::I16 | DType::U16 | DType::F16 | DType::BF16 => 2,
            DType::I32 | DType::U32 | DType::F32 => 4,
            DType::I64 | DType::U64 | DType::F64 | DType::C64 => 8,
            DType::F128 | DType::C128 => 16,
            DType::X87 => 16,
            DType::C256 => 32,
            DType::Str(n) => 4 * n,
            DType::Bytes(n) => *n,
            DType::Object => return None,
            DType::Struct(fields) => {
                let mut total = 0usize;
                for f in fields {
                    let n: usize = f.shape.iter().product::<usize>().max(1);
                    total += f.dtype.size()? * n;
                }
                total
            }
        })
    }

    /// `true` for signed or unsigned integers (excluding `bool`).
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            DType::I8
                | DType::I16
                | DType::I32
                | DType::I64
                | DType::U8
                | DType::U16
                | DType::U32
                | DType::U64
        )
    }

    /// `true` for every real floating point type.
    pub fn is_float(&self) -> bool {
        matches!(
            self,
            DType::F8E4M3
                | DType::F8E5M2
                | DType::F16
                | DType::BF16
                | DType::F32
                | DType::F64
                | DType::F128
                | DType::X87
        )
    }

    /// `true` for `complex64` / `complex128` / `complex256`.
    pub fn is_complex(&self) -> bool {
        matches!(self, DType::C64 | DType::C128 | DType::C256)
    }

    /// `true` when the type carries no numeric payload (strings, objects...).
    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_float() || self.is_complex() || *self == DType::Bool
    }
}

#[cfg(test)]
#[path = "../../../../tests/rust/pyferrite/dtype_unit.rs"]
mod tests;
