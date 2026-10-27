//! Cross-format helpers: dtype resolution, cast application, axis permutation.

use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::options::{ReadOptions, WriteOptions};
use crate::value::{Array, Value};
use ndarray::IxDyn;

/// Apply the target dtype implied by `int_as` / `float_as` to a source dtype.
pub fn resolve_target_dtype_read(src: &DType, opts: &ReadOptions) -> DType {
    if let (true, Some(k)) = (src.is_integer(), opts.int_as) {
        return k.dtype();
    }
    if let (true, Some(k)) = (src.is_float(), opts.float_as) {
        return k.dtype();
    }
    if *src == DType::X87 {
        return DType::F128;
    }
    src.clone()
}

/// Same, for the write direction; `opts.dtype` overrides everything.
pub fn resolve_target_dtype(src: &DType, opts: &WriteOptions) -> DType {
    if let Some(dt) = &opts.dtype {
        return dt.clone();
    }
    if let (true, Some(k)) = (src.is_integer(), opts.int_as) {
        return k.dtype();
    }
    if let (true, Some(k)) = (src.is_float(), opts.float_as) {
        return k.dtype();
    }
    src.clone()
}

/// Cast a freshly decoded array according to the read options.
pub fn apply_cast(a: Array, opts: &ReadOptions) -> Result<Array> {
    let target = resolve_target_dtype_read(&a.dtype(), opts);
    if target == a.dtype() {
        Ok(a)
    } else {
        a.cast(&target, opts.cast_policy)
    }
}

/// Permute array axes (used to normalise Fortran-ordered payloads).
pub fn permute(a: Array, axes: &[usize]) -> Array {
    use crate::value::Array as A;
    macro_rules! p {
        ($x:expr, $v:ident) => {
            A::$v($x.permuted_axes(IxDyn(axes)).as_standard_layout().to_owned())
        };
    }
    match a {
        A::Bool(x) => p!(x, Bool),
        A::I8(x) => p!(x, I8),
        A::I16(x) => p!(x, I16),
        A::I32(x) => p!(x, I32),
        A::I64(x) => p!(x, I64),
        A::U8(x) => p!(x, U8),
        A::U16(x) => p!(x, U16),
        A::U32(x) => p!(x, U32),
        A::U64(x) => p!(x, U64),
        A::F8E4M3(x) => p!(x, F8E4M3),
        A::F8E5M2(x) => p!(x, F8E5M2),
        A::F16(x) => p!(x, F16),
        A::BF16(x) => p!(x, BF16),
        A::F32(x) => p!(x, F32),
        A::F64(x) => p!(x, F64),
        A::F128(x) => p!(x, F128),
        A::C64(x) => p!(x, C64),
        A::C128(x) => p!(x, C128),
        A::Str(x) => p!(x, Str),
        A::Bytes(x) => p!(x, Bytes),
    }
}

