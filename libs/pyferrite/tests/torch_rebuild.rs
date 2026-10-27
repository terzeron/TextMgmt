//! Torch pickle reconstruction through the public pickle reader.

use pyferrite::{
    dtype::DType,
    formats::pickle::{read_bytes_with, storage_dtype, storage_value},
    prelude::*,
};

fn append_global(out: &mut Vec<u8>, module: &str, name: &str) {
    out.push(b'c');
    out.extend_from_slice(module.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(name.as_bytes());
    out.push(b'\n');
}

fn append_text(out: &mut Vec<u8>, text: &str) {
    out.push(b'X');
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn append_int(out: &mut Vec<u8>, n: i32) {
    out.push(b'J');
    out.extend_from_slice(&n.to_le_bytes());
}

fn tensor_pickle(offset: i32, shape: &[i32], strides: &[i32]) -> Vec<u8> {
    let mut out = vec![0x80, 0x02]; // PROTO 2
    append_global(&mut out, "torch._utils", "_rebuild_tensor_v2");
    out.push(b'('); // MARK: reducer args
    out.push(b'('); // MARK: persistent storage id
    append_text(&mut out, "storage");
    append_global(&mut out, "torch", "IntStorage");
    append_text(&mut out, "0");
    append_text(&mut out, "cpu");
    append_int(&mut out, 6);
    out.push(b't'); // TUPLE
    out.push(b'Q'); // BINPERSID
    append_int(&mut out, offset);
    out.push(b'('); // MARK: shape
    for n in shape {
        append_int(&mut out, *n);
    }
    out.push(b't');
    out.push(b'('); // MARK: stride
    for n in strides {
        append_int(&mut out, *n);
    }
    out.push(b't');
    out.push(0x89); // NEWFALSE
    append_global(&mut out, "collections", "OrderedDict");
    out.push(b')'); // EMPTY_TUPLE
    out.push(b'R'); // REDUCE OrderedDict
    out.push(b't'); // TUPLE reducer args
    out.push(b'R'); // REDUCE tensor
    out.push(b'.'); // STOP
    out
}

fn storage_bytes() -> Vec<u8> {
    [10i32, 20, 30, 40, 50, 60].into_iter().flat_map(i32::to_le_bytes).collect()
}

fn read_tensor(
    offset: i32,
    shape: &[i32],
    strides: &[i32],
    options: &ReadOptions,
) -> Result<Value> {
    let pickle = tensor_pickle(offset, shape, strides);
    read_bytes_with(&pickle, options, &mut |_| Ok(storage_value(DType::I32, storage_bytes())))
}

#[test]
fn torch_storage_classes_map_to_supported_dtypes() {
    for (name, dtype) in [
        ("FloatStorage", DType::F32),
        ("TypedFloatStorage", DType::F32),
        ("DoubleStorage", DType::F64),
        ("HalfStorage", DType::F16),
        ("BFloat16Storage", DType::BF16),
        ("LongStorage", DType::I64),
        ("IntStorage", DType::I32),
        ("ShortStorage", DType::I16),
        ("CharStorage", DType::I8),
        ("ByteStorage", DType::U8),
        ("BoolStorage", DType::Bool),
        ("ComplexFloatStorage", DType::C64),
        ("ComplexDoubleStorage", DType::C128),
        ("Float8_e4m3fnStorage", DType::F8E4M3),
        ("Float8_e5m2Storage", DType::F8E5M2),
    ] {
        assert_eq!(storage_dtype(name), Some(dtype), "{name}");
    }
    assert_eq!(storage_dtype("UnknownStorage"), None);
    assert_eq!(storage_dtype("Float"), Some(DType::F32));
}

#[test]
fn torch_pickle_rebuilds_offset_and_strided_tensors() {
    let contiguous = read_tensor(1, &[2], &[1], &ReadOptions::new()).unwrap();
    assert_eq!(
        contiguous.as_array().unwrap().as_i32().unwrap().iter().copied().collect::<Vec<_>>(),
        [20, 30]
    );

    let strided = read_tensor(0, &[2, 2], &[3, 1], &ReadOptions::new()).unwrap();
    assert_eq!(
        strided.as_array().unwrap().as_i32().unwrap().iter().copied().collect::<Vec<_>>(),
        [10, 20, 40, 50]
    );
}

#[test]
fn torch_pickle_rejects_bad_storage_ranges_and_allocation_sizes() {
    assert!(read_tensor(6, &[1], &[1], &ReadOptions::new()).is_err());
    assert!(read_tensor(0, &[2], &[8], &ReadOptions::new()).is_err());
    assert!(read_tensor(0, &[2], &[1], &ReadOptions::new().max_alloc(4)).is_err());

    let pickle = tensor_pickle(0, &[1], &[1]);
    let error =
        read_bytes_with(&pickle, &ReadOptions::new(), &mut |_| Ok(Value::None)).unwrap_err();
    assert!(format!("{error}").contains("storage"));
}
