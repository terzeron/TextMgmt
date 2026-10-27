//! Numeric primitives that Rust's standard library does not provide but that
//! Python / ML ecosystems use every day: 8-bit and 16-bit floats, quad
//! precision, x87 extended precision, and complex numbers.
//!
//! Everything here is written in safe, dependency-free Rust so the crate can be
//! used in bare-metal or `no-cc` environments.

mod bf16;
mod f128;
mod f128_arith;
mod f128_ops;
mod f16;
mod f8;
mod x87;

pub use bf16::BF16;
pub use f128::F128;
pub use f16::F16;
pub use f8::{F8E4M3, F8E5M2};
pub use x87::{decode_x87, encode_x87};

pub(crate) use f128::Class;

/// Complex number type re-exported from `num-complex` (pure Rust, no C code).
pub use num_complex::Complex;

/// Single precision complex value (numpy `complex64`, torch `cfloat`).
pub type Complex32 = Complex<f32>;
/// Double precision complex value (numpy `complex128`, torch `cdouble`).
pub type Complex64 = Complex<f64>;
/// Quad precision complex value (numpy `complex256` on platforms that have it).
pub type Complex128 = Complex<F128>;

impl F128 {
    /// Internal helper used by the x87 codec: `(sign, sig, q, is_special, is_inf)`.
    pub(crate) fn unpack_pub(self) -> (bool, u128, i32, bool, bool) {
        let u = self.unpack();
        match u.class {
            Class::Nan => (u.sign, 0, 0, true, false),
            Class::Inf => (u.sign, 0, 0, true, true),
            Class::Zero => (u.sign, 0, 0, false, false),
            Class::Finite => (u.sign, u.sig, u.q, false, false),
        }
    }
}

/// Types that can be losslessly described by a [`crate::dtype::DType`].
pub trait Element: Copy + Default + Send + Sync + 'static {}
impl<T: Copy + Default + Send + Sync + 'static> Element for T {}
