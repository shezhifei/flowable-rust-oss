// Pre-existing `unwrap()` call(s), grandfathered by the workspace clippy ratchet
// (`[workspace.lints.clippy] unwrap_used = "warn"` in the root Cargo.toml). These
// sites predate the ratchet and were NOT individually audited against Java. The
// exemption is scoped with `cfg_attr(test, ...)`, so it covers only this file's
// `#[cfg(test)]` code; a NEW unwrap() in production code is still surfaced.
// Do not add more without an audit note.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use crate::el::variable_container::VariableContainer;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

pub trait Expression {
    fn get_value(&self, scope: &dyn VariableContainer) -> Option<serde_json::Value>;
}

/// Typed strict-evaluation failure, mirroring the Java exception split behind
/// `JuelExpression.getValue` (JuelExpression.java:53-60) plus the
/// `createExpression` compile step (DefaultExpressionManager.java:90).
///
/// F-group callers (user-task name/description/category/formKey) must
/// distinguish `CompileFailed` (Java `ELException` from `createExpression` —
/// propagates) from the eval-time `FlowableException` family (catch → fallback
/// to the model text + warn). See research N4-1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionEvalError {
    /// parse/compile failed before evaluation (Java createExpression).
    CompileFailed(String),
    /// Undefined variable or unresolvable property on a non-null base
    /// (Java PropertyNotFoundException → FlowableException "Unknown property…").
    UnknownProperty(String),
    /// Unregistered method / bean has no such method
    /// (Java MethodNotFoundException → FlowableException "Unknown method…").
    UnknownMethod(String),
    /// Registered method failed mid-evaluation, or another evaluation error
    /// (Java FlowableException / ExpressionException).
    EvalFailed(String),
}

impl ExpressionEvalError {
    /// Java-style message for the strict entry (`JuelExpression.java:53-60`).
    /// `expression_text` is the full expression source.
    pub fn to_java_message(&self, expression_text: &str) -> String {
        match self {
            ExpressionEvalError::CompileFailed(cause) => {
                format!("Error while evaluating expression: {expression_text}: {cause}")
            }
            ExpressionEvalError::UnknownProperty(name) => {
                format!("Unknown property used in expression: {expression_text} ({name})")
            }
            ExpressionEvalError::UnknownMethod(name) => {
                format!("Unknown method used in expression: {expression_text} ({name})")
            }
            ExpressionEvalError::EvalFailed(cause) => {
                format!("Error while evaluating expression: {expression_text}: {cause}")
            }
        }
    }

    /// True when Java would `createExpression`-fail (propagate) rather than
    /// `getValue`-fail into a catchable `FlowableException` (fallback).
    pub fn is_compile_failure(&self) -> bool {
        matches!(self, ExpressionEvalError::CompileFailed(_))
    }
}

impl fmt::Display for ExpressionEvalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpressionEvalError::CompileFailed(cause) => {
                write!(formatter, "Error while evaluating expression: {cause}")
            }
            ExpressionEvalError::UnknownProperty(name) => {
                write!(formatter, "Unknown property used in expression: {name}")
            }
            ExpressionEvalError::UnknownMethod(name) => {
                write!(formatter, "Unknown method used in expression: {name}")
            }
            ExpressionEvalError::EvalFailed(cause) => {
                write!(formatter, "Error while evaluating expression: {cause}")
            }
        }
    }
}

impl std::error::Error for ExpressionEvalError {}

/// Maximum number of compiled expressions kept in the global cache. Once the
/// limit is hit, the cache is cleared to amortize the cost of large process
/// definitions that contain many distinct expressions.
const GLOBAL_EXPRESSION_CACHE_MAX: usize = 1024;

/// Process-wide cache of compiled expressions, keyed by expression text.
/// Identical UEL strings across many `SimpleExpression` instances share the
/// same `Arc<CompiledExpression>`, so the per-instance `OnceLock` only has
/// to clone the Arc instead of re-parsing and re-compiling.
static GLOBAL_EXPRESSION_CACHE: OnceLock<Mutex<HashMap<String, Arc<CompiledExpression>>>> =
    OnceLock::new();

fn global_expression_cache() -> &'static Mutex<HashMap<String, Arc<CompiledExpression>>> {
    GLOBAL_EXPRESSION_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Compile a UEL expression text, populating the global cache as a side
/// effect so subsequent SimpleExpression instances with identical text can
/// skip compilation. The returned Arc is safe to share across threads.
fn compile_global(text: &str) -> Option<Arc<CompiledExpression>> {
    if !(text.starts_with("${") && text.ends_with('}')) {
        return None;
    }
    let inner = &text[2..text.len() - 1];
    let compiled = ExpressionParser::new(inner)
        .parse_expression()
        .map(|ast| Compiler::new().compile(&ast))
        .map(Arc::new)?;
    let mut cache = global_expression_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if cache.len() >= GLOBAL_EXPRESSION_CACHE_MAX {
        // Simple deterministic eviction: drop everything and start over.
        // Worst case we recompile a few expressions on the next miss.
        cache.clear();
    }
    cache.insert(text.to_string(), Arc::clone(&compiled));
    Some(compiled)
}

/// Test/inspection helper: number of compiled expressions currently cached.
#[cfg(test)]
pub(crate) fn global_expression_cache_len() -> usize {
    global_expression_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .len()
}

pub struct SimpleExpression {
    expression_text: String,
    /// Phase 2: compiled bytecode for stack-based interpreter. Replaces the
    /// recursive AST evaluate() with a flat instruction loop — no Box dereferences,
    /// better CPU cache locality. Compiled once from the AST, executed many times.
    cached_compiled: OnceLock<Option<Arc<CompiledExpression>>>,
    /// Phase 1: cached fast-path detection result. ~60% of UEL expressions are
    /// pure variable lookups (`${var}`) and ~20% are simple comparisons
    /// (`${var == literal}`). Detecting these once and bypassing the AST entirely
    /// eliminates recursive evaluate() + Box dereferences for the common case.
    cached_fast_path: OnceLock<Option<FastPath>>,
}

/// Phase 1: pre-parsed fast path for the two most common expression shapes.
/// Avoids AST construction and recursive evaluation entirely.
#[derive(Clone)]
enum FastPath {
    /// `${varName}` — direct variable lookup via process_variable()
    Variable(String),
    /// `${var == literal}` or `${var != literal}` (and reversed operand order)
    Comparison {
        var: String,
        literal: Value,
        negate: bool,
    },
}

impl SimpleExpression {
    pub fn new(expression_text: String) -> Self {
        Self {
            expression_text,
            cached_compiled: OnceLock::new(),
            cached_fast_path: OnceLock::new(),
        }
    }

    fn to_f64(value: &Value) -> Option<f64> {
        match value {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.parse::<f64>().ok(),
            _ => None,
        }
    }

    fn resolve_variable(scope: &dyn VariableContainer, name: &str) -> Option<Value> {
        if let Some(type_name) = crate::el::method_registry::parse_static_type_reference(name) {
            return Some(crate::el::method_registry::static_type_marker(type_name));
        }
        let method_registry = crate::el::method_registry::current_expression_method_registry();
        if method_registry.contains_bean(name) {
            return Some(crate::el::method_registry::bean_marker(name));
        }
        match name {
            // P37: `${execution}` root object exposes the Execution JSON
            // (ProcessVariableScopeELResolver.java:27-45).
            "execution" => scope.root_object_json(),
            // P37: `${task}` is a reserved root object name in Java. The
            // engine-side Execution does not carry a task reference, so we
            // return None instead of shadowing it with a process variable
            // lookup. This is the degraded landing noted in the P37 plan
            // (authenticatedUserId/task to be wired when the engine gains
            // an auth/task context). Returning None matches Java behavior
            // when the resolver has no TaskEntity in scope.
            "task" => None,
            // P37: `${currentTenantId}` from VariableContainerELResolver.java:29-43.
            "currentTenantId" => scope
                .current_tenant_id()
                .map(|tenant| Value::String(tenant.to_string())),
            _ => scope.get_variable(name),
        }
    }

    fn number_less(left: &serde_json::Number, right: &serde_json::Number) -> bool {
        match (left.as_i64(), right.as_i64()) {
            (Some(lhs), Some(rhs)) => return lhs < rhs,
            _ => {}
        }
        match (left.as_u64(), right.as_u64()) {
            (Some(lhs), Some(rhs)) => return lhs < rhs,
            _ => {}
        }
        match (left.as_i64(), right.as_u64()) {
            (Some(lhs), Some(_)) if lhs < 0 => return true,
            (Some(_), Some(_)) => return false,
            _ => {}
        }
        match (left.as_u64(), right.as_i64()) {
            (Some(_), Some(rhs)) if rhs < 0 => return false,
            (Some(_), Some(_)) => return true,
            _ => {}
        }
        matches!((left.as_f64(), right.as_f64()), (Some(lhs), Some(rhs)) if lhs < rhs)
    }

    fn number_equal(left: &serde_json::Number, right: &serde_json::Number) -> bool {
        // Exact integer comparison when both fit in the same signed range
        // (avoids f64 precision loss for values > 2^53).
        if let (Some(l), Some(r)) = (left.as_i64(), right.as_i64()) {
            return l == r;
        }
        // Unsigned-only integers (> i64::MAX): compare as u64.
        if let (Some(l), Some(r)) = (left.as_u64(), right.as_u64()) {
            return l == r;
        }
        // Mixed sign i64 vs u64: equal only if the i64 side is non-negative
        // and values match.
        match (left.as_i64(), right.as_u64()) {
            (Some(l), Some(r)) => return l >= 0 && (l as u64) == r,
            _ => {}
        }
        match (left.as_u64(), right.as_i64()) {
            (Some(l), Some(r)) => return r >= 0 && l == (r as u64),
            _ => {}
        }
        // Float or mixed int/float: fall back to f64 with epsilon so that
        // `${5.0 == 5}` evaluates to true (Java numeric promotion).
        match (left.as_f64(), right.as_f64()) {
            (Some(l), Some(r)) => (l - r).abs() < f64::EPSILON,
            // Last resort: structural equality (handles NaN-bearing edges).
            _ => left == right,
        }
    }

