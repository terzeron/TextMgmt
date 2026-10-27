//! Bridges to the Rust and Python dataframe ecosystems.
//!
//! [`Frame`](crate::value::Frame) is the neutral table type in the middle:
//! pandas and numpy record arrays arrive as one, polars converts to and from
//! one, and every reader and writer speaks only to it.

pub mod pandas;

#[cfg(feature = "polars-interop")]
pub mod polars;

#[cfg(feature = "polars-interop")]
mod polars_from;

#[cfg(feature = "polars-interop")]
pub use polars_from::from_polars;
