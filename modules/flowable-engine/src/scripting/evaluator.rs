use crate::error::FlowableError;
use crate::scripting::ast::*;
use crate::scripting::secure_context::SecureScriptContext;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Maximum loop iterations to prevent infinite loops.
const MAX_LOOP_ITERATIONS: usize = 10_000;
/// Maximum call stack depth to prevent stack overflow.
const MAX_CALL_DEPTH: usize = 100;

/// A user-defined function captured during execution.
#[derive(Debug, Clone)]
struct UserFunction {
    params: Vec<String>,
    body: Vec<Statement>,
}

/// Evaluator that walks the AST and executes statements in a secure context.
pub struct Evaluator<'a> {
    context: &'a mut SecureScriptContext,
    functions: HashMap<String, UserFunction>,
    call_depth: usize,
}

impl<'a> Evaluator<'a> {
    pub fn new(context: &'a mut SecureScriptContext) -> Self {
        Self {
            context,
            functions: HashMap::new(),
            call_depth: 0,
        }
    }

    /// Execute a list of statements, returning the last expression value.
    pub fn execute(&mut self, statements: &[Statement]) -> Result<Option<Value>, FlowableError> {
        let mut last = None;
        for stmt in statements {
            match self.execute_statement(stmt) {
                Ok(val) => last = val,
                Err(e) => {
                    if let Some(value) = return_signal_value(&e) {
                        return Ok(Some(value));
                    }
                    return Err(e);
                }
            }
        }
        Ok(last)
    }

    fn execute_statement(&mut self, stmt: &Statement) -> Result<Option<Value>, FlowableError> {
        match stmt {
            Statement::VarDecl { name, initializer } => {
                let value = match initializer {
                    Some(expr) => self.evaluate_expression(expr)?,
                    None => Value::Null,
                };
                self.context.set_result_variable(name.clone(), value);
                Ok(None)
            }
            Statement::ExpressionStmt(expr) => {
                let val = self.evaluate_expression(expr)?;
                Ok(Some(val))
            }
            Statement::IfStmt {
                condition,
                then_body,
                else_body,
            } => {
                let cond = self.evaluate_expression(condition)?;
                if is_truthy(&cond) {
                    self.execute_block(then_body)
                } else if let Some(else_stmts) = else_body {
                    self.execute_block(else_stmts)
                } else {
                    Ok(None)
                }
            }
            Statement::ForStmt {
                init,
                condition,
                update,
                body,
            } => {
                if let Some(init_stmt) = init {
                    self.execute_statement(init_stmt)?;
                }
                let mut iterations = 0;
                loop {
                    if iterations >= MAX_LOOP_ITERATIONS {
                        return Err(FlowableError::ExecutionError(
                            "Script exceeded maximum loop iterations (10000)".to_string(),
                        ));
                    }
                    if let Some(cond) = condition {
                        let val = self.evaluate_expression(cond)?;
                        if !is_truthy(&val) {
                            break;
                        }
                    }
                    match self.execute_block(body) {
                        Ok(_) => {}
                        Err(e) => return Err(e),
                    }
                    if let Some(upd) = update {
                        self.evaluate_expression(upd)?;
                    }
                    iterations += 1;
                }
                Ok(None)
            }
            Statement::WhileStmt { condition, body } => {
                let mut iterations = 0;
                loop {
                    if iterations >= MAX_LOOP_ITERATIONS {
                        return Err(FlowableError::ExecutionError(
                            "Script exceeded maximum loop iterations (10000)".to_string(),
                        ));
                    }
                    let val = self.evaluate_expression(condition)?;
                    if !is_truthy(&val) {
                        break;
                    }
                    self.execute_block(body)?;
                    iterations += 1;
                }
                Ok(None)
            }
            Statement::FunctionDecl { name, params, body } => {
                self.functions.insert(
                    name.clone(),
                    UserFunction {
                        params: params.clone(),
                        body: body.clone(),
                    },
                );
                Ok(None)
            }
            Statement::ReturnStmt(value) => {
                let val = match value {
                    Some(expr) => self.evaluate_expression(expr)?,
                    None => Value::Null,
                };
                Err(FlowableError::ExecutionError(format!(
                    "__RETURN__:{}",
                    serde_json::to_string(&val).unwrap_or_else(|_| "null".to_string())
                )))
            }
            Statement::Block(stmts) => self.execute_block(stmts),
        }
    }