    fn values_equal(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Null, Value::Null) => true,
            (Value::Null, _) | (_, Value::Null) => false,
            (Value::Bool(lhs), Value::Bool(rhs)) => lhs == rhs,
            (Value::String(lhs), Value::String(rhs)) => lhs == rhs,
            (Value::Number(lhs), Value::Number(rhs)) => Self::number_equal(lhs, rhs),
            // Cross-type numeric comparison
            (Value::Number(_), Value::String(_)) | (Value::String(_), Value::Number(_)) => {
                match (Self::to_f64(left), Self::to_f64(right)) {
                    (Some(l), Some(r)) => (l - r).abs() < f64::EPSILON,
                    _ => false,
                }
            }
            _ => left == right,
        }
    }

    fn values_less(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Number(lhs), Value::Number(rhs)) => Self::number_less(lhs, rhs),
            _ => match (Self::to_f64(left), Self::to_f64(right)) {
                (Some(l), Some(r)) => l < r,
                (None, None) => match (left, right) {
                    (Value::String(l), Value::String(r)) => l < r,
                    (Value::Bool(l), Value::Bool(r)) => l < r,
                    _ => false,
                },
                _ => false,
            },
        }
    }

    fn values_greater(left: &Value, right: &Value) -> bool {
        Self::values_less(right, left)
    }

    fn is_truthy(value: &Value) -> bool {
        match value {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::String(s) => !s.is_empty(),
            Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0),
            Value::Array(a) => !a.is_empty(),
            Value::Object(_) => true,
        }
    }

    /// P104 `empty` operator — `BooleanOperations.empty`
    /// (BooleanOperations.java:176-190): null, empty string, empty
    /// array/collection and empty map evaluate to true; anything else
    /// (number, boolean, non-empty container) is false.
    fn is_empty(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.is_empty(),
            Value::Object(m) => m.is_empty(),
            _ => false,
        }
    }

    /// P104 `base[property]` bracket access, mirroring the JUEL property
    /// resolvers behind `AstBracket`/`AstProperty.eval` (AstProperty.java:67-82):
    /// - List: `ListELResolver.getValue` (ListELResolver.java:60-75) coerces the
    ///   property to an int index and returns **null** when `idx < 0 || idx >= size`.
    /// - Map: `MapELResolver.getValue` (MapELResolver.java:55-64) uses the raw
    ///   property object as the key; a missing key returns **null**. JSON object
    ///   keys are strings, so a non-string property (e.g. a number) yields null,
    ///   matching Java `map.get(Integer)` against a String-keyed map.
    /// - Any other base (string, number, boolean) is unresolvable and yields
    ///   null, following the lenient convention of the existing `.property` access.
    fn index_value(base: &Value, property: &Value) -> Value {
        match base {
            Value::Array(arr) => match Self::coerce_index(property) {
                Some(idx) if idx >= 0 && (idx as usize) < arr.len() => arr[idx as usize].clone(),
                _ => Value::Null,
            },
            Value::Object(map) => match property {
                Value::String(key) => map.get(key).cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            },
            _ => Value::Null,
        }
    }

    /// Coerce a bracket property to a list index, mirroring
    /// `ListELResolver.coerce` (ListELResolver.java:140-158): Number → intValue,
    /// String → Integer.parseInt, Boolean → true=1 / false=0; anything else is
    /// uncoercible.
    fn coerce_index(property: &Value) -> Option<i64> {
        match property {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) => s.parse::<i64>().ok(),
            Value::Bool(true) => Some(1),
            Value::Bool(false) => Some(0),
            _ => None,
        }
    }

    /// P104 reserved JUEL operator keywords (Scanner.java:161-176). Matched
    /// case-sensitively — an uppercase `OR`/`And` scans as an ordinary
    /// identifier in JUEL (Scanner.java:433-448) — and never a bare variable
    /// reference.
    fn is_operator_keyword(s: &str) -> bool {
        matches!(
            s,
            "and"
                | "or"
                | "eq"
                | "ne"
                | "lt"
                | "le"
                | "ge"
                | "gt"
                | "div"
                | "mod"
                | "not"
                | "empty"
        )
    }

    /// JSON string sentinels for IEEE non-finite arithmetic results
    /// (P1-2 / M2). `serde_json::Number` cannot hold ∞/NaN; Java returns
    /// `Double` here (`NumberOperations.div:128`, `mod:136`). `to_f64`
    /// parses these sentinels back (`"Infinity"` / `"-Infinity"` / `"NaN"`
    /// are valid `f64::from_str` inputs — no leading space).
    pub(crate) fn nonfinite_sentinel(result: f64) -> Value {
        if result.is_nan() {
            Value::String("NaN".to_string())
        } else if result == f64::INFINITY {
            Value::String("Infinity".to_string())
        } else if result == f64::NEG_INFINITY {
            Value::String("-Infinity".to_string())
        } else if result.fract() == 0.0 && result >= i64::MIN as f64 && result <= i64::MAX as f64 {
            Value::Number(serde_json::Number::from(result as i64))
        } else {
            serde_json::Number::from_f64(result)
                .map(Value::Number)
                .unwrap_or(Value::String(result.to_string()))
        }
    }

    /// Internal BigDecimal/BigInteger marker for the Java
    /// `isBigDecimalOrBigInteger` path (`NumberOperations.div:125-126`,
    /// `mod:135-136`). `serde_json::Value` cannot represent BigDecimal;
    /// tests construct this tag for A-DIV-BD-0 / A-MOD-BD-0 (M3: 表达式层
    /// 内部构造).
    #[cfg_attr(not(test), allow(dead_code, reason = "BD operand constructor used by gold-standard tests"))]
    pub(crate) fn big_decimal_operand(text: &str) -> Value {
        let mut map = serde_json::Map::new();
        map.insert(
            "__flowable_bd".to_string(),
            Value::String(text.to_string()),
        );
        Value::Object(map)
    }

    fn as_big_decimal(value: &Value) -> Option<f64> {
        match value {
            Value::Object(map) => map
                .get("__flowable_bd")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok()),
            _ => None,
        }
    }

    /// Java `isFloatOrDoubleOrDotEe` (NumberOperations.mod:135): Float /
    /// Double, or a String containing `.` / `e` / `E`. Drives the Double-`%`
    /// branch (A-MOD-DBL-0 / A-MOD-BD-0 → NaN).
    fn is_float_or_double_like(value: &Value) -> bool {
        match value {
            Value::Number(n) => n
                .as_f64()
                .is_some_and(|f| f.fract() != 0.0 || n.as_i64().is_none() && n.as_u64().is_none()),
            Value::String(s) => {
                let t = s.trim();
                t.contains('.') || t.contains('e') || t.contains('E')
            }
            _ => false,
        }
    }

    /// Gold-standard arithmetic (P1-2), aligned with Java
    /// `NumberOperations.div/mod` (flowable-engine-8).
    ///
    /// | case | result |
    /// |---|---|
    /// | Long/Integer/Double `/` 0 | `Value::String("Infinity"\|"-Infinity"\|"NaN")` |
    /// | BigDecimal `/` 0 | `Err` (ELException) |
    /// | Long `%` 0 | `Err` (ELException) |
    /// | BigDecimal / float / double `%` 0 | `Value::String("NaN")` |
    /// | type-coerce failure | `Err` (never disguised as Null) |
    /// | any non-finite (incl. overflow / composition) | string sentinel |
    fn arithmetic_op(left: &Value, right: &Value, op: char) -> Result<Value, String> {
        let left_bd = Self::as_big_decimal(left);
        let right_bd = Self::as_big_decimal(right);
        let either_bd = left_bd.is_some() || right_bd.is_some();

        // Long/Integer path for + - * % (Java Long arithmetic).
        // X1: Java `Long` +,-,* wrap on overflow (two's complement,
        // NumberOperations.add/sub/mul :80,:99,:118). Use wrapping_* so a
        // silent wrap is NOT promoted to f64 and NOT an Err.
        if matches!(op, '+' | '-' | '*' | '%') {
            if let (Value::Number(lhs), Value::Number(rhs)) = (left, right) {
                if let (Some(lhs), Some(rhs)) = (lhs.as_i64(), rhs.as_i64()) {
                    match op {
                        '+' => return Ok(Value::Number(lhs.wrapping_add(rhs).into())),
                        '-' => return Ok(Value::Number(lhs.wrapping_sub(rhs).into())),
                        '*' => return Ok(Value::Number(lhs.wrapping_mul(rhs).into())),
                        '%' => {
                            if rhs == 0 {
                                // Java NumberOperations.mod:141 Long%0 → throw
                                return Err("ELException: / by zero".to_string());
                            }
                            return Ok(Value::Number((lhs % rhs).into()));
                        }
                        _ => {}
                    }
                }
                if let (Some(lhs), Some(rhs)) = (lhs.as_u64(), rhs.as_u64()) {
                    match op {
                        '+' => return Ok(Value::Number(lhs.wrapping_add(rhs).into())),
                        '*' => return Ok(Value::Number(lhs.wrapping_mul(rhs).into())),
                        '%' => {
                            if rhs == 0 {
                                return Err("ELException: / by zero".to_string());
                            }
                            return Ok(Value::Number((lhs % rhs).into()));
                        }
                        _ => {}
                    }
                }
            }
        }

        let l = match left_bd.or_else(|| Self::to_f64(left)) {
            Some(v) => v,
            None => {
                return Err(format!(
                    "ELException: cannot coerce left operand to number for '{op}'"
                ));
            }
        };
        let r = match right_bd.or_else(|| Self::to_f64(right)) {
            Some(v) => v,
            None => {
                return Err(format!(
                    "ELException: cannot coerce right operand to number for '{op}'"
                ));
            }
        };

        match op {
            '/' => {
                if either_bd && r == 0.0 {
                    // Java BigDecimal.divide → ArithmeticException / ELException
                    return Err("ELException: BigDecimal divide by zero".to_string());
                }
                // Long/Integer/Double: IEEE division → ±∞ / NaN sentinels
                Ok(Self::nonfinite_sentinel(l / r))
            }
            '%' => {
                let double_mod = either_bd
                    || Self::is_float_or_double_like(left)
                    || Self::is_float_or_double_like(right);
                if double_mod {
                    // Java NumberOperations.mod:135-136 Double% → NaN on %0
                    return Ok(Self::nonfinite_sentinel(l % r));
                }
                if r == 0.0 {
                    return Err("ELException: / by zero".to_string());
                }
                Ok(Self::nonfinite_sentinel(l % r))
            }
            '+' | '-' | '*' => {
                let result = match op {
                    '+' => l + r,
                    '-' => l - r,
                    '*' => l * r,
                    other => {
                        return Err(format!(
                            "ELException: unknown arithmetic operator '{other}'"
                        ));
                    }
                };
                Ok(Self::nonfinite_sentinel(result))
            }
            other => Err(format!("ELException: unknown arithmetic operator '{other}'")),
        }
    }

    /// Dispatch a method invocation on a JSON-like value.
    ///
    /// Supports a small set of built-ins covering the most common UEL idioms:
    /// strings (`toUpperCase`, `toLowerCase`, `length`, `trim`, `contains`,
    /// `startsWith`, `endsWith`, `replace`, `substring`), numbers (`abs`,
    /// `floor`, `ceil`, `round`), and arrays (`size`, `isEmpty`).
    /// Anything else returns `Value::Null`, matching the lenient
    /// "no such method" behaviour of the existing expression engine.
    ///
    /// This is the legacy lenient entry: a registered method that fails is
    /// flattened to `None`, exactly like an unregistered one. Strict callers
    /// (e.g. expression listeners, which must surface evaluation errors the
    /// way Java's `ExpressionExecutionListener` does) use
    /// [`SimpleExpression::invoke_method_strict`].
    fn invoke_method(receiver: &Value, method: &str, args: &[Value]) -> Option<Value> {
        if let Some((receiver_name, is_static_type)) =
            crate::el::method_registry::marker_receiver(receiver)
        {
            let registry = crate::el::method_registry::current_expression_method_registry();
            let outcome = if is_static_type {
                registry.invoke_static(receiver_name, method, args)
            } else {
                registry.invoke_bean(receiver_name, method, args)
            };
            // Lenient: unregistered (None) and registered-but-failed
            // (Some(Err)) both collapse to `None`; only the strict entry
            // below keeps them apart.
            return outcome.and_then(Result::ok);
        }
        // Lenient keeps historic "no such method → null" (via the bytecode
        // path's `.unwrap_or(Null)` / AST `None` short-circuit).
        Self::invoke_builtin_method(receiver, method, args).or(Some(Value::Null))
    }

    /// Strict method dispatch used by [`SimpleExpression::get_value_strict`].
    ///
    /// Contract (research §2.1.2 / AstMethod.java:83-101):
    /// - `Ok(Some(Value::Null))` — **legal null**: receiver base is `Value::Null`
    ///   (`AstMethod` `answerNullIfBaseIsNull`).
    /// - `Ok(Some(value))` — a built-in or registered method produced `value`.
    /// - `Err(UnknownMethod)` — bean/static receiver with an unregistered
    ///   method, or a built-in "no such method" on a non-null receiver
    ///   (`MethodNotFoundException`).
    /// - `Err(EvalFailed)` — the method was found but its execution failed
    ///   (`JuelExpression.java:59-60`).
    fn invoke_method_strict(
        receiver: &Value,
        method: &str,
        args: &[Value],
    ) -> Result<Option<Value>, ExpressionEvalError> {
        // AstMethod.java:85-88 — base == null returns null (legal null).
        if receiver.is_null() {
            return Ok(Some(Value::Null));
        }
        if let Some((receiver_name, is_static_type)) =
            crate::el::method_registry::marker_receiver(receiver)
        {
            let registry = crate::el::method_registry::current_expression_method_registry();
            let outcome = if is_static_type {
                registry.invoke_static(receiver_name, method, args)
            } else {
                registry.invoke_bean(receiver_name, method, args)
            };
            return match outcome {
                None => Err(ExpressionEvalError::UnknownMethod(format!(
                    "{receiver_name}.{method}"
                ))),
                Some(Ok(value)) => Ok(Some(value)),
                Some(Err(message)) => Err(ExpressionEvalError::EvalFailed(format!(
                    "error evaluating '{receiver_name}.{method}(...)': {message}"
                ))),
            };
        }
        match Self::invoke_builtin_method(receiver, method, args) {
            Some(value) => Ok(Some(value)),
            // Built-in "no such method" on a non-null base → MethodNotFoundException.
            None => Err(ExpressionEvalError::UnknownMethod(format!(
                "{}.{}",
                Self::type_label(receiver),
                method
            ))),
        }
    }

    /// Short type label used in `UnknownMethod` messages.
    fn type_label(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Bool(_) => "Boolean",
            Value::Number(_) => "Number",
            Value::String(_) => "String",
            Value::Array(_) => "List",
            Value::Object(_) => "Map",
        }
    }

    /// Built-in methods available on plain JSON values (no registry involved).
    /// Returns `None` when the method does not exist on the receiver — strict
    /// callers turn that into `UnknownMethod`; lenient callers keep collapsing
    /// it to null. Methods that exist but yield null return `Some(Value::Null)`.
    fn invoke_builtin_method(receiver: &Value, method: &str, args: &[Value]) -> Option<Value> {
        match receiver {
            Value::String(s) => match method {
                "toUpperCase" if args.is_empty() => Some(Value::String(s.to_uppercase())),
                "toLowerCase" if args.is_empty() => Some(Value::String(s.to_lowercase())),
                "length" if args.is_empty() => Some(Value::Number(serde_json::Number::from(
                    s.chars().count() as i64,
                ))),
                "trim" if args.is_empty() => Some(Value::String(s.trim().to_string())),
                "contains" if args.len() == 1 => {
                    let needle = match &args[0] {
                        Value::String(v) => v.as_str(),
                        other => return Some(Value::Bool(s.contains(&other.to_string()))),
                    };
                    Some(Value::Bool(s.contains(needle)))
                }
                "startsWith" if args.len() == 1 => {
                    let prefix = match &args[0] {
                        Value::String(v) => v.as_str(),
                        other => return Some(Value::Bool(s.starts_with(&other.to_string()))),
                    };
                    Some(Value::Bool(s.starts_with(prefix)))
                }
                "endsWith" if args.len() == 1 => {
                    let suffix = match &args[0] {
                        Value::String(v) => v.as_str(),
                        other => return Some(Value::Bool(s.ends_with(&other.to_string()))),
                    };
                    Some(Value::Bool(s.ends_with(suffix)))
                }
                "replace" if args.len() == 2 => {
                    let from = match &args[0] {
                        Value::String(v) => std::borrow::Cow::Borrowed(v.as_str()),
                        other => std::borrow::Cow::Owned(other.to_string()),
                    };
                    let to = match &args[1] {
                        Value::String(v) => std::borrow::Cow::Borrowed(v.as_str()),
                        other => std::borrow::Cow::Owned(other.to_string()),
                    };
                    Some(Value::String(s.replace(&*from, &to)))
                }
                "substring" if (1..=2).contains(&args.len()) => {
                    let chars: Vec<char> = s.chars().collect();
                    let len = chars.len() as i64;
                    let start = match Self::to_f64(&args[0]) {
                        Some(v) => v as i64,
                        None => return Some(Value::Null),
                    };
                    let end = if args.len() == 2 {
                        match Self::to_f64(&args[1]) {
                            Some(v) => v as i64,
                            None => return Some(Value::Null),
                        }
                    } else {
                        len
                    };
                    let start = start.max(0).min(len) as usize;
                    let end = end.max(0).min(len) as usize;
                    if end < start {
                        Some(Value::String(String::new()))
                    } else {
                        Some(Value::String(chars[start..end].iter().collect()))
                    }
                }
                _ => None,
            },
            Value::Number(n) => match method {
                "abs" if args.is_empty() => match n.as_f64() {
                    Some(v) => serde_json::Number::from_f64(v.abs()).map(Value::Number),
                    None => Some(Value::Null),
                },
                "floor" if args.is_empty() => match n.as_f64() {
                    Some(v) => Some(Value::Number(serde_json::Number::from(v.floor() as i64))),
                    None => Some(Value::Null),
                },
                "ceil" if args.is_empty() => match n.as_f64() {
                    Some(v) => Some(Value::Number(serde_json::Number::from(v.ceil() as i64))),
                    None => Some(Value::Null),
                },
                "round" if args.is_empty() => match n.as_f64() {
                    Some(v) => Some(Value::Number(serde_json::Number::from(v.round() as i64))),
                    None => Some(Value::Null),
                },
                _ => None,
            },
            Value::Array(arr) => match method {
                "size" | "length" if args.is_empty() => {
                    Some(Value::Number(serde_json::Number::from(arr.len() as i64)))
                }
                "isEmpty" if args.is_empty() => Some(Value::Bool(arr.is_empty())),
                _ => None,
            },
            // Bool/Map receivers have no built-in methods. Null is handled by
            // the caller (AstMethod base==null) and never reaches here.
            Value::Bool(_) | Value::Null | Value::Object(_) => None,
        }
    }

    /// Phase 1: Detect whether the expression text matches a fast-path pattern.
    /// Called once per SimpleExpression via OnceLock; subsequent get_value calls
    /// skip detection entirely.
    fn detect_fast_path(text: &str) -> Option<FastPath> {
        let text = text.trim();
        if !(text.starts_with("${") && text.ends_with('}')) {
            return None;
        }
        let inner = &text[2..text.len() - 1];

        // Phase 1.1: Pure variable lookup ${varName}
        // Exclude keywords true/false/null — these are literals handled by the AST path
        if Self::is_valid_identifier(inner) && !Self::is_keyword(inner) {
            return Some(FastPath::Variable(inner.to_string()));
        }

        // Phase 1.2: Simple comparison ${var == literal} or ${var != literal}
        Self::detect_comparison(inner)
    }

    /// A valid UEL identifier: starts with alpha/underscore, contains only
    /// alphanumeric + underscore. No dots, spaces, or operators.
    fn is_valid_identifier(s: &str) -> bool {
        let bytes = s.as_bytes();
        if bytes.is_empty() {
            return false;
        }
        let first = bytes[0];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return false;
        }
        bytes[1..]
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
    }

    /// Check if the string is a reserved keyword: literal keywords
    /// (true/false/null) or a P104 operator keyword. Used to keep the pure
    /// variable-lookup fast path from claiming `${empty}` / `${eq}` style
    /// expressions that the parser treats as operators.
    fn is_keyword(s: &str) -> bool {
        s.eq_ignore_ascii_case("true")
            || s.eq_ignore_ascii_case("false")
            || s.eq_ignore_ascii_case("null")
            || Self::is_operator_keyword(s)
    }

    /// Detect `${var == literal}` or `${var != literal}` patterns.
    /// Supports reversed operand order: `${literal == var}`.
    fn detect_comparison(inner: &str) -> Option<FastPath> {
        let bytes = inner.as_bytes();
        let mut depth = 0i32;
        let mut op_pos = None;
        let mut negate = false;

        for i in 0..bytes.len() {
            match bytes[i] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b'=' if depth == 0 && i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                    op_pos = Some(i);
                    break;
                }
                b'!' if depth == 0 && i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                    op_pos = Some(i);
                    negate = true;
                    break;
                }
                _ => {}
            }
        }

        let op_pos = op_pos?;
        let left = inner[..op_pos].trim();
        let right = inner[op_pos + 2..].trim();

        // One side must be identifier, the other a literal
        let (var, literal_str) = if Self::is_valid_identifier(left) {
            (left, right)
        } else if Self::is_valid_identifier(right) {
            (right, left)
        } else {
            return None;
        };

        let literal = Self::parse_literal_value(literal_str)?;

        Some(FastPath::Comparison {
            var: var.to_string(),
            literal,
            negate,
        })
    }

    /// Parse a literal token (true, false, null, number, quoted string) into a Value.
    fn parse_literal_value(s: &str) -> Option<Value> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("true") {
            return Some(Value::Bool(true));
        }
        if s.eq_ignore_ascii_case("false") {
            return Some(Value::Bool(false));
        }
        if s.eq_ignore_ascii_case("null") {
            return Some(Value::Null);
        }
        if let Some(stripped) = s.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            return Some(Value::String(stripped.to_string()));
        }
        if let Some(stripped) = s.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
            return Some(Value::String(stripped.to_string()));
        }
        if let Ok(integer) = s.parse::<i64>() {
            return Some(Value::Number(integer.into()));
        }
        if let Ok(integer) = s.parse::<u64>() {
            return Some(Value::Number(integer.into()));
        }
        if let Ok(float) = s.parse::<f64>() {
            return serde_json::Number::from_f64(float).map(Value::Number);
        }
        None
    }
}

