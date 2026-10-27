use super::*;
use crate::dtype::DType;
use crate::formats::hdf5::write::write_bytes;
use crate::options::WriteOptions;
use crate::value::Array;
use ndarray::arr1;

fn array_value() -> Value {
    Value::Array(Array::I16(arr1(&[7, 8, 9]).into_dyn()))
}

fn encoded(value: &Value) -> Vec<u8> {
    write_bytes(value, &WriteOptions::new()).unwrap()
}

#[test]
fn read_and_index_round_trip_nested_dataset_and_report_format() {
    let mut nested = Dict::new();
    nested.insert(Value::Str("leaf".into()), array_value());
    let mut root = Dict::new();
    root.insert(Value::Str("nested".into()), Value::Dict(nested));
    let bytes = encoded(&Value::Dict(root));

    assert_eq!(format_version(&bytes).unwrap(), 0);
    let Value::Dict(decoded) = read_bytes(&bytes, &ReadOptions::new()).unwrap() else {
        panic!("root must decode as a group")
    };
    assert_eq!(
        decoded
            .get("nested")
            .unwrap()
            .as_dict()
            .unwrap()
            .get("leaf")
            .unwrap()
            .as_array()
            .unwrap()
            .shape(),
        [3]
    );
    let entries = index(&bytes).unwrap();
    assert_eq!(entries, [("nested/leaf".into(), vec![3], DType::I16, entries[0].3)]);
    assert!(read_bytes(b"not hdf5", &ReadOptions::new()).is_err());
    assert!(format_version(b"not hdf5").is_err());
}

#[test]
fn max_alloc_rejects_a_dataset_before_materializing_it() {
    let mut root = Dict::new();
    root.insert(Value::Str("data".into()), array_value());
    let bytes = encoded(&Value::Dict(root));
    let err = read_bytes(&bytes, &ReadOptions::new().max_alloc(5)).unwrap_err();
    assert!(err.to_string().contains("max_alloc"));
}

#[test]
fn empty_groups_and_unknown_root_messages_are_distinguished() {
    let empty = encoded(&Value::Dict(Dict::new()));
    assert!(is_group(&objhdr::parse(&empty, &Superblock::parse(&empty).unwrap(), 96).unwrap()));
    assert!(
        matches!(read_bytes(&empty, &ReadOptions::new()).unwrap(), Value::Dict(d) if d.is_empty())
    );

    let mut unknown = empty;
    let root = Superblock::parse(&unknown).unwrap().root_address as usize;
    unknown[root + 16..root + 18].copy_from_slice(&99u16.to_le_bytes());
    let error = read_bytes(&unknown, &ReadOptions::new()).unwrap_err();
    assert!(error.to_string().contains("neither a dataset nor a group"));
    assert!(!is_group(&[objhdr::Message { kind: 99, data: vec![] }]));
    for kind in [
        super::super::msg::MSG_LINK_INFO,
        super::super::msg::MSG_LINK,
        super::super::msg::MSG_SYMBOL_TABLE,
    ] {
        assert!(is_group(&[objhdr::Message { kind, data: vec![] }]));
    }
}

#[test]
fn undefined_links_are_skipped_by_read_and_index() {
    let mut root = Dict::new();
    root.insert(Value::Str("gone".into()), array_value());
    let mut bytes = encoded(&Value::Dict(root));
    let sb = Superblock::parse(&bytes).unwrap();
    let msgs = objhdr::parse(&bytes, &sb, sb.root_address).unwrap();
    let symbol = msgs.iter().find(|m| m.kind == super::super::msg::MSG_SYMBOL_TABLE).unwrap();
    let btree = u64::from_le_bytes(symbol.data[..8].try_into().unwrap()) as usize;
    let snod = u64::from_le_bytes(bytes[btree + 32..btree + 40].try_into().unwrap()) as usize;
    bytes[snod + 16..snod + 24].copy_from_slice(&u64::MAX.to_le_bytes());

    assert!(
        matches!(read_bytes(&bytes, &ReadOptions::new()).unwrap(), Value::Dict(d) if d.is_empty())
    );
    assert!(index(&bytes).unwrap().is_empty());
}

#[test]
fn deep_group_walk_stops_at_the_safety_limit() {
    let mut value = array_value();
    for _ in 0..66 {
        let mut dict = Dict::new();
        dict.insert(Value::Str("x".into()), value);
        value = Value::Dict(dict);
    }
    let bytes = encoded(&value);
    assert!(matches!(read_bytes(&bytes, &ReadOptions::new()).unwrap(), Value::Dict(_)));
    assert!(index(&bytes).unwrap().is_empty());
}
