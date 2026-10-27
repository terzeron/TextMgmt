//! Recovering a [`Frame`] from a pickled `pandas.DataFrame`.
//!
//! pandas does not pickle a table; it pickles a `BlockManager`, which groups
//! columns by dtype into two-dimensional blocks, each carrying the positions of
//! the columns it holds. Reassembling a frame means splitting those blocks back
//! into rows and reuniting them with the column index.
//!
//! Layouts differ across pandas versions — the manager may carry its contents
//! as constructor arguments or as pickled state, and blocks may arrive as
//! `_unpickle_block` calls or as bare tuples. All of these are accepted.

use crate::error::Result;
use crate::value::{Array, Frame, Series, Value};

mod arrow;
mod blocks;
mod index;

use blocks::block_parts;
use index::{index_labels, index_values};

/// `true` when a value looks like something pandas produced.
pub fn is_pandas(v: &Value) -> bool {
    matches!(v, Value::Object(o) if o.module.starts_with("pandas"))
}

/// Try to turn a pickled pandas object into a [`Frame`].
///
/// Returns `Ok(None)` when the object is recognisably pandas but laid out in a
/// way this converter does not handle, so the caller can keep the inert
/// [`PyObject`](crate::value::PyObject) rather than lose information.
///
/// ```no_run
/// # use pyferrite::prelude::*;
/// let v = read("frame.pkl")?;
/// if let Some(frame) = pyferrite::interop::pandas::to_frame(&v)? {
///     println!("{} rows x {} columns", frame.height(), frame.width());
/// }
/// # Ok::<(), pyferrite::Error>(())
/// ```
pub fn to_frame(v: &Value) -> Result<Option<Frame>> {
    let o = match v {
        Value::Object(o) if is_pandas(v) => o,
        _ => return Ok(None),
    };
    let mgr = match o.state.as_deref() {
        Some(Value::Dict(d)) => match d.get("_mgr").or_else(|| d.get("_data")) {
            Some(m) => m,
            None => return Ok(None),
        },
        Some(other) => other,
        None => return Ok(None),
    };
    from_manager(mgr)
}

/// The two halves of a block manager, however it chose to store them.
fn manager_parts(mgr: &Value) -> Option<(Vec<Value>, Vec<Value>)> {
    let parts: Vec<Value> = match mgr {
        // Modern pandas passes `(blocks, axes)` as constructor arguments.
        Value::Object(o) if !o.args.is_empty() => o.args.clone(),
        // Older pandas pickles `(axes, blocks)` as state.
        Value::Object(o) => match o.state.as_deref() {
            Some(Value::Tuple(t)) | Some(Value::List(t)) => t.clone(),
            Some(Value::Dict(d)) => vec![d.get("axes")?.clone(), d.get("blocks")?.clone()],
            _ => return None,
        },
        Value::Tuple(t) | Value::List(t) => t.clone(),
        _ => return None,
    };
    let seq = |v: &Value| match v {
        Value::Tuple(t) | Value::List(t) => Some(t.clone()),
        _ => None,
    };
    let (a, b) = (seq(parts.first()?)?, seq(parts.get(1)?)?);
    // Whichever half holds index objects is the axes; the other holds blocks.
    if a.iter().any(is_index) {
        Some((b, a))
    } else {
        Some((a, b))
    }
}

fn is_index(v: &Value) -> bool {
    matches!(v, Value::Object(o)
        if o.name.contains("Index") || o.name == "_new_Index")
}

fn from_manager(mgr: &Value) -> Result<Option<Frame>> {
    let (blocks, axes) = match manager_parts(mgr) {
        Some(p) => p,
        None => return Ok(None),
    };
    // axes[0] labels the columns, axes[1] the rows.
    let names = index_labels(axes.first());
    let row_index = axes.get(1).and_then(index_values);

    let mut columns: Vec<Option<Series>> = vec![None; names.len()];
    for block in &blocks {
        let (values, placement) = match block_parts(block) {
            Some(p) => p,
            None => continue,
        };
        for (row, pos) in placement.iter().enumerate() {
            let col = match split_row(&values, row, placement.len()) {
                Some(c) => c,
                None => continue,
            };
            if *pos >= columns.len() {
                columns.resize(*pos + 1, None);
            }
            let name = names.get(*pos).cloned().unwrap_or_else(|| format!("column_{pos}"));
            columns[*pos] = Some(Series::new(&name, col));
        }
    }
    let cols: Vec<Series> = columns.into_iter().flatten().collect();
    if cols.is_empty() {
        return Ok(None);
    }
    let mut f = Frame::from_columns(cols)?;
    f.index = row_index.map(|a| Series::new("index", a));
    Ok(Some(f))
}

/// Row `r` of a block, where a block holding a single column may be flat.
fn split_row(a: &Array, r: usize, count: usize) -> Option<Array> {
    if a.ndim() == 1 {
        return if count == 1 && r == 0 { Some(a.clone()) } else { None };
    }
    a.row(r)
}
