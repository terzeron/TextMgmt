//! Container formats: the same dict must survive every one of them.

mod common;
use common::*;
use pyferrite::prelude::*;

fn container_roundtrip(ext: &str) {
    let t = Tmp::new(&format!("fmt-{ext}"));
    let p = t.join(&format!("m.{ext}"));
    let src = sample_dict();
    write(&p, &src).unwrap();
    let back = read(&p).unwrap();
    let (a, b) = (arrays(&src), arrays(&back));
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>(), "{ext}: key set");
    for (k, v) in &a {
        assert_same(v, &b[k], &format!("{ext}/{k}"));
    }
}

#[test]
fn npz_roundtrip() {
    container_roundtrip("npz");
}

#[test]
fn pickle_roundtrip() {
    container_roundtrip("pkl");
}

#[test]
fn joblib_roundtrip() {
    container_roundtrip("joblib");
}

#[test]
fn joblib_writer_handles_non_arrays_compression_and_numeric_casts() {
    let t = Tmp::new("joblib-write-options");
    for (name, compression) in
        [("deflate", Compression::Deflate(0)), ("zlib", Compression::Zlib(9))]
    {
        let path = t.join(&format!("{name}.joblib"));
        let src = Value::List(vec![Value::Int(-7), Value::Str("text".into()), Value::None]);
        write_with(&path, &src, &WriteOptions::new().compression(compression)).unwrap();
        assert_eq!(read(&path).unwrap(), src);
    }

    let unchanged_path = t.join("unchanged.joblib");
    let unchanged = Value::Array(Array::I32(ndarray::arr1(&[-3, 0, 9]).into_dyn()));
    write(&unchanged_path, &unchanged).unwrap();
    assert_eq!(read(&unchanged_path).unwrap(), unchanged);

    let path = t.join("cast.joblib");
    let src = Value::Array(Array::F64(ndarray::arr1(&[1.25f64, 2.5]).into_dyn()));
    write_with(
        &path,
        &src,
        &WriteOptions::new().float_as(FloatKind::F32).compression(Compression::zlib()),
    )
    .unwrap();
    let decoded = read(&path).unwrap();
    assert_eq!(decoded.as_array().unwrap().dtype(), DType::F32);
    assert_eq!(
        decoded.as_array().unwrap().as_f32().unwrap().iter().copied().collect::<Vec<_>>(),
        [1.25, 2.5]
    );
}

#[test]
fn hdf5_roundtrip() {
    container_roundtrip("h5");
}

#[test]
fn torch_roundtrip() {
    container_roundtrip("pt");
}

#[test]
fn nested_dicts_survive_every_container() {
    for ext in ["npz", "pkl", "h5", "pt", "joblib"] {
        let t = Tmp::new(&format!("nest-{ext}"));
        let p = t.join(&format!("n.{ext}"));
        let mut inner = Dict::new();
        inner.insert(Value::Str("weight".into()), Value::Array(f32_2x3()));
        inner.insert(Value::Str("bias".into()), Value::Array(i64_4()));
        let mut outer = Dict::new();
        outer.insert(Value::Str("layer1".into()), Value::Dict(inner));
        outer.insert(Value::Str("head".into()), Value::Array(u8_5()));
        let src = Value::Dict(outer);
        write(&p, &src).unwrap();
        let back = read(&p).unwrap();
        let (a, b) = (arrays(&src), arrays(&back));
        assert_eq!(a.len(), b.len(), "{ext}: array count");
        for (k, v) in &a {
            assert_same(v, b.get(k).unwrap_or_else(|| panic!("{ext}: missing {k}")), k);
        }
    }
}

#[test]
#[cfg(feature = "deflate")]
fn compression_does_not_change_the_data() {
    let t = Tmp::new("compress");
    let src = sample_dict();
    let plain = t.join("plain.npz");
    let squashed = t.join("squashed.npz");
    write(&plain, &src).unwrap();
    write_with(&squashed, &src, &WriteOptions::new().compression(Compression::deflate())).unwrap();
    assert!(std::fs::metadata(&squashed).unwrap().len() > 0);
    let (a, b) = (arrays(&read(&plain).unwrap()), arrays(&read(&squashed).unwrap()));
    for (k, v) in &a {
        assert_same(v, &b[k], k);
    }
}

#[test]
fn python_scalars_and_containers_survive_a_pickle() {
    let t = Tmp::new("pyvals");
    let p = t.join("v.pkl");
    let mut d = Dict::new();
    d.insert(Value::Str("s".into()), Value::Str("héllo wörld".into()));
    d.insert(Value::Str("i".into()), Value::Int(-9_007_199_254_740_993));
    d.insert(Value::Str("f".into()), Value::Float(0.1 + 0.2));
    d.insert(Value::Str("t".into()), Value::Bool(true));
    d.insert(Value::Str("n".into()), Value::None);
    d.insert(Value::Str("by".into()), Value::Bytes(vec![0, 255, 17]));
    d.insert(
        Value::Str("l".into()),
        Value::List(vec![Value::Int(1), Value::Str("two".into()), Value::None]),
    );
    d.insert(Value::Str("tp".into()), Value::Tuple(vec![Value::Float(1.5), Value::Bool(false)]));
    let src = Value::Dict(d);
    write(&p, &src).unwrap();
    let back = read(&p).unwrap();
    let bd = back.as_dict().unwrap();
    assert_eq!(bd.get("s").and_then(|v| v.as_str()), Some("héllo wörld"));
    assert_eq!(bd.get("i").and_then(|v| v.as_i64()), Some(-9_007_199_254_740_993));
    assert_eq!(bd.get("f").and_then(|v| v.as_f64()), Some(0.1 + 0.2));
    assert!(matches!(bd.get("n"), Some(Value::None)));
    assert!(matches!(bd.get("by"), Some(Value::Bytes(b)) if b == &[0, 255, 17]));
    assert!(matches!(bd.get("l"), Some(Value::List(l)) if l.len() == 3));
    assert!(matches!(bd.get("tp"), Some(Value::Tuple(l)) if l.len() == 2));
}

#[test]
fn format_can_be_forced_against_the_extension() {
    let t = Tmp::new("forced");
    let p = t.join("mystery.bin");
    let src = sample_dict();
    write_with(&p, &src, &WriteOptions::new().format(Format::Npz)).unwrap();
    // Detection works from the magic bytes even though the name lies.
    let back = read(&p).unwrap();
    assert_eq!(arrays(&back).len(), 4);
}

#[test]
fn values_stay_mutable_after_reading() {
    // The whole point of returning owned data: rescale weights before use.
    let t = Tmp::new("mutable");
    let p = t.join("m.npz");
    write(&p, &sample_dict()).unwrap();
    let mut v = read(&p).unwrap();
    for a in v.arrays_mut() {
        if let Some(f) = a.as_f32_mut() {
            f.iter_mut().for_each(|x| *x *= 2.0);
        }
    }
    let w = v.as_dict().unwrap().get("w").unwrap().as_array().unwrap().as_f32().unwrap();
    assert_eq!(w.as_slice().unwrap()[0], 2.0);
}