/// Coerce a scalar / list value into an [`Array`] so it can be written to a
/// format that only stores arrays.
///
/// This is the validation the user asked for: writing a `str` where numpy
/// expects an array fails loudly instead of producing a broken file.
pub fn value_to_array(v: &Value) -> Result<Array> {
    use ndarray::ArrayD;
    let mk = |data: Vec<f64>| -> Result<Array> {
        let n = data.len();
        Ok(Array::F64(
            ArrayD::from_shape_vec(IxDyn(&[n]), data).map_err(|e| Error::format(e.to_string()))?,
        ))
    };
    Ok(match v {
        Value::Array(a) => a.clone(),
        Value::Float(f) => mk(vec![*f])?,
        Value::Int(i) => Array::I64(
            ArrayD::from_shape_vec(IxDyn(&[] as &[usize]), vec![*i])
                .map_err(|e| Error::format(e.to_string()))?,
        ),
        Value::Bool(b) => Array::Bool(
            ArrayD::from_shape_vec(IxDyn(&[] as &[usize]), vec![*b])
                .map_err(|e| Error::format(e.to_string()))?,
        ),
        Value::List(items) | Value::Tuple(items) => {
            if items.iter().all(|x| matches!(x, Value::Int(_) | Value::Bool(_))) {
                let data: Vec<i64> = items.iter().filter_map(|x| x.as_i64()).collect();
                let n = data.len();
                Array::I64(
                    ArrayD::from_shape_vec(IxDyn(&[n]), data)
                        .map_err(|e| Error::format(e.to_string()))?,
                )
            } else if items.iter().all(|x| x.as_f64().is_some()) {
                mk(items.iter().filter_map(|x| x.as_f64()).collect())?
            } else if items.iter().all(|x| matches!(x, Value::Array(_))) {
                return stack(items);
            } else {
                return Err(Error::invalid(
                    "cannot write a heterogeneous list as a numeric array; \
                     convert it to a Value::Array or a Value::Frame first",
                ));
            }
        }
        other => {
            return Err(Error::invalid(format!(
                "cannot write a `{}` as a numeric array — numpy expects numbers, \
                 not `{}`",
                other.type_name(),
                other.type_name()
            )))
        }
    })
}

