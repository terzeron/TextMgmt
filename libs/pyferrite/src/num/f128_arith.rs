//! The four arithmetic operations on [`F128`], and the operator impls.
//!
//! Each handles the IEEE special cases first, then delegates the significand
//! work to the helpers in [`super::f128_ops`].

use super::f128::{Class, F128};
use super::f128_ops::{add_mag, inf, mul_wide, nan, round_pack};
use core::ops::{Add, Div, Mul, Neg, Sub};

/// Core addition honouring IEEE special values.
pub(crate) fn add(x: F128, y: F128) -> F128 {
    let (a, b) = (x.unpack(), y.unpack());
    match (a.class, b.class) {
        (Class::Nan, _) | (_, Class::Nan) => nan(),
        (Class::Inf, Class::Inf) => {
            if a.sign == b.sign {
                inf(a.sign)
            } else {
                nan()
            }
        }
        (Class::Inf, _) => inf(a.sign),
        (_, Class::Inf) => inf(b.sign),
        (Class::Zero, Class::Zero) => F128(((a.sign && b.sign) as u128) << 127),
        (Class::Zero, _) => y,
        (_, Class::Zero) => x,
        _ => add_mag(a, b, a.sign != b.sign),
    }
}

/// Core multiplication honouring IEEE special values.
pub(crate) fn mul(x: F128, y: F128) -> F128 {
    let (a, b) = (x.unpack(), y.unpack());
    let sign = a.sign ^ b.sign;
    match (a.class, b.class) {
        (Class::Nan, _) | (_, Class::Nan) => nan(),
        (Class::Inf, Class::Zero) | (Class::Zero, Class::Inf) => nan(),
        (Class::Inf, _) | (_, Class::Inf) => inf(sign),
        (Class::Zero, _) | (_, Class::Zero) => F128((sign as u128) << 127),
        _ => {
            let p = mul_wide(a.sig, b.sig);
            let bits = p.bits();
            let shift = bits.saturating_sub(120);
            let (sig, sticky) = p.shr_sticky(shift);
            round_pack(sign, sig, a.q + b.q + shift as i32, sticky)
        }
    }
}

/// Core division honouring IEEE special values.
pub(crate) fn div(x: F128, y: F128) -> F128 {
    let (a, b) = (x.unpack(), y.unpack());
    let sign = a.sign ^ b.sign;
    match (a.class, b.class) {
        (Class::Nan, _) | (_, Class::Nan) => nan(),
        (Class::Inf, Class::Inf) | (Class::Zero, Class::Zero) => nan(),
        (Class::Inf, _) | (_, Class::Zero) => inf(sign),
        (Class::Zero, _) | (_, Class::Inf) => F128((sign as u128) << 127),
        _ => {
            // Normalise both significands so the quotient lands in [1, 2).
            let (mut sa, mut qa) = (a.sig, a.q);
            let (mut sb, mut qb) = (b.sig, b.q);
            let na = 112 - (127 - sa.leading_zeros() as i32);
            if na > 0 {
                sa <<= na as u32;
                qa -= na;
            }
            let nb = 112 - (127 - sb.leading_zeros() as i32);
            if nb > 0 {
                sb <<= nb as u32;
                qb -= nb;
            }
            // 120 bits of quotient via restoring division.
            const N: u32 = 120;
            let mut rem = sa;
            let mut quo: u128 = 0;
            for _ in 0..N {
                rem <<= 1;
                quo <<= 1;
                if rem >= sb {
                    rem -= sb;
                    quo |= 1;
                }
            }
            round_pack(sign, quo, qa - qb - N as i32, rem != 0)
        }
    }
}

impl Add for F128 {
    type Output = F128;
    fn add(self, o: F128) -> F128 {
        add(self, o)
    }
}
impl Sub for F128 {
    type Output = F128;
    fn sub(self, o: F128) -> F128 {
        add(self, o.neg())
    }
}
impl Mul for F128 {
    type Output = F128;
    fn mul(self, o: F128) -> F128 {
        mul(self, o)
    }
}
impl Div for F128 {
    type Output = F128;
    fn div(self, o: F128) -> F128 {
        div(self, o)
    }
}
impl Neg for F128 {
    type Output = F128;
    fn neg(self) -> F128 {
        F128(self.0 ^ (1u128 << 127))
    }
}
