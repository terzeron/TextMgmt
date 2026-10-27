//! Container formats understood by the crate, plus sniffing helpers.

pub mod hdf5;
pub mod joblib;
pub mod npy;
pub mod npz;
pub mod pickle;
pub mod pt;

mod detect;
mod helpers;
pub use detect::{detect_from_bytes, detect_from_path};
pub use helpers::{
    apply_cast, permute, resolve_target_dtype, resolve_target_dtype_read, value_to_array,
};

use std::path::Path;

/// Every supported on-disk container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `.npy` — a single numpy array with a text header.
    Npy,
    /// `.npz` — a zip of `.npy` members (`numpy.savez`).
    Npz,
    /// `.pkl` / `.pickle` — a raw pickle stream.
    Pickle,
    /// `.pt` / `.pth` — `torch.save` (zip container or legacy pickle).
    Pt,
    /// `.joblib` — `joblib.dump` (pickle + out-of-band array blocks).
    Joblib,
    /// `.h5` / `.hdf5` — HDF5, pure-Rust subset.
    Hdf5,
}

impl Format {
    /// Canonical extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Npy => "npy",
            Format::Npz => "npz",
            Format::Pickle => "pkl",
            Format::Pt => "pt",
            Format::Joblib => "joblib",
            Format::Hdf5 => "h5",
        }
    }

    /// Map a file extension onto a format.
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "npy" => Format::Npy,
            "npz" => Format::Npz,
            "pkl" | "pickle" | "p" => Format::Pickle,
            "pt" | "pth" | "bin" | "ckpt" => Format::Pt,
            "joblib" | "jbl" => Format::Joblib,
            "h5" | "hdf5" | "he5" | "hdf" => Format::Hdf5,
            _ => return None,
        })
    }

    /// Map a path onto a format using its extension only.
    pub fn from_path(p: &Path) -> Option<Self> {
        p.extension().and_then(|e| e.to_str()).and_then(Format::from_extension)
    }

    /// `true` when the format can hold more than one named array.
    pub fn is_container(self) -> bool {
        matches!(self, Format::Npz | Format::Pt | Format::Hdf5 | Format::Joblib | Format::Pickle)
    }
}