/// Stack a list of equally shaped arrays along a new leading axis.
fn stack(items: &[Value]) -> Result<Array> {
    let arrays: Vec<&Array> = items.iter().filter_map(|v| v.as_array()).collect();
    let first = arrays.first().ok_or_else(|| Error::invalid("empty array list"))?;
    let inner = first.shape().to_vec();
    let dt = first.dtype();
    for a in &arrays {
        if a.shape() != inner.as_slice() || a.dtype() != dt {
            return Err(Error::invalid("stacked arrays must share dtype and shape"));
        }
    }
    let mut flat: Vec<u8> = Vec::new();
    for a in &arrays {
        flat.extend(crate::value::to_bytes(a, crate::dtype::Endian::Little)?);
    }
    let mut shape = vec![arrays.len()];
    shape.extend(inner);
    crate::value::from_bytes(&dt, crate::dtype::Endian::Little, &shape, &flat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtype::{Endian, IntKind};

    fn opts_int(kind: Option<IntKind>) -> ReadOptions {
        ReadOptions { int_as: kind, ..ReadOptions::default() }
    }

    #[test]
    fn resolves_read_and_write_dtype_overrides() {
        assert_eq!(
            resolve_target_dtype_read(&DType::I16, &opts_int(Some(IntKind::I64))),
            DType::I64
        );
        assert_eq!(
            resolve_target_dtype_read(
                &DType::F32,
                &ReadOptions {
                    float_as: Some(crate::dtype::FloatKind::F64),
                    ..ReadOptions::default()
                }
            ),
            DType::F64
        );
        assert_eq!(resolve_target_dtype_read(&DType::X87, &ReadOptions::default()), DType::F128);
        assert_eq!(
            resolve_target_dtype_read(&DType::Bytes(2), &ReadOptions::default()),
            DType::Bytes(2)
        );
        assert_eq!(
            resolve_target_dtype(
                &DType::I16,
                &WriteOptions { int_as: Some(IntKind::I64), ..WriteOptions::default() }
            ),
            DType::I64
        );
        assert_eq!(
            resolve_target_dtype(
                &DType::F32,
                &WriteOptions {
                    float_as: Some(crate::dtype::FloatKind::F64),
                    ..WriteOptions::default()
                }
            ),
            DType::F64
        );
        assert_eq!(
            resolve_target_dtype(
                &DType::I16,
                &WriteOptions {
                    dtype: Some(DType::U8),
                    int_as: Some(IntKind::I64),
                    ..WriteOptions::default()
                }
            ),
            DType::U8
        );
        assert_eq!(resolve_target_dtype(&DType::Bool, &WriteOptions::default()), DType::Bool);
    }

    #[test]
    fn apply_cast_returns_same_type_or_converts_to_requested_type() {
        let input = Array::I16(ndarray::arr1(&[4i16]).into_dyn());
        assert_eq!(apply_cast(input.clone(), &ReadOptions::default()).unwrap(), input);
        let converted = apply_cast(input, &opts_int(Some(IntKind::I64))).unwrap();
        assert_eq!(converted, Array::I64(ndarray::arr1(&[4i64]).into_dyn()));
    }

    #[test]
    fn value_to_array_handles_scalars_lists_and_rejects_unsupported_values() {
        assert_eq!(
            value_to_array(&Value::Array(Array::U8(ndarray::arr1(&[2]).into_dyn()))).unwrap(),
            Array::U8(ndarray::arr1(&[2]).into_dyn())
        );
        assert!(matches!(value_to_array(&Value::Float(1.5)).unwrap(), Array::F64(_)));
        assert_eq!(
            value_to_array(&Value::Int(7)).unwrap(),
            Array::I64(ndarray::arr0(7).into_dyn())
        );
        assert_eq!(
            value_to_array(&Value::Bool(true)).unwrap(),
            Array::Bool(ndarray::arr0(true).into_dyn())
        );
        assert_eq!(
            value_to_array(&Value::List(vec![Value::Int(1), Value::Bool(true)])).unwrap(),
            Array::I64(ndarray::arr1(&[1, 1]).into_dyn())
        );
        assert_eq!(
            value_to_array(&Value::Tuple(vec![Value::Float(1.0), Value::Int(2)])).unwrap(),
            Array::F64(ndarray::arr1(&[1.0, 2.0]).into_dyn())
        );
        assert_eq!(
            value_to_array(&Value::List(Vec::new())).unwrap(),
            Array::I64(ndarray::arr1(&[]).into_dyn())
        );
        assert!(value_to_array(&Value::List(vec![Value::Int(1), Value::Str("x".into())])).is_err());
        assert!(value_to_array(&Value::Str("x".into())).is_err());
    }

    #[test]
    fn stacks_arrays_and_rejects_incompatible_or_empty_stacks() {
        let a = Value::Array(Array::I32(ndarray::arr1(&[1, 2]).into_dyn()));
        let b = Value::Array(Array::I32(ndarray::arr1(&[3, 4]).into_dyn()));
        assert_eq!(
            value_to_array(&Value::List(vec![a.clone(), b])).unwrap(),
            Array::I32(ndarray::arr2(&[[1, 2], [3, 4]]).into_dyn())
        );
        assert!(stack(&[]).is_err());
        assert!(stack(&[a.clone(), Value::Array(Array::I64(ndarray::arr1(&[3, 4]).into_dyn()))])
            .is_err());
        assert!(stack(&[a, Value::Array(Array::I32(ndarray::arr1(&[3]).into_dyn()))]).is_err());
        assert!(matches!(
            stack(&[Value::Array(Array::Str(ndarray::arr1(&["x".to_owned()]).into_dyn()))])
                .unwrap(),
            Array::Str(_)
        ));
    }

    #[test]
    fn permutes_every_array_variant_and_preserves_values() {
        let dtypes = [
            DType::Bool,
            DType::I8,
            DType::I16,
            DType::I32,
            DType::I64,
            DType::U8,
            DType::U16,
            DType::U32,
            DType::U64,
            DType::F8E4M3,
            DType::F8E5M2,
            DType::F16,
            DType::BF16,
            DType::F32,
            DType::F64,
            DType::F128,
            DType::C64,
            DType::C128,
            DType::Str(1),
            DType::Bytes(1),
        ];
        for dt in dtypes {
            let bytes = vec![0; dt.size().unwrap() * 2];
            let input = crate::value::from_bytes(&dt, Endian::Little, &[2, 1], &bytes).unwrap();
            let result = permute(input, &[1, 0]);
            assert_eq!(result.shape(), &[1, 2], "dtype {dt:?}");
        }
    }
}
