//! `bfloat16` (torch `bfloat16`, ml_dtypes `bfloat16`) — the top 16 bits of an `f32`.

use std::cmp::Ordering;
use std::fmt;

/// Brain-float 16 stored as raw 16 bits.
#[derive(Clone, Copy, Default, PartialEq)]
#[repr(transparent)]
pub struct BF16(pub u16);

impl BF16 {
    /// `ZERO`.
    pub const ZERO: BF16 = BF16(0);
    /// `ONE`.
    pub const ONE: BF16 = BF16(0x3F80);

    /// Build one from bits.
    #[inline]
    pub fn from_bits(b: u16) -> Self {
        BF16(b)
    }
    /// Convert to bits.
    #[inline]
    pub fn to_bits(self) -> u16 {
        self.0
    }

    /// Round-to-nearest-even narrowing of an `f32`.
    pub fn from_f32(v: f32) -> Self {
        let x = v.to_bits();
        if (x & 0x7F80_0000) == 0x7F80_0000 && (x & 0x007F_FFFF) != 0 {
            return BF16(((x >> 16) as u16) | 0x0040);
        }
        let rounding = 0x7FFFu32 + ((x >> 16) & 1);
        BF16((x.wrapping_add(rounding) >> 16) as u16)
    }

    /// Exact widening to `f32`.
    #[inline]
    pub fn to_f32(self) -> f32 {
        f32::from_bits((self.0 as u32) << 16)
    }
    /// Build one from f64.
    #[inline]
    pub fn from_f64(v: f64) -> Self {
        BF16::from_f32(v as f32)
    }
    /// Convert to f64.
    #[inline]
    pub fn to_f64(self) -> f64 {
        self.to_f32() as f64
    }
    /// `true` when this is nan.
    #[inline]
    pub fn is_nan(self) -> bool {
        self.to_f32().is_nan()
    }
}

impl fmt::Debug for BF16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f32())
    }
}
impl fmt::Display for BF16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f32())
    }
}
impl PartialOrd for BF16 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.to_f32().partial_cmp(&other.to_f32())
    }
}
