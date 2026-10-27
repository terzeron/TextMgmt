//! Pulling `(values, placement)` out of one `BlockManager` block.

use crate::value::{Array, Value};

/// Decode a block into its values and the column positions it occupies.
pub fn block_parts(block: &Value) -> Option<(Array, Vec<usize>)> {
    let (values, placement) = match block {
        // `pandas._libs.internals._unpickle_block(values, placement, ndim)`
        Value::Object(o) if o.args.len() >= 2 => (o.args[0].clone(), o.args[1].clone()),
        Value::Object(o) => match o.state.as_deref()? {
            Value::Dict(d) => (d.get("values")?.clone(), d.get("placement")?.clone()),
            Value::Tuple(t) | Value::List(t) => (t.first()?.clone(), t.get(1)?.clone()),
            _ => return None,
        },
        Value::Tuple(t) | Value::List(t) => (t.first()?.clone(), t.get(1)?.clone()),
        _ => return None,
    };
    Some((values_array(&values)?, positions(&placement)))
}

/// Unwrap the array inside a block, seeing through pandas extension arrays.
pub fn values_array(v: &Value) -> Option<Array> {
    match v {
        Value::Array(a) => Some(a.clone()),
        // Extension arrays such as StringArray and Categorical are pickled as
        // `__pyx_unpickle_NDArrayBacked(cls, checksum, state)`, where the state
        // tuple holds the dtype and the backing values.
        // pandas 3 strings: an ArrowStringArray wrapping a pyarrow array.
        Value::Object(o) if o.name == "_restore_array" => super::arrow::restore_array(o),
        Value::Object(o) if o.name == "chunked_array" => super::arrow::chunked_array(o),
        Value::Dict(d) => d.iter().find_map(|(_, x)| values_array(x)),
        Value::Object(o) => {
            for candidate in o.args.iter().chain(o.state.as_deref()) {
                if let Some(a) = values_array(candidate) {
                    return Some(a);
                }
            }
            None
        }
        Value::Tuple(items) | Value::List(items) => {
            if let Some(a) = items.iter().find_map(values_array) {
                return Some(a);
            }
            // A plain list of strings is itself a column.
            let strs: Vec<String> =
                items.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            if !strs.is_empty() && strs.len() == items.len() {
                let n = strs.len();
                return ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&[1, n]), strs)
                    .ok()
                    .map(Array::Str);
            }
            None
        }
        _ => None,
    }
}

/// Decode a `slice`, an integer array or a list of column positions.
pub fn positions(v: &Value) -> Vec<usize> {
    match v {
        Value::Array(a) => (0..a.len()).filter_map(|i| int_at(a, i)).collect(),
        Value::List(items) | Value::Tuple(items) => {
            items.iter().filter_map(|x| x.as_i64()).map(|x| x as usize).collect()
        }
        Value::Object(o) if o.name == "slice" => {
            let n = |i: usize, d: i64| o.args.get(i).and_then(|v| v.as_i64()).unwrap_or(d);
            let (start, stop, step) = (n(0, 0), n(1, 0), n(2, 1).max(1));
            (start..stop).step_by(step as usize).map(|x| x as usize).collect()
        }
        _ => Vec::new(),
    }
}

fn int_at(a: &Array, i: usize) -> Option<usize> {
    a.as_i64()
        .map(|x| x[i])
        .or_else(|| a.as_i32().map(|x| x[i] as i64))
        .or_else(|| a.as_i16().map(|x| x[i] as i64))
        .filter(|x| *x >= 0)
        .map(|x| x as usize)
}
