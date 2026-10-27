//! Read/write knobs: laziness, numeric down-casting, safety limits.

mod read;
mod write;

pub use read::ReadOptions;
pub use write::WriteOptions;

/// What to do when a value does not fit into the requested target type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CastPolicy {
    /// Refuse the conversion with [`crate::Error::Cast`] (default — safest).
    #[default]
    Strict,
    /// Clamp to the target's representable range; floats round to nearest even.
    Saturate,
    /// Two's-complement wrap-around for integers, IEEE overflow for floats.
    Wrap,
}

/// Compression used inside container formats (`npz`, `joblib`, HDF5 chunks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Compression {
    /// Store bytes verbatim.
    #[default]
    None,
    /// RFC 1951 raw deflate (what `numpy.savez_compressed` writes).
    Deflate(u8),
    /// RFC 1950 zlib wrapper (what `joblib` and HDF5 gzip use).
    Zlib(u8),
}

impl Compression {
    /// Deflate at the usual default level 6.
    pub fn deflate() -> Self {
        Compression::Deflate(6)
    }
    /// Zlib at the usual default level 6.
    pub fn zlib() -> Self {
        Compression::Zlib(6)
    }
}
