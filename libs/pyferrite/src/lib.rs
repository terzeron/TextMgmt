#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod api;
pub mod dtype;
pub mod error;
pub mod formats;
pub mod lazy;
pub mod num;
pub mod options;
pub mod prelude;
pub mod util;
pub mod value;
pub mod zip;

pub mod interop;

pub use api::{convert, flatten_arrays, read, read_lazy, read_with, write, write_lazy, write_with};
pub use error::{Error, Result};
pub use options::{ReadOptions, WriteOptions};
pub use value::{Array, Dict, Frame, PyObject, Series, Value};
