//! Frames, and the bridges to pandas and polars.

mod common;
use common::*;
use ndarray::{ArrayD, IxDyn};
use pyferrite::interop::pandas;
use pyferrite::prelude::*;

/// A hand-built pandas pickle graph, matching what pandas 2.x and 3.x emit:
/// `DataFrame` state `{_mgr: BlockManager(blocks, axes)}`, blocks created by
/// `_unpickle_block(values, slice, ndim)`.
fn fake_pandas() -> Value {
    let block = |values: Array, start: i64, stop: i64| {
        let mut slice = PyObject::new("builtins", "slice");
        slice.args = vec![Value::Int(start), Value::Int(stop), Value::Int(1)];
        let mut b = PyObject::new("pandas._libs.internals", "_unpickle_block");
        b.args = vec![Value::Array(values), Value::Object(slice), Value::Int(2)];
        Value::Object(b)
    };
    let ints = Array::I64(ArrayD::from_shape_vec(IxDyn(&[1, 4]), vec![1, 2, 3, 4]).unwrap());
    let floats =
        Array::F64(ArrayD::from_shape_vec(IxDyn(&[1, 4]), vec![0.5, 1.5, 2.5, 3.5]).unwrap());

    let labels = Array::Str(
        ArrayD::from_shape_vec(IxDyn(&[2]), vec!["a".to_string(), "b".to_string()]).unwrap(),
    );
    let mut col_dict = Dict::new();
    col_dict.insert(Value::Str("data".into()), Value::Array(labels));
    let mut col_index = PyObject::new("pandas.core.indexes.base", "_new_Index");
    col_index.args = vec![Value::None, Value::Dict(col_dict)];

    let mut row_dict = Dict::new();
    row_dict.insert(Value::Str("start".into()), Value::Int(0));
    row_dict.insert(Value::Str("stop".into()), Value::Int(4));
    row_dict.insert(Value::Str("step".into()), Value::Int(1));
    let mut row_index = PyObject::new("pandas.core.indexes.range", "RangeIndex");
    row_index.args = vec![Value::Dict(row_dict)];

    let mut mgr = PyObject::new("pandas.core.internals.managers", "BlockManager");
    mgr.args = vec![
        Value::Tuple(vec![block(ints, 0, 1), block(floats, 1, 2)]),
        Value::Tuple(vec![Value::Object(col_index), Value::Object(row_index)]),
    ];

    let mut state = Dict::new();
    state.insert(Value::Str("_mgr".into()), Value::Object(mgr));
    let mut df = PyObject::new("pandas", "DataFrame");
    df.state = Some(Box::new(Value::Dict(state)));
    Value::Object(df)
}

#[test]
fn a_pandas_block_manager_becomes_a_frame() {
    let f = pandas::to_frame(&fake_pandas()).unwrap().expect("should convert");
    assert_eq!(f.names(), vec!["a", "b"]);
    assert_eq!(f.height(), 4);
    assert_eq!(f.column("a").unwrap().values.dtype(), DType::I64);
    assert_eq!(f.column("b").unwrap().values.dtype(), DType::F64);
    assert_eq!(f.column("a").unwrap().values.as_i64().unwrap().as_slice().unwrap(), &[1, 2, 3, 4]);
}

#[test]
fn the_range_index_is_recovered() {
    let f = pandas::to_frame(&fake_pandas()).unwrap().unwrap();
    let idx = f.index.as_ref().expect("a RangeIndex should be materialised");
    assert_eq!(idx.values.as_i64().unwrap().as_slice().unwrap(), &[0, 1, 2, 3]);
}

#[test]
fn a_non_pandas_value_is_left_alone() {
    assert!(pandas::to_frame(&Value::Array(f32_2x3())).unwrap().is_none());
    let mut o = PyObject::new("sklearn.linear_model", "LinearRegression");
    o.state = Some(Box::new(Value::None));
    assert!(pandas::to_frame(&Value::Object(o)).unwrap().is_none());
}

#[test]
fn an_unrecognised_pandas_layout_is_kept_not_guessed() {
    // A pandas object with no manager must return None rather than a wrong frame.
    let mut df = PyObject::new("pandas", "DataFrame");
    df.state = Some(Box::new(Value::Dict(Dict::new())));
    assert!(pandas::to_frame(&Value::Object(df)).unwrap().is_none());
}

#[test]
fn frames_round_trip_through_a_record_array() {
    let t = Tmp::new("frame-npy");
    let p = t.join("f.npy");
    let f = pandas::to_frame(&fake_pandas()).unwrap().unwrap();
    write(&p, &Value::Frame(f)).unwrap();
    let back = read(&p).unwrap();
    let arrays = flatten_arrays(&back);
    assert_eq!(arrays.len(), 2, "both columns should survive");
}

#[test]
fn a_series_carries_nulls_separately_from_its_values() {
    let mut s = Series::new("x", i64_4());
    assert!(s.validity.is_none(), "no nulls by default");
    s.validity = Some(vec![true, false, true, true]);
    // The buffer stays dense, so it can still be written straight to numpy.
    assert_eq!(s.values.len(), 4);
    assert_eq!(s.validity.as_ref().unwrap().iter().filter(|v| !**v).count(), 1);
}
