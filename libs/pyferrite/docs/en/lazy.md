# Streaming large files

A checkpoint that does not fit in RAM still has to be read. `pyferrite`
exposes chunked reading and writing through two handles: `LazyReader` and
`LazyWriter`.

Supported for `.npy`, `.npz` and `.h5`. Pickle-based formats (`.pkl`, `.pt`,
`.joblib`) cannot stream, because their payloads are interleaved with the
object graph — you have to walk a pickle from byte zero to know where anything
lives.

## Reading

```rust,no_run
use pyferrite::prelude::*;

let opts = ReadOptions::new().chunk_elements(1 << 16);   // 65 536 at a time
let mut r = read_lazy("huge.npy", &opts)?;

let mut running_sum = 0.0f64;
while let Some(chunk) = r.next_chunk()? {
    for x in chunk.data.as_f32().unwrap() {
        running_sum += *x as f64;
    }
}
# Ok::<(), pyferrite::Error>(())
```

### `Chunk`

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Which array this came from. For `.npy` it is the file stem; for `.npz` and `.h5`, the member or dataset name. |
| `dtype` | `DType` | Element type after any configured cast |
| `full_shape` | `Vec<usize>` | Shape of the *complete* array, not of this chunk |
| `offset` | `usize` | Flat C-order index where this chunk begins |
| `data` | `Array` | The chunk itself, always one-dimensional |

`len()`, `total()` and `is_last()` are provided for the common bookkeeping.

Because `offset` is a flat index into `full_shape`, reassembling is
straightforward, and so is deciding you only need part of the file:

```rust,no_run
# use pyferrite::prelude::*;
# let mut r = read_lazy("huge.npy", &ReadOptions::new())?;
while let Some(c) = r.next_chunk()? {
    let row = c.offset / c.full_shape[1];
    if row >= 1000 { break; }             // only the first 1000 rows are needed
}
# Ok::<(), pyferrite::Error>(())
```

`LazyReader` also implements `Iterator<Item = Result<Chunk>>`, and
`collect_chunks()` gathers everything if you change your mind. `names()` lists
the arrays a file contains without decoding any of them.

## Writing

The header of a `.npy` file precedes its data, so the shape and element type
must be declared before the first chunk arrives:

```rust,no_run
use pyferrite::prelude::*;
use ndarray::{ArrayD, IxDyn};

let mut w = write_lazy("out.npy", DType::F32, &[100_000, 128], &WriteOptions::new())?;

for i in 0..100_000 {
    let row = Array::F32(
        ArrayD::from_shape_vec(IxDyn(&[128]), vec![i as f32; 128]).unwrap(),
    );
    w.push(&row)?;
}
w.finish()?;
# Ok::<(), pyferrite::Error>(())
```

| Method | Behaviour |
|---|---|
| `push(&Array)` | Appends a chunk, casting it to the declared `dtype` if needed. Errors if it would overflow the declared shape. |
| `remaining()` | Elements still expected |
| `finish()` | Flushes and closes. **Errors if fewer elements arrived than declared**, rather than leaving a short file that looks valid. |

Chunks do not have to be uniform in size, and they do not have to match the
declared element type — each is cast on the way through, subject to
`WriteOptions::cast_policy`.

## Memory behaviour

Peak usage while streaming a `.npy` is roughly `chunk_elements` times the
element size, plus a small constant. `.npz` inflates one member at a time, so
peak usage is one decompressed member. HDF5 materialises one dataset at a time
for the same reason: chunk indexes are B-trees, and the chunks of one dataset
can be scattered anywhere in the file.

The HDF5 writer buffers a dataset before emitting it, because object headers
carry the addresses of data that has not been written yet. Streaming *out* to
HDF5 therefore bounds the caller's memory but not the library's; use `.npy` if
you need a genuinely constant-memory writer.
