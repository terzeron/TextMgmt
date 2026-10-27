//! Column-oriented table — the neutral ground between pandas and polars.

use super::Array;
use crate::error::{Error, Result};

/// A named 1-D column.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    /// Name of this item.
    pub name: String,
    /// Always 1-D. Owned, therefore mutable in place.
    pub values: Array,
    /// Optional validity mask (`true` = present). `None` means "no nulls".
    pub validity: Option<Vec<bool>>,
}

impl Series {
    /// Create a new value.
    pub fn new(name: impl Into<String>, values: Array) -> Self {
        Series { name: name.into(), values, validity: None }
    }
    /// Len.
    pub fn len(&self) -> usize {
        self.values.len()
    }
    /// `true` when this is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Mutable access to the underlying array (rescale a column in place).
    pub fn values_mut(&mut self) -> &mut Array {
        &mut self.values
    }
}

/// An ordered set of equally long [`Series`], plus an optional index column.
///
/// `pandas.DataFrame`, `polars.DataFrame` and numpy record arrays all map onto
/// this shape, which is why the crate uses it as the single tabular currency.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// The columns, in order.
    pub columns: Vec<Series>,
    /// pandas keeps an explicit index; polars does not. `None` for polars.
    pub index: Option<Series>,
}

impl Frame {
    /// Create a new value.
    pub fn new() -> Self {
        Frame::default()
    }

    /// Build from columns, verifying that every column has the same height.
    pub fn from_columns(columns: Vec<Series>) -> Result<Self> {
        if let Some(first) = columns.first() {
            let h = first.len();
            for c in &columns {
                if c.len() != h {
                    return Err(Error::invalid(format!(
                        "column `{}` has {} rows, expected {}",
                        c.name,
                        c.len(),
                        h
                    )));
                }
            }
        }
        Ok(Frame { columns, index: None })
    }

    /// Number of rows.
    pub fn height(&self) -> usize {
        self.columns.first().map(|c| c.len()).unwrap_or(0)
    }
    /// Number of columns.
    pub fn width(&self) -> usize {
        self.columns.len()
    }
    /// Column names in order.
    pub fn names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }
    /// Column.
    pub fn column(&self, name: &str) -> Option<&Series> {
        self.columns.iter().find(|c| c.name == name)
    }
    /// Mutable column lookup — normalise a feature without copying the frame.
    pub fn column_mut(&mut self, name: &str) -> Option<&mut Series> {
        self.columns.iter_mut().find(|c| c.name == name)
    }
    /// Push column.
    pub fn push_column(&mut self, s: Series) -> Result<()> {
        if !self.columns.is_empty() && s.len() != self.height() {
            return Err(Error::invalid(format!(
                "column `{}` has {} rows, frame has {}",
                s.name,
                s.len(),
                self.height()
            )));
        }
        self.columns.push(s);
        Ok(())
    }
    /// Drop column.
    pub fn drop_column(&mut self, name: &str) -> Option<Series> {
        let i = self.columns.iter().position(|c| c.name == name)?;
        Some(self.columns.remove(i))
    }
}
