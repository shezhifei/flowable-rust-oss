use crate::el::variable_container::VariableContainer;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub trait Expression {
    fn get_value(&self, scope: &dyn VariableContainer) -> Option<serde_json::Value>;
}

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
        if let Some(type_name) =
            crate::el::method_registry::parse_static_type_reference(name)
        {
            return Some(crate::el::method_registry::static_type_marker(type_name));
        }
        let method_registry =
            crate::el::method_registry::current_expression_method_registry();
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
            "and" | "or" | "eq" | "ne" | "lt" | "le" | "ge" | "gt" | "div" | "mod" | "not"
                | "empty"
        )
    }

    fn arithmetic_op(left: &Value, right: &Value, op: char) -> Option<Value> {
        if matches!(op, '+' | '-' | '*' | '%') {
            if let (Value::Number(lhs), Value::Number(rhs)) = (left, right) {
                if let (Some(lhs), Some(rhs)) = (lhs.as_i64(), rhs.as_i64()) {
                    return Some(Value::Number(match op {
                        '+' => lhs.checked_add(rhs)?.into(),
                        '-' => lhs.checked_sub(rhs)?.into(),
                        '*' => lhs.checked_mul(rhs)?.into(),
                        '%' if rhs != 0 => (lhs % rhs).into(),
                        _ => return None,
                    }));
                }
                if let (Some(lhs), Some(rhs)) = (lhs.as_u64(), rhs.as_u64()) {
                    return Some(Value::Number(match op {
                        '+' => lhs.checked_add(rhs)?.into(),
                        '*' => lhs.checked_mul(rhs)?.into(),
                        '%' if rhs != 0 => (lhs % rhs).into(),
                        _ => return None,
                    }));
                }
            }
        }
        let l = Self::to_f64(left)?;
        let r = Self::to_f64(right)?;
        let result = match op {
            '+' => l + r,
            '-' => l - r,
            '*' => l * r,
            '/' => {
                if r == 0.0 {
                    return Some(Value::Null);
                }
                l / r
            }
            '%' => {
                if r == 0.0 {
                    return Some(Value::Null);
                }
                l % r
            }
            _ => return None,
        };
        // Try to return integer if possible
        if result.fract() == 0.0 && result >= i64::MIN as f64 && result <= i64::MAX as f64 {
            Some(Value::Number(serde_json::Number::from(result as i64)))
        } else {
            serde_json::Number::from_f64(result).map(Value::Number)
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
    fn invoke_method(receiver: &Value, method: &str, args: &[Value]) -> Option<Value> {
        if let Some((receiver_name, is_static_type)) =
            crate::el::method_registry::marker_receiver(receiver)
        {
            let registry = crate::el::method_registry::current_expression_method_registry();
            return if is_static_type {
                registry.invoke_static(receiver_name, method, args)
            } else {
                registry.invoke_bean(receiver_name, method, args)
            };
        }
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
                _ => Some(Value::Null),
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
                _ => Some(Value::Null),
            },
            Value::Array(arr) => match method {
                "size" | "length" if args.is_empty() => {
                    Some(Value::Number(serde_json::Number::from(arr.len() as i64)))
                }
                "isEmpty" if args.is_empty() => Some(Value::Bool(arr.is_empty())),
                _ => Some(Value::Null),
            },
            Value::Bool(_) | Value::Null | Value::Object(_) => Some(Value::Null),
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
    Conditional(
        Box<ExpressionAst>,
        Box<ExpressionAst>,
        Box<ExpressionAst>,
    ),
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
                if let Some(result) = SimpleExpression::arithmetic_op(&l, &r, '+') {
                    Some(result)
                } else {
                    // String concatenation fallback for +
                    None
                }
            }
            ExpressionAst::Sub(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(SimpleExpression::arithmetic_op(&l, &r, '-').unwrap_or(Value::Null))
            }
            ExpressionAst::Mul(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(SimpleExpression::arithmetic_op(&l, &r, '*').unwrap_or(Value::Null))
            }
            ExpressionAst::Div(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(SimpleExpression::arithmetic_op(&l, &r, '/').unwrap_or(Value::Null))
            }
            ExpressionAst::Mod(left, right) => {
                let l = left.evaluate(scope)?;
                let r = right.evaluate(scope)?;
                Some(SimpleExpression::arithmetic_op(&l, &r, '%').unwrap_or(Value::Null))
            }
            ExpressionAst::Not(operand) => {
                let v = operand.evaluate(scope)?;
                Some(Value::Bool(!SimpleExpression::is_truthy(&v)))
            }
            ExpressionAst::Neg(operand) => {
                let v = operand.evaluate(scope)?;
                if let Some(n) = SimpleExpression::to_f64(&v) {
                    serde_json::Number::from_f64(-n).map(Value::Number)
                } else {
                    Some(Value::Null)
                }
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
                    if let Some(result) = SimpleExpression::arithmetic_op(&l, &r, '+') {
                        stack.push(result);
                    } else {
                        return None;
                    }
                }
                Instruction::Sub => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(SimpleExpression::arithmetic_op(&l, &r, '-').unwrap_or(Value::Null));
                }
                Instruction::Mul => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(SimpleExpression::arithmetic_op(&l, &r, '*').unwrap_or(Value::Null));
                }
                Instruction::Div => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(SimpleExpression::arithmetic_op(&l, &r, '/').unwrap_or(Value::Null));
                }
                Instruction::Mod => {
                    let r = stack.pop()?;
                    let l = stack.pop()?;
                    stack.push(SimpleExpression::arithmetic_op(&l, &r, '%').unwrap_or(Value::Null));
                }
                Instruction::Not => {
                    let v = stack.pop()?;
                    stack.push(Value::Bool(!SimpleExpression::is_truthy(&v)));
                }
                Instruction::Neg => {
                    let v = stack.pop()?;
                    if let Some(n) = SimpleExpression::to_f64(&v) {
                        stack.push(
                            serde_json::Number::from_f64(-n)
                                .map(Value::Number)
                                .unwrap_or(Value::Null),
                        );
                    } else {
                        stack.push(Value::Null);
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
/// Rules:
/// - Literal text is copied as-is.
/// - `\${` is an escaped dollar-brace and yields the two characters `${`
///   (the backslash is consumed).
/// - `${…}` segments are compiled with the existing `ExpressionParser` +
///   `Compiler` and evaluated against `scope`. Nested braces are tracked so
///   `${fn({a:1})}` boundaries are correct; our UEL subset may still reject
///   the inner syntax, but scanning is brace-aware.
/// - A segment that fails to parse/evaluate contributes an empty string —
///   aligned with pure-expression `get_value` → `None` treated as empty by
///   mail body templates (`evaluate_mail_body_template`).
/// - Pure whole-string `${…}` and pure literals are both handled here so
///   call sites can use one evaluator for mail text/html/textVar/htmlVar.
///
/// Other EL call sites must keep using [`SimpleExpression`] so pure
/// `${…}` / literal paths are unchanged.
pub fn evaluate_composite_expression(text: &str, scope: &dyn VariableContainer) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Escaped composite start: `\${` → literal `${`.
        if bytes[i] == b'\\'
            && i + 2 < bytes.len()
            && bytes[i + 1] == b'$'
            && bytes[i + 2] == b'{'
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
                // Unclosed `${` — treat remaining text as literal (JUEL would
                // fail at parse; we degrade to literal to avoid hard errors
                // on malformed mail bodies).
                out.push_str(&text[i..]);
                break;
            }
            let inner = &text[expr_start..j];
            let whole = format!("${{{}}}", inner);
            let segment = SimpleExpression::new(whole)
                .get_value(scope)
                .map(|v| value_to_composite_string(&v))
                .unwrap_or_default();
            out.push_str(&segment);
            i = j + 1;
            continue;
        }

        // Safe: we walk UTF-8 by character when not on ASCII `$`/`\` markers.
        let ch = text[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
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

impl Expression for SimpleExpression {
    fn get_value(&self, scope: &dyn VariableContainer) -> Option<serde_json::Value> {
        // Phase 1: fast path for common expression shapes (pure variable lookup,
        // simple comparison). Detected once, cached in cached_fast_path.
        let fast_path = self
            .cached_fast_path
            .get_or_init(|| Self::detect_fast_path(&self.expression_text));
        if let Some(fp) = fast_path {
            return match fp {
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
            };
        }

        // Phase 2: compile AST to bytecode once, then execute via stack-based
        // interpreter. Replaces recursive evaluate() with a flat instruction loop.
        // The per-instance OnceLock stores an `Option<Arc<CompiledExpression>>`
        // so that, on the slow path, we share a single Arc with every other
        // `SimpleExpression` carrying the same expression text via the
        // process-wide cache.
        let compiled = self.cached_compiled.get_or_init(|| {
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
        });
        compiled.as_ref().and_then(|c| c.execute(scope))
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
        evaluate_composite_expression(text, &scope)
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
    fn composite_expression_failed_segment_becomes_empty() {
        // Missing variable → pure SimpleExpression returns None → empty segment.
        let mut vars = HashMap::new();
        vars.insert("known".to_string(), Value::from("ok"));
        assert_eq!(
            eval_composite("A=${known};B=${missing};C", vars),
            "A=ok;B=;C"
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
        assert_eq!(
            expression.get_value(&scope),
            Some(Value::Number(15.into()))
        );
        // Second call uses cache
        assert_eq!(
            expression.get_value(&scope),
            Some(Value::Number(15.into()))
        );
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
        assert_eq!(
            eval("${order eq 7}", vars.clone()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            eval("${order ne 3}", vars.clone()),
            Some(Value::Bool(true))
        );
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
        assert_eq!(eval("${empty 'abc'}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${empty 0}", vars.clone()), Some(Value::Bool(false)));
        assert_eq!(eval("${empty false}", vars.clone()), Some(Value::Bool(false)));

        let mut vars = HashMap::new();
        vars.insert("emptyList".to_string(), Value::Array(vec![]));
        vars.insert("fullList".to_string(), Value::Array(vec![Value::from(1)]));
        vars.insert("emptyMap".to_string(), Value::Object(serde_json::Map::new()));
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
}
