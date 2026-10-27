//! IEEE-754 `binary16` (numpy `float16`, torch `half`) — pure Rust, no `half` crate.

use std::cmp::Ordering;
use std::fmt;

/// Half precision float stored as raw 16 bits.
#[derive(Clone, Copy, Default, PartialEq)]
#[repr(transparent)]
pub struct F16(pub u16);

impl F16 {
    /// `ZERO`.
    pub const ZERO: F16 = F16(0);
    /// `ONE`.
    pub const ONE: F16 = F16(0x3C00);
    /// `NAN`.
    pub const NAN: F16 = F16(0x7E00);
    /// `INFINITY`.
    pub const INFINITY: F16 = F16(0x7C00);
    /// `NEG_INFINITY`.
    pub const NEG_INFINITY: F16 = F16(0xFC00);

    /// Reinterpret raw bits.
    #[inline]
    pub fn from_bits(b: u16) -> Self {
        F16(b)
    }
    /// Convert to bits.
    #[inline]
    pub fn to_bits(self) -> u16 {
        self.0
    }

    /// Convert an `f32` using round-to-nearest-even, with correct handling of
    /// sub-normals, overflow to infinity and NaN preservation.
    pub fn from_f32(v: f32) -> Self {
        let x = v.to_bits();
        let sign = ((x >> 16) & 0x8000) as u16;
        let mut exp = ((x >> 23) & 0xFF) as i32;
        let mant = x & 0x007F_FFFF;

        if exp == 0xFF {
            let m = (mant >> 13) as u16;
            return F16(sign | 0x7C00 | if mant != 0 { m.max(1) } else { 0 });
        }
        exp = exp - 127 + 15;
        if exp >= 0x1F {
            return F16(sign | 0x7C00);
        }
        if exp <= 0 {
            if exp < -10 {
                return F16(sign);
            }
            let mant = mant | 0x0080_0000;
            let shift = (14 - exp) as u32;
            let mut m = mant >> shift;
            let rem = mant & ((1u32 << shift) - 1);
            let half = 1u32 << (shift - 1);
            if rem > half || (rem == half && (m & 1) == 1) {
                m += 1;
            }
            return F16(sign | m as u16);
        }
        let mut m = (mant >> 13) as u16;
        let rem = mant & 0x1FFF;
        let mut e = exp as u16;
        if rem > 0x1000 || (rem == 0x1000 && (m & 1) == 1) {
            m += 1;
            if m == 0x400 {
                m = 0;
                e += 1;
                if e >= 0x1F {
                    return F16(sign | 0x7C00);
                }
            }
        }
        F16(sign | (e << 10) | m)
    }

    /// Exact widening to `f32` (always lossless).
    pub fn to_f32(self) -> f32 {
        let h = self.0 as u32;
        let sign = (h & 0x8000) << 16;
        let exp = (h >> 10) & 0x1F;
        let mant = h & 0x3FF;
        let bits = if exp == 0 {
            if mant == 0 {
                sign
            } else {
                let lz = mant.leading_zeros() - 22;
                let e = 127 - 15 - lz;
                sign | (e << 23) | (((mant << (lz + 1)) & 0x3FF) << 13)
            }
        } else if exp == 0x1F {
            sign | 0x7F80_0000 | (mant << 13)
        } else {
            sign | ((exp + 127 - 15) << 23) | (mant << 13)
        };
        f32::from_bits(bits)
    }

    /// Build one from f64.
    #[inline]
    pub fn from_f64(v: f64) -> Self {
        F16::from_f32(v as f32)
    }
    /// Convert to f64.
    #[inline]
    pub fn to_f64(self) -> f64 {
        self.to_f32() as f64
    }
    /// `true` when this is nan.
    #[inline]
    pub fn is_nan(self) -> bool {
        (self.0 & 0x7C00) == 0x7C00 && (self.0 & 0x03FF) != 0
    }
}

impl fmt::Debug for F16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f32())
    }
}
impl fmt::Display for F16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f32())
    }
}
impl PartialOrd for F16 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.to_f32().partial_cmp(&other.to_f32())
    }
}
