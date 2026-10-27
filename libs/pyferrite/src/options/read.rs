use crate::dtype::{FloatKind, IntKind};
use crate::formats::Format;
use crate::options::CastPolicy;

/// Everything that influences how a file is decoded.
///
/// Construct with [`ReadOptions::new`] and chain the builder methods; every
/// field is public so struct-update syntax works too.
#[derive(Clone, Debug)]
pub struct ReadOptions {
    /// Decode arrays chunk by chunk instead of materialising them whole.
    ///
    /// When `true`, [`crate::read_lazy`] hands back an iterator of
    /// [`crate::lazy::Chunk`]s and peak memory stays around
    /// `chunk_elements * element_size`.
    pub lazy: bool,
    /// Number of *elements* per chunk in lazy mode. Default: 1 Mi elements.
    pub chunk_elements: usize,
    /// Force every integer array/scalar to this width.
    pub int_as: Option<IntKind>,
    /// Force every real floating point array/scalar to this width.
    pub float_as: Option<FloatKind>,
    /// Behaviour when a forced cast would lose information.
    pub cast_policy: CastPolicy,
    /// Override format auto-detection.
    pub format: Option<Format>,
    /// Also capture callables on the code-execution blocklist (`os`, `sys`,
    /// `subprocess`, `eval`, ...) as inert [`crate::value::PyObject`] values.
    ///
    /// `false` by default: such a reference is refused with an error. Other
    /// classes outside the numpy/torch allow-list are always captured inertly.
    /// Nothing is executed in either mode.
    pub allow_unknown_globals: bool,
    /// Hard ceiling on a single allocation, in bytes. Guards against corrupt
    /// or hostile headers claiming petabyte-sized arrays. Default: 8 GiB.
    pub max_alloc: usize,
    /// Keep the raw structured-dtype layout instead of turning record arrays
    /// into a [`crate::value::Frame`].
    pub keep_record_arrays: bool,
}

impl Default for ReadOptions {
    fn default() -> Self {
        ReadOptions {
            lazy: false,
            chunk_elements: 1 << 20,
            int_as: None,
            float_as: None,
            cast_policy: CastPolicy::Strict,
            format: None,
            allow_unknown_globals: false,
            max_alloc: 8 << 30,
            keep_record_arrays: false,
        }
    }
}

impl ReadOptions {
    /// Default options: eager, no casting, strict.
    pub fn new() -> Self {
        Self::default()
    }
    /// Enable chunked decoding.
    pub fn lazy(mut self, yes: bool) -> Self {
        self.lazy = yes;
        self
    }
    /// Elements per chunk in lazy mode (must be non-zero).
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
    /// Choose the overflow behaviour of forced casts.
    pub fn cast_policy(mut self, p: CastPolicy) -> Self {
        self.cast_policy = p;
        self
    }
    /// Skip sniffing and assume `fmt`.
    pub fn format(mut self, fmt: Format) -> Self {
        self.format = Some(fmt);
        self
    }
    /// Raise or lower the single-allocation guard.
    pub fn max_alloc(mut self, bytes: usize) -> Self {
        self.max_alloc = bytes;
        self
    }
}

impl ReadOptions {
    /// Capture blocklisted callables (`os.system`, `eval`, ...) as inert
    /// [`PyObject`](crate::value::PyObject) values instead of refusing the file,
    /// for inspecting a suspicious pickle.
    ///
    /// Nothing is ever executed either way. Classes that are merely unknown,
    /// such as a fitted scikit-learn estimator, are captured inertly by default.
    pub fn allow_unknown_globals(mut self, yes: bool) -> Self {
        self.allow_unknown_globals = yes;
        self
    }
}
