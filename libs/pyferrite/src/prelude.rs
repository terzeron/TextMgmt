//! `use pyferrite::prelude::*;` pulls in everything needed for day-to-day work.

pub use crate::api::{
    convert, flatten_arrays, read, read_lazy, read_with, write, write_lazy, write_with,
};
pub use crate::dtype::{DType, Endian, FloatKind, IntKind};
pub use crate::error::{Error, Result};
pub use crate::formats::Format;
pub use crate::lazy::{Chunk, LazyReader, LazyWriter};
pub use crate::num::{Complex32, Complex64, BF16, F128, F16, F8E4M3, F8E5M2};
pub use crate::options::{CastPolicy, Compression, ReadOptions, WriteOptions};
pub use crate::value::{Array, Dict, Frame, PyObject, Series, Value};
