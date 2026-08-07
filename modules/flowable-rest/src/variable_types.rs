//! Shared Java-compatible REST variable type mapping.
//!
//! The engine stores variables as `serde_json::Value`, so it does not retain
//! Java's `Integer` versus `Long` runtime class. REST responses recover the
//! distinction deterministically from the integer's i32 range. Explicit REST
//! types are converted before values cross into the engine.

use crate::error::ApiError;
use serde_json::{Number, Value};

pub(crate) fn rest_variable_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if integer_fits_i32(number) => "integer",
        Value::Number(number) if number.is_i64() || number.is_u64() => "long",
        Value::Number(_) => "double",
        Value::String(_) => "string",
        Value::Array(_) | Value::Object(_) => "json",
    }
}

pub(crate) fn convert_explicit_variable_value(
    variable_name: Option<&str>,
    variable_type: Option<&str>,
    value: &Value,
) -> Result<Value, ApiError> {
    let Some(variable_type) = variable_type else {
        return Ok(value.clone());
    };

    // Every Java RestVariableConverter accepts null. Validate the converter
    // name first so an unknown explicit type never bypasses validation.
    let known_type = matches!(
        variable_type,
        "string" | "integer" | "long" | "double" | "boolean" | "json"
    );
    if !known_type {
        return Err(unsupported_type(variable_name, variable_type));
    }
    if value.is_null() {
        return Ok(Value::Null);
    }

    match variable_type {
        "integer" => value
            .as_number()
            .map(|number| Value::Number(Number::from(number_to_i32(number))))
            .ok_or_else(|| converter_error("Converter can only convert integers")),
        "long" => value
            .as_number()
            .map(|number| Value::Number(Number::from(number_to_i64(number))))
            .ok_or_else(|| converter_error("Converter can only convert longs")),
        "double" => value
            .as_number()
            .and_then(|number| Number::from_f64(number.as_f64()?))
            .map(Value::Number)
            .ok_or_else(|| converter_error("Converter can only convert doubles")),
        "boolean" => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| converter_error("Converter can only convert booleans")),
        "string" => value
            .as_str()
            .map(|value| Value::String(value.to_string()))
            .ok_or_else(|| converter_error("Converter can only convert strings")),
        // Java JsonObjectRestVariableConverter converts Map/List values to a
        // JsonNode and returns all other JSON scalar values unchanged.
        "json" => Ok(value.clone()),
        _ => Err(unsupported_type(variable_name, variable_type)),
    }
}

fn integer_fits_i32(number: &Number) -> bool {
    number
        .as_i64()
        .is_some_and(|value| i32::try_from(value).is_ok())
        || number
            .as_u64()
            .is_some_and(|value| value <= i32::MAX as u64)
}

fn number_to_i32(number: &Number) -> i32 {
    if let Some(value) = number.as_i64() {
        value as i32
    } else if let Some(value) = number.as_u64() {
        value as i32
    } else {
        number.as_f64().unwrap_or_default() as i32
    }
}

fn number_to_i64(number: &Number) -> i64 {
    if let Some(value) = number.as_i64() {
        value
    } else if let Some(value) = number.as_u64() {
        value as i64
    } else {
        number.as_f64().unwrap_or_default() as i64
    }
}

fn unsupported_type(variable_name: Option<&str>, variable_type: &str) -> ApiError {
    ApiError::bad_request(format!(
        "Variable '{}' has unsupported type: '{}'.",
        variable_name.unwrap_or("null"),
        variable_type
    ))
}

fn converter_error(message: &str) -> ApiError {
    ApiError::bad_request(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integer_response_type_uses_i32_boundaries() {
        assert_eq!(rest_variable_type(&json!(i32::MIN)), "integer");
        assert_eq!(rest_variable_type(&json!(i32::MAX)), "integer");
        assert_eq!(rest_variable_type(&json!(i32::MIN as i64 - 1)), "long");
        assert_eq!(rest_variable_type(&json!(i32::MAX as i64 + 1)), "long");
    }

    #[test]
    fn numeric_converters_follow_java_number_value_methods() {
        assert_eq!(
            convert_explicit_variable_value(Some("v"), Some("integer"), &json!(42.9)).unwrap(),
            json!(42)
        );
        assert_eq!(
            convert_explicit_variable_value(
                Some("v"),
                Some("integer"),
                &json!(i32::MAX as i64 + 1),
            )
            .unwrap(),
            json!(i32::MIN)
        );
        assert_eq!(
            convert_explicit_variable_value(Some("v"), Some("long"), &json!(2_147_483_648.9_f64),)
                .unwrap(),
            json!(2_147_483_648_i64)
        );
    }

    #[test]
    fn unknown_type_is_rejected_even_for_null() {
        let error =
            convert_explicit_variable_value(Some("v"), Some("mystery"), &Value::Null).unwrap_err();
        assert!(matches!(
            error,
            ApiError::BadRequest(message)
                if message == "Variable 'v' has unsupported type: 'mystery'."
        ));
    }

    #[test]
    fn json_converter_accepts_collections_and_scalars() {
        for value in [
            json!({ "answer": 42 }),
            json!([1, 2]),
            json!("scalar"),
            json!(7),
        ] {
            assert_eq!(
                convert_explicit_variable_value(Some("v"), Some("json"), &value).unwrap(),
                value
            );
        }
    }
}
