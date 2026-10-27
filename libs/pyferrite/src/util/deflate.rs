//! Thin, feature-gated wrapper around `miniz_oxide` (pure Rust deflate).
//!
//! With `--no-default-features` the crate still builds; only stored (level 0)
//! members remain readable, which is exactly what `numpy.savez` produces.

use crate::error::{Error, Result};

/// Inflate a raw RFC 1951 stream.
#[cfg(feature = "deflate")]
pub fn decompress_raw(data: &[u8], hint: usize) -> Result<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_with_limit(data, hint.max(1 << 16).max(data.len() * 8))
        .map_err(|e| Error::Compression(format!("inflate failed: {e:?}")))
}

/// Inflate an RFC 1950 (zlib) stream.
#[cfg(feature = "deflate")]
pub fn decompress_zlib(data: &[u8], hint: usize) -> Result<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
        data,
        hint.max(1 << 16).max(data.len() * 8),
    )
    .map_err(|e| Error::Compression(format!("zlib inflate failed: {e:?}")))
}

/// Deflate to a raw RFC 1951 stream.
#[cfg(feature = "deflate")]
pub fn compress_raw(data: &[u8], level: u8) -> Result<Vec<u8>> {
    Ok(miniz_oxide::deflate::compress_to_vec(data, level.min(10)))
}

/// Deflate to an RFC 1950 (zlib) stream.
#[cfg(feature = "deflate")]
pub fn compress_zlib(data: &[u8], level: u8) -> Result<Vec<u8>> {
    Ok(miniz_oxide::deflate::compress_to_vec_zlib(data, level.min(10)))
}

/// Inflate a raw RFC 1951 stream. Always fails without the `deflate` feature.
#[cfg(not(feature = "deflate"))]
pub fn decompress_raw(_d: &[u8], _h: usize) -> Result<Vec<u8>> {
    Err(Error::unsupported("deflate feature is disabled"))
}
/// Inflate an RFC 1950 (zlib) stream. Always fails without the `deflate` feature.
#[cfg(not(feature = "deflate"))]
pub fn decompress_zlib(_d: &[u8], _h: usize) -> Result<Vec<u8>> {
    Err(Error::unsupported("deflate feature is disabled"))
}
/// Deflate to a raw RFC 1951 stream. Always fails without the `deflate` feature.
#[cfg(not(feature = "deflate"))]
pub fn compress_raw(_d: &[u8], _l: u8) -> Result<Vec<u8>> {
    Err(Error::unsupported("deflate feature is disabled"))
}
/// Deflate to an RFC 1950 (zlib) stream. Always fails without the `deflate` feature.
#[cfg(not(feature = "deflate"))]
pub fn compress_zlib(_d: &[u8], _l: u8) -> Result<Vec<u8>> {
    Err(Error::unsupported("deflate feature is disabled"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "deflate")]
    #[test]
    fn raw_and_zlib_streams_round_trip_and_reject_invalid_data() {
        let input = b"repeated payload repeated payload repeated payload";
        let raw = compress_raw(input, 255).unwrap();
        assert_eq!(decompress_raw(&raw, input.len()).unwrap(), input);
        let zlib = compress_zlib(input, 255).unwrap();
        assert_eq!(decompress_zlib(&zlib, input.len()).unwrap(), input);
        assert!(decompress_raw(b"invalid", 0).is_err());
        assert!(decompress_zlib(b"invalid", 0).is_err());
    }

    #[cfg(not(feature = "deflate"))]
    #[test]
    fn all_operations_report_disabled_feature() {
        assert!(decompress_raw(b"", 0).is_err());
        assert!(decompress_zlib(b"", 0).is_err());
        assert!(compress_raw(b"", 0).is_err());
        assert!(compress_zlib(b"", 0).is_err());
    }
}
