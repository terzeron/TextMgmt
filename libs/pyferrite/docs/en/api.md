# API reference

Everything in this page is re-exported from `pyferrite::prelude`.

---

## Top-level functions

### `read(path) -> Result<Value>`

Reads a file with default options.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | `impl AsRef<Path>` | File to read. The format is detected from the leading magic bytes; if those are inconclusive the extension decides. |

Returns a [`Value`](types.md#value): an `Array` for `.npy`, a `Dict` for the
container formats, and whatever the object graph holds for a pickle.

```rust,no_run
# use pyferrite::prelude::*;
let v = read("weights.npz")?;
println!("{}", v.type_name());
# Ok::<(), pyferrite::Error>(())
```

### `read_with(path, opts) -> Result<Value>`

As `read`, with control over casting, allocation limits and format detection.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | `impl AsRef<Path>` | File to read |
| `opts` | `&ReadOptions` | See [ReadOptions](#readoptions) |

```rust,no_run
# use pyferrite::prelude::*;
// Force every float in the file to f32 and cap allocation at 512 MiB.
let opts = ReadOptions::new().float_as(FloatKind::F32).max_alloc(512 << 20);
let v = read_with("big.pkl", &opts)?;
# Ok::<(), pyferrite::Error>(())
```

### `write(path, value) -> Result<()>`

Writes `value` with default options. The format comes from the extension, so
the path must have one this crate recognises.

| Parameter | Type | Meaning |
|---|---|---|
| `path` | `impl AsRef<Path>` | Destination |
| `value` | `&Value` | What to write. `.npy` accepts a single array; every other format accepts arbitrary nesting. |

### `write_with(path, value, opts) -> Result<()>`

| Parameter | Type | Meaning |
|---|---|---|
| `path` | `impl AsRef<Path>` | Destination |
| `value` | `&Value` | What to write |
| `opts` | `&WriteOptions` | See [WriteOptions](#writeoptions) |

Validation happens before the file is created, so a rejected write leaves no
partial file behind.

### `read_lazy(path, opts) -> Result<LazyReader>`

Opens a file for chunk-at-a-time reading. Supported for `.npy`, `.npz` and
`.h5`. See [lazy.md](lazy.md).

### `write_lazy(path, dtype, shape, opts) -> Result<LazyWriter>`

| Parameter | Type | Meaning |
|---|---|---|
| `path` | `impl AsRef<Path>` | Destination |
| `dtype` | `DType` | Element type to store. Chunks of any type are cast to it. |
| `shape` | `&[usize]` | Full final shape. Declared upfront because the header precedes the data. |
| `opts` | `&WriteOptions` | Compression, cast policy, format override |

### `convert(src, dst, read_opts, write_opts) -> Result<()>`

Reads one file and writes it back out in another format.

```rust,no_run
# use pyferrite::prelude::*;
convert("model.pt", "model.h5", &ReadOptions::new(), &WriteOptions::new())?;
# Ok::<(), pyferrite::Error>(())
```

### `flatten_arrays(&Value) -> Vec<(String, &Array)>`

Collects every array in a value, keyed by a dotted path. Nested dicts join
their keys with `.`, so a torch `state_dict` comes back reading exactly the way
Python prints it: `layer1.weight`, `layer1.bias`, and so on. List and tuple
elements contribute their index.

---

## `ReadOptions`

Every method takes `self` and returns `Self`, so they chain. All are optional.

| Method | Type | Default | Effect |
|---|---|---|---|
| `lazy(bool)` | `bool` | `false` | Marks the intent to stream. `read_lazy` is the real entry point; this flag lets a caller carry the choice in a config struct. |
| `chunk_elements(usize)` | `usize` | `1 << 20` | Elements per chunk when streaming. Memory use is roughly this times the element size. |
| `int_as(IntKind)` | `IntKind` | none | Convert every integer array to this width. `I8` … `I64`, `U8` … `U64`. |
| `float_as(FloatKind)` | `FloatKind` | none | Convert every float array to this width: `F8E4M3`, `F8E5M2`, `F16`, `BF16`, `F32`, `F64`, `F128`. |
| `cast_policy(CastPolicy)` | `CastPolicy` | `Strict` | What to do when a value does not fit. See [types.md](types.md#casting). |
| `format(Format)` | `Format` | auto | Skip detection and force a reader. |
| `allow_unknown_globals(bool)` | `bool` | `false` | Capture blocklisted callables (`os`, `eval`, ...) as inert `PyObject`s instead of refusing the file. Unknown classes are captured inertly either way, and nothing is executed. |
| `max_alloc(usize)` | `usize` | 8 GiB | Refuse any single allocation above this. Protects against hostile or corrupt headers. |
| `keep_record_arrays(bool)` | `bool` | `false` | Keep numpy record arrays as raw structured arrays rather than converting them to a `Frame`. |

```rust,no_run
# use pyferrite::prelude::*;
let opts = ReadOptions::new()
    .float_as(FloatKind::F16)
    .cast_policy(CastPolicy::Saturate)   // clamp instead of failing
    .max_alloc(1 << 30)
    .chunk_elements(4096);
# let _ = opts;
```

---

## `WriteOptions`

| Method | Type | Default | Effect |
|---|---|---|---|
| `lazy(bool)` | `bool` | `false` | Intent marker, mirroring `ReadOptions::lazy`. |
| `chunk_elements(usize)` | `usize` | `1 << 20` | Chunk size when streaming out. |
| `int_as(IntKind)` | `IntKind` | none | Store integers at this width. |
| `float_as(FloatKind)` | `FloatKind` | none | Store floats at this width — this is how you write f64 data as f16. |
| `dtype(DType)` | `DType` | none | Force one exact element type, overriding `int_as` and `float_as`. |
| `shape(impl Into<Vec<usize>>)` | `Vec<usize>` | source shape | The shape the file should declare. The element count must match, or you get `Error::Shape`. This is how you distinguish `2` from `[2]` from `[[2]]`. |
| `cast_policy(CastPolicy)` | `CastPolicy` | `Strict` | Behaviour on narrowing. |
| `format(Format)` | `Format` | from extension | Force a writer regardless of the file name. |
| `compression(Compression)` | `Compression` | `None` | `None`, `Compression::deflate()`, `Compression::zlib()`. Applies to `.npz` members, `.joblib` and `.pt`. |
| `pickle_protocol(u8)` | `u8` | `4` | Protocol for `.pkl`, `.pt` and `.joblib`. Values 2–5 are supported; 2 is the most portable. |
| `fortran_order(bool)` | `bool` | `false` | Write `.npy` in column-major order. |

```rust,no_run
# use pyferrite::prelude::*;
# let value = Value::Array(pyferrite::value::Array::F64(ndarray::ArrayD::zeros(ndarray::IxDyn(&[6]))));
let opts = WriteOptions::new()
    .float_as(FloatKind::F32)
    .shape([2, 3])
    .compression(Compression::deflate())
    .pickle_protocol(2);
write_with("out.npz", &value, &opts)?;
# Ok::<(), pyferrite::Error>(())
```

---

## `Value`

The Rust mirror of a Python object graph.

| Variant | Python counterpart |
|---|---|
| `None` | `None` |
| `Bool(bool)` | `bool` |
| `Int(i64)` | `int` that fits |
| `BigInt(Vec<u8>)` | arbitrary-precision `int`, little-endian two's complement |
| `Float(f64)` | `float` |
| `Complex(Complex64)` | `complex` |
| `Str(String)` | `str` |
| `Bytes(Vec<u8>)` | `bytes`, `bytearray` |
| `List(Vec<Value>)` | `list` |
| `Tuple(Vec<Value>)` | `tuple` |
| `Set(Vec<Value>)` | `set`, `frozenset` |
| `Dict(Dict)` | `dict`, `OrderedDict` — insertion order preserved |
| `Array(Array)` | `np.ndarray`, `torch.Tensor` |
| `Frame(Frame)` | `pandas.DataFrame`, `polars.DataFrame`, numpy record array |
| `Object(PyObject)` | any other class, captured inertly |

### Accessors

Read-only: `as_str`, `as_f64`, `as_i64`, `as_array`, `as_dict`, `as_frame`,
`as_object`, `type_name`.

Mutable: `as_array_mut`, `as_dict_mut`, `as_frame_mut`, and `arrays_mut()`,
which yields `&mut Array` for every array anywhere in the tree.

```rust,no_run
# use pyferrite::prelude::*;
let mut v = read("model.pkl")?;
if let Some(d) = v.as_dict_mut() {
    d.insert(Value::Str("epoch".into()), Value::Int(12));
}
for a in v.arrays_mut() {
    if let Some(f) = a.as_f64_mut() {
        let mean = f.iter().sum::<f64>() / f.len() as f64;
        f.iter_mut().for_each(|x| *x -= mean);   // centre in place
    }
}
write("model.pkl", &v)?;
# Ok::<(), pyferrite::Error>(())
```

---

## `Dict`

An insertion-ordered map, because Python dicts are ordered and round-tripping
must not shuffle a `state_dict`.

`insert`, `get`, `get_mut`, `get_by`, `remove`, `contains_key`, `iter`,
`iter_mut`, `keys`, `values_mut`, `into_entries`. `get` and friends accept a
`&str` directly for the common case of string keys.

## `Array`

An enum over `ndarray::ArrayD<T>` for all twenty element types. `dtype()`,
`shape()`, `len()`, `ndim()`, `reshape(&[usize])`, `cast(&DType, CastPolicy)`,
plus `as_f32()` / `as_f32_mut()` / `into_f32()` style accessors for each type.

## `Frame` and `Series`

`Frame` is a list of named `Series`, each a `Array` plus an optional validity
bitmap for nulls. `from_columns`, `height`, `width`, `names`, `column`,
`column_mut`, `push_column`, `drop_column`.

## `PyObject`

What an unrecognised Python class becomes: `module`, `name`, `args`, `kwargs`,
`state`, `list_items`, `dict_items`, with `qualname()`, `attr()` and
`attr_mut()`. Nothing is called; the constructor arguments are simply
recorded, so you can inspect or rewrite them and write the object back out.
