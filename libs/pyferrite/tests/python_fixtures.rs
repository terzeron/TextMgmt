//! Files written by real Python libraries (pandas 3.0, scikit-learn 1.9), kept
//! small and checked in under `tests/fixtures`. They pin down layouts that
//! hand-built values in the other tests cannot reproduce.

use pyferrite::interop::pandas;
use pyferrite::prelude::*;

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn pandas3_frame_keeps_column_names_stored_as_arrow_strings() {
    // pandas 3 pickles column labels as an ArrowStringArray (pyarrow buffers)
    // and block placements as `slice` objects.
    let v = read(fixture("pandas3_frame.pkl")).unwrap();
    let f = pandas::to_frame(&v).unwrap().expect("a DataFrame");
    let names: Vec<&str> = f.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["t", "sin", "label"]);
    let t = f.columns[0].values.as_f64().expect("float64 column");
    assert_eq!(t.iter().copied().collect::<Vec<_>>(), [0.0, 1.0, 2.0, 3.0]);
    match &f.columns[2].values {
        Array::Str(s) => {
            assert_eq!(s.iter().cloned().collect::<Vec<_>>(), ["a", "bb", "ccc", "dddd"])
        }
        other => panic!("string column decoded as {:?}", other.dtype()),
    }
}

#[test]
fn a_fitted_sklearn_estimator_is_captured_inertly_with_its_arrays() {
    // Classes outside the allow-list are not run: they come back as inert
    // descriptions, so the fitted arrays inside remain readable.
    let v = read(fixture("sklearn_knn.pkl")).unwrap();
    let o = match &v {
        Value::Object(o) => o,
        other => panic!("expected an inert object, got {}", other.type_name()),
    };
    assert_eq!(o.name, "KNeighborsClassifier");
    let state = o.state.as_deref().and_then(|s| s.as_dict()).expect("estimator state");
    let x = state.get("_fit_X").and_then(|x| x.as_array()).expect("_fit_X");
    assert_eq!(x.shape(), [6, 2]);
    // The KD-tree inside keeps its nodes in a structured numpy array.
    assert!(state.get("_tree").is_some());
}

#[test]
fn a_joblib_estimator_with_out_of_band_record_arrays_is_read() {
    // joblib stores arrays after the pickle stream, including the KD-tree's
    // structured node table, which takes a separate decoding path.
    let v = read(fixture("sklearn_knn.joblib")).unwrap();
    let state = match &v {
        Value::Object(o) => o.state.as_deref().and_then(|s| s.as_dict()).expect("state"),
        other => panic!("expected an inert object, got {}", other.type_name()),
    };
    assert_eq!(state.get("_fit_X").and_then(|x| x.as_array()).unwrap().shape(), [6, 2]);
}

#[test]
fn a_pickle_that_would_call_os_system_is_refused() {
    let p = fixture("malicious_os_system.pkl");
    let e = read(&p).unwrap_err();
    assert!(format!("{e}").contains("os"), "the message should name the callable: {e}");
    // Even when allowed, nothing is executed: the call is kept as data.
    let v = read_with(&p, &ReadOptions::new().allow_unknown_globals(true)).unwrap();
    let hook = v.as_dict().and_then(|d| d.get("hook")).expect("hook");
    assert!(matches!(hook, Value::Object(o) if o.name == "system"));
}

#[test]
fn a_pickled_record_array_becomes_a_frame() {
    let v = read(fixture("record_array.pkl")).unwrap();
    let rec = v.as_dict().and_then(|d| d.get("rec")).expect("rec");
    let f = rec.as_frame().expect("a structured array decodes as a Frame");
    let names: Vec<&str> = f.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "v"]);
    assert_eq!(
        f.columns[1].values.as_f64().unwrap().iter().copied().collect::<Vec<_>>(),
        [2.5, -1.0]
    );
}

#[test]
fn numpy_scalar_pickles_preserve_numeric_and_text_values() {
    let values = read(fixture("numpy_scalars.pkl")).unwrap();
    let Value::Tuple(values) = values else { panic!("expected a tuple of numpy scalars") };
    assert_eq!(values.len(), 15);
    assert!(matches!(values[0], Value::Bool(true)));
    assert_eq!(
        values[1..9].iter().map(Value::as_i64).collect::<Vec<_>>(),
        [
            Some(-8),
            Some(-1600),
            Some(-32000),
            Some(-64000),
            Some(250),
            Some(65000),
            Some(4_000_000_000),
            Some(1_234_567_890_123)
        ]
    );
    assert_eq!(values[9].as_f64(), Some(1.5));
    assert_eq!(values[10].as_f64(), Some(-2.25));
    assert_eq!(values[11].as_f64(), Some(3.125));
    let Value::Complex(c64) = values[12] else { panic!("expected complex64") };
    assert_eq!((c64.re, c64.im), (1.25, -2.5));
    let Value::Complex(c128) = values[13] else { panic!("expected complex128") };
    assert_eq!((c128.re, c128.im), (-3.5, 4.75));
    assert_eq!(values[14].as_str(), Some("한글"));
}

#[test]
fn numpy_array_pickles_cover_buffer_layouts_records_and_object_values() {
    let Value::Tuple(values) = read(fixture("numpy_arrays.pkl")).unwrap() else {
        panic!("expected a tuple of arrays")
    };
    let Array::I16(fortran) = values[0].as_array().unwrap() else { panic!("expected int16 array") };
    assert_eq!(fortran.shape(), [2, 2]);
    assert_eq!(fortran.iter().copied().collect::<Vec<_>>(), [1, 2, 3, 4]);

    let Value::List(objects) = &values[1] else { panic!("expected inert object array values") };
    assert_eq!(objects, &[Value::Int(1), Value::Str("x".into())]);

    let frame = values[2].as_frame().expect("structured numpy array becomes a Frame");
    assert_eq!(frame.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "score"]);
    let Array::I32(ids) = &frame.columns[0].values else { panic!("expected int32 record field") };
    assert_eq!(ids.iter().copied().collect::<Vec<_>>(), [7, 8]);

    let empty = values[3].as_array().expect("empty numpy array");
    assert_eq!(empty.shape(), [0, 2]);
    assert!(empty.is_empty());
}

#[test]
fn aligned_numpy_structured_dtype_is_rejected() {
    let error = read(fixture("numpy_aligned_record.pkl")).unwrap_err();
    assert!(matches!(error, pyferrite::Error::Unsupported(_)), "got {error}");
}

#[test]
fn h5py_booleans_come_back_as_bool() {
    // h5py writes numpy bool as an HDF5 enum {FALSE = 0, TRUE = 1}.
    let v = read(fixture("h5py_bool.h5")).unwrap();
    let d = v.as_dict().expect("root group");
    match d.get("flags").and_then(|x| x.as_array()) {
        Some(Array::Bool(b)) => {
            assert_eq!(b.iter().copied().collect::<Vec<_>>(), [true, false, true, true])
        }
        other => panic!("flags decoded as {:?}", other.map(|a| a.dtype())),
    }
    // a plain uint8 dataset is not mistaken for a boolean
    assert_eq!(d.get("small").and_then(|x| x.as_array()).unwrap().dtype(), DType::U8);
}
