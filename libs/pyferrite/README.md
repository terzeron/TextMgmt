# pyferrite

[![crates.io](https://img.shields.io/crates/v/pyferrite.svg)](https://crates.io/crates/pyferrite)
[![docs.rs](https://docs.rs/pyferrite/badge.svg)](https://docs.rs/pyferrite)
[![GitHub](https://img.shields.io/badge/github-89605502155%2Fpyferrite-blue?logo=github)](https://github.com/89605502155/pyferrite)
[![licence: MIT](https://img.shields.io/badge/licence-MIT-green.svg)](https://github.com/89605502155/pyferrite/blob/main/LICENSE)

Source code: <https://github.com/89605502155/pyferrite>, release [`v0.0.2`](https://github.com/89605502155/pyferrite/releases/tag/v0.0.2).

Read and write Python machine-learning artefacts from Rust — `.npy`, `.npz`,
`.pkl`, `.pt`, `.joblib` and `.h5` — with **no C, no C++, no FFI and no
compiled artefacts from any other language**. If you have `rustc`, you can
build it, including on bare-metal and embedded targets.

`.forbid(unsafe_code)` is enforced crate-wide.

```toml
[dependencies]
pyferrite = "0.0.2"
```

## Why

Moving a model between Python and Rust usually means either linking against
libhdf5 and libtorch, or reimplementing the file formats badly. `pyferrite`
does the second job properly, so a Rust inference stack can load reference
weights, and a Rust training loop can hand results back to the Python tooling
your team already uses. That makes it practical to cross-check a Rust
implementation against the Python original on byte-identical data.

## A tour in four lines

```rust,no_run
use pyferrite::prelude::*;

let mut weights = read("model.pt")?;          // dict of tensors
for a in weights.arrays_mut() {               // everything is owned, so mutable
    if let Some(f) = a.as_f32_mut() {
        f.iter_mut().for_each(|x| *x *= 0.5); // rescale before loading
    }
}
write("model.h5", &weights)?;                 // and out again, any format
# Ok::<(), pyferrite::Error>(())
```

## What it does

| | |
|---|---|
| **Formats** | `.npy` `.npz` `.pkl`/`.pickle` `.pt`/`.pth` `.joblib` `.h5`/`.hdf5` |
| **Python types** | `dict` `list` `tuple` `set` `str` `bytes` `int` (arbitrary precision) `float` `complex` `bool` `None` |
| **Arrays** | `np.ndarray` and `torch.Tensor` → [`ndarray::ArrayD`], both directions |
| **Tables** | `pandas` / `polars` frames via [`Frame`], with a feature-gated polars bridge |
| **Element types** | `bool`, `i8`–`i64`, `u8`–`u64`, `f16`, `bf16`, `f32`, `f64`, `f128`, x87 80-bit, `complex64/128/256`, and 8-bit floats (`e4m3`, `e5m2`) |
| **Streaming** | chunked reading *and* writing for files larger than RAM |
| **Casting** | pick the stored width on the way in and on the way out |

## Choosing numeric types

Both directions are configurable, which is the point of the `*_as` options:

```rust,no_run
use pyferrite::prelude::*;

// "I know these should be f32, whatever the file says."
let opts = ReadOptions::new().float_as(FloatKind::F32).int_as(IntKind::I32);
let v = read_with("weights.npz", &opts)?;

// And symmetrically on the way out, clamping instead of failing.
let out = WriteOptions::new()
    .float_as(FloatKind::F16)
    .cast_policy(CastPolicy::Saturate);
write_with("small.npz", &v, &out)?;
# Ok::<(), pyferrite::Error>(())
```

`CastPolicy::Strict` is the default and refuses any conversion that would lose
information; `Saturate` clamps to the target range; `Wrap` truncates like a C
cast. Note that `f8` here means an eight-**bit** float, unlike numpy's `f8`,
which means an eight-**byte** one.

## Streaming a file that does not fit in memory

```rust,no_run
use pyferrite::prelude::*;

let mut r = read_lazy("huge.npy", &ReadOptions::new().chunk_elements(1 << 16))?;
while let Some(chunk) = r.next_chunk()? {
    // chunk.data is a small owned array; chunk.offset says where it belongs
    // inside chunk.full_shape.
}

let mut w = write_lazy("huge_out.npy", DType::F32, &[10_000, 512], &WriteOptions::new())?;
# let block = pyferrite::value::Array::F32(ndarray::ArrayD::zeros(ndarray::IxDyn(&[512])));
for _ in 0..10_000 {
    w.push(&block)?;
}
w.finish()?;   // fails if fewer elements arrived than were declared
# Ok::<(), pyferrite::Error>(())
```

## Writing is validated

Shapes and types are checked before anything reaches the disk, because numpy
distinguishes a scalar `2` from `[2]` from `[[2]]`:

```rust,no_run
use pyferrite::prelude::*;

# let value = Value::Array(pyferrite::value::Array::F64(ndarray::ArrayD::zeros(ndarray::IxDyn(&[6]))));
// Declare the exact shape numpy should see.
write_with("x.npy", &value, &WriteOptions::new().shape([1, 6]))?;

// And a string is not an array, so this is an error, not a corrupt file:
assert!(write("x.npy", &Value::Str("hello".into())).is_err());
# Ok::<(), pyferrite::Error>(())
```

## Safety

Pickle is an executable format, and `pyferrite` does not execute it. The
virtual machine resolves an allow-list of numpy, torch and `collections`
constructors; anything else becomes an inert [`PyObject`] that records the
class name and its arguments without calling anything. References to `os`,
`sys`, `subprocess`, `eval` and similar are refused unless
`ReadOptions::allow_unknown_globals` asks to capture them inertly for
inspection. Allocation is bounded by
`ReadOptions::max_alloc`, so a hostile header cannot exhaust memory.

## Dataframes

`Frame` is the neutral table type. A numpy record array becomes one on read; a
pickled `pandas.DataFrame` is reassembled from its `BlockManager` by
[`interop::pandas::to_frame`]; and with the `polars-interop` feature, `Frame`
converts to and from a `polars::DataFrame`.

```rust,no_run
# use pyferrite::prelude::*;
let v = read("frame.pkl")?;
if let Some(frame) = pyferrite::interop::pandas::to_frame(&v)? {
    println!("{} rows x {} columns", frame.height(), frame.width());
}
# Ok::<(), pyferrite::Error>(())
```

## Documentation

Full per-function, per-parameter documentation with worked examples:

- English — [`docs/en/`](docs/en/index.md)
- Русский — [`docs/ru/`](docs/ru/index.md)

## Feature flags

| Flag | Default | Effect |
|---|---|---|
| `deflate` | yes | compressed `.npz`, gzip/zlib `.joblib`, deflate-filtered HDF5 |
| `polars-interop` | no | converts [`Frame`] to and from a `polars::DataFrame` |

Turning `deflate` off removes the only non-trivial dependency; uncompressed
files still work.

## Status

Version 0.0.2 is an early release: complete enough to move real models
between Python and Rust, but the API may still change before 0.1.0.

The minimum supported Rust version is 1.70 for the default features and for
`--no-default-features`. On a toolchain that old, resolve dependencies with
`CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update` (or Cargo's
MSRV-aware resolver) so that transitive crates are not newer than the
compiler. The `polars-interop` feature follows the requirements of polars 0.35
itself and needs a more recent compiler.

## Licence

MIT, see [`LICENSE`](https://github.com/89605502155/pyferrite/blob/main/LICENSE).

Authors, Bryansk State Engineering Technological University (BGITU), Bryansk, Russia:

- Andrey O. Ferubko, postgraduate student, Department of Information
  Technologies, ferubko1999@yandex.ru
- Oleg D. Kazakov, Candidate of Economic Sciences, Associate Professor, Head of
  the Department of Information Technologies (scientific supervisor),
  kazakov@bgitu.ru

Авторы: Ферубко Андрей Олегович, аспирант кафедры информационных технологий, и
Казаков Олег Дмитриевич, канд. экон. наук, доцент, заведующий кафедрой
информационных технологий (научный руководитель), Брянский государственный
инженерно-технологический университет (БГИТУ).

[`ndarray::ArrayD`]: https://docs.rs/ndarray/latest/ndarray/type.ArrayD.html
[`Frame`]: crate::value::Frame
[`interop::pandas::to_frame`]: crate::interop::pandas::to_frame
[`PyObject`]: crate::value::PyObject
