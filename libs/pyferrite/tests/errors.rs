//! Validation and refusal: the library must fail loudly, never silently.

mod common;
use common::*;
use pyferrite::prelude::*;

#[test]
fn a_string_cannot_be_written_as_an_array() {
    let t = Tmp::new("err-str");
    let e = write(t.join("x.npy"), &Value::Str("hello".into())).unwrap_err();
    assert!(matches!(e, Error::InvalidArgument(_)), "got {e}");
    assert!(format!("{e}").contains("npy"), "the message should name the format: {e}");
}

#[test]
fn a_heterogeneous_list_cannot_become_a_numeric_array() {
    let t = Tmp::new("err-mixed");
    let mixed = Value::List(vec![Value::Int(1), Value::Str("two".into())]);
    assert!(write(t.join("x.npy"), &mixed).is_err());
}

#[test]
fn an_unknown_extension_is_reported_not_guessed() {
    let t = Tmp::new("err-ext");
    let e = write(t.join("x.wat"), &Value::Array(f32_2x3())).unwrap_err();
    assert!(format!("{e}").contains("format"), "got {e}");
}

#[test]
fn truncated_files_do_not_panic() {
    let t = Tmp::new("err-trunc");
    let p = t.join("x.npy");
    write(&p, &Value::Array(f32_2x3())).unwrap();
    let full = std::fs::read(&p).unwrap();
    for cut in [1, 8, 40, full.len() - 4] {
        let q = t.join(&format!("cut{cut}.npy"));
        std::fs::write(&q, &full[..cut]).unwrap();
        assert!(read(&q).is_err(), "a {cut}-byte file must be an error, not a panic");
    }
}

#[test]
fn garbage_is_rejected_by_every_reader() {
    let t = Tmp::new("err-garbage");
    for ext in ["npy", "npz", "pkl", "h5", "pt", "joblib"] {
        let p = t.join(&format!("g.{ext}"));
        std::fs::write(&p, b"this is not a machine learning artefact at all").unwrap();
        assert!(read(&p).is_err(), "{ext} accepted garbage");
    }
}

#[test]
fn max_alloc_caps_a_hostile_header() {
    let t = Tmp::new("err-alloc");
    let p = t.join("x.npy");
    write(&p, &Value::Array(f32_2x3())).unwrap();
    let e = read_with(&p, &ReadOptions::new().max_alloc(8)).unwrap_err();
    assert!(matches!(e, Error::InvalidArgument(_)), "got {e}");
}

#[test]
fn dangerous_pickle_globals_are_refused_by_default() {
    // A hand-built pickle calling os.system must never be honoured.
    let t = Tmp::new("err-evil");
    let p = t.join("evil.pkl");
    let mut b = vec![0x80, 0x02]; // PROTO 2
    b.extend_from_slice(b"cos\nsystem\n"); // GLOBAL os system
    b.extend_from_slice(b"("); // MARK
    b.extend_from_slice(b"X\x02\x00\x00\x00ls"); // BINUNICODE "ls"
    b.extend_from_slice(b"tR."); // TUPLE, REDUCE, STOP
    std::fs::write(&p, &b).unwrap();
    let e = read(&p).unwrap_err();
    assert!(format!("{e}").to_lowercase().contains("os"), "got {e}");
}

#[test]
fn unknown_globals_are_inert_not_executed() {
    let t = Tmp::new("err-unknown");
    let p = t.join("obj.pkl");
    let mut b = vec![0x80, 0x02];
    b.extend_from_slice(b"cmy.package\nMyModel\n");
    b.extend_from_slice(b")R."); // EMPTY_TUPLE, REDUCE, STOP
    std::fs::write(&p, &b).unwrap();
    let v = read_with(&p, &ReadOptions::new().allow_unknown_globals(true)).unwrap();
    let o = v.as_object().expect("an unknown class becomes an inert PyObject");
    assert_eq!(o.qualname(), "my.package.MyModel");
}

#[test]
fn hdf5_refuses_a_dtype_it_cannot_express() {
    let t = Tmp::new("err-h5dt");
    let mut d = Dict::new();
    d.insert(Value::Str("z".into()), Value::Array(c128_2()));
    let e = write(t.join("x.h5"), &Value::Dict(d)).unwrap_err();
    assert!(matches!(e, Error::Unsupported(_)), "got {e}");
}

#[test]
fn hdf5_rejects_an_illegal_link_name() {
    let t = Tmp::new("err-h5name");
    let mut d = Dict::new();
    d.insert(Value::Str("a/b".into()), Value::Array(f32_2x3()));
    assert!(write(t.join("x.h5"), &Value::Dict(d)).is_err());
}

#[test]
fn errors_carry_a_readable_message() {
    let t = Tmp::new("err-msg");
    let e = read(t.join("nope.npy")).unwrap_err();
    let s = format!("{e}");
    assert!(!s.is_empty() && !s.contains("Error {"), "unhelpful message: {s}");
}

#[test]
fn a_failed_npz_write_leaves_no_partial_file() {
    let t = Tmp::new("err-npz-partial");
    let p = t.join("x.npz");
    let mut d = Dict::new();
    d.insert(Value::Str("ok".into()), Value::Array(f32_2x3()));
    let big = ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&[2]), vec![1i64, 1000]).unwrap();
    d.insert(Value::Str("bad".into()), Value::Array(Array::I64(big)));
    // 1000 does not fit in i8, and the default policy is strict.
    let opts = WriteOptions::new().int_as(IntKind::I8);
    assert!(write_with(&p, &Value::Dict(d), &opts).is_err());
    assert!(!p.exists(), "nothing may be written when a member is rejected");
}

#[test]
fn a_header_promising_more_data_than_the_file_holds_is_rejected_before_allocating() {
    let t = Tmp::new("err-npy-lie");
    let p = t.join("lie.npy");
    // A valid header claiming 2^30 float64 values (8 GiB), followed by 8 bytes.
    let dict = "{'descr': '<f8', 'fortran_order': False, 'shape': (1073741824,), }";
    let mut header = dict.to_string();
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
    bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(&[0u8; 8]);
    std::fs::write(&p, &bytes).unwrap();
    let e = read(&p).unwrap_err();
    assert!(format!("{e}").contains("truncated"), "got {e}");
}
