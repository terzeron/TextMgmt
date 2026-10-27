use ndarray::ArrayD;
use pyferrite::prelude::*;

fn arr_f32() -> Array {
    Array::F32(
        ArrayD::from_shape_vec(ndarray::IxDyn(&[2, 3]), (0..6).map(|i| i as f32 * 1.5).collect())
            .unwrap(),
    )
}
fn arr_i64() -> Array {
    Array::I64(ArrayD::from_shape_vec(ndarray::IxDyn(&[4]), vec![-1i64, 0, 7, 1 << 40]).unwrap())
}
fn arr_f64() -> Array {
    Array::F64(
        ArrayD::from_shape_vec(ndarray::IxDyn(&[3, 2]), vec![0.5, -1.25, 3.0, 1e-300, 2.0, 9.75])
            .unwrap(),
    )
}
fn arr_u8() -> Array {
    Array::U8(ArrayD::from_shape_vec(ndarray::IxDyn(&[5]), vec![0u8, 1, 128, 254, 255]).unwrap())
}

fn main() -> Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let mut d = Dict::new();
    d.insert(Value::Str("w".into()), Value::Array(arr_f32()));
    d.insert(Value::Str("b".into()), Value::Array(arr_i64()));
    d.insert(Value::Str("x".into()), Value::Array(arr_f64()));
    d.insert(Value::Str("q".into()), Value::Array(arr_u8()));
    let dict = Value::Dict(d);

    write(out.join("w.npy"), &Value::Array(arr_f32()))?;
    write(out.join("w.npz"), &dict)?;
    write(out.join("w.pkl"), &dict)?;
    write(out.join("w.joblib"), &dict)?;
    write(out.join("w.h5"), &dict)?;
    write(out.join("w.pt"), &dict)?;

    // nested group in HDF5
    let mut inner = Dict::new();
    inner.insert(Value::Str("deep".into()), Value::Array(arr_f32()));
    let mut outer = Dict::new();
    outer.insert(Value::Str("layer1".into()), Value::Dict(inner));
    outer.insert(Value::Str("top".into()), Value::Array(arr_i64()));
    write(out.join("nested.h5"), &Value::Dict(outer))?;

    // down-cast on write: f64 source stored as f32
    let opts = WriteOptions::new().float_as(FloatKind::F32);
    write_with(out.join("down.npy"), &Value::Array(arr_f64()), &opts)?;

    // lazy write
    let mut lw = write_lazy(out.join("lazy.npy"), DType::F32, &[10, 100], &WriteOptions::new())?;
    for c in 0..10 {
        let chunk = Array::F32(
            ArrayD::from_shape_vec(
                ndarray::IxDyn(&[100]),
                (0..100).map(|i| (c * 100 + i) as f32).collect(),
            )
            .unwrap(),
        );
        lw.push(&chunk)?;
    }
    lw.finish()?;

    // validation must reject this
    match write(out.join("bad.npy"), &Value::Str("not an array".into())) {
        Err(e) => println!("rejected as expected: {e}"),
        Ok(_) => println!("BUG: string accepted into .npy"),
    }
    println!("written");
    Ok(())
}
