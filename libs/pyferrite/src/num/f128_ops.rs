//! Software arithmetic for [`F128`]: add, sub, mul, div with round-to-nearest-even.

use super::f128::{Unpacked, EMAX, F128, MANT};

/// Assemble `sign * sig * 2^q` back into IEEE-754 `binary128`.
///
/// `sticky` carries the OR of every bit that has already been discarded.
pub(crate) fn round_pack(sign: bool, mut sig: u128, mut q: i32, mut sticky: bool) -> F128 {
    let sgn = (sign as u128) << 127;
    if sig == 0 {
        return F128(sgn);
    }
    // Place the MSB of `sig` at bit 115 (3 guard bits below the 112-bit field).
    let msb = 127 - sig.leading_zeros() as i32;
    let shift = 115 - msb;
    if shift > 0 {
        sig <<= shift as u32;
        q -= shift;
    } else if shift < 0 {
        let n = (-shift) as u32;
        sticky |= (sig & ((1u128 << n) - 1)) != 0;
        sig >>= n;
        q += -shift;
    }
    let mut e = q + 16498; // q + 3 + MANT + BIAS
    if e <= 0 {
        let extra = 1 - e;
        if extra >= 128 {
            return F128(sgn);
        }
        sticky |= (sig & ((1u128 << extra as u32) - 1)) != 0;
        sig >>= extra as u32;
        e = 1;
    }
    let lsb = (sig >> 3) & 1;
    let guard = (sig >> 2) & 1;
    let rest = (sig & 3) != 0 || sticky;
    sig >>= 3;
    if guard == 1 && (rest || lsb == 1) {
        sig += 1;
    }
    if sig >= 1u128 << (MANT + 1) {
        sig >>= 1;
        e += 1;
    }
    if sig < 1u128 << MANT {
        e = 0; // sub-normal
    }
    if e >= EMAX as i32 {
        return F128(sgn | ((EMAX as u128) << MANT));
    }
    F128(sgn | ((e as u128) << MANT) | (sig & ((1u128 << MANT) - 1)))
}

pub(super) fn nan() -> F128 {
    F128::NAN
}
pub(super) fn inf(sign: bool) -> F128 {
    F128(((sign as u128) << 127) | ((EMAX as u128) << MANT))
}

/// 256-bit unsigned helper used by multiplication and division.
#[derive(Clone, Copy)]
pub(crate) struct U256 {
    pub hi: u128,
    pub lo: u128,
}

pub(crate) fn mul_wide(a: u128, b: u128) -> U256 {
    let (a0, a1) = (a as u64 as u128, a >> 64);
    let (b0, b1) = (b as u64 as u128, b >> 64);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & u64::MAX as u128) + (p10 & u64::MAX as u128);
    let lo = (p00 & u64::MAX as u128) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    U256 { hi, lo }
}

impl U256 {
    pub(super) fn bits(&self) -> u32 {
        if self.hi != 0 {
            256 - self.hi.leading_zeros()
        } else {
            128 - self.lo.leading_zeros()
        }
    }
    /// Shift right by `n`, returning the truncated value and a sticky flag.
    pub(super) fn shr_sticky(&self, n: u32) -> (u128, bool) {
        if n == 0 {
            return (self.lo, self.hi != 0);
        }
        if n >= 256 {
            return (0, self.hi != 0 || self.lo != 0);
        }
        let (v, sticky);
        if n < 128 {
            v = (self.lo >> n) | (self.hi.checked_shl(128 - n).unwrap_or(0));
            sticky = (self.lo & ((1u128 << n) - 1)) != 0;
        } else {
            let k = n - 128;
            v = self.hi >> k;
            sticky = self.lo != 0 || (k > 0 && (self.hi & ((1u128 << k) - 1)) != 0);
        }
        (v, sticky)
    }
}

/// Right-shift with "jamming": the OR of all discarded bits lands in bit 0,
/// which keeps the sticky information inside the value itself.
#[inline]
pub(super) fn shr_jam(x: u128, n: u32) -> u128 {
    if n == 0 {
        x
    } else if n >= 128 {
        (x != 0) as u128
    } else {
        (x >> n) | (((x & ((1u128 << n) - 1)) != 0) as u128)
    }
}

pub(super) fn add_mag(a: Unpacked, b: Unpacked, sub: bool) -> F128 {
    let (mut sa, qa) = (a.sig << 3, a.q - 3);
    let (mut sb, qb) = (b.sig << 3, b.q - 3);
    // Align on the coarser (larger) exponent, jamming the finer operand.
    let q = qa.max(qb);
    if qa < q {
        sa = shr_jam(sa, (q - qa) as u32);
    }
    if qb < q {
        sb = shr_jam(sb, (q - qb) as u32);
    }
    if !sub {
        return round_pack(a.sign, sa + sb, q, false);
    }
    if sa >= sb {
        round_pack(a.sign, sa - sb, q, false)
    } else {
        round_pack(!a.sign, sb - sa, q, false)
    }
}
