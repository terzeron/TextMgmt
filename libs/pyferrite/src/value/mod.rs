//! The dynamic value model: everything a Python artefact can contain.

mod array;
mod cast;
mod codec;
mod dict;
mod encode;
mod frame;
mod object;
mod slice;
mod wide;

pub use array::Array;
pub use codec::from_bytes;
pub use dict::Dict;
pub use encode::{to_bytes, to_bytes_x87, write_bytes_to};
pub use frame::{Frame, Series};
pub use object::PyObject;
pub use wide::{to_wide, Wide};

use crate::num::Complex64;

/// A decoded Python value.
///
/// Basic Python types land on basic **owned, mutable** Rust types; `np.ndarray`
/// and `torch.Tensor` land on [`Array`] (backed by `ndarray::ArrayD`);
/// `pandas.DataFrame` and `polars.DataFrame` land on [`Frame`].
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Python `None`.
    None,
    /// Booleans, one byte each, as in numpy `bool_`.
    Bool(bool),
    /// Python `int` that fits in 64 bits.
    Int(i64),
    /// Arbitrary precision `int`, little-endian two's complement.
    BigInt(Vec<u8>),
    /// A Python `float`.
    Float(f64),
    /// A Python `complex`.
    Complex(Complex64),
    /// Fixed-width UTF-32 text, the inner value being the character count.
    Str(String),
    /// Fixed-width byte string, the inner value being the length.
    Bytes(Vec<u8>),
    /// A Python `list`.
    List(Vec<Value>),
    /// A Python `tuple`.
    Tuple(Vec<Value>),
    /// A Python `set` or `frozenset`.
    Set(Vec<Value>),
    /// A Python `dict`, insertion order preserved.
    Dict(Dict),
    /// `np.ndarray`, `torch.Tensor`, HDF5 dataset, npy payload.
    Array(Array),
    /// `pandas.DataFrame`, `polars.DataFrame`, numpy record array.
    Frame(Frame),
    /// Anything else, captured structurally without executing Python.
    Object(PyObject),
}

impl Value {
    /// `&str` view for `Value::Str`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Numeric view; integers and bools widen to `f64`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            Value::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }
    /// Borrow the contents as i64, when the type matches.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }
    /// Borrow the contents as array, when the type matches.
    pub fn as_array(&self) -> Option<&Array> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    /// Mutable array view — scale or normalise weights before loading them.
    pub fn as_array_mut(&mut self) -> Option<&mut Array> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    /// Borrow the contents as dict, when the type matches.
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
    /// Borrow the contents as dict mut, when the type matches.
    pub fn as_dict_mut(&mut self) -> Option<&mut Dict> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
    /// Borrow the contents as frame, when the type matches.
    pub fn as_frame(&self) -> Option<&Frame> {
        match self {
            Value::Frame(f) => Some(f),
            _ => None,
        }
    }
    /// Borrow the contents as frame mut, when the type matches.
    pub fn as_frame_mut(&mut self) -> Option<&mut Frame> {
        match self {
            Value::Frame(f) => Some(f),
            _ => None,
        }
    }
    /// Borrow the contents as object, when the type matches.
    pub fn as_object(&self) -> Option<&PyObject> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }
    /// Short human-readable type name, handy in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::None => "None",
            Value::Bool(_) => "bool",
            Value::Int(_) | Value::BigInt(_) => "int",
            Value::Float(_) => "float",
            Value::Complex(_) => "complex",
            Value::Str(_) => "str",
            Value::Bytes(_) => "bytes",
            Value::List(_) => "list",
            Value::Tuple(_) => "tuple",
            Value::Set(_) => "set",
            Value::Dict(_) => "dict",
            Value::Array(_) => "ndarray",
            Value::Frame(_) => "dataframe",
            Value::Object(_) => "object",
        }
    }
    /// Depth-first walk over every [`Array`] contained anywhere in the value.
    pub fn arrays_mut(&mut self) -> Vec<&mut Array> {
        let mut out = Vec::new();
        collect(self, &mut out);
        out
    }
}

fn collect<'a>(v: &'a mut Value, out: &mut Vec<&'a mut Array>) {
    match v {
        Value::Array(a) => out.push(a),
        Value::List(xs) | Value::Tuple(xs) | Value::Set(xs) => {
            xs.iter_mut().for_each(|x| collect(x, out))
        }
        Value::Dict(d) => d.values_mut().for_each(|x| collect(x, out)),
        Value::Frame(f) => f.columns.iter_mut().for_each(|c| out.push(&mut c.values)),
        Value::Object(o) => {
            if let Some(s) = o.state.as_deref_mut() {
                collect(s, out);
            }
            o.args.iter_mut().for_each(|x| collect(x, out));
            o.list_items.iter_mut().for_each(|x| collect(x, out));
            o.dict_items.values_mut().for_each(|x| collect(x, out));
        }
        _ => {}
    }
}

/// Ergonomic conversions into [`Value`].
///
/// These let you write `array.into()` and `42.into()` instead of naming the
/// variant every time, which matters most when building a dict literal.
mod from_impls {
    use super::{Array, Dict, Frame, PyObject, Value};

    macro_rules! from {
        ($($t:ty => $variant:ident $(as $cast:ty)?),* $(,)?) => {
            $(impl From<$t> for Value {
                fn from(v: $t) -> Value {
                    Value::$variant(v $(as $cast)?)
                }
            })*
        };
    }

    from! {
        Array => Array,
        Dict => Dict,
        Frame => Frame,
        PyObject => Object,
        bool => Bool,
        i8 => Int as i64,
        i16 => Int as i64,
        i32 => Int as i64,
        i64 => Int,
        u8 => Int as i64,
        u16 => Int as i64,
        u32 => Int as i64,
        f32 => Float as f64,
        f64 => Float,
        String => Str,
    }

    impl From<Vec<u8>> for Value {
        /// A byte vector becomes `bytes`, matching what Python would pickle.
        /// For a `u8` *array*, build an [`Array::U8`] and convert that instead.
        fn from(v: Vec<u8>) -> Value {
            Value::Bytes(v)
        }
    }

    impl From<&str> for Value {
        fn from(v: &str) -> Value {
            Value::Str(v.to_string())
        }
    }

    impl<T: Into<Value>> From<Option<T>> for Value {
        fn from(v: Option<T>) -> Value {
            match v {
                Some(x) => x.into(),
                None => Value::None,
            }
        }
    }
}
