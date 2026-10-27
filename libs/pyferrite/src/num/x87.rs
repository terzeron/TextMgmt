//! x87 80-bit extended precision ("long double" on x86 Linux/macOS).
//!
//! numpy exposes this as `numpy.longdouble` / descr `'<f16'` with an item size
//! of 12 or 16 bytes (only the first 10 bytes are meaningful, the rest is
//! padding). This module converts it to and from the crate's canonical
//! [`F128`] so that the rest of the code never has to care.

use super::f128::F128;
use super::f128_ops::round_pack;

/// Decode 10 little-endian bytes of x87 extended precision into [`F128`].
pub fn decode_x87(b: &[u8]) -> F128 {
    debug_assert!(b.len() >= 10);
    let mut m = [0u8; 8];
    m.copy_from_slice(&b[0..8]);
    let sig = u64::from_le_bytes(m) as u128;
    let se = u16::from_le_bytes([b[8], b[9]]);
    let sign = se >> 15 == 1;
    let e = (se & 0x7FFF) as i32;
    if e == 0x7FFF {
        // Inf when only the explicit integer bit is set, NaN otherwise.
        return if sig == 1u128 << 63 {
            if sign {
                F128::NEG_INFINITY
            } else {
                F128::INFINITY
            }
        } else {
            F128::NAN
        };
    }
    if sig == 0 {
        return F128((sign as u128) << 127);
    }
    let q = if e == 0 { -16382 - 63 } else { e - 16383 - 63 };
    round_pack(sign, sig, q, false)
}

/// Encode an [`F128`] into 10 little-endian bytes of x87 extended precision.
pub fn encode_x87(v: F128) -> [u8; 10] {
    let u = v.unpack_pub();
    let mut out = [0u8; 10];
    let sign = u.0;
    if u.3 {
        // NaN / Inf
        let se = 0x7FFFu16 | ((sign as u16) << 15);
        let sig: u64 = if u.4 { 0x8000_0000_0000_0000 } else { 0xC000_0000_0000_0000 };
        out[0..8].copy_from_slice(&sig.to_le_bytes());
        out[8..10].copy_from_slice(&se.to_le_bytes());
        return out;
    }
    let (mut sig, mut q) = (u.1, u.2);
    if sig == 0 {
        let se = (sign as u16) << 15;
        out[8..10].copy_from_slice(&se.to_le_bytes());
        return out;
    }
    // Normalise the significand to exactly 64 bits with round-to-nearest-even.
    let bits = 128 - sig.leading_zeros() as i32;
    let shift = 64 - bits;
    if shift > 0 {
        sig <<= shift as u32;
        q -= shift;
    } else if shift < 0 {
        let n = (-shift) as u32;
        let rem = sig & ((1u128 << n) - 1);
        let half = 1u128 << (n - 1);
        sig >>= n;
        if rem > half || (rem == half && sig & 1 == 1) {
            sig += 1;
            if sig >= 1u128 << 64 {
                sig >>= 1;
                q += 1;
            }
        }
        q += -shift;
    }
    let e = q + 16383 + 63;
    let e = e.clamp(0, 0x7FFE) as u16;
    let se = e | ((sign as u16) << 15);
    out[0..8].copy_from_slice(&(sig as u64).to_le_bytes());
    out[8..10].copy_from_slice(&se.to_le_bytes());
    out
}
