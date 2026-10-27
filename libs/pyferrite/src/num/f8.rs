//! Minifloat (8-bit) formats used by modern quantised ML pipelines.
//!
//! Two encodings are provided, matching `ml_dtypes` / OCP naming:
//!
//! * [`F8E4M3`] — 1 sign / 4 exponent / 3 mantissa bits, bias 7, **no infinity**
//!   (`float8_e4m3fn`). Finite range ±448.
//! * [`F8E5M2`] — 1 sign / 5 exponent / 2 mantissa bits, bias 15, IEEE-like with
//!   infinities and NaN (`float8_e5m2`). Finite range ±57344.
//!
//! These are exactly what the user-facing `FloatKind::F8*` down-cast targets use.

use std::fmt;

/// Decode an arbitrary minifloat layout into `f32`.
fn decode(bits: u8, ebits: u32, mbits: u32, bias: i32, has_inf: bool) -> f32 {
    let sign = if bits >> 7 == 1 { -1.0f32 } else { 1.0f32 };
    let emask = (1u8 << ebits) - 1;
    let e = ((bits >> mbits) & emask) as i32;
    let m = (bits & ((1u8 << mbits) - 1)) as u32;
    let emax = emask as i32;
    if e == emax {
        if has_inf {
            return if m == 0 { sign * f32::INFINITY } else { f32::NAN };
        }
        if m == (1u32 << mbits) - 1 {
            return f32::NAN;
        }
    }
    if e == 0 {
        // sub-normal
        let scale = (2.0f32).powi(1 - bias - mbits as i32);
        sign * (m as f32) * scale
    } else {
        let scale = (2.0f32).powi(e - bias - mbits as i32);
        sign * ((1u32 << mbits) + m) as f32 * scale
    }
}

/// Encode `f32` into an arbitrary minifloat layout using round-to-nearest-even.
fn encode(v: f32, ebits: u32, mbits: u32, bias: i32, has_inf: bool) -> u8 {
    let emask = (1u8 << ebits) - 1;
    let sign: u8 = if v.is_sign_negative() { 0x80 } else { 0 };
    if v.is_nan() {
        return sign | (emask << mbits) | ((1u8 << mbits) - 1);
    }
    let a = v.abs();
    let max_e = if has_inf { emask as i32 - 1 } else { emask as i32 };
    let max_m = if has_inf { (1u32 << mbits) - 1 } else { (1u32 << mbits) - 2 };
    let maxval = ((1u32 << mbits) + max_m) as f32 * (2.0f32).powi(max_e - bias - mbits as i32);
    if a.is_infinite() || a > maxval {
        return if has_inf {
            sign | (emask << mbits)
        } else {
            sign | ((max_e as u8) << mbits) | max_m as u8
        };
    }
    if a == 0.0 {
        return sign;
    }
    let mut e = a.log2().floor() as i32 + bias;
    if e < 1 {
        e = 0;
    }
    let scale = (2.0f32).powi(e.max(1) - bias - mbits as i32);
    let mut q = (a / scale).round();
    // round-half-to-even fix-up
    let exact = a / scale;
    if (exact - exact.floor() - 0.5).abs() < f32::EPSILON && (q as u32) % 2 == 1 {
        q -= 1.0;
    }
    let implicit = if e == 0 { 0u32 } else { 1u32 << mbits };
    let mut qi = q as u32;
    if qi >= implicit + (1 << mbits) {
        // mantissa overflowed into the next binade
        e += 1;
        qi >>= 1;
    }
    if e > max_e {
        return if has_inf {
            sign | (emask << mbits)
        } else {
            sign | ((max_e as u8) << mbits) | max_m as u8
        };
    }
    let m = qi.saturating_sub(implicit) as u8 & ((1u8 << mbits) - 1);
    sign | ((e as u8) << mbits) | m
}

macro_rules! minifloat {
    ($name:ident, $e:expr, $m:expr, $bias:expr, $inf:expr, $doc:expr) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Default, PartialEq)]
        #[repr(transparent)]
        pub struct $name(pub u8);

        impl $name {
            /// Number of exponent bits.
            pub const EXP_BITS: u32 = $e;
            /// Number of mantissa bits.
            pub const MANT_BITS: u32 = $m;

            /// Build one from bits.
            #[inline]
            pub fn from_bits(b: u8) -> Self {
                $name(b)
            }
            /// Convert to bits.
            #[inline]
            pub fn to_bits(self) -> u8 {
                self.0
            }
            /// Build one from f32.
            #[inline]
            pub fn from_f32(v: f32) -> Self {
                $name(encode(v, $e, $m, $bias, $inf))
            }
            /// Convert to f32.
            #[inline]
            pub fn to_f32(self) -> f32 {
                decode(self.0, $e, $m, $bias, $inf)
            }
            /// Build one from f64.
            #[inline]
            pub fn from_f64(v: f64) -> Self {
                Self::from_f32(v as f32)
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

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.to_f32())
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.to_f32())
            }
        }
        impl PartialOrd for $name {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                self.to_f32().partial_cmp(&o.to_f32())
            }
        }
    };
}

minifloat!(F8E4M3, 4, 3, 7, false, "8-bit float, 4 exponent / 3 mantissa bits (`float8_e4m3fn`).");
minifloat!(F8E5M2, 5, 2, 15, true, "8-bit float, 5 exponent / 2 mantissa bits (`float8_e5m2`).");
