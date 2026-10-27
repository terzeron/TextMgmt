//! IEEE-754 `binary128` (quad precision), implemented purely in software.
//!
//! Rust has no native `f128` on stable, and hardware quad support is rare, so
//! [`F128`] stores the raw 128 bits and implements conversion + arithmetic in
//! this crate (see `f128_ops.rs`). This lets the library round-trip numpy's
//! `float128`/`longdouble` columns without ever losing a bit.

use std::cmp::Ordering;
use std::fmt;

/// Exponent bias of `binary128`.
pub(crate) const BIAS: i32 = 16383;
/// Number of explicit mantissa bits.
pub(crate) const MANT: u32 = 112;
/// Maximum biased exponent field (`Inf`/`NaN`).
pub(crate) const EMAX: u32 = 0x7FFF;

/// Quad-precision float held as raw IEEE-754 `binary128` bits.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct F128(pub u128);

/// Sign / integer-significand / exponent decomposition (`value = sig * 2^q`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Unpacked {
    pub sign: bool,
    pub sig: u128,
    pub q: i32,
    pub class: Class,
}

/// IEEE number class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Zero,
    Finite,
    Inf,
    Nan,
}

impl F128 {
    /// `ZERO`.
    pub const ZERO: F128 = F128(0);
    /// `ONE`.
    pub const ONE: F128 = F128(0x3FFF_0000_0000_0000_0000_0000_0000_0000);
    /// `NAN`.
    pub const NAN: F128 = F128(0x7FFF_8000_0000_0000_0000_0000_0000_0000);
    /// `INFINITY`.
    pub const INFINITY: F128 = F128(0x7FFF_0000_0000_0000_0000_0000_0000_0000);
    /// `NEG_INFINITY`.
    pub const NEG_INFINITY: F128 = F128(0xFFFF_0000_0000_0000_0000_0000_0000_0000);

    /// Build one from bits.
    #[inline]
    pub fn from_bits(b: u128) -> Self {
        F128(b)
    }
    /// Convert to bits.
    #[inline]
    pub fn to_bits(self) -> u128 {
        self.0
    }
    /// `true` when this is nan.
    #[inline]
    pub fn is_nan(self) -> bool {
        self.unpack().class == Class::Nan
    }
    /// `true` when this is infinite.
    #[inline]
    pub fn is_infinite(self) -> bool {
        self.unpack().class == Class::Inf
    }
    /// `true` when this is sign negative.
    #[inline]
    pub fn is_sign_negative(self) -> bool {
        self.0 >> 127 == 1
    }
    /// Abs.
    #[inline]
    pub fn abs(self) -> Self {
        F128(self.0 & !(1u128 << 127))
    }

    pub(crate) fn unpack(self) -> Unpacked {
        let sign = self.0 >> 127 == 1;
        let e = ((self.0 >> MANT) & EMAX as u128) as u32;
        let m = self.0 & ((1u128 << MANT) - 1);
        if e == EMAX {
            let class = if m == 0 { Class::Inf } else { Class::Nan };
            return Unpacked { sign, sig: m, q: 0, class };
        }
        if e == 0 {
            let class = if m == 0 { Class::Zero } else { Class::Finite };
            return Unpacked { sign, sig: m, q: -BIAS + 1 - MANT as i32, class };
        }
        Unpacked {
            sign,
            sig: m | (1u128 << MANT),
            q: e as i32 - BIAS - MANT as i32,
            class: Class::Finite,
        }
    }

    /// Exact widening from `f64` — never loses information.
    pub fn from_f64(v: f64) -> Self {
        let b = v.to_bits();
        let sign = (b >> 63) as u128;
        let e = ((b >> 52) & 0x7FF) as i32;
        let m = (b & ((1u64 << 52) - 1)) as u128;
        let out = if e == 0x7FF {
            (EMAX as u128) << MANT | (m << 60)
        } else if e == 0 && m == 0 {
            0
        } else if e == 0 {
            // f64 sub-normal is a normal binary128 number
            let lz = m.leading_zeros() - (127 - 51);
            let sig = (m << (lz + 1)) & ((1u128 << 52) - 1);
            let ne = (-1022 - 1 - lz as i32 + BIAS) as u128;
            (ne << MANT) | (sig << 60)
        } else {
            (((e - 1023 + BIAS) as u128) << MANT) | (m << 60)
        };
        F128((sign << 127) | out)
    }

    /// Narrowing to `f64` with round-to-nearest-even (may overflow to `inf`).
    pub fn to_f64(self) -> f64 {
        let u = self.unpack();
        let s = if u.sign { -1.0f64 } else { 1.0f64 };
        match u.class {
            Class::Zero => 0.0 * s,
            Class::Inf => f64::INFINITY * s,
            Class::Nan => f64::NAN,
            Class::Finite => {
                // Convert sig * 2^q via repeated scaling to stay exact.
                let mut sig = u.sig;
                let mut q = u.q;
                // Drop bits below f64 precision with round-to-nearest-even.
                let bits = 128 - sig.leading_zeros();
                if bits > 53 {
                    let drop = bits - 53;
                    let rem = sig & ((1u128 << drop) - 1);
                    let half = 1u128 << (drop - 1);
                    sig >>= drop;
                    if rem > half || (rem == half && sig & 1 == 1) {
                        sig += 1;
                    }
                    q += drop as i32;
                }
                s * ldexp(sig as f64, q)
            }
        }
    }

    /// Build one from f32.
    #[inline]
    pub fn from_f32(v: f32) -> Self {
        F128::from_f64(v as f64)
    }
    /// Convert to f32.
    #[inline]
    pub fn to_f32(self) -> f32 {
        self.to_f64() as f32
    }
}

/// `x * 2^e` without intermediate overflow/underflow (std has no `ldexp`).
fn ldexp(x: f64, mut e: i32) -> f64 {
    let mut r = x;
    while e > 1000 {
        r *= (2.0f64).powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        r *= (2.0f64).powi(-1000);
        e += 1000;
    }
    r * (2.0f64).powi(e)
}

impl fmt::Debug for F128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f64())
    }
}
impl fmt::Display for F128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f64())
    }
}
impl PartialOrd for F128 {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        let (a, b) = (self.unpack(), o.unpack());
        if a.class == Class::Nan || b.class == Class::Nan {
            return None;
        }
        if a.class == Class::Zero && b.class == Class::Zero {
            return Some(Ordering::Equal);
        }
        if a.sign != b.sign {
            return Some(if a.sign { Ordering::Less } else { Ordering::Greater });
        }
        let key = |x: u128| x & !(1u128 << 127);
        let ord = key(self.0).cmp(&key(o.0));
        Some(if a.sign { ord.reverse() } else { ord })
    }
}
