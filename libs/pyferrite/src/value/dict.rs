//! Insertion-ordered mapping mirroring Python's `dict`.

use super::Value;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// An ordered `dict`. Keys keep their original Python type, which matters for
/// `state_dict`s whose keys are strings but also for tuple-keyed lookup tables.
#[derive(Clone, Debug)]
pub struct Dict {
    entries: Vec<(Value, Value)>,
    string_index: HashMap<u64, Vec<usize>>,
    string_index_valid: bool,
}

impl PartialEq for Dict {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Default for Dict {
    fn default() -> Self {
        Self::new()
    }
}

fn string_hash(key: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

impl Dict {
    /// Create a new value.
    pub fn new() -> Self {
        Dict { entries: Vec::new(), string_index: HashMap::new(), string_index_valid: true }
    }
    /// With capacity.
    pub fn with_capacity(n: usize) -> Self {
        Dict {
            entries: Vec::with_capacity(n),
            string_index: HashMap::with_capacity(n),
            string_index_valid: true,
        }
    }
    /// Len.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// `true` when this is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// Insert or overwrite, preserving the original position on overwrite.
    pub fn insert(&mut self, k: Value, v: Value) {
        let existing = match &k {
            Value::Str(key) if self.string_index_valid => self.find_string_index(key),
            _ => self.entries.iter().position(|(key, _)| key == &k),
        };
        if let Some(index) = existing {
            self.entries[index].1 = v;
            return;
        }
        let index = self.entries.len();
        if self.string_index_valid {
            if let Value::Str(key) = &k {
                self.string_index.entry(string_hash(key)).or_default().push(index);
            }
        }
        self.entries.push((k, v));
    }
    /// Look up by string key (the common case for `state_dict`s).
    pub fn get(&self, key: &str) -> Option<&Value> {
        if self.string_index_valid {
            return self
                .find_string_index(key)
                .and_then(|index| self.entries.get(index).map(|(_, value)| value));
        }
        self.entries.iter().find(|(k, _)| k.as_str() == Some(key)).map(|(_, v)| v)
    }
    /// Mutable lookup by string key — the entry point for in-place rescaling.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        let index = if self.string_index_valid {
            self.find_string_index(key)
        } else {
            self.entries.iter().position(|(k, _)| k.as_str() == Some(key))
        }?;
        self.entries.get_mut(index).map(|(_, value)| value)
    }
    /// Look up by an arbitrary key value.
    pub fn get_by(&self, key: &Value) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    /// Remove.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let index = if self.string_index_valid {
            self.find_string_index(key)
        } else {
            self.entries.iter().position(|(k, _)| k.as_str() == Some(key))
        }?;
        let value = self.entries.remove(index).1;
        if self.string_index_valid {
            self.rebuild_string_index();
        }
        Some(value)
    }
    /// Contains key.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    /// Iter.
    pub fn iter(&self) -> std::slice::Iter<'_, (Value, Value)> {
        self.entries.iter()
    }
    /// Iter mut.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, (Value, Value)> {
        self.string_index_valid = false;
        self.string_index.clear();
        self.entries.iter_mut()
    }
    /// Every key that is a string, in insertion order.
    pub fn keys(&self) -> Vec<String> {
        self.entries.iter().filter_map(|(k, _)| k.as_str().map(|s| s.to_string())).collect()
    }
    /// Values mut.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Value> {
        self.entries.iter_mut().map(|(_, v)| v)
    }
    /// Take the contents as entries, when the type matches.
    pub fn into_entries(self) -> Vec<(Value, Value)> {
        self.entries
    }

    fn find_string_index(&self, key: &str) -> Option<usize> {
        self.string_index
            .get(&string_hash(key))?
            .iter()
            .copied()
            .find(|index| self.entries[*index].0.as_str() == Some(key))
    }

    fn rebuild_string_index(&mut self) {
        self.string_index.clear();
        self.string_index.reserve(self.entries.len());
        for (index, (key, _)) in self.entries.iter().enumerate() {
            if let Some(key) = key.as_str() {
                self.string_index.entry(string_hash(key)).or_default().push(index);
            }
        }
    }
}

impl FromIterator<(Value, Value)> for Dict {
    fn from_iter<T: IntoIterator<Item = (Value, Value)>>(it: T) -> Self {
        let mut dict = Self::new();
        for (key, value) in it {
            dict.insert(key, value);
        }
        dict
    }
}

impl<'a> IntoIterator for &'a Dict {
    type Item = &'a (Value, Value);
    type IntoIter = std::slice::Iter<'a, (Value, Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::Dict;
    use crate::value::Value;
    use std::time::{Duration, Instant};

    #[test]
    fn inserts_many_string_keys_without_quadratic_scan() {
        let mut dict = Dict::new();
        let started = Instant::now();
        for index in 0..25_000 {
            dict.insert(Value::Str(format!("feature_{index:05}")), Value::Int(index));
        }

        assert_eq!(dict.len(), 25_000);
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "string-key insertion took {:?}",
            started.elapsed()
        );
        for index in [0, 1, 1234, 12_345, 24_999] {
            assert_eq!(
                dict.get(&format!("feature_{index:05}")).and_then(Value::as_i64),
                Some(index)
            );
        }
    }

    #[test]
    fn indexed_string_operations_preserve_order_overwrite_remove_and_mutation() {
        let mut dict = Dict::new();
        dict.insert(Value::Str("first".into()), Value::Int(1));
        dict.insert(Value::Str("second".into()), Value::Int(2));
        dict.insert(Value::Str("first".into()), Value::Int(3));
        assert_eq!(dict.keys(), vec!["first".to_owned(), "second".to_owned()]);
        assert_eq!(dict.get("first").and_then(Value::as_i64), Some(3));
        assert_eq!(dict.remove("first").and_then(|value| value.as_i64()), Some(3));
        assert_eq!(dict.get("second").and_then(Value::as_i64), Some(2));

        for (key, _) in dict.iter_mut() {
            if key.as_str() == Some("second") {
                *key = Value::Str("renamed".into());
            }
        }
        assert!(dict.get("second").is_none());
        assert_eq!(dict.get("renamed").and_then(Value::as_i64), Some(2));
    }
}
