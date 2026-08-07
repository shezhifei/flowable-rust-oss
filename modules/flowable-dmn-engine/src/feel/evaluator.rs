use super::ast::{BinaryOp, Expr, UnaryOp};
use crate::error::DmnError;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub fn evaluate(expr: &Expr, context: &HashMap<String, Value>) -> Result<Value, DmnError> {
    let mut scope = context.clone();
    evaluate_inner(expr, &mut scope)
}

fn evaluate_inner(expr: &Expr, scope: &mut HashMap<String, Value>) -> Result<Value, DmnError> {
    match expr {
        Expr::Null => Ok(Value::Null),
        Expr::Bool(value) => Ok(Value::Bool(*value)),
        Expr::Number(value) => Ok(numeric_value(*value)),
        Expr::String(value) => Ok(Value::String(value.clone())),
        Expr::Name(name) => Ok(scope.get(name).cloned().unwrap_or(Value::Null)),
        Expr::List(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| evaluate_inner(item, scope))
                .collect::<Result<_, _>>()?,
        )),
        Expr::Context(entries) => {
            let mut object = Map::new();
            for (key, expression) in entries {
                object.insert(key.clone(), evaluate_inner(expression, scope)?);
            }
            Ok(Value::Object(object))
        }
        Expr::Range {
            start,
            end,
            start_inclusive,
            end_inclusive,
        } => Ok(json!({
            "start": evaluate_inner(start, scope)?, "end": evaluate_inner(end, scope)?,
            "startInclusive": start_inclusive, "endInclusive": end_inclusive
        })),
        Expr::Unary {
            op: UnaryOp::Negate,
            expr,
        } => Ok(json!(-number(&evaluate_inner(expr, scope)?)?)),
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
        } => Ok(Value::Bool(!truthy(&evaluate_inner(expr, scope)?))),
        Expr::Binary { op, left, right } => evaluate_binary(*op, left, right, scope),
        Expr::If {
            condition,
            then_expr,
            else_expr,
        } => {
            if truthy(&evaluate_inner(condition, scope)?) {
                evaluate_inner(then_expr, scope)
            } else {
                evaluate_inner(else_expr, scope)
            }
        }
        Expr::Path { target, key } => Ok(evaluate_inner(target, scope)?
            .get(key)
            .cloned()
            .unwrap_or(Value::Null)),
        Expr::Filter { target, predicate } => {
            let values = evaluate_inner(target, scope)?
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut result = Vec::new();
            for value in values {
                scope.insert("item".to_string(), value.clone());
                if truthy(&evaluate_inner(predicate, scope)?) {
                    result.push(value);
                }
            }
            Ok(Value::Array(result))
        }
        Expr::Call { name, args } => evaluate_call(name, args, scope),
        Expr::For {
            variable,
            input,
            body,
        } => {
            let values = evaluate_inner(input, scope)?
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut result = Vec::new();
            for value in values {
                scope.insert(variable.clone(), value);
                result.push(evaluate_inner(body, scope)?);
            }
            Ok(Value::Array(result))
        }
        Expr::Quantified {
            every,
            variable,
            input,
            predicate,
        } => {
            let values = evaluate_inner(input, scope)?
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut result = *every;
            for value in values {
                scope.insert(variable.clone(), value);
                let current = truthy(&evaluate_inner(predicate, scope)?);
                if (*every && !current) || (!*every && current) {
                    result = current;
                    break;
                }
            }
            Ok(Value::Bool(result))
        }
    }
}

