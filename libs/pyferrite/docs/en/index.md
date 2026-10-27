# pyferrite — documentation

`pyferrite` reads and writes the file formats Python machine-learning code
produces, from Rust, without linking against a single line of C.

- [API reference](api.md) — every public function and every parameter
- [Data model and numeric types](types.md)
- [Streaming large files](lazy.md)
- [Format-by-format notes](formats.md)
- [Errors and validation](errors.md)

Русская версия: [`../ru/index.md`](../ru/index.md)

## Installing

```toml
[dependencies]
pyferrite = "0.0.2"
```

Version `0.0.2` is an early release. The public API may still change
before `0.1.0`.

### Feature flags

| Flag | Default | What it adds | Cost |
|---|---|---|---|
| `deflate` | **on** | Compressed `.npz` members, gzip- and zlib-wrapped `.joblib`, and the deflate filter in HDF5 | one dependency, `miniz_oxide` (pure Rust) |
| `polars-interop` | off | `Frame` ⇄ `polars::DataFrame` conversion | pulls in `polars` |

To build with nothing but the standard library and `ndarray`:

```toml
pyferrite = { version = "0.0.2", default-features = false }
```

Uncompressed files still work in that configuration; a compressed one returns
`Error::Compression` rather than producing wrong numbers.

## Design commitments

**No foreign code.** No C, no C++, no FFI, no pre-built binaries. Every
format, every checksum, every float encoding and the entire pickle virtual
machine are implemented in this crate. That is why it works in bare-metal and
embedded environments where only `rustc` is available.

**Everything you get back is owned.** There are no borrowed views into a
memory-mapped file, so `let mut` is all you need to rescale weights in place
before feeding them to a model. Every array type also has an explicit
`as_*_mut()` accessor.

**Failures are loud.** A shape mismatch, an out-of-range cast, a truncated
file or a datatype the writer cannot express is an `Err`, never a silently
mangled file.

**Pickle is never executed.** See [errors.md](errors.md#pickle-safety).

## Quick start

```rust,no_run
use pyferrite::prelude::*;

// Read anything; the format comes from the magic bytes, with the file
// extension as a fallback.
let mut model = read("checkpoint.pt")?;

// Walk every array in the tree, whatever the nesting.
for (name, array) in flatten_arrays(&model) {
    println!("{name}: {:?} {:?}", array.dtype(), array.shape());
}

// Mutate in place.
for a in model.arrays_mut() {
    if let Some(w) = a.as_f32_mut() {
        w.iter_mut().for_each(|x| *x *= 0.5);
    }
}

// Write it back out in a different format entirely.
write("checkpoint.h5", &model)?;
# Ok::<(), pyferrite::Error>(())
```

## Format support at a glance

| Extension | Read | Write | Stream in | Stream out |
|---|---|---|---|---|
| `.npy` | yes | yes | yes | yes |
| `.npz` | yes | yes | yes | no |
| `.pkl`, `.pickle` | yes | yes | no | no |
| `.pt`, `.pth` | yes (zip layout) | yes | no | no |
| `.joblib` | yes | yes | no | no |
| `.h5`, `.hdf5` | subset | subset | yes | yes |

Streaming is unavailable where the payload is interleaved with the object
graph — a pickle has to be walked from the start to know where anything is.
[formats.md](formats.md) documents the exact HDF5 subset.
