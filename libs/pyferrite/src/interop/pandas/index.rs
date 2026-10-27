//! Reading labels and values out of a pandas index object.

use super::blocks::values_array;
use crate::value::{Array, Value};

/// Column or row labels of an index object, as strings.
pub fn index_labels(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => {
            (0..a.len()).map(|i| a.as_str_at(i).unwrap_or_else(|| format!("column_{i}"))).collect()
        }
        Some(Value::List(items)) | Some(Value::Tuple(items)) => {
            let labels: Vec<String> =
                items.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            if labels.len() == items.len() {
                labels
            } else {
                // Nested: `_new_Index` wraps its payload one level down.
                items.iter().find_map(|x| non_empty(index_labels(Some(x)))).unwrap_or_default()
            }
        }
        Some(Value::Object(o)) => {
            // `_new_Index(cls, {"data": ..., "name": ...})`, or a RangeIndex
            // described by start/stop/step.
            for arg in o.args.iter().chain(o.state.as_deref()) {
                if let Value::Dict(d) = arg {
                    if let Some(labels) =
                        d.get("data").and_then(|x| non_empty(index_labels(Some(x))))
                    {
                        return labels;
                    }
                    // pandas 3: `data` is an ArrowStringArray over pyarrow buffers.
                    if let Some(Array::Str(s)) = d.get("data").and_then(values_array) {
                        return s.iter().cloned().collect();
                    }
                    if let Some(r) = range_index(d) {
                        return r.into_iter().map(|i| i.to_string()).collect();
                    }
                }
                if let Some(labels) = non_empty(index_labels(Some(arg))) {
                    return labels;
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// The index as an array, when it holds data rather than a bare range.
pub fn index_values(v: &Value) -> Option<Array> {
    match v {
        Value::Array(a) => Some(a.clone()),
        Value::Object(o) => {
            for arg in o.args.iter().chain(o.state.as_deref()) {
                if let Value::Dict(d) = arg {
                    if let Some(r) = range_index(d) {
                        let n = r.len();
                        return ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&[n]), r)
                            .ok()
                            .map(Array::I64);
                    }
                    if let Some(a) = d.get("data").and_then(values_array) {
                        return Some(a);
                    }
                }
                if let Some(a) = values_array(arg) {
                    return Some(a);
                }
            }
            None
        }
        _ => None,
    }
}

fn range_index(d: &crate::value::Dict) -> Option<Vec<i64>> {
    let start = d.get("start")?.as_i64()?;
    let stop = d.get("stop")?.as_i64()?;
    let step = d.get("step").and_then(|v| v.as_i64()).unwrap_or(1).max(1);
    Some((start..stop).step_by(step as usize).collect())
}

fn non_empty(v: Vec<String>) -> Option<Vec<String>> {
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}
