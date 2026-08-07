use crate::value::DbValue;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct DbRow {
    columns: HashMap<String, DbValue>,
}

impl DbRow {
    pub fn new() -> Self {
        Self {
            columns: HashMap::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            columns: HashMap::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, column: String, value: DbValue) {
        self.columns.insert(column, value);
    }

    pub fn get(&self, column: &str) -> Option<&DbValue> {
        self.columns.get(column).or_else(|| {
            self.columns
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(column))
                .map(|(_, v)| v)
        })
    }

    pub fn get_text(&self, column: &str) -> Option<String> {
        self.get(column).and_then(|v| match v {
            crate::value::DbValue::Text(s) => Some(s.clone()),
            _ => None,
        })
    }

    pub fn get_integer(&self, column: &str) -> Option<i64> {
        self.get(column).and_then(|v| match v {
            crate::value::DbValue::Integer(i) => Some(*i),
            _ => None,
        })
    }

    pub fn get_real(&self, column: &str) -> Option<f64> {
        self.get(column).and_then(|v| match v {
            crate::value::DbValue::Real(f) => Some(*f),
            _ => None,
        })
    }

    pub fn get_boolean(&self, column: &str) -> Option<bool> {
        self.get(column).and_then(|v| match v {
            crate::value::DbValue::Boolean(b) => Some(*b),
            _ => None,
        })
    }

    pub fn get_blob(&self, column: &str) -> Option<Vec<u8>> {
        self.get(column).and_then(|v| match v {
            crate::value::DbValue::Blob(b) => Some(b.clone()),
            _ => None,
        })
    }

    pub fn columns(&self) -> impl Iterator<Item = (&String, &DbValue)> {
        self.columns.iter()
    }

    pub fn len(&self) -> usize {
        self.columns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

impl Default for DbRow {
    fn default() -> Self {
        Self::new()
    }
}