/// Task 7: parsed expression AST. Built once by `ExpressionParser`, evaluated many
/// times against different `Execution`s without re-parsing. Replaces the previous
/// parse-and-evaluate-in-one-pass design that re-parsed on every `get_value` call.
#[derive(Debug, Clone)]
enum ExpressionAst {
    Literal(Value),
    Variable(String),
    Conditional(Box<ExpressionAst>, Box<ExpressionAst>, Box<ExpressionAst>),
    Or(Box<ExpressionAst>, Box<ExpressionAst>),
    And(Box<ExpressionAst>, Box<ExpressionAst>),
    Equal(Box<ExpressionAst>, Box<ExpressionAst>),
    NotEqual(Box<ExpressionAst>, Box<ExpressionAst>),
    LessEq(Box<ExpressionAst>, Box<ExpressionAst>),
    GreaterEq(Box<ExpressionAst>, Box<ExpressionAst>),
    Less(Box<ExpressionAst>, Box<ExpressionAst>),
    Greater(Box<ExpressionAst>, Box<ExpressionAst>),
    Add(Box<ExpressionAst>, Box<ExpressionAst>),
    Sub(Box<ExpressionAst>, Box<ExpressionAst>),
    Mul(Box<ExpressionAst>, Box<ExpressionAst>),
    Div(Box<ExpressionAst>, Box<ExpressionAst>),
    Mod(Box<ExpressionAst>, Box<ExpressionAst>),
    Not(Box<ExpressionAst>),
    Neg(Box<ExpressionAst>),
    /// P104: `empty` unary operator — `AstUnary.EMPTY` (AstUnary.java:37-40),
    /// truthiness per `BooleanOperations.empty` (BooleanOperations.java:176-190).
    Empty(Box<ExpressionAst>),
    /// P104: `base[expr]` bracket access — `AstBracket` (AstBracket.java:22-61).
    Index(Box<ExpressionAst>, Box<ExpressionAst>),
    Property(Box<ExpressionAst>, String),
    MethodCall(Box<ExpressionAst>, String, Vec<ExpressionAst>),
}

impl ExpressionAst {
    #[allow(dead_code)]
    fn evaluate(&self, scope: &dyn VariableContainer) -> Option<Value> {
        match self {
            ExpressionAst::Literal(v) => Some(v.clone()),
            ExpressionAst::Variable(name) => SimpleExpression::resolve_variable(scope, name),
            ExpressionAst::Conditional(condition, when_true, when_false) => {
                let condition = condition.evaluate(scope)?;
                if SimpleExpression::is_truthy(&condition) {
                    when_true.evaluate(scope)
                } else {
                    when_false.evaluate(scope)
                }
            }
            // Short-circuit: if left is truthy, return it without evaluating right.
            ExpressionAst::Or(left, right) => {
                let l = left.evaluate(scope)?;
                if SimpleExpression::is_truthy(&l) {
                    return Some(l);
                }
                right.evaluate(scope)
            }
            // Short-circuit: if left is falsy, return Bool(false) without evaluating right.
            ExpressionAst::And(left, right) => {
                let l = left.evaluate(scope)?;
                if !SimpleExpression::is_truthy(&l) {
                    return Some(Value::Bool(false));
                }
                right.evaluate(scope)
            }
            ExpressionAst::Equal(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(Value::Bool(SimpleExpression::values_equal(&l, &r)))
            }
            ExpressionAst::NotEqual(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(Value::Bool(!SimpleExpression::values_equal(&l, &r)))
            }
            ExpressionAst::LessEq(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(Value::Bool(
                    SimpleExpression::values_less(&l, &r) || SimpleExpression::values_equal(&l, &r),
                ))
            }
            ExpressionAst::GreaterEq(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(Value::Bool(
                    SimpleExpression::values_greater(&l, &r)
                        || SimpleExpression::values_equal(&l, &r),
                ))
            }
            ExpressionAst::Less(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(Value::Bool(SimpleExpression::values_less(&l, &r)))
            }
            ExpressionAst::Greater(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(Value::Bool(SimpleExpression::values_greater(&l, &r)))
            }
            ExpressionAst::Add(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                match SimpleExpression::arithmetic_op(&l, &r, '+') {
                    Ok(result) => Some(result),
                    // String concatenation fallback for +; arithmetic errors
                    // stay None on the lenient path (never disguised as Null).
                    Err(_) => None,
                }
            }
            ExpressionAst::Sub(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                SimpleExpression::arithmetic_op(&l, &r, '-').ok()
            }
            ExpressionAst::Mul(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                SimpleExpression::arithmetic_op(&l, &r, '*').ok()
            }
            ExpressionAst::Div(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                SimpleExpression::arithmetic_op(&l, &r, '/').ok()
            }
            ExpressionAst::Mod(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                SimpleExpression::arithmetic_op(&l, &r, '%').ok()
            }
            ExpressionAst::Not(operand) => {
                let v = operand.evaluate(scope)?;
                Some(Value::Bool(!SimpleExpression::is_truthy(&v)))
            }
            ExpressionAst::Neg(operand) => {
                let v = operand.evaluate(scope)?;
                SimpleExpression::to_f64(&v).map(|n| SimpleExpression::nonfinite_sentinel(-n))
            }
            ExpressionAst::Empty(operand) => {
                let v = operand.evaluate(scope)?;
                Some(Value::Bool(SimpleExpression::is_empty(&v)))
            }
            ExpressionAst::Index(base, property) => {
                let base_v = base.evaluate(scope)?;
                let prop_v = property.evaluate(scope)?;
                Some(SimpleExpression::index_value(&base_v, &prop_v))
            }
            ExpressionAst::Property(base, name) => {
                let v = base.evaluate(scope)?;
                Some(match v {
                    Value::Object(map) => map.get(name).cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                })
            }
            ExpressionAst::MethodCall(base, method, args_ast) => {
                let receiver = base.evaluate(scope)?;
                let mut args = Vec::with_capacity(args_ast.len());
                for arg_ast in args_ast {
                    args.push(arg_ast.evaluate(scope)?);
                }
                SimpleExpression::invoke_method(&receiver, method, &args)
            }
        }
    }

    /// Strict counterpart of [`ExpressionAst::evaluate`].
    ///
    /// Contract (research §2.1.2): undefined variables, unknown properties and
    /// unregistered methods are `Err`; legal null is only a defined-null
    /// variable value or `AstMethod`/`AstProperty` with a null base.
    #[allow(dead_code)]
    fn evaluate_strict(
        &self,
        scope: &dyn VariableContainer,
    ) -> Result<Option<Value>, ExpressionEvalError> {
        // Unwrap a required sub-evaluation: a lenient `None` from a child
        // short-circuits the whole node to `Ok(None)`; an `Err` propagates.
        macro_rules! require {
            ($evaluation:expr) => {
                match $evaluation {
                    Ok(Some(value)) => value,
                    Ok(None) => return Ok(None),
                    Err(error) => return Err(error),
                }
            };
        }

        let result = match self {
            ExpressionAst::Literal(v) => Some(v.clone()),
            ExpressionAst::Variable(name) => match SimpleExpression::resolve_variable(scope, name) {
                Some(v) => Some(v),
                None => return Err(ExpressionEvalError::UnknownProperty(name.clone())),
            },
            ExpressionAst::Conditional(condition, when_true, when_false) => {
                let condition_value = require!(condition.evaluate_strict(scope));
                if SimpleExpression::is_truthy(&condition_value) {
                    when_true.evaluate_strict(scope)?
                } else {
                    when_false.evaluate_strict(scope)?
                }
            }
            ExpressionAst::Or(left, right) => {
                let left_value = require!(left.evaluate_strict(scope));
                if SimpleExpression::is_truthy(&left_value) {
                    Some(left_value)
                } else {
                    right.evaluate_strict(scope)?
                }
            }
            ExpressionAst::And(left, right) => {
                let left_value = require!(left.evaluate_strict(scope));
                if !SimpleExpression::is_truthy(&left_value) {
                    Some(Value::Bool(false))
                } else {
                    right.evaluate_strict(scope)?
                }
            }
            ExpressionAst::Equal(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(Value::Bool(SimpleExpression::values_equal(&l, &r)))
            }
            ExpressionAst::NotEqual(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(Value::Bool(!SimpleExpression::values_equal(&l, &r)))
            }
            ExpressionAst::LessEq(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(Value::Bool(
                    SimpleExpression::values_less(&l, &r) || SimpleExpression::values_equal(&l, &r),
                ))
            }
            ExpressionAst::GreaterEq(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(Value::Bool(
                    SimpleExpression::values_greater(&l, &r)
                        || SimpleExpression::values_equal(&l, &r),
                ))
            }
            ExpressionAst::Less(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(Value::Bool(SimpleExpression::values_less(&l, &r)))
            }
            ExpressionAst::Greater(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(Value::Bool(SimpleExpression::values_greater(&l, &r)))
            }
            ExpressionAst::Add(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                match SimpleExpression::arithmetic_op(&l, &r, '+') {
                    Ok(result) => Some(result),
                    // String concatenation has no dedicated path here; a hard
                    // arithmetic failure is surfaced (P1-2: never Null).
                    Err(cause) => {
                        return Err(ExpressionEvalError::EvalFailed(cause));
                    }
                }
            }
            ExpressionAst::Sub(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(
                    SimpleExpression::arithmetic_op(&l, &r, '-')
                        .map_err(ExpressionEvalError::EvalFailed)?,
                )
            }
            ExpressionAst::Mul(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(
                    SimpleExpression::arithmetic_op(&l, &r, '*')
                        .map_err(ExpressionEvalError::EvalFailed)?,
                )
            }
            ExpressionAst::Div(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(
                    SimpleExpression::arithmetic_op(&l, &r, '/')
                        .map_err(ExpressionEvalError::EvalFailed)?,
                )
            }
            ExpressionAst::Mod(left, right) => {
                let l = require!(left.evaluate_strict(scope));
                let r = require!(right.evaluate_strict(scope));
                Some(
                    SimpleExpression::arithmetic_op(&l, &r, '%')
                        .map_err(ExpressionEvalError::EvalFailed)?,
                )
            }
            ExpressionAst::Not(operand) => {
                let v = require!(operand.evaluate_strict(scope));
                Some(Value::Bool(!SimpleExpression::is_truthy(&v)))
            }
            ExpressionAst::Neg(operand) => {
                let v = require!(operand.evaluate_strict(scope));
                match SimpleExpression::to_f64(&v) {
                    Some(n) => Some(SimpleExpression::nonfinite_sentinel(-n)),
                    None => {
                        return Err(ExpressionEvalError::EvalFailed(format!(
                            "ELException: cannot coerce operand to number for unary minus"
                        )));
                    }
                }
            }
            ExpressionAst::Empty(operand) => {
                let v = require!(operand.evaluate_strict(scope));
                Some(Value::Bool(SimpleExpression::is_empty(&v)))
            }
            ExpressionAst::Index(base, property) => {
                let base_v = require!(base.evaluate_strict(scope));
                let prop_v = require!(property.evaluate_strict(scope));
                Some(CompiledExpression::index_strict(&base_v, &prop_v)?)
            }
            ExpressionAst::Property(base, name) => {
                let v = require!(base.evaluate_strict(scope));
                Some(CompiledExpression::property_strict(&v, name)?)
            }
            ExpressionAst::MethodCall(base, method, args_ast) => {
                let receiver = require!(base.evaluate_strict(scope));
                let mut args = Vec::with_capacity(args_ast.len());
                for arg_ast in args_ast {
                    args.push(require!(arg_ast.evaluate_strict(scope)));
                }
                SimpleExpression::invoke_method_strict(&receiver, method, &args)?
            }
        };
        Ok(result)
    }
}

/// Phase 2: Bytecode instruction for the stack-based expression interpreter.
/// Replaces recursive `Box<ExpressionAst>` tree traversal with a flat `Vec` loop.
/// Small literals (bool, int, float) are inlined; strings are interned in a pool.
#[derive(Clone, Debug)]
enum Instruction {
    PushNull,
    PushBool(bool),
    PushInt(i64),
    PushFloat(f64),
    PushStr(usize), // index into string_pool
    LoadVar(usize), // index into string_pool
    Pop,
    Eq,
    Neq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Not,
    Neg,
    /// P104: pop value, push Bool(empty(value)) — `BooleanOperations.empty`.
    Empty,
    /// P104: pop property then base, push base[property] — bracket access.
    Index,
    /// If stack top is truthy, jump to absolute target. Does NOT pop.
    JumpIfTrue(usize),
    /// If stack top is falsy, jump to absolute target. Does NOT pop.
    JumpIfFalse(usize),
    /// Unconditional jump to absolute target.
    Jump(usize),
    /// Pop value, push value.property_name (name from string_pool).
    Property(usize),
    /// Pop receiver + arg_count args, push method result.
    MethodCall(usize, usize), // method name index, arg count
}

/// Phase 2: A compiled expression — flat instruction list + string pool.
/// Built once from an AST, executed many times without recursion or heap allocation
/// per-evaluation (stack is reused).
struct CompiledExpression {
    instructions: Vec<Instruction>,
    string_pool: Vec<String>,
}

