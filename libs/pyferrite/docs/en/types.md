# Data model and numeric types

## `DType`

Every reader maps its native type tags onto this one enum, and every writer
maps back out of it. That keeps the conversion work proportional to the number
of formats plus the number of types, instead of their product.

| Group | Variants | Bytes |
|---|---|---|
| Boolean | `Bool` | 1 |
| Signed | `I8` `I16` `I32` `I64` | 1, 2, 4, 8 |
| Unsigned | `U8` `U16` `U32` `U64` | 1, 2, 4, 8 |
| 8-bit floats | `F8E4M3` `F8E5M2` | 1 |
| Half | `F16` `BF16` | 2 |
| Single, double | `F32` `F64` | 4, 8 |
| Quad | `F128` | 16 |
| x87 extended | `X87` | 10 (stored in 12 or 16) |
| Complex | `C64` `C128` `C256` | 8, 16, 32 |
| Text and bytes | `Str(n)` `Bytes(n)` | fixed width |
| Opaque | `Object` | pointer-sized in numpy; decoded recursively here |
| Records | `Struct(Vec<Field>)` | sum of fields |

Predicates: `size()`, `is_integer()`, `is_float()`, `is_complex()`,
`is_numeric()`.

### A naming trap worth knowing

numpy names dtypes by **byte** count, so `f8` is a double. `pyferrite` names
the small floats by **bit** count, so `FloatKind::F8E4M3` is an eight-bit
float. Both spellings parse correctly in `FloatKind::parse`, and the numpy
descriptor strings in file headers are always read with numpy's meaning. The
ambiguity only exists in prose, never in the format.

## The float types written from scratch

None of these exist in stable Rust, so the crate implements them.

**`F16`** — IEEE binary16. Round-to-nearest-even, subnormals, NaN payload
preservation.

**`BF16`** — bfloat16: the top sixteen bits of an `f32`, keeping the full
exponent range at the cost of mantissa bits. This is why it does not overflow
where `F16` does:

```rust
# use pyferrite::prelude::*;
let big = BF16::from_f32(3.0e38);
assert!(big.to_f32().is_finite());          // f16 would give inf here
```

**`F8E4M3`** — four exponent bits, three mantissa bits, bias 7, no infinities,
maximum 448. **`F8E5M2`** — five and two, bias 15, IEEE-shaped, maximum 57344.
These are the two formats used for quantised inference.

**`F128`** — IEEE binary128, with software addition, subtraction,
multiplication and division. Correct rounding, subnormals, and the usual
special cases.

```rust
# use pyferrite::prelude::*;
let third = F128::from_f64(1.0) / F128::from_f64(3.0);
assert_eq!((third * F128::from_f64(3.0)).to_f64(), 1.0);
```

**x87 80-bit** — what numpy calls `longdouble` on x86 Linux. Decoded to and
encoded from `F128`, including the explicit integer bit that makes this format
unlike every other IEEE type.

**Complex** — `Complex32`, `Complex64` and `Complex128`, the last built on
`F128`, covering numpy's `complex64`, `complex128` and `complex256`.

## Casting

`Array::cast(&DType, CastPolicy)` converts between any two element types. All
conversions pivot through a wide scalar (`i128`, `u128`, `F128` or a complex
pair) so that no path loses precision it did not have to.

| Policy | Out-of-range behaviour |
|---|---|
| `Strict` *(default)* | `Err(Error::Cast)` naming the offending value and the target range |
| `Saturate` | Clamp to the target minimum or maximum |
| `Wrap` | Truncate the bit pattern, like a C cast |

```rust
# use pyferrite::prelude::*;
# use ndarray::{ArrayD, IxDyn};
let a = Array::I64(ArrayD::from_shape_vec(IxDyn(&[2]), vec![-99_999, 99_999]).unwrap());

assert!(a.cast(&DType::I16, CastPolicy::Strict).is_err());

let clamped = a.cast(&DType::I16, CastPolicy::Saturate).unwrap();
assert_eq!(clamped.as_i16().unwrap().as_slice().unwrap(), &[i16::MIN, i16::MAX]);
```

Float narrowing never errors under `Strict` — rounding is not the same as
overflow — but a value beyond the target's range becomes an infinity, which is
usually a bug you want to hear about. Use `Saturate` if you would rather clamp.

## `Value` mutability

Every read returns owned data. There is no borrowed view of a mapped file to
worry about, so mutation needs nothing beyond `let mut`:

```rust,no_run
# use pyferrite::prelude::*;
let mut v = read("weights.npy")?;
v.as_array_mut()
    .and_then(|a| a.as_f32_mut())
    .map(|w| w.iter_mut().for_each(|x| *x = x.clamp(-1.0, 1.0)));
# Ok::<(), pyferrite::Error>(())
```

`arrays_mut()` does the same across a whole nested container in one pass.

## Frames

`Frame` is the neutral table type sitting between pandas, polars and numpy
record arrays. A numpy structured array becomes a `Frame` on read unless
`ReadOptions::keep_record_arrays` says otherwise. Each `Series` carries a name,
an `Array` of values, and an optional validity bitmap for nulls, which is the
one thing numpy has no way to express.

With the `polars-interop` feature enabled, `Frame` converts to and from
`polars::DataFrame`. pandas frames arrive as their pickled `BlockManager`; the
numeric blocks are recovered, and anything exotic is preserved as a `PyObject`
rather than guessed at.

### Reading a pandas frame

`interop::pandas::to_frame` reassembles a pickled `pandas.DataFrame`. pandas
pickles a `BlockManager` rather than a table: columns are grouped by dtype into
two-dimensional blocks, each recording which column positions it holds. The
converter splits those blocks back into columns and reunites them with the
column index.

```rust,no_run
# use pyferrite::prelude::*;
let v = read("frame.pkl")?;
match pyferrite::interop::pandas::to_frame(&v)? {
    Some(f) => println!("{:?}", f.names()),
    None => println!("kept as an inert object: {}", v.type_name()),
}
# Ok::<(), pyferrite::Error>(())
```

Layouts differ across pandas versions — the manager may carry its contents as
constructor arguments or as pickled state, and blocks may arrive as
`_unpickle_block` calls or as bare tuples. All are accepted, and extension
arrays such as `StringArray` are unwrapped to their backing values. When a
layout is not recognised, the function returns `Ok(None)` and the caller keeps
the inert `PyObject`, so nothing is silently lost.

### Converting to polars

With the `polars-interop` feature:

```rust,no_run
# #[cfg(feature = "polars-interop")] {
# use pyferrite::prelude::*;
# let frame = pyferrite::value::Frame::default();
let df = pyferrite::interop::polars::to_polars(&frame)?;
let back = pyferrite::interop::from_polars(&df)?;
# }
# Ok::<(), pyferrite::Error>(())
```

Nulls travel both ways: polars nulls become entries in the `Series` validity
bitmap, and the numeric buffer underneath stays dense so it can still be
written straight to numpy. Types polars has no slot for — `f8`, `f16`, `bf16`,
`f128` — are widened to `f64`; complex numbers are rejected rather than
mangled.
