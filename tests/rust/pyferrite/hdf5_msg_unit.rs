use super::*;

fn superblock() -> Superblock {
    Superblock {
        version: 3,
        offset_size: 8,
        length_size: 8,
        base_address: 0,
        root_address: 0,
        eof_address: 4096,
    }
}

fn datatype_message(class: u8, flags: u8, members: u16, size: u32, tail: &[u8]) -> Vec<u8> {
    let mut data = vec![class, flags, members as u8, (members >> 8) as u8];
    data.extend(size.to_le_bytes());
    data.extend(tail);
    data
}

fn old_layout(version: u8, class: u8, dims: &[u32], payload: &[u8]) -> Vec<u8> {
    let mut data = vec![version, dims.len() as u8, class];
    data.extend([0; 5]);
    if class != 0 {
        data.extend(123u64.to_le_bytes());
    }
    for dim in dims {
        data.extend(dim.to_le_bytes());
    }
    if class == 0 {
        data.extend((payload.len() as u32).to_le_bytes());
        data.extend(payload);
    }
    data
}

#[test]
fn dataspace_reads_both_versions_and_reports_truncation() {
    let sb = superblock();
    assert_eq!(
        dataspace(&[1, 2, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0], &sb)
            .unwrap(),
        [3, 5]
    );
    assert_eq!(dataspace(&[2, 1, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0], &sb).unwrap(), [7]);
    assert!(dataspace(&[1, 1, 0], &sb).is_err());
}

#[test]
fn datatype_covers_numeric_opaque_enum_and_error_classes() {
    assert_eq!(
        datatype(&datatype_message(0, 0x08, 0, 1, &[])).unwrap(),
        (DType::I8, Endian::Little)
    );
    assert_eq!(datatype(&datatype_message(0, 0x08, 0, 2, &[])).unwrap().0, DType::I16);
    assert_eq!(datatype(&datatype_message(0, 0x08, 0, 4, &[])).unwrap().0, DType::I32);
    assert_eq!(datatype(&datatype_message(0, 0x08, 0, 8, &[])).unwrap().0, DType::I64);
    assert_eq!(datatype(&datatype_message(0, 0, 0, 1, &[])).unwrap().0, DType::U8);
    assert_eq!(datatype(&datatype_message(0, 1, 0, 2, &[])).unwrap(), (DType::U16, Endian::Big));
    assert_eq!(datatype(&datatype_message(0, 0, 0, 4, &[])).unwrap().0, DType::U32);
    assert_eq!(datatype(&datatype_message(0, 0, 0, 8, &[])).unwrap().0, DType::U64);
    assert_eq!(datatype(&datatype_message(1, 0, 0, 2, &[])).unwrap().0, DType::F16);
    assert_eq!(datatype(&datatype_message(1, 0, 0, 4, &[])).unwrap().0, DType::F32);
    assert_eq!(datatype(&datatype_message(1, 0, 0, 8, &[])).unwrap().0, DType::F64);
    assert_eq!(datatype(&datatype_message(1, 0, 0, 16, &[])).unwrap().0, DType::F128);
    assert_eq!(
        datatype(&datatype_message(3, 0, 0, 5, &[])).unwrap(),
        (DType::Bytes(5), Endian::Native)
    );
    assert_eq!(datatype(&datatype_message(4, 0, 0, 1, &[])).unwrap().0, DType::U8);

    let mut boolean = datatype_message(8, 2, 0, 1, &[]);
    boolean.extend(b"FALSE\0TRUE\0");
    assert_eq!(datatype(&boolean).unwrap(), (DType::Bool, Endian::Native));
    assert_eq!(datatype(&datatype_message(8, 0, 0, 1, &[])).unwrap().0, DType::U8);
    assert_eq!(datatype(&datatype_message(8, 0, 0, 2, &[])).unwrap().0, DType::U16);
    assert_eq!(datatype(&datatype_message(8, 0, 0, 4, &[])).unwrap().0, DType::U32);
    assert_eq!(datatype(&datatype_message(8, 0, 0, 8, &[])).unwrap().0, DType::U64);
    assert_eq!(datatype(&datatype_message(9, 0, 0, 4, &[])).unwrap().0, DType::Bytes(4));
    assert!(datatype(&datatype_message(0, 0, 0, 3, &[])).is_err());
    assert!(datatype(&datatype_message(1, 0, 0, 3, &[])).is_err());
    assert!(datatype(&datatype_message(15, 0, 0, 1, &[])).is_err());
    assert!(datatype(&[0]).is_err());
}

#[test]
fn layout_reads_legacy_compact_contiguous_and_chunked_messages() {
    let sb = superblock();
    assert!(
        matches!(layout(&old_layout(1, 0, &[], b"abc"), &sb).unwrap(), Layout::Compact(bytes) if bytes == b"abc")
    );
    assert!(matches!(
        layout(&old_layout(2, 1, &[3, 4], &[]), &sb).unwrap(),
        Layout::Contiguous { address: 123, size: 12 }
    ));
    assert!(
        matches!(layout(&old_layout(1, 2, &[3, 4], &[]), &sb).unwrap(), Layout::Chunked { address: 123, dims, elem_size: 4 } if dims == [3])
    );
}

#[test]
fn layout_reads_current_versions_and_rejects_unknown_classes() {
    let sb = superblock();
    assert!(
        matches!(layout(&[3, 0, 3, 0, b'x', b'y', b'z'], &sb).unwrap(), Layout::Compact(bytes) if bytes == b"xyz")
    );

    let mut contiguous = vec![4, 1];
    contiguous.extend(77u64.to_le_bytes());
    contiguous.extend(88u64.to_le_bytes());
    assert!(matches!(
        layout(&contiguous, &sb).unwrap(),
        Layout::Contiguous { address: 77, size: 88 }
    ));

    let mut chunked = vec![3, 2, 2];
    chunked.extend(99u64.to_le_bytes());
    chunked.extend(2u32.to_le_bytes());
    chunked.extend(5u32.to_le_bytes());
    assert!(
        matches!(layout(&chunked, &sb).unwrap(), Layout::Chunked { address: 99, dims, elem_size: 5 } if dims == [2])
    );

    assert!(layout(&[3, 9], &sb).is_err());
    assert!(layout(&[5], &sb).is_err());
    assert!(layout(&[3, 7], &sb).is_err());
}

#[test]
fn filters_reads_v1_names_v2_ids_and_filter_values() {
    let mut v1 = vec![1, 1];
    v1.extend([0; 6]);
    v1.extend(7u16.to_le_bytes());
    v1.extend(3u16.to_le_bytes());
    v1.extend(0u16.to_le_bytes());
    v1.extend(1u16.to_le_bytes());
    v1.extend(b"zip");
    v1.extend(6u32.to_le_bytes());
    v1.extend([0; 4]);
    assert_eq!(filters(&v1).unwrap(), [7]);

    let mut v2 = vec![2, 2];
    v2.extend(3u16.to_le_bytes());
    v2.extend(0u16.to_le_bytes());
    v2.extend(1u16.to_le_bytes());
    v2.extend(11u32.to_le_bytes());
    v2.extend(300u16.to_le_bytes());
    v2.extend(2u16.to_le_bytes());
    v2.extend(0u16.to_le_bytes());
    v2.extend(0u16.to_le_bytes());
    v2.extend(b"xy");
    assert_eq!(filters(&v2).unwrap(), [3, 300]);
    assert!(filters(&[1, 1]).is_err());
}
