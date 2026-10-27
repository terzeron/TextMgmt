use crate::dtype::{DType, FloatKind, IntKind};
use crate::formats::Format;
use crate::options::{CastPolicy, Compression};

/// Everything that influences how a value is encoded.
#[derive(Clone, Debug)]
pub struct WriteOptions {
    /// Stream the payload in chunks instead of buffering it whole.
    pub lazy: bool,
    /// Number of *elements* per chunk in lazy mode. Default: 1 Mi elements.
    pub chunk_elements: usize,
    /// Force every integer to this width on the way out.
    pub int_as: Option<IntKind>,
    /// Force every real float to this width on the way out.
    pub float_as: Option<FloatKind>,
    /// Explicit target element type; wins over `int_as` / `float_as`.
    pub dtype: Option<DType>,
    /// Explicit target shape. `numpy` distinguishes `shape = (2,)` from
    /// `shape = (1, 2)`, so this is how you pin the exact one you need.
    pub shape: Option<Vec<usize>>,
    /// Behaviour when a forced cast would lose information.
    pub cast_policy: CastPolicy,
    /// Override format selection (otherwise taken from the file extension).
    pub format: Option<Format>,
    /// Compression for container formats.
    pub compression: Compression,
    /// Pickle protocol to emit (2..=5). Default 4 — readable by Python 3.4+.
    pub pickle_protocol: u8,
    /// Write arrays in Fortran (column-major) order.
    pub fortran_order: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions {
            lazy: false,
            chunk_elements: 1 << 20,
            int_as: None,
            float_as: None,
            dtype: None,
            shape: None,
            cast_policy: CastPolicy::Strict,
            format: None,
            compression: Compression::None,
            pickle_protocol: 4,
            fortran_order: false,
        }
    }
}

impl WriteOptions {
    /// Create a new value.
    pub fn new() -> Self {
        Self::default()
    }
    /// Enable chunked writing.
    pub fn lazy(mut self, yes: bool) -> Self {
        self.lazy = yes;
        self
    }
    /// Elements per chunk in lazy mode.
    pub fn chunk_elements(mut self, n: usize) -> Self {
        self.chunk_elements = n.max(1);
        self
    }
    /// Force integers to `kind`.
    pub fn int_as(mut self, kind: IntKind) -> Self {
        self.int_as = Some(kind);
        self
    }
    /// Force floats to `kind`.
    pub fn float_as(mut self, kind: FloatKind) -> Self {
        self.float_as = Some(kind);
        self
    }
    /// Pin the exact element type.
    pub fn dtype(mut self, dt: DType) -> Self {
        self.dtype = Some(dt);
        self
    }
    /// Pin the exact shape — `(2,)` and `(1, 2)` are different files.
    pub fn shape(mut self, shape: impl Into<Vec<usize>>) -> Self {
        self.shape = Some(shape.into());
        self
    }
    /// Choose the overflow behaviour of forced casts.
    pub fn cast_policy(mut self, p: CastPolicy) -> Self {
        self.cast_policy = p;
        self
    }
    /// Skip extension sniffing and use `fmt`.
    pub fn format(mut self, fmt: Format) -> Self {
        self.format = Some(fmt);
        self
    }
    /// Set container compression.
    pub fn compression(mut self, c: Compression) -> Self {
        self.compression = c;
        self
    }
    /// Choose the pickle protocol (clamped to 2..=5).
    pub fn pickle_protocol(mut self, p: u8) -> Self {
        self.pickle_protocol = p.clamp(2, 5);
        self
    }
}

impl WriteOptions {
    /// Write `.npy` payloads in column-major (Fortran) order.
    ///
    /// numpy reads either order transparently; this only changes how the bytes
    /// are laid out on disk, and the header records which was used.
    pub fn fortran_order(mut self, yes: bool) -> Self {
        self.fortran_order = yes;
        self
    }
}
