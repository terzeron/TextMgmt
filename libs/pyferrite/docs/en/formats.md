# Format-by-format notes

## `.npy`

Complete support: every numpy descriptor this crate has a `DType` for, both
byte orders, C and Fortran order, structured (record) dtypes, and object
arrays, whose elements are decoded by running the pickle machine recursively.

Version 1, 2 and 3 headers are read; version 1 is written unless the header
would exceed 65 535 bytes, in which case version 2 is used. The header is
padded so the data begins on a 64-byte boundary, matching numpy.

Record arrays become a [`Frame`](types.md#frames) by default. Set
`ReadOptions::keep_record_arrays` to get the raw structured array instead.

## `.npz`

A ZIP archive of `.npy` members, read and written with this crate's own ZIP
implementation (including ZIP64 and CRC-32). Stored and deflated members are
both supported.

numpy has no notion of nesting, so nested dicts are flattened with `/`
separators on write and rebuilt on read. A `{"grp": {"inner": ...}}` value
becomes a member named `grp/inner.npy`, which `np.load(...)["grp/inner"]` reads
back directly. Keys may therefore not themselves contain `/`.

A bare array is written as `arr_0`, and a list as `arr_0`, `arr_1`, …, matching
`numpy.savez`.

## `.pkl` / `.pickle`

Protocols 0 through 5, including framing, memoisation, extension registry
opcodes and protocol-5 out-of-band buffer opcodes.

The virtual machine **never executes Python**. Constructors are resolved
against an allow-list covering numpy, torch, `collections` and `_codecs`;
`os`, `sys`, `subprocess` and similar are refused unless
`ReadOptions::allow_unknown_globals` is set. Anything else becomes an inert
`PyObject` recording the class name and its arguments.

numpy arrays are recognised in all three shapes numpy emits them: the classic
`_reconstruct` plus `BUILD` sequence, the `scalar` path for zero-dimensional
values, and the `_frombuffer` path that numpy 2.x uses under protocol 5.

On write, protocol 4 is the default; 2 is the most portable across old Python
versions. The array encoding mirrors numpy's own, so `pickle.load` returns a
real `np.ndarray`, not a surrogate.

## `.pt` / `.pth`

The ZIP layout introduced in torch 1.6: `archive/data.pkl` alongside one raw
blob per storage under `archive/data/`. Persistent ids of the form
`('storage', torch.FloatStorage, key, location, numel)` are resolved against
those blobs. Both contiguous and strided tensors are handled, the latter by
gathering through the recorded strides.

Written files carry `data.pkl`, `version` and `byteorder` members and use
`torch._utils._rebuild_tensor_v2`, which is what `torch.load` expects. The
pickle stream is always written with protocol 2, as `torch.save` does, whatever
`WriteOptions::pickle_protocol` says: the `weights_only` loader that
`torch.load` uses by default since PyTorch 2.6 rejects the opcodes of protocols
4 and 5.

**Legacy checkpoints** — the pre-1.6 format of five concatenated pickles
followed by a storage table — are **not** supported. Reading one produces an
error naming the problem and suggesting
`torch.save(obj, path, _use_new_zipfile_serialization=True)`. This is a
deliberate refusal rather than a best guess: silently mis-reading weights is
worse than failing.

## `.joblib`

joblib writes a pickle in which arrays are replaced by
`NumpyArrayWrapper` objects, with the raw buffer following immediately in the
same byte stream. The reader interleaves accordingly, honouring the alignment
padding that joblib 1.2 and later insert.

Uncompressed, zlib and gzip containers are read. bz2, xz and lz4 are not:
each would need a compressor this crate cannot supply without foreign code, so
they produce a clear error suggesting `compress=0`, `'zlib'` or `'gzip'`.

## `.h5` / `.hdf5`

A pure-Rust subset. What is deliberately narrow is documented rather than
approximated.

### Reading

| Area | Supported |
|---|---|
| Superblock | versions 0, 1, 2, 3 |
| Object headers | version 1 and version 2, with continuation blocks |
| Groups | symbol-table B-trees (classic) and link messages (new style) |
| Data layout | contiguous, compact, chunked (version-1 B-tree index) |
| Filters | deflate, shuffle, fletcher32 |
| Datatypes | integer, floating-point, bitfield, enumeration, fixed-length string |

### Writing

Superblock version 0, version-1 object headers, symbol-table groups, and
contiguous little-endian datasets, nested to any depth. Files are read by
`h5py`, `h5ls` and the reference C library.

### Not supported

Variable-length strings, compound and array datatypes, object references,
virtual and external datasets, attributes, and any filter beyond the three
listed. Encountering one is an `Error::Unsupported` that names the feature.

Bool arrays are written as unsigned 8-bit integers, since HDF5 has no native
boolean; h5py writes an enumeration for the same data, which this crate reads
back as `U8`. Complex numbers have no representation in the writing subset —
cast them or use another format.

## Detection

`read` sniffs the leading bytes: `\x93NUMPY` for `.npy`, `\x89HDF\r\n\x1a\n`
for HDF5, `0x80` for a pickle, and `PK\x03\x04` for a ZIP, which is then
disambiguated by looking at the first member's name — a `.npy` member means
`.npz`, anything else means torch. Only if that is inconclusive does the
extension decide. `ReadOptions::format` overrides everything.