impl CompiledExpression {
    fn execute(&self, scope: &dyn VariableContainer) -> Option<Value> {
        let mut stack: Vec<Value> = Vec::with_capacity(16);
        let mut pc = 0;

        while pc < self.instructions.len() {
            match &self.instructions[pc] {
                Instruction::PushNull => stack.push(Value::Null),
                Instruction::PushBool(b) => stack.push(Value::Bool(*b)),
                Instruction::PushInt(n) => stack.push(Value::Number((*n).into())),
                Instruction::PushFloat(f) => {
                    stack.push(
                        serde_json::Number::from_f64(*f)
                            .map(Value::Number)
                            .unwrap_or(Value::Null),
                    );
                }
                Instruction::PushStr(idx) => {
                    stack.push(Value::String(self.string_pool[*idx].clone()));
                }
                Instruction::LoadVar(idx) => {
                    let name = &self.string_pool[*idx];
                    match SimpleExpression::resolve_variable(scope, name) {
                        Some(v) => stack.push(v),
                        None => return None, // variable not found — propagate None
                    }
                }
                Instruction::Pop => {
                    stack.pop();
                }
                Instruction::Eq => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(Value::Bool(SimpleExpression::values_equal(&l, &r)));
                }
                Instruction::Neq => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(Value::Bool(!SimpleExpression::values_equal(&l, &r)));
                }
                Instruction::Lt => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(Value::Bool(SimpleExpression::values_less(&l, &r)));
                }
                Instruction::Gt => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(Value::Bool(SimpleExpression::values_greater(&l, &r)));
                }
                Instruction::LtEq => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(Value::Bool(
                        SimpleExpression::values_less(&l, &r)
                            || SimpleExpression::values_equal(&l, &r),
                    ));
                }
                Instruction::GtEq => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(Value::Bool(
                        SimpleExpression::values_greater(&l, &r)
                            || SimpleExpression::values_equal(&l, &r),
                    ));
                }
                Instruction::Add => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    match SimpleExpression::arithmetic_op(&l, &r, '+') {
                        Ok(result) => stack.push(result),
                        Err(_) => return None,
                    }
                }
                Instruction::Sub => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    match SimpleExpression::arithmetic_op(&l, &r, '-') {
                        Ok(result) => stack.push(result),
                        Err(_) => return None,
                    }
                }
                Instruction::Mul => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    match SimpleExpression::arithmetic_op(&l, &r, '*') {
                        Ok(result) => stack.push(result),
                        Err(_) => return None,
                    }
                }
                Instruction::Div => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    match SimpleExpression::arithmetic_op(&l, &r, '/') {
                        Ok(result) => stack.push(result),
                        Err(_) => return None,
                    }
                }
                Instruction::Mod => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    match SimpleExpression::arithmetic_op(&l, &r, '%') {
                        Ok(result) => stack.push(result),
                        Err(_) => return None,
                    }
                }
                Instruction::Not => {
                    let v = stack.pop()?;
                    stack.push(Value::Bool(!SimpleExpression::is_truthy(&v)));
                }
                Instruction::Neg => {
                    let v = stack.pop()?;
                    if let Some(n) = SimpleExpression::to_f64(&v) {
                        stack.push(SimpleExpression::nonfinite_sentinel(-n));
                    } else {
                        return None;
                    }
                }
                Instruction::Empty => {
                    let v = stack.pop()?;
                    stack.push(Value::Bool(SimpleExpression::is_empty(&v)));
                }
                Instruction::Index => {
                    let prop = stack.pop()?;
                    let base = stack.pop()?;
                    stack.push(SimpleExpression::index_value(&base, &prop));
                }
                Instruction::JumpIfTrue(target) => {
                    if let Some(top) = stack.last()
                        && SimpleExpression::is_truthy(top)
                    {
                        pc = *target;
                        continue;
                    }
                }
                Instruction::JumpIfFalse(target) => {
                    if let Some(top) = stack.last()
                        && !SimpleExpression::is_truthy(top)
                    {
                        pc = *target;
                        continue;
                    }
                }
                Instruction::Jump(target) => {
                    pc = *target;
                    continue;
                }
                Instruction::Property(idx) => {
                    let name = &self.string_pool[*idx];
                    let v = stack.pop()?;
                    stack.push(match v {
                        Value::Object(map) => map.get(name).cloned().unwrap_or(Value::Null),
                        _ => Value::Null,
                    });
                }
                Instruction::MethodCall(idx, arg_count) => {
                    let method = &self.string_pool[*idx];
                    let mut args = Vec::with_capacity(*arg_count);
                    for _ in 0..*arg_count {
                        args.push(stack.pop()?);
                    }
                    args.reverse();
                    let receiver = stack.pop()?;
                    stack.push(
                        SimpleExpression::invoke_method(&receiver, method, &args)
                            .unwrap_or(Value::Null),
                    );
                }
            }
            pc += 1;
        }

        stack.pop()
    }

    /// Property access shared by the bytecode and AST strict paths.
    ///
    /// Java resolver chain:
    /// - `AstProperty.eval` (AstProperty.java:67-71): base == null → null
    ///   (legal null, not an error).
    /// - `MapELResolver.getValue` (MapELResolver.java:55-61): Map/JSON object
    ///   missing key resolves to null (setPropertyResolved).
    /// - `ListELResolver` / `ArrayELResolver`: out-of-range index → null.
    /// - `CouldNotResolvePropertyELResolver` (CouldNotResolvePropertyELResolver.java:34-35):
    ///   non-null base no resolver claims → PropertyNotFoundException.
    fn property_strict(base: &Value, name: &str) -> Result<Value, ExpressionEvalError> {
        match base {
            Value::Null => Ok(Value::Null),
            Value::Object(map) => Ok(map.get(name).cloned().unwrap_or(Value::Null)),
            Value::Array(_) => Err(ExpressionEvalError::UnknownProperty(format!(
                "{name} on List"
            ))),
            other => Err(ExpressionEvalError::UnknownProperty(format!(
                "{name} on {}",
                SimpleExpression::type_label(other)
            ))),
        }
    }

    /// Bracket access `base[property]` on the strict path. Index-type rules
    /// stay lenient (ListELResolver/MapELResolver return null for a missing
    /// key / OOB index); a non-null primitive base is unresolvable → Err.
    fn index_strict(base: &Value, property: &Value) -> Result<Value, ExpressionEvalError> {
        match base {
            Value::Null => Ok(Value::Null),
            Value::Array(_) | Value::Object(_) => Ok(SimpleExpression::index_value(base, property)),
            other => Err(ExpressionEvalError::UnknownProperty(format!(
                "[{property}] on {}",
                SimpleExpression::type_label(other)
            ))),
        }
    }

    /// Strict counterpart of [`CompiledExpression::execute`].
    ///
    /// Contract (research §2.1.2): undefined variables, unknown properties and
    /// unregistered methods are `Err`; legal null is only a defined-null
    /// variable value or `AstMethod`/`AstProperty` with a null base.
    fn execute_strict(
        &self,
        scope: &dyn VariableContainer,
    ) -> Result<Option<Value>, ExpressionEvalError> {
        let mut stack: Vec<Value> = Vec::with_capacity(16);
        let mut pc = 0;

        // Stack underflow only happens with a malformed compiled stream (the
        // compiler never emits one); preserve the lenient `None` outcome
        // instead of manufacturing a new error class.
        macro_rules! pop_required {
            () => {
                match stack.pop() {
                    Some(value) => value,
                    None => return Ok(None),
                }
            };
        }

        while pc < self.instructions.len() {
            match &self.instructions[pc] {
                Instruction::PushNull => stack.push(Value::Null),
                Instruction::PushBool(b) => stack.push(Value::Bool(*b)),
                Instruction::PushInt(n) => stack.push(Value::Number((*n).into())),
                Instruction::PushFloat(f) => {
                    stack.push(
                        serde_json::Number::from_f64(*f)
                            .map(Value::Number)
                            .unwrap_or(Value::Null),
                    );
                }
                Instruction::PushStr(idx) => {
                    stack.push(Value::String(self.string_pool[*idx].clone()));
                }
                Instruction::LoadVar(idx) => {
                    let name = &self.string_pool[*idx];
                    // Undefined variable → PropertyNotFoundException.
                    // Defined-null stays `Value::Null` (legal null).
                    match SimpleExpression::resolve_variable(scope, name) {
                        Some(v) => stack.push(v),
                        None => {
                            return Err(ExpressionEvalError::UnknownProperty(name.clone()));
                        }
                    }
                }
                Instruction::Pop => {
                    stack.pop();
                }
                Instruction::Eq => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(Value::Bool(SimpleExpression::values_equal(&l, &r)));
                }
                Instruction::Neq => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(Value::Bool(!SimpleExpression::values_equal(&l, &r)));
                }
                Instruction::Lt => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(Value::Bool(SimpleExpression::values_less(&l, &r)));
                }
                Instruction::Gt => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(Value::Bool(SimpleExpression::values_greater(&l, &r)));
                }
                Instruction::LtEq => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(Value::Bool(
                        SimpleExpression::values_less(&l, &r)
                            || SimpleExpression::values_equal(&l, &r),
                    ));
                }
                Instruction::GtEq => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(Value::Bool(
                        SimpleExpression::values_greater(&l, &r)
                            || SimpleExpression::values_equal(&l, &r),
                    ));
                }
                Instruction::Add => {
                    let r = pop_required!();
                    let l = pop_required!();
                    match SimpleExpression::arithmetic_op(&l, &r, '+') {
                        Ok(result) => stack.push(result),
                        Err(cause) => return Err(ExpressionEvalError::EvalFailed(cause)),
                    }
                }
                Instruction::Sub => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(
                        SimpleExpression::arithmetic_op(&l, &r, '-')
                            .map_err(ExpressionEvalError::EvalFailed)?,
                    );
                }
                Instruction::Mul => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(
                        SimpleExpression::arithmetic_op(&l, &r, '*')
                            .map_err(ExpressionEvalError::EvalFailed)?,
                    );
                }
                Instruction::Div => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(
                        SimpleExpression::arithmetic_op(&l, &r, '/')
                            .map_err(ExpressionEvalError::EvalFailed)?,
                    );
                }
                Instruction::Mod => {
                    let r = pop_required!();
                    let l = pop_required!();
                    stack.push(
                        SimpleExpression::arithmetic_op(&l, &r, '%')
                            .map_err(ExpressionEvalError::EvalFailed)?,
                    );
                }
                Instruction::Not => {
                    let v = pop_required!();
                    stack.push(Value::Bool(!SimpleExpression::is_truthy(&v)));
                }
                Instruction::Neg => {
                    let v = pop_required!();
                    match SimpleExpression::to_f64(&v) {
                        Some(n) => stack.push(SimpleExpression::nonfinite_sentinel(-n)),
                        None => {
                            return Err(ExpressionEvalError::EvalFailed(
                                "ELException: cannot coerce operand to number for unary minus"
                                    .to_string(),
                            ));
                        }
                    }
                }
                Instruction::Empty => {
                    let v = pop_required!();
                    stack.push(Value::Bool(SimpleExpression::is_empty(&v)));
                }
                Instruction::Index => {
                    let prop = pop_required!();
                    let base = pop_required!();
                    stack.push(Self::index_strict(&base, &prop)?);
                }
                Instruction::JumpIfTrue(target) => {
                    if let Some(top) = stack.last()
                        && SimpleExpression::is_truthy(top)
                    {
                        pc = *target;
                        continue;
                    }
                }
                Instruction::JumpIfFalse(target) => {
                    if let Some(top) = stack.last()
                        && !SimpleExpression::is_truthy(top)
                    {
                        pc = *target;
                        continue;
                    }
                }
                Instruction::Jump(target) => {
                    pc = *target;
                    continue;
                }
                Instruction::Property(idx) => {
                    let name = &self.string_pool[*idx];
                    let v = pop_required!();
                    stack.push(Self::property_strict(&v, name)?);
                }
                Instruction::MethodCall(idx, arg_count) => {
                    let method = &self.string_pool[*idx];
                    let mut args = Vec::with_capacity(*arg_count);
                    for _ in 0..*arg_count {
                        args.push(pop_required!());
                    }
                    args.reverse();
                    let receiver = pop_required!();
                    match SimpleExpression::invoke_method_strict(&receiver, method, &args)? {
                        Some(value) => stack.push(value),
                        None => stack.push(Value::Null),
                    }
                }
            }
            pc += 1;
        }

        Ok(stack.pop())
    }
}

/// Phase 2: Compiles an `ExpressionAst` tree into a flat `CompiledExpression`.
struct Compiler {
    instructions: Vec<Instruction>,
    string_pool: Vec<String>,
}

impl Compiler {
    fn new() -> Self {
        Self {
            instructions: Vec::new(),
            string_pool: Vec::new(),
        }
    }

    fn intern_string(&mut self, s: &str) -> usize {
        self.string_pool
            .iter()
            .position(|v| v == s)
            .unwrap_or_else(|| {
                let idx = self.string_pool.len();
                self.string_pool.push(s.to_string());
                idx
            })
    }

    fn emit(&mut self, instr: Instruction) -> usize {
        let idx = self.instructions.len();
        self.instructions.push(instr);
        idx
    }

    fn compile(mut self, ast: &ExpressionAst) -> CompiledExpression {
        self.emit_ast(ast);
        CompiledExpression {
            instructions: self.instructions,
            string_pool: self.string_pool,
        }
    }

    fn emit_ast(&mut self, ast: &ExpressionAst) {
        match ast {
            ExpressionAst::Literal(v) => self.emit_literal(v),
            ExpressionAst::Variable(name) => {
                let idx = self.intern_string(name);
                self.emit(Instruction::LoadVar(idx));
            }
            ExpressionAst::Conditional(condition, when_true, when_false) => {
                self.emit_ast(condition);
                let jump_false = self.emit(Instruction::JumpIfFalse(0));
                self.emit(Instruction::Pop);
                self.emit_ast(when_true);
                let jump_end = self.emit(Instruction::Jump(0));
                let false_target = self.instructions.len();
                self.emit(Instruction::Pop);
                self.emit_ast(when_false);
                let end_target = self.instructions.len();
                self.instructions[jump_false] = Instruction::JumpIfFalse(false_target);
                self.instructions[jump_end] = Instruction::Jump(end_target);
            }
            // a || b: if a is truthy, keep it and skip b; else pop a, eval b
            ExpressionAst::Or(left, right) => {
                self.emit_ast(left);
                let jump_true = self.emit(Instruction::JumpIfTrue(0)); // placeholder
                self.emit(Instruction::Pop); // pop falsy left
                self.emit_ast(right);
                let target = self.instructions.len();
                self.instructions[jump_true] = Instruction::JumpIfTrue(target);
            }
            // a && b: if a is falsy, pop a and push false; else pop a, eval b
            ExpressionAst::And(left, right) => {
                self.emit_ast(left);
                let jump_false = self.emit(Instruction::JumpIfFalse(0)); // placeholder
                self.emit(Instruction::Pop); // pop truthy left
                self.emit_ast(right);
                let jump_end = self.emit(Instruction::Jump(0)); // placeholder
                let l_false = self.instructions.len();
                self.emit(Instruction::Pop); // pop falsy left
                self.emit(Instruction::PushBool(false));
                let l_end = self.instructions.len();
                self.instructions[jump_false] = Instruction::JumpIfFalse(l_false);
                self.instructions[jump_end] = Instruction::Jump(l_end);
            }
            ExpressionAst::Equal(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Eq);
            }
            ExpressionAst::NotEqual(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Neq);
            }
            ExpressionAst::LessEq(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::LtEq);
            }
            ExpressionAst::GreaterEq(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::GtEq);
            }
            ExpressionAst::Less(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Lt);
            }
            ExpressionAst::Greater(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Gt);
            }
            ExpressionAst::Add(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Add);
            }
            ExpressionAst::Sub(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Sub);
            }
            ExpressionAst::Mul(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Mul);
            }
            ExpressionAst::Div(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Div);
            }
            ExpressionAst::Mod(l, r) => {
                self.emit_ast(l);
                self.emit_ast(r);
                self.emit(Instruction::Mod);
            }
            ExpressionAst::Not(operand) => {
                self.emit_ast(operand);
                self.emit(Instruction::Not);
            }
            ExpressionAst::Neg(operand) => {
                self.emit_ast(operand);
                self.emit(Instruction::Neg);
            }
            ExpressionAst::Empty(operand) => {
                self.emit_ast(operand);
                self.emit(Instruction::Empty);
            }
            ExpressionAst::Index(base, property) => {
                self.emit_ast(base);
                self.emit_ast(property);
                self.emit(Instruction::Index);
            }
            ExpressionAst::Property(base, name) => {
                self.emit_ast(base);
                let idx = self.intern_string(name);
                self.emit(Instruction::Property(idx));
            }
            ExpressionAst::MethodCall(base, method, args) => {
                self.emit_ast(base);
                for arg in args {
                    self.emit_ast(arg);
                }
                let idx = self.intern_string(method);
                self.emit(Instruction::MethodCall(idx, args.len()));
            }
        }
    }

    fn emit_literal(&mut self, v: &Value) {
        match v {
            Value::Null => {
                self.emit(Instruction::PushNull);
            }
            Value::Bool(b) => {
                self.emit(Instruction::PushBool(*b));
            }
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    self.emit(Instruction::PushInt(i));
                } else if let Some(f) = n.as_f64() {
                    self.emit(Instruction::PushFloat(f));
                } else {
                    self.emit(Instruction::PushNull);
                }
            }
            Value::String(s) => {
                let idx = self.intern_string(s);
                self.emit(Instruction::PushStr(idx));
            }
            // For complex literals (arrays, objects), push as string representation
            _ => {
                let s = v.to_string();
                let idx = self.intern_string(&s);
                self.emit(Instruction::PushStr(idx));
            }
        }
    }
}

/// Maximum recursive nesting depth for UEL expression parsing.
/// P142c: deployer-controlled expressions must not stack-overflow the parser.
/// Each nested `(...)` / ternary re-enters the full precedence chain (~10
/// frames), so 128 is too deep for Windows debug stacks; 64 rejects abuse
/// while leaving headroom for real process expressions.
const MAX_EXPRESSION_NESTING_DEPTH: usize = 64;