    fn execute_block(&mut self, stmts: &[Statement]) -> Result<Option<Value>, FlowableError> {
        let mut last = None;
        for stmt in stmts {
            last = self.execute_statement(stmt)?;
        }
        Ok(last)
    }

    // ── Expression evaluation ───────────────────────────────

    fn evaluate_expression(&mut self, expr: &Expression) -> Result<Value, FlowableError> {
        match expr {
            Expression::Literal(val) => Ok(val.clone()),
            Expression::Variable(name) => Ok(self
                .context
                .get_variable(name)
                .cloned()
                .unwrap_or(Value::Null)),
            Expression::ArrayLiteral(elements) => {
                let mut arr = Vec::new();
                for elem in elements {
                    arr.push(self.evaluate_expression(elem)?);
                }
                Ok(Value::Array(arr))
            }
            Expression::ObjectLiteral(entries) => {
                let mut map = Map::new();
                for (key, val_expr) in entries {
                    let val = self.evaluate_expression(val_expr)?;
                    map.insert(key.clone(), val);
                }
                Ok(Value::Object(map))
            }
            Expression::BinaryOp {
                left,
                operator,
                right,
            } => {
                let left_val = self.evaluate_expression(left)?;
                // Short-circuit for logical operators
                match operator {
                    BinaryOperator::And => {
                        if !is_truthy(&left_val) {
                            return Ok(left_val);
                        }
                        return self.evaluate_expression(right);
                    }
                    BinaryOperator::Or => {
                        if is_truthy(&left_val) {
                            return Ok(left_val);
                        }
                        return self.evaluate_expression(right);
                    }
                    _ => {}
                }
                let right_val = self.evaluate_expression(right)?;
                apply_binary_op(*operator, &left_val, &right_val)
            }
            Expression::UnaryOp { operator, operand } => {
                let val = self.evaluate_expression(operand)?;
                match operator {
                    UnaryOperator::Not => Ok(Value::Bool(!is_truthy(&val))),
                    UnaryOperator::Negate => {
                        let n = value_to_f64(&val).ok_or_else(|| {
                            FlowableError::ExecutionError(format!(
                                "Cannot negate non-numeric value: {:?}",
                                val
                            ))
                        })?;
                        Ok(f64_to_value(-n))
                    }
                }
            }
            Expression::PropertyAccess { object, property } => {
                let obj = self.evaluate_expression(object)?;
                Ok(access_property(&obj, property))
            }
            Expression::IndexAccess { object, index } => {
                let obj = self.evaluate_expression(object)?;
                let idx = self.evaluate_expression(index)?;
                Ok(access_index(&obj, &idx))
            }
            Expression::FunctionCall { callee, arguments } => {
                // Evaluate arguments first
                let mut args = Vec::new();
                for arg in arguments {
                    args.push(self.evaluate_expression(arg)?);
                }
                self.call_function(callee, &args)
            }
            Expression::Assignment { name, value } => {
                let val = self.evaluate_expression(value)?;
                self.context.set_result_variable(name.clone(), val.clone());
                Ok(val)
            }
            Expression::CompoundAssignment {
                name,
                operator,
                value,
            } => {
                let current = self
                    .context
                    .get_variable(name)
                    .cloned()
                    .unwrap_or(Value::Null);
                let rhs = self.evaluate_expression(value)?;
                let result = apply_binary_op(*operator, &current, &rhs)?;
                self.context
                    .set_result_variable(name.clone(), result.clone());
                Ok(result)
            }
        }
    }

