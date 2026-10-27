//! Inert capture of Python objects the crate does not model natively.

use super::{Dict, Value};

/// A Python object recorded structurally instead of executed.
///
/// The pickle machine never calls Python, so anything outside the numpy/torch
/// allow-list ends up here with its constructor arguments and `__setstate__`
/// payload preserved. That is enough to inspect scikit-learn estimators,
/// custom `nn.Module` wrappers, and to write the object back out unchanged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PyObject {
    /// e.g. `"sklearn.linear_model._base"`.
    pub module: String,
    /// e.g. `"LinearRegression"`.
    pub name: String,
    /// Positional arguments captured from `REDUCE` / `NEWOBJ`.
    pub args: Vec<Value>,
    /// Keyword arguments captured from `NEWOBJ_EX`.
    pub kwargs: Dict,
    /// Payload from `BUILD` (`__setstate__` / `__dict__`).
    pub state: Option<Box<Value>>,
    /// Items appended through `APPEND`/`APPENDS` (list-like objects).
    pub list_items: Vec<Value>,
    /// Items set through `SETITEM`/`SETITEMS` (dict-like objects).
    pub dict_items: Dict,
}

impl PyObject {
    /// Create a new value.
    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        PyObject { module: module.into(), name: name.into(), ..Default::default() }
    }
    /// Fully qualified name, `module.name`.
    pub fn qualname(&self) -> String {
        if self.module.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.module, self.name)
        }
    }
    /// Attribute lookup inside the captured `__dict__` state.
    pub fn attr(&self, name: &str) -> Option<&Value> {
        match self.state.as_deref() {
            Some(Value::Dict(d)) => d.get(name),
            Some(Value::Tuple(t)) => match t.first() {
                Some(Value::Dict(d)) => d.get(name),
                _ => None,
            },
            _ => None,
        }
    }
    /// Mutable attribute lookup, for editing weights stored on an estimator.
    pub fn attr_mut(&mut self, name: &str) -> Option<&mut Value> {
        match self.state.as_deref_mut() {
            Some(Value::Dict(d)) => d.get_mut(name),
            Some(Value::Tuple(t)) => match t.first_mut() {
                Some(Value::Dict(d)) => d.get_mut(name),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_and_qualified_name_handle_empty_and_named_modules() {
        let unnamed = PyObject::new("", "Thing");
        assert_eq!(unnamed.qualname(), "Thing");
        let named = PyObject::new("pkg.module", "Thing");
        assert_eq!(named.qualname(), "pkg.module.Thing");
        assert_eq!(
            named,
            PyObject { module: "pkg.module".into(), name: "Thing".into(), ..PyObject::default() }
        );
    }

    #[test]
    fn attributes_read_from_dict_state_or_first_tuple_item() {
        let mut dict = Dict::new();
        dict.insert(Value::Str("coef".into()), Value::Int(2));
        let object =
            PyObject { state: Some(Box::new(Value::Dict(dict.clone()))), ..PyObject::default() };
        assert_eq!(object.attr("coef"), Some(&Value::Int(2)));
        assert_eq!(object.attr("missing"), None);

        let tuple_state = PyObject {
            state: Some(Box::new(Value::Tuple(vec![Value::Dict(dict)]))),
            ..PyObject::default()
        };
        assert_eq!(tuple_state.attr("coef"), Some(&Value::Int(2)));
        assert_eq!(tuple_state.attr("missing"), None);
    }

    #[test]
    fn mutable_attributes_edit_supported_state_shapes_and_ignore_others() {
        let mut dict = Dict::new();
        dict.insert(Value::Str("coef".into()), Value::Int(2));
        let mut object =
            PyObject { state: Some(Box::new(Value::Dict(dict))), ..PyObject::default() };
        *object.attr_mut("coef").unwrap() = Value::Int(3);
        assert_eq!(object.attr("coef"), Some(&Value::Int(3)));

        let mut tuple = PyObject {
            state: Some(Box::new(Value::Tuple(vec![Value::Dict(Dict::new())]))),
            ..PyObject::default()
        };
        assert!(tuple.attr_mut("missing").is_none());
        let mut other_state = PyObject {
            state: Some(Box::new(Value::Tuple(vec![Value::Int(1)]))),
            ..PyObject::default()
        };
        assert!(other_state.attr("x").is_none());
        assert!(other_state.attr_mut("x").is_none());
        let no_state = PyObject::default();
        assert!(no_state.attr("x").is_none());
    }
}
