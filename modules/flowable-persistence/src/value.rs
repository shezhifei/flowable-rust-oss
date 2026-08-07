use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DbValue {
    Null,
    /// Typed nulls so Postgres can bind with the correct SQL type.
    NullInteger,
    NullBoolean,
    NullBlob,
    Text(String),
    Integer(i64),
    Real(f64),
    Boolean(bool),
    Blob(Vec<u8>),
}

impl DbValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            DbValue::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            DbValue::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_real(&self) -> Option<f64> {
        match self {
            DbValue::Real(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            DbValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_blob(&self) -> Option<&[u8]> {
        match self {
            DbValue::Blob(b) => Some(b),
            _ => None,
        }
    }
}

impl From<String> for DbValue {
    fn from(s: String) -> Self {
        DbValue::Text(s)
    }
}

impl From<&str> for DbValue {
    fn from(s: &str) -> Self {
        DbValue::Text(s.to_string())
    }
}

impl From<i64> for DbValue {
    fn from(i: i64) -> Self {
        DbValue::Integer(i)
    }
}

impl From<i32> for DbValue {
    fn from(i: i32) -> Self {
        DbValue::Integer(i as i64)
    }
}

impl From<f64> for DbValue {
    fn from(f: f64) -> Self {
        DbValue::Real(f)
    }
}

impl From<bool> for DbValue {
    fn from(b: bool) -> Self {
        DbValue::Boolean(b)
    }
}

impl From<Vec<u8>> for DbValue {
    fn from(b: Vec<u8>) -> Self {
        DbValue::Blob(b)
    }
}

impl From<Option<String>> for DbValue {
    fn from(opt: Option<String>) -> Self {
        match opt {
            Some(s) => DbValue::Text(s),
            None => DbValue::Null,
        }
    }
}

impl From<Option<&str>> for DbValue {
    fn from(opt: Option<&str>) -> Self {
        match opt {
            Some(s) => DbValue::Text(s.to_string()),
            None => DbValue::Null,
        }
    }
}

impl From<Option<i64>> for DbValue {
    fn from(opt: Option<i64>) -> Self {
        match opt {
            Some(i) => DbValue::Integer(i),
            None => DbValue::NullInteger,
        }
    }
}

impl From<Option<i32>> for DbValue {
    fn from(opt: Option<i32>) -> Self {
        match opt {
            Some(i) => DbValue::Integer(i as i64),
            None => DbValue::NullInteger,
        }
    }
}

impl From<Option<bool>> for DbValue {
    fn from(opt: Option<bool>) -> Self {
        match opt {
            Some(b) => DbValue::Boolean(b),
            None => DbValue::NullBoolean,
        }
    }
}

impl From<Option<Vec<u8>>> for DbValue {
    fn from(opt: Option<Vec<u8>>) -> Self {
        match opt {
            Some(b) => DbValue::Blob(b),
            None => DbValue::NullBlob,
        }
    }
}

impl From<Option<f64>> for DbValue {
    fn from(opt: Option<f64>) -> Self {
        match opt {
            Some(f) => DbValue::Real(f),
            None => DbValue::Null,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DbParams {
    pub values: Vec<DbValue>,
}

impl DbParams {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<V: Into<DbValue>>(&mut self, value: V) {
        self.values.push(value.into());
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
