//! Parsing and rendering of numpy `descr` strings such as `'<f8'`, `'|S12'`.

use super::{DType, Field};
use crate::error::{Error, Result};

/// Byte order carried by a numpy `descr`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endian {
    /// Least significant byte first.
    Little,
    /// Most significant byte first.
    Big,
    /// `|` — order is irrelevant (single-byte or opaque types).
    Native,
}

impl Endian {
    /// `true` when the on-disk order differs from this machine's order.
    pub fn needs_swap(self) -> bool {
        match self {
            Endian::Little => cfg!(target_endian = "big"),
            Endian::Big => cfg!(target_endian = "little"),
            Endian::Native => false,
        }
    }
    /// Borrow the contents as char, when the type matches.
    pub fn as_char(self) -> char {
        match self {
            Endian::Little => '<',
            Endian::Big => '>',
            Endian::Native => '|',
        }
    }
}

/// Parse a scalar numpy `descr` string into `(DType, Endian)`.
///
/// Supported forms: `b1`, `i1..i8`, `u1..u8`, `f2/f4/f8/f16`, `c8/c16/c32`,
/// `U{n}`, `S{n}`, `a{n}`, `O`, `V{n}` (treated as raw bytes), each optionally
/// prefixed by `<`, `>`, `=` or `|`.
pub fn parse_descr(s: &str) -> Result<(DType, Endian)> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::format("empty dtype descriptor"));
    }
    let (endian, rest) = match s.as_bytes()[0] {
        b'<' => (Endian::Little, &s[1..]),
        b'>' => (Endian::Big, &s[1..]),
        b'=' => {
            (if cfg!(target_endian = "little") { Endian::Little } else { Endian::Big }, &s[1..])
        }
        b'|' => (Endian::Native, &s[1..]),
        _ => (Endian::Native, s),
    };
    if rest.is_empty() {
        return Err(Error::format(format!("dtype `{s}` has no type character")));
    }
    let kind = rest.as_bytes()[0] as char;
    let n: usize = rest[1..].parse().unwrap_or(0);
    let dt = match (kind, n) {
        ('b', _) => DType::Bool,
        ('?', _) => DType::Bool,
        ('i', 1) => DType::I8,
        ('i', 2) => DType::I16,
        ('i', 4) => DType::I32,
        ('i', 8) => DType::I64,
        ('u', 1) => DType::U8,
        ('u', 2) => DType::U16,
        ('u', 4) => DType::U32,
        ('u', 8) => DType::U64,
        ('f', 2) => DType::F16,
        ('f', 4) => DType::F32,
        ('f', 8) => DType::F64,
        ('f', 12) | ('f', 16) => DType::X87,
        ('g', _) => DType::X87,
        ('c', 8) => DType::C64,
        ('c', 16) => DType::C128,
        ('c', 24) | ('c', 32) => DType::C256,
        ('U', k) => DType::Str(k),
        ('S', k) | ('a', k) => DType::Bytes(k),
        ('V', k) => DType::Bytes(k),
        ('O', _) => DType::Object,
        ('M', _) | ('m', _) => DType::I64, // datetime64 / timedelta64 payload
        _ => return Err(Error::unsupported(format!("numpy dtype `{s}`"))),
    };
    Ok((dt, endian))
}

/// Render a [`DType`] back into a numpy `descr` string.
pub fn to_descr(dt: &DType, endian: Endian) -> Result<String> {
    let e = endian.as_char();
    let one = |c: char| format!("|{c}1");
    Ok(match dt {
        DType::Bool => one('b'),
        DType::I8 => "|i1".into(),
        DType::U8 => "|u1".into(),
        DType::F8E4M3 | DType::F8E5M2 => "|u1".into(),
        DType::I16 => format!("{e}i2"),
        DType::I32 => format!("{e}i4"),
        DType::I64 => format!("{e}i8"),
        DType::U16 => format!("{e}u2"),
        DType::U32 => format!("{e}u4"),
        DType::U64 => format!("{e}u8"),
        DType::F16 | DType::BF16 => format!("{e}f2"),
        DType::F32 => format!("{e}f4"),
        DType::F64 => format!("{e}f8"),
        DType::F128 | DType::X87 => format!("{e}f16"),
        DType::C64 => format!("{e}c8"),
        DType::C128 => format!("{e}c16"),
        DType::C256 => format!("{e}c32"),
        DType::Str(n) => format!("{e}U{n}"),
        DType::Bytes(n) => format!("|S{n}"),
        DType::Object => "|O".into(),
        DType::Struct(_) => {
            return Err(Error::unsupported("structured dtype in scalar descr position"))
        }
    })
}

/// Build a structured dtype from `(name, descr, shape)` triples.
pub fn struct_from_fields(items: Vec<(String, String, Vec<usize>)>) -> Result<DType> {
    let mut fields = Vec::with_capacity(items.len());
    for (name, descr, shape) in items {
        let (dtype, _) = parse_descr(&descr)?;
        fields.push(Field { name, dtype, shape });
    }
    Ok(DType::Struct(fields))
}

#[cfg(test)]
#[path = "../../../../tests/rust/pyferrite/dtype_descr_unit.rs"]
mod tests;
