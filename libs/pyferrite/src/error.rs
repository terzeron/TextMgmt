//! Error and result types used across the whole crate.
//!
//! The crate deliberately avoids proc-macro helper crates (`thiserror`, ...)
//! so that the dependency graph stays tiny and 100% pure Rust.

use std::fmt;

/// Convenient alias used by every fallible function of the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure mode the library can produce.
#[derive(Debug)]
pub enum Error {
    /// Underlying I/O failure (file missing, permissions, truncated read...).
    Io(std::io::Error),
    /// The container/file could not be recognised or its magic is wrong.
    Format(String),
    /// The file is well formed but uses a feature this crate does not support.
    Unsupported(String),
    /// A numeric or structural conversion is impossible / lossy under the
    /// currently active [`crate::options::CastPolicy`].
    Cast(String),
    /// Caller supplied contradictory or invalid options / arguments.
    InvalidArgument(String),
    /// Shape mismatch between requested and actual element counts.
    Shape {
        /// The shape that was required.
        expected: Vec<usize>,
        /// The element count actually supplied.
        got: usize,
    },
    /// A pickle stream contained an opcode or global we refuse to execute.
    Pickle(String),
    /// Decompression failure.
    Compression(String),
    /// A lazily-streamed source was used after it had been exhausted.
    Exhausted,
}

impl Error {
    pub(crate) fn format<S: Into<String>>(m: S) -> Self {
        Error::Format(m.into())
    }
    pub(crate) fn unsupported<S: Into<String>>(m: S) -> Self {
        Error::Unsupported(m.into())
    }
    pub(crate) fn cast<S: Into<String>>(m: S) -> Self {
        Error::Cast(m.into())
    }
    pub(crate) fn invalid<S: Into<String>>(m: S) -> Self {
        Error::InvalidArgument(m.into())
    }
    pub(crate) fn pickle<S: Into<String>>(m: S) -> Self {
        Error::Pickle(m.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Format(m) => write!(f, "malformed file: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported feature: {m}"),
            Error::Cast(m) => write!(f, "conversion error: {m}"),
            Error::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
            Error::Shape { expected, got } => {
                write!(f, "shape {expected:?} does not match {got} elements")
            }
            Error::Pickle(m) => write!(f, "pickle error: {m}"),
            Error::Compression(m) => write!(f, "compression error: {m}"),
            Error::Exhausted => write!(f, "stream already exhausted"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(e: std::string::FromUtf8Error) -> Self {
        Error::Format(format!("invalid utf-8: {e}"))
    }
}

#[cfg(test)]
#[path = "../../../tests/rust/pyferrite/error_unit.rs"]
mod tests;