struct ExpressionParser<'a> {
    input: &'a str,
    pos: usize,
    /// Current recursive nesting depth of `parse_expression` (parens / ternary / ...).
    depth: usize,
}

impl<'a> ExpressionParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            depth: 0,
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn consume(&mut self, expected: char) -> bool {
        self.skip_whitespace();
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// P104: match a lexical operator keyword (e.g. `and`, `or`, `eq`, `empty`)
    /// at the current position. The keyword must be a standalone identifier:
    /// the full identifier span is scanned and compared, so `order`/`landscape`
    /// are never split into `or`/`and` (JUEL Scanner.java:433-448 scans
    /// identifier characters and matches the whole name against the keyword map).
    /// Keywords are case-sensitive lowercase, matching JUEL's keyword map — an
    /// uppercase `OR` scans as an ordinary IDENTIFIER (Scanner.java:161-176).
    fn match_keyword(&self, keyword: &str) -> bool {
        let bytes = self.input.as_bytes();
        let start = self.pos;
        if start >= bytes.len() {
            return false;
        }
        let first = bytes[start];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return false;
        }
        // Leading boundary: the preceding char must not extend the identifier.
        if start > 0 {
            let prev = bytes[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                return false;
            }
        }
        let end = start + keyword.len();
        if end > bytes.len() || &self.input[start..end] != keyword {
            return false;
        }
        // Trailing boundary: the next char must not extend the identifier.
        if end < bytes.len() {
            let next = bytes[end];
            if next.is_ascii_alphanumeric() || next == b'_' {
                return false;
            }
        }
        true
    }

    fn parse_expression(&mut self) -> Option<ExpressionAst> {
        if self.depth >= MAX_EXPRESSION_NESTING_DEPTH {
            // Over-deep nesting → parse failure (None), never panic / stack overflow.
            return None;
        }
        self.depth += 1;
        let result = self.parse_conditional();
        self.depth -= 1;
        result
    }

    fn parse_conditional(&mut self) -> Option<ExpressionAst> {
        let condition = self.parse_or()?;
        self.skip_whitespace();
        if !self.consume('?') {
            return Some(condition);
        }

        let when_true = self.parse_expression()?;
        if !self.consume(':') {
            return None;
        }
        let when_false = self.parse_expression()?;
        Some(ExpressionAst::Conditional(
            Box::new(condition),
            Box::new(when_true),
            Box::new(when_false),
        ))
    }

    fn parse_or(&mut self) -> Option<ExpressionAst> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_whitespace();
            if self.pos + 1 < self.input.len() && &self.input[self.pos..self.pos + 2] == "||" {
                self.pos += 2;
                let right = self.parse_and()?;
                left = ExpressionAst::Or(Box::new(left), Box::new(right));
            } else if self.match_keyword("or") {
                // P104: JUEL `or` alias for `||` (Scanner.java:169).
                self.pos += "or".len();
                let right = self.parse_and()?;
                left = ExpressionAst::Or(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_and(&mut self) -> Option<ExpressionAst> {
        let mut left = self.parse_equality()?;
        loop {
            self.skip_whitespace();
            if self.pos + 1 < self.input.len() && &self.input[self.pos..self.pos + 2] == "&&" {
                self.pos += 2;
                let right = self.parse_equality()?;
                left = ExpressionAst::And(Box::new(left), Box::new(right));
            } else if self.match_keyword("and") {
                // P104: JUEL `and` alias for `&&` (Scanner.java:168).
                self.pos += "and".len();
                let right = self.parse_equality()?;
                left = ExpressionAst::And(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_equality(&mut self) -> Option<ExpressionAst> {
        let mut left = self.parse_comparison()?;
        loop {
            self.skip_whitespace();
            if self.pos + 1 < self.input.len() {
                let op = &self.input[self.pos..self.pos + 2];
                if op == "==" {
                    self.pos += 2;
                    let right = self.parse_comparison()?;
                    left = ExpressionAst::Equal(Box::new(left), Box::new(right));
                    continue;
                } else if op == "!=" {
                    self.pos += 2;
                    let right = self.parse_comparison()?;
                    left = ExpressionAst::NotEqual(Box::new(left), Box::new(right));
                    continue;
                }
            }
            // P104: JUEL `eq`/`ne` aliases for `==`/`!=` (Scanner.java:172-173).
            if self.match_keyword("eq") {
                self.pos += 2;
                let right = self.parse_comparison()?;
                left = ExpressionAst::Equal(Box::new(left), Box::new(right));
            } else if self.match_keyword("ne") {
                self.pos += 2;
                let right = self.parse_comparison()?;
                left = ExpressionAst::NotEqual(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_comparison(&mut self) -> Option<ExpressionAst> {
        let mut left = self.parse_addition()?;
        loop {
            self.skip_whitespace();
            if self.pos + 1 < self.input.len() {
                let two_char = &self.input[self.pos..self.pos + 2];
                if two_char == "<=" {
                    self.pos += 2;
                    let right = self.parse_addition()?;
                    left = ExpressionAst::LessEq(Box::new(left), Box::new(right));
                    continue;
                } else if two_char == ">=" {
                    self.pos += 2;
                    let right = self.parse_addition()?;
                    left = ExpressionAst::GreaterEq(Box::new(left), Box::new(right));
                    continue;
                }
            }
            if self.pos < self.input.len() {
                let ch = self.input.as_bytes()[self.pos];
                if ch == b'<' {
                    self.pos += 1;
                    let right = self.parse_addition()?;
                    left = ExpressionAst::Less(Box::new(left), Box::new(right));
                    continue;
                } else if ch == b'>' {
                    self.pos += 1;
                    let right = self.parse_addition()?;
                    left = ExpressionAst::Greater(Box::new(left), Box::new(right));
                    continue;
                }
            }
            // P104: JUEL `lt`/`le`/`ge`/`gt` aliases for `<`/`<=`/`>=`/`>`
            // (Scanner.java:170-171,174-175).
            if self.match_keyword("lt") {
                self.pos += 2;
                let right = self.parse_addition()?;
                left = ExpressionAst::Less(Box::new(left), Box::new(right));
            } else if self.match_keyword("le") {
                self.pos += 2;
                let right = self.parse_addition()?;
                left = ExpressionAst::LessEq(Box::new(left), Box::new(right));
            } else if self.match_keyword("ge") {
                self.pos += 2;
                let right = self.parse_addition()?;
                left = ExpressionAst::GreaterEq(Box::new(left), Box::new(right));
            } else if self.match_keyword("gt") {
                self.pos += 2;
                let right = self.parse_addition()?;
                left = ExpressionAst::Greater(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_addition(&mut self) -> Option<ExpressionAst> {
        let mut left = self.parse_multiplication()?;
        loop {
            self.skip_whitespace();
            let operator = match self.peek() {
                Some('+') => ExpressionAst::Add,
                Some('-') => ExpressionAst::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplication()?;
            left = operator(Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_multiplication(&mut self) -> Option<ExpressionAst> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_whitespace();
            let operator: Option<fn(Box<ExpressionAst>, Box<ExpressionAst>) -> ExpressionAst> =
                match self.peek() {
                    Some('*') => Some(ExpressionAst::Mul),
                    Some('/') => Some(ExpressionAst::Div),
                    Some('%') => Some(ExpressionAst::Mod),
                    _ => None,
                };
            if let Some(op) = operator {
                self.advance();
                let right = self.parse_unary()?;
                left = op(Box::new(left), Box::new(right));
                continue;
            }
            // P104: JUEL `div`/`mod` aliases for `/`/`%` at the multiplication
            // precedence level (Scanner.java:165-166; `mul := unary (MUL unary |
            // DIV unary | MOD unary)*` Parser.java:744-772).
            if self.match_keyword("div") {
                self.pos += "div".len();
                let right = self.parse_unary()?;
                left = ExpressionAst::Div(Box::new(left), Box::new(right));
            } else if self.match_keyword("mod") {
                self.pos += "mod".len();
                let right = self.parse_unary()?;
                left = ExpressionAst::Mod(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_unary(&mut self) -> Option<ExpressionAst> {
        self.skip_whitespace();
        if self.peek() == Some('!') {
            self.advance();
            let operand = self.parse_unary()?;
            return Some(ExpressionAst::Not(Box::new(operand)));
        }
        if self.peek() == Some('-') {
            self.advance();
            let operand = self.parse_unary()?;
            return Some(ExpressionAst::Neg(Box::new(operand)));
        }
        // P104: JUEL `not` alias for `!` (Scanner.java:167).
        if self.match_keyword("not") {
            self.pos += "not".len();
            let operand = self.parse_unary()?;
            return Some(ExpressionAst::Not(Box::new(operand)));
        }
        // P104: JUEL `empty` unary operator (Scanner.java:164; AstUnary.EMPTY
        // AstUnary.java:37-40; `unary := ... | EMPTY unary | ...` Parser.java:788-790).
        if self.match_keyword("empty") {
            self.pos += "empty".len();
            let operand = self.parse_unary()?;
            return Some(ExpressionAst::Empty(Box::new(operand)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Option<ExpressionAst> {
        self.skip_whitespace();

        // Parenthesized expression
        if self.peek() == Some('(') {
            self.advance();
            let inner = self.parse_expression()?;
            self.skip_whitespace();
            if self.peek() == Some(')') {
                self.advance();
            }
            // Allow chained method/property access on a parenthesized expression
            return self.parse_suffix_chain(inner);
        }

        // Quoted string literals must be atomic so content like
        // `'2036-11-14T11:12:22Z'` (hyphens / colons) is not split by operators.
        // Used by timer start expressions (Java StartTimerEventTest
        // testExpressionStartTimerEvent: `${'2036-11-14T11:12:22'}`).
        if let Some(quote) = self.peek().filter(|c| *c == '\'' || *c == '"') {
            self.advance(); // opening quote
            let start = self.pos;
            while self.pos < self.input.len() {
                let ch = self.input.as_bytes()[self.pos] as char;
                if ch == quote {
                    let content = self.input[start..self.pos].to_string();
                    self.advance(); // closing quote
                    return self.parse_suffix_chain(ExpressionAst::Literal(Value::String(content)));
                }
                self.pos += 1;
            }
            // Unterminated quote
            return None;
        }

        // Collect the operand token (a literal or a base identifier)
        let start = self.pos;
        let mut depth = 0i32;
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos];
            if ch == b'(' {
                depth += 1;
            } else if ch == b')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            } else if depth == 0
                && (ch == b','
                    || ch == b'+'
                    || ch == b'-'
                    || ch == b'*'
                    || ch == b'/'
                    || ch == b'%'
                    || ch == b'<'
                    || ch == b'>'
                    || ch == b'='
                    || ch == b'!'
                    || ch == b'&'
                    || ch == b'|'
                    || ch == b'?'
                    || ch == b':'
                    || ch == b'['
                    || ch == b']'
                    || ch.is_ascii_whitespace())
            {
                // P104: `[`/`]` delimit bracket indexes so `list[0]` scans as a
                // base token; whitespace ends a bare token so a following
                // lexical operator keyword (`eq`, `and`, ...) is scanned fresh.
                break;
            } else if depth == 0 && ch == b'.' {
                // `.` is a property-access separator, but in numeric literals
                // like `3.14` it must not break the token. Look ahead: if the
                // next char is a digit, treat it as part of the literal.
                if self.pos + 1 < self.input.len()
                    && self.input.as_bytes()[self.pos + 1].is_ascii_digit()
                {
                    self.pos += 1;
                    continue;
                }
                break;
            }
            self.pos += 1;
        }
        let token = self.input[start..self.pos].trim();
        if token.is_empty() {
            return None;
        }
        let base = Self::parse_token(token)?;

        // Chained .property or .method() — bare "foo()" without a receiver is
        // not valid UEL syntax, so we don't attempt to consume a trailing '('.
        self.parse_suffix_chain(base)
    }

    /// Parse a bare token into either a `Literal` or a `Variable`. Replaces the
    /// old `SimpleExpression::parse_operand` which mixed parsing with variable
    /// lookup (and thus could not be cached).
    fn parse_token(token: &str) -> Option<ExpressionAst> {
        let trimmed = token.trim();

        if trimmed.eq_ignore_ascii_case("true") {
            return Some(ExpressionAst::Literal(Value::Bool(true)));
        }
        if trimmed.eq_ignore_ascii_case("false") {
            return Some(ExpressionAst::Literal(Value::Bool(false)));
        }
        if trimmed.eq_ignore_ascii_case("null") {
            return Some(ExpressionAst::Literal(Value::Null));
        }
        // P104: reserved operator keywords are not valid variable references —
        // JUEL reserves `and`/`or`/`eq`/... (Scanner.java:161-176), so a bare
        // occurrence fails the parse, as it does in JUEL.
        if SimpleExpression::is_operator_keyword(trimmed) {
            return None;
        }
        if let Some(stripped) = trimmed.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            return Some(ExpressionAst::Literal(Value::String(stripped.to_string())));
        }
        if let Some(stripped) = trimmed
            .strip_prefix('\'')
            .and_then(|v| v.strip_suffix('\''))
        {
            return Some(ExpressionAst::Literal(Value::String(stripped.to_string())));
        }
        if let Ok(integer) = trimmed.parse::<i64>() {
            return Some(ExpressionAst::Literal(Value::Number(integer.into())));
        }
        if let Ok(integer) = trimmed.parse::<u64>() {
            return Some(ExpressionAst::Literal(Value::Number(integer.into())));
        }
        if let Ok(float) = trimmed.parse::<f64>() {
            return serde_json::Number::from_f64(float)
                .map(Value::Number)
                .map(ExpressionAst::Literal);
        }
        // Otherwise it's a variable reference — resolved at evaluate time.
        Some(ExpressionAst::Variable(trimmed.to_string()))
    }

    /// After parsing a value, look for chained `.identifier` or `.identifier(args)`
    /// accesses and apply them.
    fn parse_suffix_chain(&mut self, mut value: ExpressionAst) -> Option<ExpressionAst> {
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some('.') => {
                    self.advance(); // consume '.'

                    // Read the member identifier
                    let member_start = self.pos;
                    while self.pos < self.input.len() {
                        let ch = self.input.as_bytes()[self.pos];
                        if ch.is_ascii_alphanumeric() || ch == b'_' {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let member = self.input[member_start..self.pos].to_string();
                    if member.is_empty() {
                        return Some(value);
                    }

                    self.skip_whitespace();
                    if self.peek() == Some('(') {
                        // method call: receiver.method(args)
                        let args = self.parse_method_args()?;
                        value = ExpressionAst::MethodCall(Box::new(value), member, args);
                    } else {
                        // property access
                        value = ExpressionAst::Property(Box::new(value), member);
                    }
                }
                Some('[') => {
                    // P104: bracket index `base[expr]` — AstBracket
                    // (Parser.java:831-841). The property is a full expression,
                    // so `list[i]`, `map['key']` and `bean[prop]` all work.
                    self.advance(); // consume '['
                    let property = self.parse_expression()?;
                    self.skip_whitespace();
                    if self.peek() != Some(']') {
                        return None;
                    }
                    self.advance(); // consume ']'
                    value = ExpressionAst::Index(Box::new(value), Box::new(property));
                }
                _ => return Some(value),
            }
        }
    }

    /// Parse a method invocation `method(args)` argument list. Returns the AST
    /// for each argument; evaluation happens later in `ExpressionAst::evaluate`.
    fn parse_method_args(&mut self) -> Option<Vec<ExpressionAst>> {
        self.consume('(');
        let mut args: Vec<ExpressionAst> = Vec::new();
        self.skip_whitespace();
        if self.peek() != Some(')') {
            loop {
                let arg = self.parse_expression()?;
                args.push(arg);
                self.skip_whitespace();
                if self.peek() == Some(',') {
                    self.advance();
                    self.skip_whitespace();
                } else {
                    break;
                }
            }
        }
        self.skip_whitespace();
        self.consume(')');
        Some(args)
    }
}

/// Evaluate mixed literal + `${…}` text the way JUEL composite
/// `ValueExpression`s do (`ExpressionManager.createExpression` on
/// `"Hello ${gender}!"`).
///
/// Rules (W1 / research §2.1.4 — S semantics):
/// - Literal text is copied as-is.
/// - `\${` is an escaped dollar-brace and yields the two characters `${`
///   (the backslash is consumed).
/// - `${…}` segments are compiled with the existing `ExpressionParser` +
///   `Compiler` and evaluated strictly against `scope`. Nested braces are
///   tracked so `${fn({a:1})}` boundaries are correct; our UEL subset may
///   still reject the inner syntax, but scanning is brace-aware.
/// - A segment that fails to parse/evaluate is `Err` (Java
///   `createExpression`/`getValue` throws; the whole composite fails).
/// - A **legal null** segment (defined-null variable / null base) contributes
///   an empty string — JUEL string concatenation of null is "".
/// - Unclosed `${` is `CompileFailed` (Java parse failure).
///
/// Other EL call sites must keep using [`SimpleExpression`] so pure
/// `${…}` / literal paths are unchanged.
pub fn evaluate_composite_expression(
    text: &str,
    scope: &dyn VariableContainer,
) -> Result<String, ExpressionEvalError> {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Escaped composite start: `\${` → literal `${`.
        if bytes[i] == b'\\' && i + 2 < bytes.len() && bytes[i + 1] == b'$' && bytes[i + 2] == b'{'
        {
            out.push('$');
            out.push('{');
            i += 3;
            continue;
        }

        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Scan to matching `}` with brace depth (inner `{` in the
            // expression body increments depth).
            let expr_start = i + 2;
            let mut depth = 1usize;
            let mut j = expr_start;
            while j < bytes.len() {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth != 0 {
                // Unclosed `${` — Java `createExpression` fails at parse.
                return Err(ExpressionEvalError::CompileFailed(format!(
                    "unclosed '${{' in composite expression: {text}"
                )));
            }
            let inner = &text[expr_start..j];
            let whole = format!("${{{}}}", inner);
            // Strict segment evaluation: error ≠ legal-null. Legal null
            // concatenates as empty string (JUEL); errors propagate.
            let segment = match SimpleExpression::new(whole).get_value_strict(scope) {
                Ok(Some(value)) => value_to_composite_string(&value),
                Ok(None) => String::new(),
                Err(error) => return Err(error),
            };
            out.push_str(&segment);
            i = j + 1;
            continue;
        }

        // Safe: we walk UTF-8 by character when not on ASCII `$`/`\` markers.
        let ch = text[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(out)
}

fn value_to_composite_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

impl SimpleExpression {
    /// Strict evaluation entry for callers that must surface evaluation
    /// failures the way Java Flowable does (e.g. expression listeners —
    /// `ExpressionExecutionListener.java:34-37` ignores the return value but
    /// lets `expression.getValue(...)` exceptions escape and roll the command
    /// back).
    ///
    /// Contract (research §2.1.2):
    /// - `Ok(Some(value))` — the expression evaluated to `value`.
    /// - `Ok(Some(Value::Null))` — **legal null**: a defined-null variable, or
    ///   `AstMethod`/`AstProperty` with a null base.
    /// - `Err(UnknownProperty)` — undefined variable / unresolvable property
    ///   (`PropertyNotFoundException` → FlowableException).
    /// - `Err(UnknownMethod)` — unregistered method (`MethodNotFoundException`).
    /// - `Err(EvalFailed)` — registered method failed mid-evaluation.
    /// - `Err(CompileFailed)` — parse/compile failed (`createExpression`).
    pub fn get_value_strict(
        &self,
        scope: &dyn VariableContainer,
    ) -> Result<Option<Value>, ExpressionEvalError> {
        let fast_path = self
            .cached_fast_path
            .get_or_init(|| Self::detect_fast_path(&self.expression_text));
        if let Some(fp) = fast_path {
            return Self::eval_fast_path_strict(fp, scope);
        }
        // Parse/compile failure is a typed CompileFailed (Java createExpression
        // / ELException) so F-group callers can propagate it instead of
        // catching it as a eval-time FlowableException.
        let compiled = self.compiled();
        match compiled.as_ref() {
            Some(compiled) => compiled.execute_strict(scope),
            None => Err(ExpressionEvalError::CompileFailed(format!(
                "could not parse or compile '{}'",
                self.expression_text
            ))),
        }
    }

    /// Cached compiled form, shared with the process-wide expression cache.
    fn compiled(&self) -> &Option<Arc<CompiledExpression>> {
        self.cached_compiled.get_or_init(|| {
            let text = self.expression_text.trim();
            // Fast global-cache lookup avoids the parse + compile work entirely
            // when a previous instance has already paid the cost.
            {
                let cache = global_expression_cache()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if let Some(arc) = cache.get(text).cloned() {
                    return Some(arc);
                }
            }
            compile_global(text)
        })
    }

    /// Evaluate a detected [`FastPath`] on the strict path. Undefined operands
    /// are `UnknownProperty` (Java `ExclusiveGatewayTest` / research §2.1.2).
    fn eval_fast_path_strict(
        fast_path: &FastPath,
        scope: &dyn VariableContainer,
    ) -> Result<Option<Value>, ExpressionEvalError> {
        match fast_path {
            FastPath::Variable(name) => match Self::resolve_variable(scope, name) {
                Some(v) => Ok(Some(v)),
                None => Err(ExpressionEvalError::UnknownProperty(name.clone())),
            },
            FastPath::Comparison {
                var,
                literal,
                negate,
            } => {
                // Comparison operands must resolve. An undefined variable is
                // PropertyNotFoundException, not a silent null comparison.
                let var_val = match Self::resolve_variable(scope, var) {
                    Some(v) => v,
                    None => return Err(ExpressionEvalError::UnknownProperty(var.clone())),
                };
                let eq = SimpleExpression::values_equal(&var_val, literal);
                Ok(Some(Value::Bool(if *negate { !eq } else { eq })))
            }
        }
    }

    /// Evaluate a detected [`FastPath`]. Pure variable lookup and simple
    /// comparisons never invoke methods, so the result is always lenient.
    fn eval_fast_path(fast_path: &FastPath, scope: &dyn VariableContainer) -> Option<Value> {
        match fast_path {
            FastPath::Variable(name) => Self::resolve_variable(scope, name),
            FastPath::Comparison {
                var,
                literal,
                negate,
            } => {
                // An unresolved operand evaluates to null. Preserve that null
                // result for condition callers instead of manufacturing a
                // Boolean comparison, except for an explicit comparison with
                // null itself. UelExpressionCondition can then enforce Java's
                // non-Boolean condition contract.
                let eq = match scope.get_variable(var) {
                    Some(var_val) => SimpleExpression::values_equal(&var_val, literal),
                    None if literal.is_null() => true,
                    None => return None,
                };
                Some(Value::Bool(if *negate { !eq } else { eq }))
            }
        }
    }
}

impl Expression for SimpleExpression {
    fn get_value(&self, scope: &dyn VariableContainer) -> Option<serde_json::Value> {
        // Lenient entry retained for condition evaluation and every existing
        // call site: undefined variables and evaluation failures all surface
        // as `None`/null. Callers that need Java's "listener expression errors
        // fail the command" behaviour use `get_value_strict`.
        let fast_path = self
            .cached_fast_path
            .get_or_init(|| Self::detect_fast_path(&self.expression_text));
        if let Some(fp) = fast_path {
            return Self::eval_fast_path(fp, scope);
        }

        // Phase 2: compile AST to bytecode once, then execute via stack-based
        // interpreter. Replaces recursive evaluate() with a flat instruction loop.
        self.compiled().as_ref().and_then(|c| c.execute(scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::el::variable_container::MapVariableContainer;
    use std::collections::HashMap;

    fn eval(expr: &str, variables: HashMap<String, Value>) -> Option<Value> {
        let scope = MapVariableContainer::from_map(variables);
        let expression = SimpleExpression::new(expr.to_string());
        expression.get_value(&scope)
    }

    fn eval_composite(text: &str, variables: HashMap<String, Value>) -> String {
        let scope = MapVariableContainer::from_map(variables);
        evaluate_composite_expression(text, &scope).expect("composite should evaluate")
    }

    #[test]
    fn composite_expression_expands_mixed_literals_and_segments() {
        let mut vars = HashMap::new();
        vars.insert("gender".to_string(), Value::from("Mx"));
        vars.insert("orderId".to_string(), Value::from(42));
        assert_eq!(
            eval_composite("Hello ${gender}, your order ${orderId}!", vars),
            "Hello Mx, your order 42!"
        );
    }

    #[test]
    fn composite_expression_preserves_escaped_dollar_brace() {
        let vars = HashMap::new();
        assert_eq!(
            eval_composite(r"Price is \${amount} USD", vars),
            "Price is ${amount} USD"
        );
    }

    #[test]
    fn composite_expression_failed_segment_is_error() {
        // Missing variable → UnknownProperty (Java getValue throws); the whole
        // composite fails. Legal-null still concatenates as empty string.
        let mut vars = HashMap::new();
        vars.insert("known".to_string(), Value::from("ok"));
        vars.insert("nullVar".to_string(), Value::Null);
        let scope = MapVariableContainer::from_map(vars.clone());
        let err = evaluate_composite_expression("A=${known};B=${missing};C", &scope)
            .expect_err("undefined segment must fail the composite");
        assert!(
            matches!(err, ExpressionEvalError::UnknownProperty(_)),
            "expected UnknownProperty, got: {err:?}"
        );
        // Legal null → empty segment.
        assert_eq!(
            eval_composite("A=${known};B=${nullVar};C", vars),
            "A=ok;B=;C"
        );
    }

    #[test]
    fn composite_expression_unclosed_brace_is_compile_failed() {
        let scope = MapVariableContainer::from_map(HashMap::new());
        let err = evaluate_composite_expression("Hello ${unclosed", &scope)
            .expect_err("unclosed ${ must be CompileFailed");
        assert!(
            matches!(err, ExpressionEvalError::CompileFailed(_)),
            "expected CompileFailed, got: {err:?}"
        );
    }

    #[test]
    fn composite_expression_pure_literal_and_pure_expr_unchanged() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), Value::from("Ada"));
        assert_eq!(eval_composite("just text", HashMap::new()), "just text");
        assert_eq!(eval_composite("${name}", vars), "Ada");
    }

    #[test]
    fn production_expression_parser_contains_no_panic_macros() {
        let source = include_str!("expression.rs");
        assert!(!source.contains(&["unreachable!", "("].concat()));
        assert!(!source.contains(&["panic!", "("].concat()));
    }

    #[test]
    fn global_expression_cache_shares_compiled_form_across_instances() {
        // Use arithmetic so the Phase-1 fast path does not short-circuit compile.
        let expr_text = "${cacheShareA + cacheShareB}";
        let mut vars = HashMap::new();
        vars.insert("cacheShareA".to_string(), Value::from(2));
        vars.insert("cacheShareB".to_string(), Value::from(3));
        let scope = MapVariableContainer::from_map(vars);

        let first = SimpleExpression::new(expr_text.to_string());
        assert_eq!(first.get_value(&scope), Some(Value::from(5)));

        let second = SimpleExpression::new(expr_text.to_string());
        assert_eq!(second.get_value(&scope), Some(Value::from(5)));

        // Both instances should hold the same Arc address for the compiled form.
        // Cache length is process-global and not asserted here because parallel
        // unit tests may insert/evict other expressions concurrently.
        let first_ptr = first
            .cached_compiled
            .get()
            .and_then(|opt| opt.as_ref())
            .map(Arc::as_ptr);
        let second_ptr = second
            .cached_compiled
            .get()
            .and_then(|opt| opt.as_ref())
            .map(Arc::as_ptr);
        assert_eq!(first_ptr, second_ptr);
        assert!(first_ptr.is_some());
        assert!(global_expression_cache_len() > 0);
    }

    #[test]
    fn test_variable_lookup() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), Value::String("test".to_string()));
        assert_eq!(
            eval("${name}", vars),
            Some(Value::String("test".to_string()))
        );
    }

    #[test]
    fn missing_fast_path_comparison_preserves_null_result() {
        assert_eq!(
            eval("${x != 'a'}", HashMap::new()),
            None,
            "an unresolved comparison operand must remain null for condition validation"
        );
    }

    #[test]
    fn p37_resolves_execution_and_current_tenant_root_objects() {
        let root = serde_json::json!({
            "id": "execution-37",
            "activityId": "task-37",
        });
        let scope = MapVariableContainer::from_map(HashMap::new())
            .with_tenant_id(Some("tenant-37".to_string()))
            .with_root_object_json(Some(root));

        assert_eq!(
            SimpleExpression::new("${execution.id}".to_string()).get_value(&scope),
            Some(Value::String("execution-37".to_string()))
        );
        assert_eq!(
            SimpleExpression::new("${execution.activityId}".to_string()).get_value(&scope),
            Some(Value::String("task-37".to_string()))
        );
        assert_eq!(
            SimpleExpression::new("${currentTenantId}".to_string()).get_value(&scope),
            Some(Value::String("tenant-37".to_string()))
        );
    }

    #[test]
    fn p37_preserves_integer_precision_across_numeric_operators() {
        assert_eq!(
            eval("${9007199254740992 == 9007199254740993}", HashMap::new()),
            Some(Value::Bool(false))
        );
        assert_eq!(
            eval("${9007199254740992 < 9007199254740993}", HashMap::new()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            eval("${9007199254740992 + 1}", HashMap::new()),
            Some(Value::Number(9007199254740993_u64.into()))
        );
    }

    #[test]
    fn p37_rejects_non_numeric_addition() {
        assert_eq!(eval("${'left' + 'right'}", HashMap::new()), None);
    }

    #[test]
    fn p37_task_root_object_returns_none_without_task_context() {
        // `${task}` is a reserved Java root object name. The engine-side
        // Execution does not carry a TaskEntity, so it must resolve to None
        // rather than falling through to a process variable named "task".
        let mut vars = HashMap::new();
        vars.insert(
            "task".to_string(),
            Value::String("should-not-shadow".to_string()),
        );
        assert_eq!(eval("${task}", vars), None);
    }

    #[test]
    fn p37_cross_type_numeric_equality_uses_f64_fallback() {
        // int vs float must compare equal when mathematically equal
        // (Java numeric promotion). The original f64+epsilon path is
        // preserved for the float/int mix; only same-type integer
        // comparisons switch to exact i64/u64.
        assert_eq!(eval("${5.0 == 5}", HashMap::new()), Some(Value::Bool(true)));
        assert_eq!(eval("${5 == 5.0}", HashMap::new()), Some(Value::Bool(true)));
        assert_eq!(
            eval("${5.5 == 5}", HashMap::new()),
            Some(Value::Bool(false))
        );
    }

    #[test]
    fn test_equality() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Value::Number(5.into()));
        assert_eq!(eval("${x == 5}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x == 3}", vars), Some(Value::Bool(false)));
    }

    #[test]
    fn test_not_equal() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Value::Number(5.into()));
        assert_eq!(eval("${x != 3}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x != 5}", vars), Some(Value::Bool(false)));
    }

    #[test]
    fn test_comparison() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Value::Number(5.into()));
        assert_eq!(eval("${x > 3}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x < 3}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${x >= 5}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x <= 4}", vars), Some(Value::Bool(false)));
    }

    #[test]
    fn test_logical_operators() {
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), Value::Bool(true));
        vars.insert("b".to_string(), Value::Bool(false));
        assert_eq!(eval("${a && a}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${a && b}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${a || b}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${b || b}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${!b}", vars), Some(Value::Bool(true)));
    }

    #[test]
    fn test_arithmetic() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Value::Number(10.into()));
        vars.insert("y".to_string(), Value::Number(3.into()));
        assert_eq!(
            eval("${x + y}", vars.clone()),
            Some(Value::Number(13.into()))
        );
        assert_eq!(
            eval("${x - y}", vars.clone()),
            Some(Value::Number(7.into()))
        );
        assert_eq!(
            eval("${x * y}", vars.clone()),
            Some(Value::Number(30.into()))
        );
        // 10 / 3 = 3.333... (float division)
        let div_result = eval("${x / y}", vars.clone()).unwrap();
        assert!(matches!(div_result, Value::Number(_)));
        // 10 % 3 = 1
        assert_eq!(eval("${x % y}", vars), Some(Value::Number(1.into())));
    }

    #[test]
    fn test_property_access() {
        let mut vars = HashMap::new();
        let mut obj = serde_json::Map::new();
        obj.insert("name".to_string(), Value::String("Alice".to_string()));
        obj.insert("age".to_string(), Value::Number(30.into()));
        vars.insert("person".to_string(), Value::Object(obj));
        assert_eq!(
            eval("${person.name}", vars.clone()),
            Some(Value::String("Alice".to_string()))
        );
        assert_eq!(eval("${person.age}", vars), Some(Value::Number(30.into())));
    }

    #[test]
    fn test_complex_expression() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Value::Number(10.into()));
        vars.insert("y".to_string(), Value::Number(5.into()));
        vars.insert("z".to_string(), Value::Number(3.into()));
        assert_eq!(
            eval("${x > y && z < y}", vars.clone()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            eval("${x + y > z * 4}", vars.clone()),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn test_parentheses() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Value::Number(2.into()));
        vars.insert("y".to_string(), Value::Number(3.into()));
        vars.insert("z".to_string(), Value::Number(4.into()));
        assert_eq!(
            eval("${x * (y + z)}", vars.clone()),
            Some(Value::Number(14.into()))
        );
    }

    #[test]
    fn test_null_handling() {
        let vars = HashMap::new();
        assert_eq!(eval("${null}", vars.clone()), Some(Value::Null));
        assert_eq!(eval("${null == null}", vars), Some(Value::Bool(true)));
    }

    #[test]
    fn test_method_call_string() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), Value::String("alice".to_string()));
        assert_eq!(
            eval("${name.toUpperCase()}", vars.clone()),
            Some(Value::String("ALICE".to_string()))
        );
        assert_eq!(
            eval("${name.toLowerCase()}", vars.clone()),
            Some(Value::String("alice".to_string()))
        );
        assert_eq!(
            eval("${name.length()}", vars.clone()),
            Some(Value::Number(5.into()))
        );
        assert_eq!(
            eval("${name.contains('li')}", vars.clone()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            eval("${name.contains('zz')}", vars.clone()),
            Some(Value::Bool(false))
        );
        assert_eq!(
            eval("${name.startsWith('al')}", vars.clone()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            eval("${name.endsWith('ce')}", vars),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn test_method_call_string_with_args() {
        let mut vars = HashMap::new();
        vars.insert("msg".to_string(), Value::String("hello world".to_string()));
        assert_eq!(
            eval("${msg.replace('world', 'rust')}", vars.clone()),
            Some(Value::String("hello rust".to_string()))
        );
        assert_eq!(
            eval("${msg.substring(0, 5)}", vars.clone()),
            Some(Value::String("hello".to_string()))
        );
        assert_eq!(
            eval("${msg.substring(6)}", vars),
            Some(Value::String("world".to_string()))
        );
    }

    #[test]
    fn test_method_call_number() {
        let mut vars = HashMap::new();
        vars.insert(
            "n".to_string(),
            Value::Number(serde_json::Number::from_f64(-3.5).unwrap()),
        );
        assert_eq!(
            eval("${n.abs()}", vars.clone()),
            Some(Value::Number(serde_json::Number::from_f64(3.5).unwrap()))
        );
        let neg_floor = eval("${(-3.2).floor()}", vars.clone()).unwrap();
        assert_eq!(neg_floor, Value::Number((-4_i64).into()));
        let ceil_val = eval("${3.2.ceil()}", vars.clone()).unwrap();
        assert_eq!(ceil_val, Value::Number(4_i64.into()));
        let round_val = eval("${3.7.round()}", vars).unwrap();
        assert_eq!(round_val, Value::Number(4_i64.into()));
    }

    #[test]
    fn test_method_call_array() {
        let mut vars = HashMap::new();
        vars.insert(
            "items".to_string(),
            Value::Array(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string()),
            ]),
        );
        assert_eq!(
            eval("${items.size()}", vars.clone()),
            Some(Value::Number(2.into()))
        );
        assert_eq!(
            eval("${items.isEmpty()}", vars.clone()),
            Some(Value::Bool(false))
        );
        let mut empty_vars = HashMap::new();
        empty_vars.insert("items".to_string(), Value::Array(vec![]));
        assert_eq!(
            eval("${items.isEmpty()}", empty_vars),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn test_chained_property_and_method() {
        let mut vars = HashMap::new();
        let mut inner = serde_json::Map::new();
        inner.insert("city".to_string(), Value::String("paris".to_string()));
        let mut outer = serde_json::Map::new();
        outer.insert("address".to_string(), Value::Object(inner));
        vars.insert("person".to_string(), Value::Object(outer));
        assert_eq!(
            eval("${person.address.city.toUpperCase()}", vars),
            Some(Value::String("PARIS".to_string()))
        );
    }

    #[test]
    fn test_method_in_comparison() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), Value::String("Alice".to_string()));
        // `name.length() == 5` is a typical UEL pattern.
        assert_eq!(eval("${name.length() == 5}", vars), Some(Value::Bool(true)));
    }

    /// Task 7: verify AST caching — repeated evaluations on the same SimpleExpression
    /// must produce consistent results without re-parsing.
    #[test]
    fn test_ast_caching_repeated_eval() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Value::Number(10.into()));
        let scope = MapVariableContainer::from_map(vars.clone());
        let expression = SimpleExpression::new("${x + 5}".to_string());
        // First call parses + caches
        assert_eq!(expression.get_value(&scope), Some(Value::Number(15.into())));
        // Second call uses cache
        assert_eq!(expression.get_value(&scope), Some(Value::Number(15.into())));
        // Different execution — same cached AST, different variable binding
        let mut vars2 = HashMap::new();
        vars2.insert("x".to_string(), Value::Number(20.into()));
        let scope2 = MapVariableContainer::from_map(vars2);
        assert_eq!(
            expression.get_value(&scope2),
            Some(Value::Number(25.into()))
        );
    }

    // ---- P104: EL lexical operator dialect ---------------------------------

    #[test]
    fn p104_lexical_operator_aliases() {
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), Value::Bool(true));
        vars.insert("b".to_string(), Value::Bool(false));
        vars.insert("x".to_string(), Value::Number(5.into()));
        vars.insert("y".to_string(), Value::Number(3.into()));
        assert_eq!(eval("${a or b}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${b or b}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${a and b}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${x eq 5}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x eq 3}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${x ne 3}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x ne 5}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${x lt 6}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x le 5}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x ge 5}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${x gt 3}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(
            eval("${10 div 2}", vars.clone()),
            Some(Value::Number(5.into()))
        );
        assert_eq!(
            eval("${10 mod 3}", vars.clone()),
            Some(Value::Number(1.into()))
        );
        assert_eq!(eval("${not b}", vars), Some(Value::Bool(true)));
    }

    #[test]
    fn p104_uppercase_keywords_are_ordinary_identifiers() {
        // JUEL's keyword map is case-sensitive (Scanner.java:161-176), so an
        // uppercase `Not`/`Or` is an IDENTIFIER, never an operator.
        let mut vars = HashMap::new();
        vars.insert("Not".to_string(), Value::Bool(true));
        vars.insert("Or".to_string(), Value::Bool(true));
        assert_eq!(eval("${Not}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${Or}", vars.clone()), Some(Value::Bool(true)));
        // `Not` (uppercase) is a variable while `eq` (lowercase) stays the
        // operator — the keyword map is case-sensitive.
        assert_eq!(eval("${Not eq true}", vars), Some(Value::Bool(true)));
    }

    #[test]
    fn p104_lexical_string_equality() {
        let mut vars = HashMap::new();
        vars.insert("status".to_string(), Value::String("approve".to_string()));
        assert_eq!(
            eval("${status eq 'approve'}", vars.clone()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            eval("${status eq 'reject'}", vars.clone()),
            Some(Value::Bool(false))
        );
        assert_eq!(
            eval("${status ne 'reject'}", vars.clone()),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn p104_lexical_keyword_boundaries_do_not_split_identifiers() {
        let mut vars = HashMap::new();
        vars.insert("order".to_string(), Value::Number(7.into()));
        vars.insert("landscape".to_string(), Value::Number(9.into()));
        vars.insert("org".to_string(), Value::Number(3.into()));
        // `order`/`landscape`/`org` are ordinary variables, never split into
        // the `or` keyword (JUEL Scanner.java:433-448 scans the whole name).
        assert_eq!(
            eval("${order}", vars.clone()),
            Some(Value::Number(7.into()))
        );
        assert_eq!(
            eval("${landscape}", vars.clone()),
            Some(Value::Number(9.into()))
        );
        assert_eq!(eval("${org}", vars.clone()), Some(Value::Number(3.into())));
        // `order` as a left operand followed by the `eq` operator must parse
        // the variable, not `or` plus leftover.
        assert_eq!(eval("${order eq 7}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${order ne 3}", vars.clone()), Some(Value::Bool(true)));
    }

    #[test]
    fn p104_lexical_operator_combination() {
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), Value::String("x".to_string()));
        vars.insert("b".to_string(), Value::String("y".to_string()));
        // `${a eq 'x' and not empty b}` — keyword eq + and + not + empty in one
        // expression, mixing string comparison and the empty operator.
        assert_eq!(
            eval("${a eq 'x' and not empty b}", vars.clone()),
            Some(Value::Bool(true))
        );
        // `a` mismatch flips the whole and-chain to false.
        vars.insert("a".to_string(), Value::String("z".to_string()));
        assert_eq!(
            eval("${a eq 'x' and not empty b}", vars.clone()),
            Some(Value::Bool(false))
        );
        // Empty `b` makes `not empty b` false.
        vars.insert("a".to_string(), Value::String("x".to_string()));
        vars.insert("b".to_string(), Value::Null);
        assert_eq!(
            eval("${a eq 'x' and not empty b}", vars.clone()),
            Some(Value::Bool(false))
        );
    }

    #[test]
    fn p104_empty_operator_forms() {
        let vars = HashMap::new();
        // Literal forms: null/"" are empty; numbers and booleans are not.
        assert_eq!(eval("${empty null}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(eval("${empty ''}", vars.clone()), Some(Value::Bool(true)));
        assert_eq!(
            eval("${empty 'abc'}", vars.clone()),
            Some(Value::Bool(false))
        );
        assert_eq!(eval("${empty 0}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(
            eval("${empty false}", vars.clone()),
            Some(Value::Bool(false))
        );

        let mut vars = HashMap::new();
        vars.insert("emptyList".to_string(), Value::Array(vec![]));
        vars.insert("fullList".to_string(), Value::Array(vec![Value::from(1)]));
        vars.insert(
            "emptyMap".to_string(),
            Value::Object(serde_json::Map::new()),
        );
        let mut full_map = serde_json::Map::new();
        full_map.insert("k".to_string(), Value::from(1));
        vars.insert("fullMap".to_string(), Value::Object(full_map));
        vars.insert("nullVar".to_string(), Value::Null);
        assert_eq!(
            eval("${empty emptyList}", vars.clone()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            eval("${empty fullList}", vars.clone()),
            Some(Value::Bool(false))
        );
        assert_eq!(
            eval("${empty emptyMap}", vars.clone()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            eval("${empty fullMap}", vars.clone()),
            Some(Value::Bool(false))
        );
        // A variable explicitly set to null is empty (BooleanOperations.empty).
        assert_eq!(
            eval("${empty nullVar}", vars.clone()),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn p104_bracket_index_list_map_bean() {
        let mut vars = HashMap::new();
        vars.insert(
            "list".to_string(),
            Value::Array(vec![Value::from(10), Value::from(20), Value::from(30)]),
        );
        let mut map = serde_json::Map::new();
        map.insert("key".to_string(), Value::String("value".to_string()));
        vars.insert("map".to_string(), Value::Object(map));
        let mut person = serde_json::Map::new();
        person.insert("name".to_string(), Value::String("Alice".to_string()));
        vars.insert("person".to_string(), Value::Object(person));
        vars.insert("prop".to_string(), Value::String("name".to_string()));

        assert_eq!(
            eval("${list[0]}", vars.clone()),
            Some(Value::Number(10.into()))
        );
        assert_eq!(
            eval("${list[2]}", vars.clone()),
            Some(Value::Number(30.into()))
        );
        // Out-of-bounds and negative indexes yield null (ListELResolver.java:68-70).
        assert_eq!(eval("${list[3]}", vars.clone()), Some(Value::Null));
        assert_eq!(eval("${list[-1]}", vars.clone()), Some(Value::Null));
        assert_eq!(
            eval("${map['key']}", vars.clone()),
            Some(Value::String("value".to_string()))
        );
        // Missing map key yields null (MapELResolver.java:55-64).
        assert_eq!(eval("${map['nope']}", vars.clone()), Some(Value::Null));
        // bean[prop] where prop is a string variable holding the key.
        assert_eq!(
            eval("${person[prop]}", vars.clone()),
            Some(Value::String("Alice".to_string()))
        );
        // Chained bracket then keyword operator.
        assert_eq!(
            eval("${list[1] eq 20}", vars.clone()),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn p104_bracket_nested_and_coercions() {
        let mut vars = HashMap::new();
        vars.insert(
            "matrix".to_string(),
            Value::Array(vec![
                Value::Array(vec![Value::from(1), Value::from(2)]),
                Value::Array(vec![Value::from(3), Value::from(4)]),
            ]),
        );
        // Nested brackets: matrix[1][0] == 3.
        assert_eq!(
            eval("${matrix[1][0]}", vars.clone()),
            Some(Value::Number(3.into()))
        );
        // String numeric index coerces like ListELResolver.coerce
        // (ListELResolver.java:150-155).
        assert_eq!(
            eval("${matrix['1'][0]}", vars.clone()),
            Some(Value::Number(3.into()))
        );
        // Boolean index: true → 1, false → 0 (ListELResolver.java:147-149).
        assert_eq!(
            eval("${matrix[true][0]}", vars.clone()),
            Some(Value::Number(3.into()))
        );
        assert_eq!(
            eval("${matrix[false][1]}", vars.clone()),
            Some(Value::Number(2.into()))
        );
        // Bracket result feeds a keyword comparison.
        assert_eq!(
            eval("${matrix[0][1] eq 2}", vars.clone()),
            Some(Value::Bool(true))
        );
        // Indexing a non-container base is unresolvable → null.
        let mut scalar = HashMap::new();
        scalar.insert("n".to_string(), Value::Number(5.into()));
        assert_eq!(eval("${n[0]}", scalar), Some(Value::Null));
    }

    /// P142c: deeply nested parenthesized UEL must fail the parse (return None)
    /// rather than stack-overflow.
    #[test]
    fn p142c_expression_nesting_depth_limit() {
        let deep = format!(
            "${{{}}}",
            format!("{}true{}", "(".repeat(200), ")".repeat(200))
        );
        assert_eq!(
            eval(&deep, HashMap::new()),
            None,
            "200 nested parens must be rejected"
        );

        // Normal nesting still evaluates.
        let ok = format!(
            "${{{}}}",
            format!("{}true{}", "(".repeat(10), ")".repeat(10))
        );
        assert_eq!(eval(&ok, HashMap::new()), Some(Value::Bool(true)));
    }

    // ── P1-C / W1: strict evaluation contract (research §2.1.2) ───────────────

    use crate::el::method_registry::{ExpressionMethodRegistry, with_expression_method_registry};

    fn failing_method_registry() -> ExpressionMethodRegistry {
        let registry = ExpressionMethodRegistry::new();
        registry.register_bean_method("auditBean", "fail", |_| {
            Err("intentional audit failure".to_string())
        });
        registry.register_bean_method("auditBean", "succeed", |_| {
            Ok(Value::String("recorded".to_string()))
        });
        registry
    }

    /// Bytecode path (the production interpreter): a registered method
    /// returning an error must surface from `get_value_strict`, while the
    /// legacy `get_value` keeps flattening it to `None`.
    #[test]
    fn p1c_bytecode_strict_propagates_registered_method_failure() {
        let scope = MapVariableContainer::from_map(HashMap::new());
        let registry = failing_method_registry();
        with_expression_method_registry(&registry, || {
            let failing = SimpleExpression::new("${auditBean.fail()}".to_string());
            // Lenient entry: behaviour unchanged — the bytecode path folds a
            // failing (or missing) registered method into Null, same as
            // before P1-C.
            assert_eq!(failing.get_value(&scope), Some(Value::Null));
            // Strict entry: error with receiver/method context.
            let error = failing
                .get_value_strict(&scope)
                .expect_err("failing registered method must be an Err");
            let rendered = error.to_string();
            assert!(
                rendered.contains("auditBean.fail"),
                "error should name receiver and method, got: {rendered}"
            );
            assert!(
                rendered.contains("intentional audit failure"),
                "error should carry the method message, got: {rendered}"
            );

            // A successful method on the same bean still evaluates normally.
            let succeeding = SimpleExpression::new("${auditBean.succeed()}".to_string());
            assert_eq!(
                succeeding.get_value_strict(&scope),
                Ok(Some(Value::String("recorded".to_string())))
            );

            // The error propagates even when the call is nested in a larger
            // expression instead of being the whole result.
            let nested = SimpleExpression::new("${auditBean.fail() == 'recorded'}".to_string());
            assert!(nested.get_value_strict(&scope).is_err());
        });
    }

    /// W1 contract flip (research §2.1.2): undefined variables and unregistered
    /// methods are `Err` on the strict entry — the old lenient-null behaviour
    /// is gone. Legal null is only a defined-null variable or AstMethod with a
    /// null base.
    #[test]
    fn p1c_strict_undefined_variable_and_missing_method_are_errors() {
        let scope = MapVariableContainer::from_map(HashMap::new());

        // Fast path (pure variable lookup): undefined variable → UnknownProperty.
        assert_eq!(
            SimpleExpression::new("${notDefined}".to_string()).get_value_strict(&scope),
            Err(ExpressionEvalError::UnknownProperty("notDefined".to_string()))
        );

        // No bean of that name registered: the identifier itself is unknown.
        assert_eq!(
            SimpleExpression::new("${ghostBean.run()}".to_string()).get_value_strict(&scope),
            Err(ExpressionEvalError::UnknownProperty("ghostBean".to_string()))
        );

        let registry = ExpressionMethodRegistry::new();
        with_expression_method_registry(&registry, || {
            // Bean exists but this particular method does not → UnknownMethod.
            registry.register_bean_method("auditBean", "present", |_| Ok(Value::Null));
            assert_eq!(
                SimpleExpression::new("${auditBean.absent()}".to_string()).get_value_strict(&scope),
                Err(ExpressionEvalError::UnknownMethod("auditBean.absent".to_string()))
            );
        });
    }

    /// AST interpreter path: same strict contract as the bytecode path.
    #[test]
    fn p1c_ast_strict_path_matches_bytecode_semantics() {
        let scope = MapVariableContainer::from_map(HashMap::new());

        let unregistered = ExpressionParser::new("ghostBean.fail()")
            .parse_expression()
            .expect("fixture expression must parse");
        assert_eq!(
            unregistered.evaluate_strict(&scope),
            Err(ExpressionEvalError::UnknownProperty("ghostBean".to_string())),
            "unregistered bean identifier is UnknownProperty on the AST path too"
        );

        let undefined = ExpressionParser::new("missingVariable + 1")
            .parse_expression()
            .expect("fixture expression must parse");
        assert_eq!(
            undefined.evaluate_strict(&scope),
            Err(ExpressionEvalError::UnknownProperty("missingVariable".to_string()))
        );

        let registry = failing_method_registry();
        with_expression_method_registry(&registry, || {
            let failing = ExpressionParser::new("auditBean.fail()")
                .parse_expression()
                .expect("fixture expression must parse");
            let error = failing
                .evaluate_strict(&scope)
                .expect_err("AST strict path must propagate the method error");
            assert!(
                error.to_string().contains("intentional audit failure"),
                "got: {error}"
            );
        });
    }

    /// `${null}` literal must be legal null on the strict path.
    #[test]
    fn w1_strict_null_literal_is_legal_null() {
        let scope = MapVariableContainer::from_map(HashMap::new());
        assert_eq!(
            SimpleExpression::new("${null}".to_string()).get_value_strict(&scope),
            Ok(Some(Value::Null)),
            "null literal is legal null"
        );
    }

    /// §2.1.2 behaviour table: legal null is ONLY a defined-null variable or
    /// AstMethod/AstProperty with a null base. Everything else unresolved is
    /// Err. Lenient `get_value` is unchanged.
    #[test]
    fn w1_strict_contract_behaviour_table() {
        let mut vars = HashMap::new();
        vars.insert("definedNull".to_string(), Value::Null);
        vars.insert("definedValue".to_string(), Value::from("ok"));
        vars.insert("person".to_string(), serde_json::json!({"name": "Ada"}));
        vars.insert("emptyList".to_string(), serde_json::json!([]));
        let scope = MapVariableContainer::from_map(vars.clone());
        let registry = ExpressionMethodRegistry::new();
        registry.register_bean_method("auditBean", "echo", |args| {
            Ok(args.first().cloned().unwrap_or(Value::Null))
        });

        with_expression_method_registry(&registry, || {
            // ── legal null (2 rows + AstProperty base==null, AstProperty.java:69-71)
            assert_eq!(
                SimpleExpression::new("${definedNull}".to_string()).get_value_strict(&scope),
                Ok(Some(Value::Null)),
                "defined-null variable is legal null"
            );
            assert_eq!(
                SimpleExpression::new("${definedNull.m()}".to_string()).get_value_strict(&scope),
                Ok(Some(Value::Null)),
                "AstMethod base==null is legal null"
            );
            assert_eq!(
                SimpleExpression::new("${definedNull.prop}".to_string()).get_value_strict(&scope),
                Ok(Some(Value::Null)),
                "AstProperty base==null is legal null"
            );

            // ── success
            assert_eq!(
                SimpleExpression::new("${definedValue}".to_string()).get_value_strict(&scope),
                Ok(Some(Value::from("ok")))
            );
            assert_eq!(
                SimpleExpression::new("${auditBean.echo('hi')}".to_string())
                    .get_value_strict(&scope),
                Ok(Some(Value::from("hi")))
            );

            // ── undefined variable / unknown property
            assert_eq!(
                SimpleExpression::new("${nope}".to_string()).get_value_strict(&scope),
                Err(ExpressionEvalError::UnknownProperty("nope".to_string()))
            );
            assert_eq!(
                SimpleExpression::new("${nope}".to_string()).get_value_strict(&scope),
                Err(ExpressionEvalError::UnknownProperty("nope".to_string()))
            );
            // Property on a non-null primitive base → CouldNotResolve → Err.
            assert!(matches!(
                SimpleExpression::new("${definedValue.missingProp}".to_string()).get_value_strict(&scope),
                Err(ExpressionEvalError::UnknownProperty(_))
            ));

            // Map/JSON missing key is MapELResolver legal null (not CouldNotResolve).
            assert_eq!(
                SimpleExpression::new("${person.missing}".to_string()).get_value_strict(&scope),
                Ok(Some(Value::Null)),
                "MapELResolver missing key resolves to null"
            );

            // ── unregistered method
            assert_eq!(
                SimpleExpression::new("${auditBean.absent()}".to_string()).get_value_strict(&scope),
                Err(ExpressionEvalError::UnknownMethod("auditBean.absent".to_string()))
            );
            assert!(matches!(
                SimpleExpression::new("${definedValue.nope()}".to_string()).get_value_strict(&scope),
                Err(ExpressionEvalError::UnknownMethod(_))
            ));

            // ── parse/compile failure
            let compile_err = SimpleExpression::new("${1 +}".to_string())
                .get_value_strict(&scope)
                .expect_err("parse failure must be Err");
            assert!(
                compile_err.is_compile_failure(),
                "parse failure must be CompileFailed, got: {compile_err:?}"
            );

            // Lenient entry is unchanged: undefined/unregistered stays null.
            assert_eq!(
                SimpleExpression::new("${nope}".to_string()).get_value(&scope),
                None
            );
            assert_eq!(
                SimpleExpression::new("${auditBean.absent()}".to_string()).get_value(&scope),
                Some(Value::Null)
            );
        });
    }

    /// Comparison operands that are undefined are Err on the strict path
    /// (research: "comparison 中操作数未定义亦 Err，对齐网关").
    #[test]
    fn w1_strict_comparison_undefined_operand_is_error() {
        let scope = MapVariableContainer::from_map(HashMap::new());
        assert_eq!(
            SimpleExpression::new("${missing == 'a'}".to_string()).get_value_strict(&scope),
            Err(ExpressionEvalError::UnknownProperty("missing".to_string()))
        );
        assert_eq!(
            SimpleExpression::new("${missing != 'a'}".to_string()).get_value_strict(&scope),
            Err(ExpressionEvalError::UnknownProperty("missing".to_string()))
        );
        // Lenient keeps the null-preserving behaviour for conditions.
        assert_eq!(
            SimpleExpression::new("${missing != 'a'}".to_string()).get_value(&scope),
            None
        );
    }

    /// N4-1: CompileFailed is distinguishable from eval-time failures so the
    /// F group can propagate parse errors while catching eval errors.
    #[test]
    fn w1_compile_failure_is_typed_distinct_from_eval_failure() {
        let scope = MapVariableContainer::from_map(HashMap::new());
        let compile = SimpleExpression::new("${1 +}".to_string())
            .get_value_strict(&scope)
            .expect_err("malformed expression");
        assert!(compile.is_compile_failure());

        let eval = SimpleExpression::new("${nope}".to_string())
            .get_value_strict(&scope)
            .expect_err("undefined variable");
        assert!(!eval.is_compile_failure());
        assert!(matches!(eval, ExpressionEvalError::UnknownProperty(_)));
    }

    fn eval_strict(expr: &str) -> Result<Option<Value>, ExpressionEvalError> {
        eval_strict_with(expr, HashMap::new())
    }

    fn eval_strict_with(
        expr: &str,
        variables: HashMap<String, Value>,
    ) -> Result<Option<Value>, ExpressionEvalError> {
        let scope = MapVariableContainer::from_map(variables);
        SimpleExpression::new(expr.to_string()).get_value_strict(&scope)
    }

    // ── P1-2 arithmetic gold standard (A-DIV / A-MOD / A-TYPE-FAIL) ──────────

    #[test]
    fn a_div_long_0_is_infinity_sentinel_not_null() {
        // `${1/0}` — Java NumberOperations.div:128 Double/ → +∞
        assert_eq!(
            eval_strict("${1/0}"),
            Ok(Some(Value::String("Infinity".to_string())))
        );
        assert_ne!(
            eval_strict("${1/0}"),
            Ok(Some(Value::Null)),
            "1/0 ≠ Null is a CP hard condition"
        );
    }

    #[test]
    fn a_div_double_0_is_infinity_sentinel() {
        assert_eq!(
            eval_strict("${1.0/0}"),
            Ok(Some(Value::String("Infinity".to_string())))
        );
    }

    #[test]
    fn a_div_neg_long_0_is_neg_infinity_sentinel() {
        // Exact spelling "-Infinity" (no leading space) so to_f64 can parse it back.
        assert_eq!(
            eval_strict("${(-1)/0}"),
            Ok(Some(Value::String("-Infinity".to_string())))
        );
    }

    #[test]
    fn a_div_bd_0_is_err() {
        // M3: BigDecimal operand via internal construct (expression-layer).
        let mut vars = HashMap::new();
        vars.insert("bd".to_string(), SimpleExpression::big_decimal_operand("1"));
        let err = eval_strict_with("${bd/0}", vars).expect_err("BD /0 must Err (ELException)");
        assert!(matches!(err, ExpressionEvalError::EvalFailed(_)));
    }

    #[test]
    fn a_mod_long_0_is_err() {
        // `${1%0}` — Java NumberOperations.mod:141 Long%0 → throw
        let err = eval_strict("${1%0}").expect_err("Long %0 must Err, not Null");
        assert!(matches!(err, ExpressionEvalError::EvalFailed(_)));
        assert_ne!(eval_strict("${1%0}"), Ok(Some(Value::Null)));
    }

    #[test]
    fn a_mod_bd_0_is_nan_sentinel_not_null() {
        // Java NumberOperations.mod:135-136 BD% → Double% → NaN
        let mut vars = HashMap::new();
        vars.insert("bd".to_string(), SimpleExpression::big_decimal_operand("1"));
        assert_eq!(
            eval_strict_with("${bd%0}", vars),
            Ok(Some(Value::String("NaN".to_string())))
        );
    }

    #[test]
    fn a_mod_double_0_is_nan_sentinel() {
        assert_eq!(
            eval_strict("${1.0%0}"),
            Ok(Some(Value::String("NaN".to_string())))
        );
    }

    #[test]
    fn a_zero_over_zero_is_nan_sentinel() {
        assert_eq!(
            eval_strict("${0/0}"),
            Ok(Some(Value::String("NaN".to_string())))
        );
    }

    #[test]
    fn a_nonfinite_overflow_and_composition_use_sentinels() {
        // M2: all non-finite channels — overflow and composition must not be Null.
        assert_eq!(
            eval_strict("${1e308*10}"),
            Ok(Some(Value::String("Infinity".to_string())))
        );
        assert_ne!(
            eval_strict("${1e308*10}"),
            Ok(Some(Value::Null)),
            "overflow must not be disguised as Null"
        );
        assert_eq!(
            eval_strict("${(1/0)+1}"),
            Ok(Some(Value::String("Infinity".to_string())))
        );
        assert_ne!(eval_strict("${(1/0)+1}"), Ok(Some(Value::Null)));
    }

    #[test]
    fn a_type_fail_is_err_not_null() {
        // Type coerce failure → Err (禁止 unwrap_or(Null)).
        let err = eval_strict("${1-'abc'}").expect_err("type coerce failure must Err");
        assert!(matches!(err, ExpressionEvalError::EvalFailed(_)));
        let err = eval_strict("${1*true}").expect_err("bool operand must Err");
        assert!(matches!(err, ExpressionEvalError::EvalFailed(_)));
    }

    #[test]
    fn a_long_overflow_wraps_like_java() {
        // X1: Java NumberOperations Long +,-,* wrap (two's complement).
        // i64::MAX + 1 → i64::MIN (wrap), not Err and not f64 promotion.
        let max = Value::Number(serde_json::Number::from(i64::MAX));
        let one = Value::Number(serde_json::Number::from(1_i64));
        assert_eq!(
            SimpleExpression::arithmetic_op(&max, &one, '+'),
            Ok(Value::Number(serde_json::Number::from(i64::MIN))),
            "Long overflow must wrap like Java"
        );
        let min = Value::Number(serde_json::Number::from(i64::MIN));
        assert_eq!(
            SimpleExpression::arithmetic_op(&min, &one, '-'),
            Ok(Value::Number(serde_json::Number::from(i64::MAX)))
        );
    }

    #[test]
    fn a_to_f64_accepts_sentinels() {
        // EL numeric consumption can parse the sentinels back.
        assert_eq!(
            SimpleExpression::to_f64(&Value::String("Infinity".to_string())),
            Some(f64::INFINITY)
        );
        assert_eq!(
            SimpleExpression::to_f64(&Value::String("-Infinity".to_string())),
            Some(f64::NEG_INFINITY)
        );
        assert!(SimpleExpression::to_f64(&Value::String("NaN".to_string()))
            .is_some_and(f64::is_nan));
    }

    #[test]
    fn a_execute_strict_syncs_with_eval_strict() {
        // 强制项 6: execute_strict path must agree (strict 全改).
        let scope = MapVariableContainer::from_map(HashMap::new());
        let compiled = compile_global("${1/0}").expect("compile ${1/0}");
        assert_eq!(
            compiled.execute_strict(&scope),
            Ok(Some(Value::String("Infinity".to_string())))
        );
        let compiled = compile_global("${1%0}").expect("compile ${1%0}");
        assert!(compiled.execute_strict(&scope).is_err());
    }
}
