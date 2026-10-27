# Changelog

## [0.0.2](https://github.com/89605502155/pyferrite/releases/tag/v0.0.2)

Source: <https://github.com/89605502155/pyferrite/tree/v0.0.2>

Found by an end-to-end validation against files written by current Python
libraries (pandas 3.0, scikit-learn 1.9, PyTorch 2.14, h5py 3.14).

### Fixed

- **pandas 3.0 frames.** Column labels stored as an `ArrowStringArray`
  (pyarrow buffers) were lost and came back as `column_0`, `column_1`, ...
  Pickled Apache Arrow arrays (`pyarrow.lib._restore_array`,
  `chunked_array`) of strings, large strings, int32/int64 and float/double
  are now decoded without pyarrow, so labels and string columns survive.
- **Structured numpy arrays inside pickles** (for example the node table of a
  scikit-learn `KDTree` inside a fitted `KNeighborsClassifier`) no longer fail
  with "structured numpy dtype inside a pickle". Packed record arrays decode to
  a `Frame`, as they do from `.npy`; padded (`align=True`) layouts are refused
  with a clear error. The same applies to arrays joblib stores after the
  pickle stream.
- **h5py booleans.** numpy `bool` datasets, which h5py writes as an HDF5 enum
  `{FALSE = 0, TRUE = 1}`, were read as `uint8`; they now come back as `bool`.

### Documentation

- `ReadOptions::allow_unknown_globals` is described as it behaves: unknown
  classes are always captured as inert `PyObject`s, and the option only lets
  blocklisted callables (`os`, `sys`, `subprocess`, `eval`, ...) be captured
  for inspection instead of refused. Nothing is executed in either mode.

### Tests

- `tests/python_fixtures.rs` reads small files written by pandas 3.0,
  scikit-learn (pickle and joblib) and h5py, and a pickle that would call
  `os.system`, which must be refused.

## [0.0.1](https://github.com/89605502155/pyferrite/releases/tag/v0.0.1) — first public release

Source: <https://github.com/89605502155/pyferrite/tree/v0.0.1>

Reads and writes `.npy`, `.npz`, `.pkl`, `.pt`, `.joblib` and `.h5` from Rust
with no C, no C++, no FFI and no compiled artefacts from any other language.

### Formats

- **`.npy`** — every numpy descriptor with a `DType`, both byte orders, C and
  Fortran order, structured (record) dtypes in both directions, object arrays.
  Numeric arrays are read and written block by block, so the file image is
  never held in memory beside the array; the declared size is checked against
  the file length before anything is allocated.
- **`.npz`** — own ZIP implementation with ZIP64 and CRC-32; stored and
  deflated members; nested dicts flattened with `/` and rebuilt on read.
- **`.pkl`** — protocols 0–5 through a stack machine that never executes
  Python. numpy arrays recognised in all three forms numpy emits
  (`_reconstruct`, `scalar`, `_frombuffer`).
- **`.pt`** — the torch 1.6+ ZIP layout, with persistent-id storage resolution
  and strided-tensor gathering. Legacy checkpoints are refused explicitly.
  Written with pickle protocol 2, as `torch.save` does, so the default
  `torch.load(weights_only=True)` of PyTorch 2.6+ accepts the files.
- **`.joblib`** — inline out-of-band array blocks with alignment padding;
  uncompressed, zlib and gzip containers.
- **`.h5`** — superblocks 0–3, object headers v1 and v2 with continuations,
  symbol-table and link-message groups, contiguous, compact and chunked
  layouts, deflate, shuffle and fletcher32 filters. Writes a conformant
  version-0 subset that h5py and the reference C library read.

### Numerics

`f16`, `bf16`, `f8` (e4m3 and e5m2), `f128` with software arithmetic, x87
80-bit, and complex64/128/256 — all implemented from scratch. Casting between
any two element types under a `Strict`, `Saturate` or `Wrap` policy, with the
stored width configurable on both read and write.

### Everything else

- Chunked reading and writing for `.npy`, `.npz` and `.h5`.
- `Frame` as the neutral table type, with a pandas `BlockManager` reader and a
  feature-gated polars bridge.
- Write-time validation of container fit, element type, shape and
  representability, before any file is created (`.npz` archives are assembled
  in memory, so a rejected member leaves no partial file).
- 67 integration tests, 11 documentation tests (10 without `polars-interop`),
  and a Python cross-validation harness with 26 checks covering both
  directions, including the real `torch.load` when PyTorch is installed.
- Minimum supported Rust version 1.70 for the default features.
- Documentation in English and Russian.
