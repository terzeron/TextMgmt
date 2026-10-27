//! Pass one of the writer: turn a [`Value`] into a laid-out [`Node`] tree.

use super::layout_plan::*;
use crate::error::{Error, Result};
use crate::options::WriteOptions;
use crate::value::Value;

/// Build the plan for `value`, assigning every address starting after the
/// superblock. Returns the tree and the resulting end-of-file address.
pub fn plan<'a>(value: &'a Value, opts: &WriteOptions) -> Result<(Node<'a>, u64)> {
    let mut cursor = SUPERBLOCK as u64;
    let node = build(value, opts, &mut cursor)?;
    Ok((node, cursor))
}

fn build<'a>(value: &'a Value, opts: &WriteOptions, cursor: &mut u64) -> Result<Node<'a>> {
    match value {
        Value::Array(a) => {
            let dtype = super::write::target_dtype(&a.dtype(), opts)?;
            let shape = match &opts.shape {
                Some(s) if s.iter().product::<usize>() == a.len() => s.clone(),
                Some(s) => {
                    return Err(Error::Shape { expected: s.clone(), got: a.len() });
                }
                None => a.shape().to_vec(),
            };
            let esz = dtype.size().unwrap_or(1);
            let header_size = dataset_header_size(&dtype, shape.len())?;
            let header = *cursor;
            *cursor += header_size as u64;
            *cursor = align8(*cursor as usize) as u64;
            let data = *cursor;
            let data_size = shape.iter().product::<usize>() * esz;
            *cursor += align8(data_size) as u64;
            Ok(Node::Dataset(Dataset {
                array: a,
                dtype,
                shape,
                header,
                header_size,
                data,
                data_size,
            }))
        }
        Value::Dict(d) => {
            let mut pairs: Vec<(String, &Value)> = Vec::new();
            for (k, v) in d.iter() {
                let name = k
                    .as_str()
                    .map(|s| s.to_string())
                    .ok_or_else(|| Error::invalid("HDF5 link names must be strings"))?;
                if name.is_empty() || name.contains('/') || name.contains('\0') {
                    return Err(Error::invalid(format!("illegal HDF5 link name `{name}`")));
                }
                pairs.push((name, v));
            }
            group(pairs, opts, cursor)
        }
        Value::List(items) | Value::Tuple(items) => {
            let pairs =
                items.iter().enumerate().map(|(i, v)| (format!("item_{i}"), v)).collect::<Vec<_>>();
            group(pairs, opts, cursor)
        }
        other => Err(Error::unsupported(format!(
            "`{}` has no HDF5 representation; wrap it in a dict of arrays",
            other.type_name()
        ))),
    }
}

fn group<'a>(
    mut pairs: Vec<(String, &'a Value)>,
    opts: &WriteOptions,
    cursor: &mut u64,
) -> Result<Node<'a>> {
    // HDF5 symbol tables are ordered by name, and so are B-tree keys.
    pairs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let names: Vec<String> = pairs.iter().map(|(n, _)| n.clone()).collect();
    let (heap_size, name_offsets, sentinel) = plan_heap(&names);
    // One symbol-table node must hold every child, so size the leaf arity to fit.
    let leaf_k = ((names.len() + 1) / 2).max(4);
    let snod_size = SNOD_PREFIX + SNOD_ENTRY * 2 * leaf_k;

    let header = *cursor;
    *cursor += GROUP_HEADER as u64;
    let heap = *cursor;
    *cursor += HEAP_HEADER as u64;
    let heap_data = *cursor;
    *cursor += heap_size as u64;
    let btree = *cursor;
    *cursor += btree_size(BTREE_INTERNAL_K) as u64;
    let snod = *cursor;
    *cursor += snod_size as u64;

    let mut children = Vec::with_capacity(pairs.len());
    for (name, v) in pairs {
        children.push((name, build(v, opts, cursor)?));
    }
    Ok(Node::Group(Group {
        children,
        leaf_k,
        header,
        heap,
        heap_data,
        heap_size,
        btree,
        snod,
        name_offsets,
        sentinel,
    }))
}

impl<'a> Node<'a> {
    /// Address of this node's object header.
    pub fn header_address(&self) -> u64 {
        match self {
            Node::Group(g) => g.header,
            Node::Dataset(d) => d.header,
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/rust/pyferrite/hdf5_plan_unit.rs"]
mod tests;
