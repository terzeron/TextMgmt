use super::*;

fn push_addr(buf: &mut Vec<u8>, width: usize, value: u64) {
    buf.extend(&value.to_le_bytes()[..width]);
}

fn legacy(version: u8) -> Vec<u8> {
    let mut buf = MAGIC.to_vec();
    buf.push(version);
    buf.extend([0; 3]);
    if version == 1 {
        buf.push(0);
    }
    buf.extend([0, 8, 4, 0]);
    buf.extend([0; 4]);
    if version == 1 {
        buf.extend([0; 4]);
    }
    buf.extend([0; 4]);
    for value in [11, 0, 200, 0, 0, 55] {
        push_addr(&mut buf, 8, value);
    }
    buf
}

fn modern(version: u8) -> Vec<u8> {
    let mut buf = MAGIC.to_vec();
    buf.extend([version, 4, 2, 0]);
    for value in [10, 0, 128, 42] {
        push_addr(&mut buf, 4, value);
    }
    buf
}

#[test]
fn parses_all_supported_superblock_versions_and_address_widths() {
    for version in [0, 1] {
        let sb = Superblock::parse(&legacy(version)).unwrap();
        assert_eq!(sb.version, version);
        assert_eq!((sb.offset_size, sb.length_size), (8, 4));
        assert_eq!((sb.base_address, sb.eof_address, sb.root_address), (11, 200, 55));
    }

    for version in [2, 3] {
        let sb = Superblock::parse(&modern(version)).unwrap();
        assert_eq!(sb.version, version);
        assert_eq!((sb.offset_size, sb.length_size), (4, 2));
        assert_eq!((sb.base_address, sb.eof_address, sb.root_address), (10, 128, 42));
    }
}

#[test]
fn rejects_missing_signature_unsupported_versions_and_truncated_fields() {
    assert!(Superblock::parse(b"not HDF5").is_err());
    let mut unsupported = MAGIC.to_vec();
    unsupported.push(4);
    assert!(Superblock::parse(&unsupported).unwrap_err().to_string().contains("version 4"));
    assert!(Superblock::parse(MAGIC).is_err());
    assert!(Superblock::parse(&legacy(1)[..20]).is_err());
    assert!(Superblock::parse(&modern(3)[..18]).is_err());
}

#[test]
fn address_helpers_apply_base_eof_and_undefined_rules() {
    let sb = Superblock {
        version: 3,
        offset_size: 2,
        length_size: 2,
        base_address: 10,
        root_address: 1,
        eof_address: 12,
    };
    assert_eq!(sb.at(1), 11);
    assert!(sb.in_bounds(1));
    assert!(!sb.in_bounds(2));
    assert!(sb.undefined(0xffff));
    assert_eq!(sb.format_version(), 3);

    let wide = Superblock { offset_size: 8, ..sb };
    assert!(wide.undefined(u64::MAX));
    assert!(!wide.undefined(0xffff));
}
