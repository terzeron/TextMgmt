//! User-selectable numeric target types.
//!
//! These drive the `int_as` / `float_as` knobs of
//! [`ReadOptions`](crate::options::ReadOptions) and
//! [`WriteOptions`](crate::options::WriteOptions): *"I know these weights fit in
//! `f8`, store them as `f8`"*.

use super::DType;

/// Integer target width requested by the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntKind {
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
}

/// Floating point target width requested by the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatKind {
    /// 8-bit, 4 exponent / 3 mantissa bits.
    F8E4M3,
    /// 8-bit, 5 exponent / 2 mantissa bits.
    F8E5M2,
    /// IEEE binary16 half precision (`float16`).
    F16,
    /// bfloat16: the top 16 bits of an `f32`, keeping its exponent range.
    BF16,
    /// IEEE binary32 single precision (`float32`).
    F32,
    /// IEEE binary64 double precision (`float64`).
    F64,
    /// IEEE quad precision (software implemented).
    F128,
}

impl IntKind {
    /// Dtype.
    pub fn dtype(self) -> DType {
        match self {
            IntKind::I8 => DType::I8,
            IntKind::I16 => DType::I16,
            IntKind::I32 => DType::I32,
            IntKind::I64 => DType::I64,
            IntKind::U8 => DType::U8,
            IntKind::U16 => DType::U16,
            IntKind::U32 => DType::U32,
            IntKind::U64 => DType::U64,
        }
    }
    /// Parse from the usual short spellings (`"i32"`, `"u8"`, ...).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "i8" | "int8" => IntKind::I8,
            "i16" | "int16" => IntKind::I16,
            "i32" | "int32" => IntKind::I32,
            "i64" | "int64" => IntKind::I64,
            "u8" | "uint8" => IntKind::U8,
            "u16" | "uint16" => IntKind::U16,
            "u32" | "uint32" => IntKind::U32,
            "u64" | "uint64" => IntKind::U64,
            _ => return None,
        })
    }
}

impl FloatKind {
    /// Dtype.
    pub fn dtype(self) -> DType {
        match self {
            FloatKind::F8E4M3 => DType::F8E4M3,
            FloatKind::F8E5M2 => DType::F8E5M2,
            FloatKind::F16 => DType::F16,
            FloatKind::BF16 => DType::BF16,
            FloatKind::F32 => DType::F32,
            FloatKind::F64 => DType::F64,
            FloatKind::F128 => DType::F128,
        }
    }
    /// Parse from the usual short spellings (`"f8"`, `"bf16"`, `"f64"`, ...).
    ///
    /// Note that `"f8"` means *8-bit float* here (the user-facing meaning),
    /// **not** numpy's `f8` which is 8 *bytes*.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "f8" | "f8e4m3" | "float8" | "float8_e4m3fn" => FloatKind::F8E4M3,
            "f8e5m2" | "float8_e5m2" => FloatKind::F8E5M2,
            "f16" | "half" | "float16" => FloatKind::F16,
            "bf16" | "bfloat16" => FloatKind::BF16,
            "f32" | "float32" | "single" => FloatKind::F32,
            "f64" | "float64" | "double" => FloatKind::F64,
            "f128" | "float128" | "quad" => FloatKind::F128,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_kinds_parse_aliases_and_map_to_dtypes() {
        for (spellings, kind, dtype) in [
            (&["i8", "int8"][..], IntKind::I8, DType::I8),
            (&["i16", "int16"][..], IntKind::I16, DType::I16),
            (&["i32", "int32"][..], IntKind::I32, DType::I32),
            (&["i64", "int64"][..], IntKind::I64, DType::I64),
            (&["u8", "uint8"][..], IntKind::U8, DType::U8),
            (&["u16", "uint16"][..], IntKind::U16, DType::U16),
            (&["u32", "uint32"][..], IntKind::U32, DType::U32),
            (&["u64", "uint64"][..], IntKind::U64, DType::U64),
        ] {
            for spelling in spellings {
                assert_eq!(IntKind::parse(spelling), Some(kind));
            }
            assert_eq!(kind.dtype(), dtype);
        }
        assert_eq!(IntKind::parse("  INT32 "), Some(IntKind::I32));
        assert_eq!(IntKind::parse("i128"), None);
    }

    #[test]
    fn float_kinds_parse_aliases_and_map_to_dtypes() {
        for (spellings, kind, dtype) in [
            (&["f8", "f8e4m3", "float8", "float8_e4m3fn"][..], FloatKind::F8E4M3, DType::F8E4M3),
            (&["f8e5m2", "float8_e5m2"][..], FloatKind::F8E5M2, DType::F8E5M2),
            (&["f16", "half", "float16"][..], FloatKind::F16, DType::F16),
            (&["bf16", "bfloat16"][..], FloatKind::BF16, DType::BF16),
            (&["f32", "float32", "single"][..], FloatKind::F32, DType::F32),
            (&["f64", "float64", "double"][..], FloatKind::F64, DType::F64),
            (&["f128", "float128", "quad"][..], FloatKind::F128, DType::F128),
        ] {
            for spelling in spellings {
                assert_eq!(FloatKind::parse(spelling), Some(kind));
            }
            assert_eq!(kind.dtype(), dtype);
        }
        assert_eq!(FloatKind::parse("  F64 "), Some(FloatKind::F64));
        assert_eq!(FloatKind::parse("f80"), None);
    }
}