fn evaluate_binary(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    scope: &mut HashMap<String, Value>,
) -> Result<Value, DmnError> {
    if matches!(op, BinaryOp::And | BinaryOp::Or) {
        let left = truthy(&evaluate_inner(left, scope)?);
        if matches!(op, BinaryOp::And) && !left {
            return Ok(Value::Bool(false));
        }
        if matches!(op, BinaryOp::Or) && left {
            return Ok(Value::Bool(true));
        }
        return Ok(Value::Bool(truthy(&evaluate_inner(right, scope)?)));
    }
    let left = evaluate_inner(left, scope)?;
    let right = evaluate_inner(right, scope)?;
    match op {
        BinaryOp::Add => arithmetic_value(&left, &right, |a, b| a + b),
        BinaryOp::Subtract => arithmetic_value(&left, &right, |a, b| a - b),
        BinaryOp::Multiply => arithmetic_value(&left, &right, |a, b| a * b),
        BinaryOp::Divide => {
            let divisor = number(&right)?;
            if divisor == 0.0 {
                Err(DmnError::execution("division by zero"))
            } else {
                Ok(json!(number(&left)? / divisor))
            }
        }
        BinaryOp::Power => Ok(json!(number(&left)?.powf(number(&right)?))),
        BinaryOp::Equal => Ok(Value::Bool(left == right)),
        BinaryOp::NotEqual => Ok(Value::Bool(left != right)),
        BinaryOp::Less => compare(&left, &right, |a, b| a < b),
        BinaryOp::LessEqual => compare(&left, &right, |a, b| a <= b),
        BinaryOp::Greater => compare(&left, &right, |a, b| a > b),
        BinaryOp::GreaterEqual => compare(&left, &right, |a, b| a >= b),
        BinaryOp::In => Ok(Value::Bool(
            right
                .as_array()
                .map(|items| items.contains(&left))
                .unwrap_or(false),
        )),
        BinaryOp::And | BinaryOp::Or => unreachable!(),
    }
}

fn evaluate_call(
    name: &str,
    args: &[Expr],
    scope: &mut HashMap<String, Value>,
) -> Result<Value, DmnError> {
    let values = args
        .iter()
        .map(|arg| evaluate_inner(arg, scope))
        .collect::<Result<Vec<_>, _>>()?;
    let first = values.first().unwrap_or(&Value::Null);
    match name.to_ascii_lowercase().as_str() {
        "contains" => {
            Ok(Value::Bool(first.as_str().unwrap_or_default().contains(
                values.get(1).and_then(Value::as_str).unwrap_or_default(),
            )))
        }
        "list contains" => Ok(Value::Bool(
            first
                .as_array()
                .map(|items| {
                    values
                        .get(1)
                        .map(|needle| items.contains(needle))
                        .unwrap_or(false)
                })
                .unwrap_or(false),
        )),
        "string length" => Ok(json!(first.as_str().unwrap_or_default().chars().count())),
        "upper case" => Ok(Value::String(
            first.as_str().unwrap_or_default().to_uppercase(),
        )),
        "lower case" => Ok(Value::String(
            first.as_str().unwrap_or_default().to_lowercase(),
        )),
        "abs" => Ok(json!(number(first)?.abs())),
        "floor" => Ok(json!(number(first)?.floor())),
        "ceiling" | "ceil" => Ok(json!(number(first)?.ceil())),
        "count" => Ok(json!(
            first.as_array().map(Vec::len).unwrap_or(values.len())
        )),
        "sum" => Ok(json!(
            first
                .as_array()
                .map(|items| items.iter().filter_map(Value::as_f64).sum::<f64>())
                .unwrap_or(0.0)
        )),
        "append" => {
            let mut result = first.as_array().cloned().unwrap_or_default();
            result.extend(values.into_iter().skip(1));
            Ok(Value::Array(result))
        }
        _ => Err(DmnError::unsupported(
            "FEEL function",
            format!("unsupported FEEL function '{name}'"),
        )),
    }
}

fn number(value: &Value) -> Result<f64, DmnError> {
    value
        .as_f64()
        .ok_or_else(|| DmnError::execution("FEEL arithmetic requires numeric values"))
}
fn truthy(value: &Value) -> bool {
    value.as_bool().unwrap_or(false)
}
fn compare(
    left: &Value,
    right: &Value,
    predicate: impl Fn(f64, f64) -> bool,
) -> Result<Value, DmnError> {
    Ok(Value::Bool(predicate(number(left)?, number(right)?)))
}
fn numeric_value(value: f64) -> Value {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        Value::from(value as i64)
    } else {
        json!(value)
    }
}
fn arithmetic_value(
    left: &Value,
    right: &Value,
    operation: impl Fn(f64, f64) -> f64,
) -> Result<Value, DmnError> {
    Ok(numeric_value(operation(number(left)?, number(right)?)))
}