    fn call_function(
        &mut self,
        callee: &Expression,
        args: &[Value],
    ) -> Result<Value, FlowableError> {
        // Check for built-in functions first
        match callee {
            Expression::Variable(name) => {
                if let Some(result) = call_builtin(name, args)? {
                    return Ok(result);
                }
                // User-defined function
                if let Some(func) = self.functions.get(name).cloned() {
                    return self.call_user_function(&func, args);
                }
                Err(FlowableError::ExecutionError(format!(
                    "Undefined function: '{}'",
                    name
                )))
            }
            Expression::PropertyAccess { object, property } => {
                if let Expression::Variable(namespace) = object.as_ref()
                    && namespace == "execution"
                    && property == "setVariable"
                {
                    let name = args.first().and_then(Value::as_str).ok_or_else(|| {
                        FlowableError::ExecutionError(
                            "execution.setVariable requires a string variable name".to_string(),
                        )
                    })?;
                    let value = args.get(1).cloned().unwrap_or(Value::Null);
                    self.context
                        .set_result_variable(name.to_string(), value.clone());
                    return Ok(value);
                }
                let obj = self.evaluate_expression(object)?;
                // Built-in method calls: Math.floor, String.length, Array.length, etc.
                if let Some(result) = call_method(&obj, property, args)? {
                    return Ok(result);
                }
                // Namespace built-ins: Math.floor(x), Math.ceil(x), etc.
                if let Expression::Variable(ns) = object.as_ref()
                    && let Some(result) = call_namespace_function(ns, property, args)?
                {
                    return Ok(result);
                }
                Err(FlowableError::ExecutionError(format!(
                    "Undefined method: '.{}'",
                    property
                )))
            }
            _ => Err(FlowableError::ExecutionError(
                "Expression is not callable".to_string(),
            )),
        }
    }

    fn call_user_function(
        &mut self,
        func: &UserFunction,
        args: &[Value],
    ) -> Result<Value, FlowableError> {
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(FlowableError::ExecutionError(
                "Script exceeded maximum call depth (100)".to_string(),
            ));
        }
        self.call_depth += 1;

        // Bind parameters
        for (i, param) in func.params.iter().enumerate() {
            let val = args.get(i).cloned().unwrap_or(Value::Null);
            self.context.set_result_variable(param.clone(), val);
        }

        // Execute body, catching return signals
        let result = match self.execute_block(&func.body) {
            Ok(val) => Ok(val.unwrap_or(Value::Null)),
            Err(e) => {
                let msg = format!("{}", e);
                if msg.contains("__RETURN__:") {
                    // Extract the JSON payload after __RETURN__:
                    let json_str = msg.split_once("__RETURN__:").map(|x| x.1).unwrap_or("null");
                    let val: Value = serde_json::from_str(json_str).unwrap_or(Value::Null);
                    Ok(val)
                } else {
                    Err(e)
                }
            }
        };

        self.call_depth -= 1;
        result
    }
}

fn return_signal_value(error: &FlowableError) -> Option<Value> {
    let FlowableError::ExecutionError(message) = error else {
        return None;
    };
    let payload = message.strip_prefix("__RETURN__:")?;
    serde_json::from_str(payload).ok()
}

// ── Helpers ─────────────────────────────────────────────────

fn is_truthy(val: &Value) -> bool {
    match val {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        Value::Bool(true) => Some(1.0),
        Value::Bool(false) => Some(0.0),
        _ => None,
    }
}

fn f64_to_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
        Value::Number((n as i64).into())
    } else if let Some(num) = serde_json::Number::from_f64(n) {
        Value::Number(num)
    } else {
        Value::Null
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    // Numeric comparison with type coercion
    if let (Some(an), Some(bn)) = (value_to_f64(a), value_to_f64(b))
        && (a.is_number() || b.is_number())
    {
        return (an - bn).abs() < f64::EPSILON;
    }
    a == b
}

fn apply_binary_op(
    op: BinaryOperator,
    left: &Value,
    right: &Value,
) -> Result<Value, FlowableError> {
    match op {
        BinaryOperator::Add => {
            // String concatenation
            if left.is_string() || right.is_string() {
                let l = value_to_string(left);
                let r = value_to_string(right);
                return Ok(Value::String(format!("{}{}", l, r)));
            }
            numeric_op("+", left, right, |a, b| a + b)
        }
        BinaryOperator::Sub => numeric_op("-", left, right, |a, b| a - b),
        BinaryOperator::Mul => numeric_op("*", left, right, |a, b| a * b),
        BinaryOperator::Div => {
            let r = value_to_f64(right).ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Cannot divide by non-numeric value: {:?}",
                    right
                ))
            })?;
            if r == 0.0 {
                return Err(FlowableError::ExecutionError(
                    "Division by zero".to_string(),
                ));
            }
            numeric_op("/", left, right, |a, b| a / b)
        }
        BinaryOperator::Mod => {
            let r = value_to_f64(right).ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Cannot modulo by non-numeric value: {:?}",
                    right
                ))
            })?;
            if r == 0.0 {
                return Err(FlowableError::ExecutionError("Modulo by zero".to_string()));
            }
            numeric_op("%", left, right, |a, b| a % b)
        }
        BinaryOperator::Eq => Ok(Value::Bool(values_equal(left, right))),
        BinaryOperator::NotEq => Ok(Value::Bool(!values_equal(left, right))),
        BinaryOperator::Lt => compare_op(left, right, |a, b| a < b),
        BinaryOperator::Gt => compare_op(left, right, |a, b| a > b),
        BinaryOperator::LtEq => compare_op(left, right, |a, b| a <= b),
        BinaryOperator::GtEq => compare_op(left, right, |a, b| a >= b),
        BinaryOperator::And | BinaryOperator::Or => {
            // Should be handled by short-circuit in evaluate_expression
            Err(FlowableError::ExecutionError(
                "Logical operators should be short-circuited in evaluate_expression".to_string(),
            ))
        }
    }
}

