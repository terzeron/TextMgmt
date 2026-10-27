use super::*;
use crate::dtype::DType;
use crate::value::{Array, Dict};
use ndarray::arr1;

fn array() -> Value {
    Value::Array(Array::I16(arr1(&[10, 20, 30]).into_dyn()))
}

#[test]
fn array_plan_assigns_header_data_and_requested_shape() {
    let value = array();
    let (node, eof) = plan(&value, &WriteOptions::new().shape([1, 3])).unwrap();
    let Node::Dataset(dataset) = node else { panic!("array should plan as a dataset") };
    assert_eq!(dataset.shape, [1, 3]);
    assert_eq!(dataset.header, SUPERBLOCK as u64);
    assert_eq!(dataset.data_size, 6);
    assert_eq!(dataset.data % 8, 0);
    assert_eq!(eof as usize, dataset.data as usize + 8);
}

#[test]
fn array_plan_rejects_shape_and_unsupported_target_dtype() {
    let value = array();
    assert!(matches!(plan(&value, &WriteOptions::new().shape([2, 2])), Err(Error::Shape { .. })));
    assert!(plan(&value, &WriteOptions::new().dtype(DType::Object)).is_err());
}

#[test]
fn dictionary_plan_sorts_names_and_rejects_invalid_keys() {
    let mut dict = Dict::new();
    dict.insert(Value::Str("z".into()), array());
    dict.insert(Value::Str("a".into()), array());
    let value = Value::Dict(dict);
    let (node, _) = plan(&value, &WriteOptions::new()).unwrap();
    let Node::Group(group) = node else { panic!("dict should plan as a group") };
    assert_eq!(
        group.children.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
        ["a", "z"]
    );

    for key in [
        Value::Int(1),
        Value::Str(String::new()),
        Value::Str("a/b".into()),
        Value::Str("a\0b".into()),
    ] {
        let mut dict = Dict::new();
        dict.insert(key, array());
        assert!(plan(&Value::Dict(dict), &WriteOptions::new()).is_err());
    }
}

#[test]
fn lists_and_tuples_plan_as_groups_and_scalars_are_unsupported() {
    for value in [Value::List(vec![array()]), Value::Tuple(vec![array()])] {
        let (node, _) = plan(&value, &WriteOptions::new()).unwrap();
        let Node::Group(group) = node else { panic!("sequence should plan as a group") };
        assert_eq!(group.children[0].0, "item_0");
    }
    assert!(plan(&Value::Int(1), &WriteOptions::new()).is_err());
}
