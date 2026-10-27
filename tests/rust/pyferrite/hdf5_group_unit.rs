use super::*;

fn sb() -> Superblock {
    Superblock {
        version: 0,
        offset_size: 8,
        length_size: 8,
        base_address: 0,
        root_address: 0,
        eof_address: 4096,
    }
}

fn put_u64(buf: &mut Vec<u8>, value: u64) {
    buf.extend(value.to_le_bytes());
}

fn msg(kind: u16, data: &[u8]) -> Message {
    Message { kind, data: data.to_vec() }
}

#[test]
fn heap_string_handles_offsets_terminators_and_invalid_utf8() {
    assert_eq!(heap_string(b"one\0two", 4), "two");
    assert_eq!(heap_string(b"one", 9), "");
    assert_eq!(heap_string(b"a\xff\0", 0), "a\u{fffd}");
}

#[test]
fn local_heap_reads_its_data_segment_and_rejects_bad_signature() {
    let mut file = vec![0; 64];
    file[8..12].copy_from_slice(b"HEAP");
    file[16..24].copy_from_slice(&3u64.to_le_bytes());
    file[32..40].copy_from_slice(&48u64.to_le_bytes());
    file[48..51].copy_from_slice(b"a\0b");
    assert_eq!(local_heap(&file, &sb(), 8).unwrap(), b"a\0b");

    file[16..24].copy_from_slice(&16u64.to_le_bytes());
    file[32..40].copy_from_slice(&60u64.to_le_bytes());
    assert!(local_heap(&file, &sb(), 8).unwrap().is_empty());
    assert!(local_heap(&[0; 32], &sb(), 8).is_err());
}

#[test]
fn symbol_table_leaf_and_children_resolve_links() {
    let mut file = vec![0; 256];
    // Local heap header at 8; its string data starts at 48.
    file[8..12].copy_from_slice(b"HEAP");
    file[16..24].copy_from_slice(&6u64.to_le_bytes());
    file[32..40].copy_from_slice(&48u64.to_le_bytes());
    file[48..54].copy_from_slice(b"child\0");
    // Leaf B-tree at 64 points to symbol-table node at 128.
    file[64..68].copy_from_slice(b"TREE");
    file[70..72].copy_from_slice(&1u16.to_le_bytes());
    file[96..104].copy_from_slice(&128u64.to_le_bytes());
    file[128..132].copy_from_slice(b"SNOD");
    file[134..136].copy_from_slice(&1u16.to_le_bytes());
    file[136..144].copy_from_slice(&0u64.to_le_bytes());
    file[144..152].copy_from_slice(&777u64.to_le_bytes());

    let mut symbol = Vec::new();
    put_u64(&mut symbol, 64);
    put_u64(&mut symbol, 8);
    let children = children(&file, &sb(), &[msg(MSG_SYMBOL_TABLE, &symbol)]).unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "child");
    assert_eq!(children[0].address, 777);
}

#[test]
fn btree_walk_handles_undefined_internal_and_non_group_nodes() {
    let mut out = Vec::new();
    let undefined = u64::MAX;
    walk_btree(&[], &sb(), undefined, &[], &mut out).unwrap();

    let mut non_group = b"TREE".to_vec();
    non_group.extend([1, 0]);
    non_group.extend(0u16.to_le_bytes());
    non_group.extend([0; 16]);
    walk_btree(&non_group, &sb(), 0, &[], &mut out).unwrap();

    let mut bad = b"NOPE".to_vec();
    bad.resize(32, 0);
    assert!(walk_btree(&bad, &sb(), 0, &[], &mut out).is_err());
}

#[test]
fn btree_internal_nodes_recurse_to_symbol_table_leaves() {
    let mut file = vec![0; 192];
    // Root internal node points to a leaf node at file address 64.
    file[0..4].copy_from_slice(b"TREE");
    file[5] = 1;
    file[6..8].copy_from_slice(&1u16.to_le_bytes());
    file[24..32].copy_from_slice(&0u64.to_le_bytes());
    file[32..40].copy_from_slice(&64u64.to_le_bytes());
    // Leaf points to a symbol-table node at file address 120.
    file[64..68].copy_from_slice(b"TREE");
    file[70..72].copy_from_slice(&1u16.to_le_bytes());
    file[88..96].copy_from_slice(&0u64.to_le_bytes());
    file[96..104].copy_from_slice(&120u64.to_le_bytes());
    file[120..124].copy_from_slice(b"SNOD");
    file[126..128].copy_from_slice(&1u16.to_le_bytes());
    file[128..136].copy_from_slice(&0u64.to_le_bytes());
    file[136..144].copy_from_slice(&456u64.to_le_bytes());

    let mut links = Vec::new();
    walk_btree(&file, &sb(), 0, b"nested\0", &mut links).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!((links[0].name.as_str(), links[0].address), ("nested", 456));
}

#[test]
fn symbol_table_node_rejects_bad_signature_and_truncation() {
    let mut out = Vec::new();
    assert!(symbol_table_node(b"NOPE", &sb(), 0, &[], &mut out).is_err());
    assert!(symbol_table_node(b"SNOD\0\0\x01\0", &sb(), 0, &[], &mut out).is_err());
}

#[test]
fn link_parser_reads_flags_and_omits_soft_links() {
    let mut data = vec![0, 0x14]; // charset and creation-order fields
    data.push(1); // UTF-8 charset
    data.extend(9u64.to_le_bytes());
    data.push(4); // name length
    data.extend(b"name");
    put_u64(&mut data, 123);
    let link = parse_link(&data, &sb()).unwrap().unwrap();
    assert_eq!((link.name.as_str(), link.address), ("name", 123));

    let soft = [0, 0x08, 1, 1, b'x'];
    assert!(parse_link(&soft, &sb()).unwrap().is_none());
    assert!(parse_link(&[0], &sb()).is_err());
}

#[test]
fn children_accumulates_hard_links_and_ignores_other_messages() {
    let mut data = vec![0, 0];
    data.push(1);
    data.push(b'x');
    put_u64(&mut data, 42);
    let got = children(&[], &sb(), &[msg(MSG_LINK, &data), msg(99, &[])]).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].name.as_str(), got[0].address), ("x", 42));
}