fn numeric_op(
    op_name: &str,
    left: &Value,
    right: &Value,
    f: impl Fn(f64, f64) -> f64,
) -> Result<Value, FlowableError> {
    let l = value_to_f64(left).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "Cannot apply '{}' to non-numeric value: {:?}",
            op_name, left
        ))
    })?;
    let r = value_to_f64(right).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "Cannot apply '{}' to non-numeric value: {:?}",
            op_name, right
        ))
    })?;
    Ok(f64_to_value(f(l, r)))
}

fn compare_op(
    left: &Value,
    right: &Value,
    f: impl Fn(f64, f64) -> bool,
) -> Result<Value, FlowableError> {
    let l = value_to_f64(left).ok_or_else(|| {
        FlowableError::ExecutionError(format!("Cannot compare non-numeric value: {:?}", left))
    })?;
    let r = value_to_f64(right).ok_or_else(|| {
        FlowableError::ExecutionError(format!("Cannot compare non-numeric value: {:?}", right))
    })?;
    Ok(Value::Bool(f(l, r)))
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else {
                n.to_string()
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn access_property(obj: &Value, property: &str) -> Value {
    match obj {
        Value::Object(map) => map.get(property).cloned().unwrap_or(Value::Null),
        Value::Array(arr) => {
            if property == "length" {
                Value::Number((arr.len() as i64).into())
            } else {
                Value::Null
            }
        }
        Value::String(s) => {
            if property == "length" {
                Value::Number((s.len() as i64).into())
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    }
}

fn access_index(obj: &Value, index: &Value) -> Value {
    match obj {
        Value::Array(arr) => {
            if let Some(i) = index.as_i64() {
                arr.get(i as usize).cloned().unwrap_or(Value::Null)
            } else if let Some(i) = index.as_f64() {
                arr.get(i as usize).cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        Value::Object(map) => {
            if let Some(key) = index.as_str() {
                map.get(key).cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    }
}

// ── Built-in functions ──────────────────────────────────────

fn call_builtin(name: &str, args: &[Value]) -> Result<Option<Value>, FlowableError> {
    match name {
        "parseInt" => {
            let val = args.first().unwrap_or(&Value::Null);
            match val {
                Value::Number(n) => Ok(Some(Value::Number(
                    (n.as_f64().unwrap_or(0.0) as i64).into(),
                ))),
                Value::String(s) => {
                    let n = s.parse::<f64>().unwrap_or(0.0) as i64;
                    Ok(Some(Value::Number(n.into())))
                }
                _ => Ok(Some(Value::Number(0.into()))),
            }
        }
        "parseFloat" => {
            let val = args.first().unwrap_or(&Value::Null);
            match val {
                Value::String(s) => {
                    let n = s.parse::<f64>().unwrap_or(0.0);
                    Ok(Some(f64_to_value(n)))
                }
                Value::Number(n) => Ok(Some(Value::Number(n.clone()))),
                _ => Ok(Some(f64_to_value(0.0))),
            }
        }
        "String" => {
            let val = args.first().unwrap_or(&Value::Null);
            Ok(Some(Value::String(value_to_string(val))))
        }
        "Number" => {
            let val = args.first().unwrap_or(&Value::Null);
            let n = value_to_f64(val).unwrap_or(0.0);
            Ok(Some(f64_to_value(n)))
        }
        "isNaN" => {
            let val = args.first().unwrap_or(&Value::Null);
            Ok(Some(Value::Bool(value_to_f64(val).is_none())))
        }
        _ => Ok(None),
    }
}

fn call_namespace_function(
    namespace: &str,
    method: &str,
    args: &[Value],
) -> Result<Option<Value>, FlowableError> {
    if namespace == "Math" {
        let val = args.first().and_then(value_to_f64).unwrap_or(0.0);
        let result = match method {
            "floor" => Some(val.floor()),
            "ceil" => Some(val.ceil()),
            "round" => Some(val.round()),
            "abs" => Some(val.abs()),
            "sqrt" => Some(val.sqrt()),
            "pow" => {
                let exp = args.get(1).and_then(value_to_f64).unwrap_or(1.0);
                Some(val.powf(exp))
            }
            "min" => {
                let b = args.get(1).and_then(value_to_f64).unwrap_or(val);
                Some(val.min(b))
            }
            "max" => {
                let b = args.get(1).and_then(value_to_f64).unwrap_or(val);
                Some(val.max(b))
            }
            "random" => Some(0.5), // deterministic in sandbox
            _ => None,
        };
        return Ok(result.map(f64_to_value));
    }
    Ok(None)
}

fn call_method(obj: &Value, method: &str, args: &[Value]) -> Result<Option<Value>, FlowableError> {
    match obj {
        Value::String(s) => match method {
            "length" => Ok(Some(Value::Number((s.len() as i64).into()))),
            "toUpperCase" => Ok(Some(Value::String(s.to_uppercase()))),
            "toLowerCase" => Ok(Some(Value::String(s.to_lowercase()))),
            "trim" => Ok(Some(Value::String(s.trim().to_string()))),
            "indexOf" => {
                let needle = args.first().and_then(Value::as_str).unwrap_or("");
                Ok(Some(Value::Number(
                    (s.find(needle).map(|i| i as i64).unwrap_or(-1)).into(),
                )))
            }
            "substring" | "substr" => {
                let start = args.first().and_then(value_to_f64).unwrap_or(0.0) as usize;
                let end = args
                    .get(1)
                    .and_then(value_to_f64)
                    .map(|e| e as usize)
                    .unwrap_or(s.len());
                let start = start.min(s.len());
                let end = end.min(s.len());
                Ok(Some(Value::String(s[start..end].to_string())))
            }
            "charAt" => {
                let idx = args.first().and_then(value_to_f64).unwrap_or(0.0) as usize;
                Ok(Some(Value::String(
                    s.chars()
                        .nth(idx)
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                )))
            }
            "split" => {
                let sep = args.first().and_then(Value::as_str).unwrap_or(",");
                let parts: Vec<Value> = s
                    .split(sep)
                    .map(|part| Value::String(part.to_string()))
                    .collect();
                Ok(Some(Value::Array(parts)))
            }
            "replace" => {
                let from = args.first().and_then(Value::as_str).unwrap_or("");
                let to = args.get(1).and_then(Value::as_str).unwrap_or("");
                Ok(Some(Value::String(s.replacen(from, to, 1))))
            }
            "startsWith" => {
                let prefix = args.first().and_then(Value::as_str).unwrap_or("");
                Ok(Some(Value::Bool(s.starts_with(prefix))))
            }
            "endsWith" => {
                let suffix = args.first().and_then(Value::as_str).unwrap_or("");
                Ok(Some(Value::Bool(s.ends_with(suffix))))
            }
            "includes" | "contains" => {
                let needle = args.first().and_then(Value::as_str).unwrap_or("");
                Ok(Some(Value::Bool(s.contains(needle))))
            }
            _ => Ok(None),
        },
        Value::Array(arr) => match method {
            "length" => Ok(Some(Value::Number((arr.len() as i64).into()))),
            "push" => {
                // Note: arrays are immutable in JSON; push returns new length
                let new_len = arr.len() + args.len();
                Ok(Some(Value::Number((new_len as i64).into())))
            }
            "indexOf" => {
                let needle = args.first().unwrap_or(&Value::Null);
                let idx = arr
                    .iter()
                    .position(|v| v == needle)
                    .map(|i| i as i64)
                    .unwrap_or(-1);
                Ok(Some(Value::Number(idx.into())))
            }
            "join" => {
                let sep = args.first().and_then(Value::as_str).unwrap_or(",");
                let joined: String = arr
                    .iter()
                    .map(value_to_string)
                    .collect::<Vec<_>>()
                    .join(sep);
                Ok(Some(Value::String(joined)))
            }
            "includes" | "contains" => {
                let needle = args.first().unwrap_or(&Value::Null);
                Ok(Some(Value::Bool(arr.contains(needle))))
            }
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}
