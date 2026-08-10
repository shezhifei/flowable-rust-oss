//! UI task RestVariable JSON contract (Java RestVariable + converters).
//!
//! Engine REST variables use a different shape; this is the task-app form:
//! `{ "name", "type", "value", "scope?", "valueUrl?" }`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RestVariableScope {
    Local,
    Global,
}

impl RestVariableScope {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "global" => Some(Self::Global),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestVariable {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_url: Option<String>,
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct RestVariableError(pub String);

const BINARY: &str = "binary";
const SERIALIZABLE: &str = "serializable";

/// Convert an engine-side value into the UI RestVariable shape.
pub fn create_rest_variable(
    name: impl Into<String>,
    value: Option<Value>,
    scope: Option<RestVariableScope>,
    include_binary_value: bool,
) -> RestVariable {
    let mut rest = RestVariable {
        name: name.into(),
        r#type: None,
        value: None,
        scope: scope.map(|s| match s {
            RestVariableScope::Local => "local".into(),
            RestVariableScope::Global => "global".into(),
        }),
        value_url: None,
    };

    let Some(v) = value else {
        return rest;
    };

    match &v {
        Value::String(s) => {
            // Prefer date if ISO-8601-ish, else string.
            if parse_iso_date(s).is_some() && looks_like_date(s) {
                rest.r#type = Some("date".into());
                rest.value = Some(Value::String(s.clone()));
            } else {
                rest.r#type = Some("string".into());
                rest.value = Some(Value::String(s.clone()));
            }
        }
        Value::Bool(b) => {
            rest.r#type = Some("boolean".into());
            rest.value = Some(Value::Bool(*b));
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    // Integer vs long / short heuristics matching Java converters
                    // (Integer type is preferred when it fits i32).
                    rest.r#type = Some("integer".into());
                    rest.value = Some(Value::Number(i.into()));
                } else {
                    rest.r#type = Some("long".into());
                    rest.value = Some(Value::Number(i.into()));
                }
            } else if let Some(f) = n.as_f64() {
                rest.r#type = Some("double".into());
                rest.value = Some(Value::from(f));
            } else {
                rest.r#type = Some(SERIALIZABLE.into());
                if include_binary_value {
                    rest.value = Some(v);
                }
            }
        }
        Value::Array(_) | Value::Object(_) => {
            rest.r#type = Some(SERIALIZABLE.into());
            if include_binary_value {
                rest.value = Some(v);
            }
        }
        Value::Null => {
            rest.value = None;
        }
    }
    rest
}

/// Extract engine value from a RestVariable using type converters.
pub fn rest_variable_value(var: &RestVariable) -> Result<Option<Value>, RestVariableError> {
    let Some(ref raw) = var.value else {
        return Ok(None);
    };
    let type_name = var.r#type.as_deref().unwrap_or("string");
    match type_name {
        "string" => {
            if raw.is_string() {
                Ok(Some(raw.clone()))
            } else {
                Err(RestVariableError(
                    "Converter can only convert strings".into(),
                ))
            }
        }
        "integer" => match raw {
            Value::Number(n) => n
                .as_i64()
                .map(|i| Some(Value::Number((i as i32).into())))
                .ok_or_else(|| RestVariableError("Converter can only convert integers".into())),
            _ => Err(RestVariableError(
                "Converter can only convert integers".into(),
            )),
        },
        "long" => match raw {
            Value::Number(n) => n
                .as_i64()
                .map(|i| Some(Value::Number(i.into())))
                .ok_or_else(|| RestVariableError("Converter can only convert longs".into())),
            _ => Err(RestVariableError(
                "Converter can only convert longs".into(),
            )),
        },
        "short" => match raw {
            Value::Number(n) => n
                .as_i64()
                .map(|i| Some(Value::Number((i as i16).into())))
                .ok_or_else(|| RestVariableError("Converter can only convert shorts".into())),
            _ => Err(RestVariableError(
                "Converter can only convert shorts".into(),
            )),
        },
        "double" => match raw {
            Value::Number(n) => n
                .as_f64()
                .map(|f| Some(Value::from(f)))
                .ok_or_else(|| RestVariableError("Converter can only convert doubles".into())),
            _ => Err(RestVariableError(
                "Converter can only convert doubles".into(),
            )),
        },
        "boolean" => match raw {
            Value::Bool(b) => Ok(Some(Value::Bool(*b))),
            _ => Err(RestVariableError(
                "Converter can only convert booleans".into(),
            )),
        },
        "date" => match raw {
            Value::String(s) => {
                parse_iso_date(s)
                    .map(|dt| Some(Value::String(dt.to_rfc3339())))
                    .ok_or_else(|| {
                        RestVariableError(format!(
                            "The given variable value is not a date: '{s}'"
                        ))
                    })
            }
            _ => Err(RestVariableError(
                "Converter can only convert string to date".into(),
            )),
        },
        BINARY | SERIALIZABLE => Ok(Some(raw.clone())),
        other => Err(RestVariableError(format!("Unknown variable type '{other}'"))),
    }
}

fn looks_like_date(s: &str) -> bool {
    s.contains('T') || (s.len() >= 10 && s.as_bytes().get(4) == Some(&b'-'))
}

fn parse_iso_date(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            // Accept date-only YYYY-MM-DD
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|n| DateTime::<Utc>::from_naive_utc_and_offset(n, Utc))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn string_roundtrip() {
        let rv = create_rest_variable("a", Some(json!("hello")), Some(RestVariableScope::Local), true);
        assert_eq!(rv.r#type.as_deref(), Some("string"));
        assert_eq!(rv.scope.as_deref(), Some("local"));
        let v = rest_variable_value(&rv).unwrap().unwrap();
        assert_eq!(v, json!("hello"));
    }

    #[test]
    fn integer_roundtrip() {
        let rv = create_rest_variable("n", Some(json!(42)), None, true);
        assert_eq!(rv.r#type.as_deref(), Some("integer"));
        let v = rest_variable_value(&rv).unwrap().unwrap();
        assert_eq!(v, json!(42));
    }

    #[test]
    fn boolean_roundtrip() {
        let rv = create_rest_variable("b", Some(json!(true)), None, true);
        assert_eq!(rv.r#type.as_deref(), Some("boolean"));
        assert_eq!(rest_variable_value(&rv).unwrap().unwrap(), json!(true));
    }

    #[test]
    fn serde_shape_omits_null_type() {
        let rv = RestVariable {
            name: "x".into(),
            r#type: None,
            value: Some(json!(1)),
            scope: None,
            value_url: None,
        };
        let s = serde_json::to_string(&rv).unwrap();
        assert!(!s.contains("\"type\""));
        assert!(s.contains("\"name\":\"x\""));
    }
}
