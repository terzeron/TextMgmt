//! `.joblib` — `joblib.dump` files (pickle plus inline raw array blocks).

mod gzip;
mod read;
mod write;

pub use read::{read_bytes, read_path};
pub use write::{write_bytes, write_path};
