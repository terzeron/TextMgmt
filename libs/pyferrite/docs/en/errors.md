# Errors and validation

## The `Error` enum

| Variant | Raised when |
|---|---|
| `Io(std::io::Error)` | The file could not be opened, read or written |
| `Format(String)` | The bytes are not the format they claim to be, or are truncated |
| `Unsupported(String)` | A real feature of the format this crate does not implement, named explicitly |
| `Cast(String)` | A value would not survive a narrowing conversion under `CastPolicy::Strict` |
| `InvalidArgument(String)` | The request itself does not make sense — a string into a `.npy`, an unknown extension, an allocation over `max_alloc` |
| `Shape { expected, got }` | A declared shape does not match the data |
| `Pickle(String)` | Malformed pickle stream, or a refused global |
| `Compression(String)` | A compressed member could not be inflated, or the `deflate` feature is off |
| `Exhausted` | A lazy reader ran past the end of its data |

`Error` implements `Display` and `std::error::Error` by hand — no derive macro
crates are pulled in — so it composes with `anyhow`, `thiserror` or plain
`Box<dyn Error>`.

## Write-time validation

The brief for this crate asked that a string never be written where an array
is expected. That check, and several others, happen before the file is
created, so a rejected write leaves nothing behind:

```rust,no_run
# use pyferrite::prelude::*;
let e = write("x.npy", &Value::Str("hello".into())).unwrap_err();
assert!(format!("{e}").contains("npy"));
// invalid argument: `.npy` stores a single array; `str` is not one.
// Use `.npz`, `.pkl` or `.h5` for containers.
```

What is checked:

- **Container fit.** `.npy` holds exactly one array. Everything else accepts
  nesting.
- **Element type.** A heterogeneous list cannot become a numeric array, and the
  error says which element broke it.
- **Shape.** `WriteOptions::shape` must have the same element count as the
  data, or you get `Error::Shape { expected, got }`. This is what lets you
  write `[1, 6]` rather than `[6]` when numpy needs to see a row vector.
- **Representability.** The HDF5 writer refuses a dtype it cannot express
  instead of silently substituting one.
- **Link names.** HDF5 names may not be empty or contain `/` or a NUL.
- **Completeness.** `LazyWriter::finish` fails if fewer elements arrived than
  were declared.

## Pickle safety

Unpickling arbitrary data in Python executes arbitrary code. This crate does
not: the machine has no mechanism for calling anything.

1. **Allow-list.** `REDUCE` and `NEWOBJ` resolve against a fixed table of
   numpy, torch, `collections` and `_codecs` constructors. Each is implemented
   as a Rust function that builds the corresponding `Value`.
2. **Blocklist.** `os`, `sys`, `subprocess`, `builtins.eval`, `builtins.exec`
   and their relatives are refused with an error naming the module, unless
   `allow_unknown_globals` is set, in which case they are captured inertly
   like any other class so a suspicious file can be inspected.
3. **Inert capture.** Anything else becomes a `PyObject` recording the module,
   class name and constructor arguments. Nothing runs. You can inspect it,
   edit it, and write it back out.
4. **Bounded allocation.** Every length read from a file is checked against
   `ReadOptions::max_alloc` before anything is reserved, so a header claiming a
   petabyte fails immediately.

```rust,no_run
# use pyferrite::prelude::*;
// A pickle calling os.system is refused, not executed.
let e = read("suspicious.pkl").unwrap_err();
assert!(format!("{e}").contains("os"));
```

This makes reading an untrusted checkpoint meaningfully safer than
`pickle.load` — though "safer" is not "safe", and a file from an unknown source
still deserves suspicion.

## Robustness

Truncated and corrupt files produce errors, never panics. Every cursor read is
bounds-checked, every length is validated against the remaining input, and the
object-header walker has a cycle guard. The test suite covers truncation at
several offsets and outright garbage for all six formats.
