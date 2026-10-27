//! Reading and writing in bounded memory.

mod common;
use common::*;
use ndarray::{ArrayD, IxDyn};
use pyferrite::prelude::*;

fn ramp(n: usize) -> Array {
    Array::F32(ArrayD::from_shape_vec(IxDyn(&[n]), (0..n).map(|i| i as f32).collect()).unwrap())
}

#[test]
fn lazy_write_then_eager_read_matches_a_direct_write() {
    let t = Tmp::new("lazy-w");
    let p = t.join("big.npy");
    let mut w = write_lazy(&p, DType::F32, &[10, 25], &WriteOptions::new()).unwrap();
    assert_eq!(w.remaining(), 250);
    for c in 0..10 {
        let chunk = Array::F32(
            ArrayD::from_shape_vec(IxDyn(&[25]), (0..25).map(|i| (c * 25 + i) as f32).collect())
                .unwrap(),
        );
        w.push(&chunk).unwrap();
    }
    assert_eq!(w.remaining(), 0);
    w.finish().unwrap();

    let v = read(&p).unwrap();
    let a = v.as_array().unwrap();
    assert_eq!(a.shape(), &[10, 25]);
    let f = a.as_f32().unwrap();
    assert_eq!(
        f.iter().copied().collect::<Vec<_>>(),
        (0..250).map(|i| i as f32).collect::<Vec<_>>()
    );
}

#[test]
fn lazy_read_visits_every_element_exactly_once() {
    let t = Tmp::new("lazy-r");
    let p = t.join("big.npy");
    write(&p, &Value::Array(ramp(1000))).unwrap();

    let mut r = read_lazy(&p, &ReadOptions::new().chunk_elements(64)).unwrap();
    let mut seen = Vec::new();
    let mut chunks = 0;
    while let Some(c) = r.next_chunk().unwrap() {
        assert_eq!(c.full_shape, vec![1000]);
        assert_eq!(c.offset, seen.len());
        seen.extend(c.data.as_f32().unwrap().iter().copied());
        chunks += 1;
    }
    assert_eq!(chunks, 16, "1000 elements in 64-element chunks");
    assert_eq!(seen, (0..1000).map(|i| i as f32).collect::<Vec<_>>());
}

#[test]
fn lazy_read_streams_every_member_of_an_archive() {
    let t = Tmp::new("lazy-npz");
    let p = t.join("m.npz");
    write(&p, &sample_dict()).unwrap();
    let mut r = read_lazy(&p, &ReadOptions::new().chunk_elements(2)).unwrap();
    let mut totals = std::collections::BTreeMap::new();
    while let Some(c) = r.next_chunk().unwrap() {
        *totals.entry(c.name.clone()).or_insert(0usize) += c.data.len();
    }
    assert_eq!(totals.get("w"), Some(&6));
    assert_eq!(totals.get("b"), Some(&4));
    assert_eq!(totals.get("x"), Some(&6));
    assert_eq!(totals.get("q"), Some(&5));
}

#[test]
fn lazy_read_streams_hdf5_datasets() {
    let t = Tmp::new("lazy-h5");
    let p = t.join("m.h5");
    write(&p, &sample_dict()).unwrap();
    let mut r = read_lazy(&p, &ReadOptions::new().chunk_elements(3)).unwrap();
    let mut n = 0;
    while let Some(c) = r.next_chunk().unwrap() {
        assert!(c.data.len() <= 3);
        n += c.data.len();
    }
    assert_eq!(n, 6 + 4 + 6 + 5);
}

#[test]
fn lazy_write_rejects_an_overlong_stream() {
    let t = Tmp::new("lazy-over");
    let mut w = write_lazy(t.join("x.npy"), DType::F32, &[4], &WriteOptions::new()).unwrap();
    assert!(w.push(&ramp(10)).is_err());
}

#[test]
fn lazy_write_rejects_an_incomplete_stream() {
    let t = Tmp::new("lazy-under");
    let mut w = write_lazy(t.join("x.npy"), DType::F32, &[100], &WriteOptions::new()).unwrap();
    w.push(&ramp(10)).unwrap();
    assert!(w.finish().is_err(), "finishing early must fail loudly");
}

#[test]
fn lazy_write_casts_each_chunk_to_the_declared_type() {
    let t = Tmp::new("lazy-cast");
    let p = t.join("x.npy");
    let mut w = write_lazy(&p, DType::F32, &[4], &WriteOptions::new()).unwrap();
    let src = Array::F64(ArrayD::from_shape_vec(IxDyn(&[4]), vec![1.0, 2.0, 3.0, 4.0]).unwrap());
    w.push(&src).unwrap();
    w.finish().unwrap();
    assert_eq!(read(&p).unwrap().as_array().unwrap().dtype(), DType::F32);
}

#[test]
fn lazy_and_eager_reads_agree() {
    let t = Tmp::new("lazy-agree");
    let p = t.join("x.npy");
    write(&p, &Value::Array(ramp(333))).unwrap();
    let eager: Vec<f32> =
        read(&p).unwrap().as_array().unwrap().as_f32().unwrap().iter().copied().collect();
    let mut r = read_lazy(&p, &ReadOptions::new().chunk_elements(7)).unwrap();
    let mut lazy = Vec::new();
    while let Some(c) = r.next_chunk().unwrap() {
        lazy.extend(c.data.as_f32().unwrap().iter().copied());
    }
    assert_eq!(eager, lazy);
}
