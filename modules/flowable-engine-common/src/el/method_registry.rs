use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock, RwLock};

const STATIC_TYPE_MARKER: &str = "__flowable_expression_static_type";
const BEAN_MARKER: &str = "__flowable_expression_bean";

type ExpressionMethod = Arc<dyn Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct ExpressionMethodRegistry {
    bean_methods: Arc<RwLock<HashMap<String, HashMap<String, ExpressionMethod>>>>,
    static_methods: Arc<RwLock<HashMap<String, HashMap<String, ExpressionMethod>>>>,
}

impl fmt::Debug for ExpressionMethodRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bean_count = self
            .bean_methods
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .len();
        let static_type_count = self
            .static_methods
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .len();
        formatter
            .debug_struct("ExpressionMethodRegistry")
            .field("bean_count", &bean_count)
            .field("static_type_count", &static_type_count)
            .finish()
    }
}

impl Default for ExpressionMethodRegistry {
    fn default() -> Self {
        let registry = Self::new();
        registry.register_java_math_methods();
        registry
    }
}

impl ExpressionMethodRegistry {
    pub fn new() -> Self {
        Self {
            bean_methods: Arc::new(RwLock::new(HashMap::new())),
            static_methods: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn register_bean_method<F>(&self, bean: &str, method: &str, function: F)
    where
        F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static,
    {
        Self::register(&self.bean_methods, bean, method, Arc::new(function));
    }

    pub fn register_static_method<F>(&self, type_name: &str, method: &str, function: F)
    where
        F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static,
    {
        Self::register(&self.static_methods, type_name, method, Arc::new(function));
    }

    pub fn contains_bean(&self, bean: &str) -> bool {
        self.bean_methods
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(bean)
    }

    pub fn evaluate(
        &self,
        expression: &str,
        scope: &dyn crate::el::variable_container::VariableContainer,
    ) -> Option<Value> {
        use crate::el::expression::Expression;

        with_expression_method_registry(self, || {
            crate::el::expression::SimpleExpression::new(expression.to_string()).get_value(scope)
        })
    }

    /// Invoke a registered bean method.
    ///
    /// The two-level result mirrors Java UEL's distinction
    /// (`AstMethod.eval`, AstMethod.java:98-101):
    /// - `None` — the bean or method is not registered. Java's resolver chain
    ///   leaves the property unresolved (lenient → null); callers must keep
    ///   treating this as "no such method", never as an evaluation failure.
    /// - `Some(Ok(value))` — the method ran and returned `value`.
    /// - `Some(Err(message))` — the method was found but its execution failed.
    ///   Java wraps the target exception in an `ELException`/`ExpressionException`
    ///   and lets it escape; strict callers must propagate this error.
    pub fn invoke_bean(
        &self,
        bean: &str,
        method: &str,
        arguments: &[Value],
    ) -> Option<Result<Value, String>> {
        Self::invoke(&self.bean_methods, bean, method, arguments)
    }

    /// Invoke a registered static-type method (`T(typeName).method(...)`).
    /// See [`Self::invoke_bean`] for the `Option<Result<_, _>>` semantics.
    pub fn invoke_static(
        &self,
        type_name: &str,
        method: &str,
        arguments: &[Value],
    ) -> Option<Result<Value, String>> {
        Self::invoke(&self.static_methods, type_name, method, arguments)
    }

    fn register(
        methods: &RwLock<HashMap<String, HashMap<String, ExpressionMethod>>>,
        receiver: &str,
        method: &str,
        function: ExpressionMethod,
    ) {
        methods
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .entry(receiver.to_string())
            .or_default()
            .insert(method.to_string(), function);
    }

    /// Returns `None` when the receiver/method pair is not registered; the
    /// registered function's own `Result` is passed through untouched so a
    /// method execution failure is no longer flattened into "no such method".
    fn invoke(
        methods: &RwLock<HashMap<String, HashMap<String, ExpressionMethod>>>,
        receiver: &str,
        method: &str,
        arguments: &[Value],
    ) -> Option<Result<Value, String>> {
        let function = methods
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(receiver)
            .and_then(|receiver_methods| receiver_methods.get(method))
            .cloned()?;
        Some(function(arguments))
    }

    fn register_java_math_methods(&self) {
        for type_name in ["java.lang.Math", "Math"] {
            self.register_static_method(type_name, "max", |arguments| {
                numeric_extreme(arguments, true)
            });
            self.register_static_method(type_name, "min", |arguments| {
                numeric_extreme(arguments, false)
            });
            self.register_static_method(type_name, "abs", |arguments| {
                let value = one_numeric_argument(arguments, "abs")?;
                number_value(value.abs())
            });
            self.register_static_method(type_name, "sqrt", |arguments| {
                let value = one_numeric_argument(arguments, "sqrt")?;
                number_value(value.sqrt())
            });
            self.register_static_method(type_name, "pow", |arguments| {
                if arguments.len() != 2 {
                    return Err("Math.pow expects two numeric arguments".to_string());
                }
                let left = numeric_value(&arguments[0])?;
                let right = numeric_value(&arguments[1])?;
                number_value(left.powf(right))
            });
        }
    }
}

fn numeric_extreme(arguments: &[Value], maximum: bool) -> Result<Value, String> {
    if arguments.len() != 2 {
        return Err("Math min/max expects two numeric arguments".to_string());
    }
    let left = numeric_value(&arguments[0])?;
    let right = numeric_value(&arguments[1])?;
    if (maximum && left >= right) || (!maximum && left <= right) {
        Ok(arguments[0].clone())
    } else {
        Ok(arguments[1].clone())
    }
}

fn one_numeric_argument(arguments: &[Value], method: &str) -> Result<f64, String> {
    if arguments.len() != 1 {
        return Err(format!("Math.{method} expects one numeric argument"));
    }
    numeric_value(&arguments[0])
}

fn numeric_value(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| "numeric argument is outside the supported range".to_string()),
        Value::String(text) => text
            .parse::<f64>()
            .map_err(|_| format!("'{text}' is not numeric")),
        _ => Err(format!("'{value}' is not numeric")),
    }
}

fn number_value(value: f64) -> Result<Value, String> {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| "numeric method returned a non-finite value".to_string())
}

thread_local! {
    static ACTIVE_REGISTRIES: RefCell<Vec<ExpressionMethodRegistry>> = const { RefCell::new(Vec::new()) };
}

struct RegistryContextGuard;

impl Drop for RegistryContextGuard {
    fn drop(&mut self) {
        ACTIVE_REGISTRIES.with(|registries| {
            registries.borrow_mut().pop();
        });
    }
}

pub fn with_expression_method_registry<T>(
    registry: &ExpressionMethodRegistry,
    operation: impl FnOnce() -> T,
) -> T {
    ACTIVE_REGISTRIES.with(|registries| registries.borrow_mut().push(registry.clone()));
    let _guard = RegistryContextGuard;
    operation()
}

pub fn current_expression_method_registry() -> ExpressionMethodRegistry {
    ACTIVE_REGISTRIES
        .with(|registries| registries.borrow().last().cloned())
        .unwrap_or_else(|| default_expression_method_registry().clone())
}

fn default_expression_method_registry() -> &'static ExpressionMethodRegistry {
    static DEFAULT_REGISTRY: OnceLock<ExpressionMethodRegistry> = OnceLock::new();
    DEFAULT_REGISTRY.get_or_init(ExpressionMethodRegistry::default)
}

pub fn static_type_marker(type_name: &str) -> Value {
    serde_json::json!({ STATIC_TYPE_MARKER: type_name })
}

pub fn bean_marker(bean: &str) -> Value {
    serde_json::json!({ BEAN_MARKER: bean })
}

pub fn marker_receiver(receiver: &Value) -> Option<(&str, bool)> {
    let object = receiver.as_object()?;
    if let Some(type_name) = object.get(STATIC_TYPE_MARKER).and_then(Value::as_str) {
        return Some((type_name, true));
    }
    object
        .get(BEAN_MARKER)
        .and_then(Value::as_str)
        .map(|bean| (bean, false))
}

pub fn parse_static_type_reference(name: &str) -> Option<&str> {
    name.trim()
        .strip_prefix("T(")?
        .strip_suffix(')')
        .map(str::trim)
        .filter(|type_name| !type_name.is_empty())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// P1-C: at the registry boundary "method not registered" (`None`) must
    /// be distinguishable from "registered method failed" (`Some(Err(_))`).
    /// Before the fix `invoke` ran `function(arguments).ok()` and collapsed
    /// both into `None`.
    #[test]
    fn p1c_invoke_distinguishes_missing_method_from_failing_method() {
        let registry = ExpressionMethodRegistry::new();
        registry.register_bean_method("auditBean", "fail", |_| {
            Err("intentional failure".to_string())
        });
        registry.register_bean_method("auditBean", "echo", |arguments| Ok(arguments[0].clone()));

        // Unknown bean and unknown method: not registered → None.
        assert_eq!(registry.invoke_bean("otherBean", "any", &[]), None);
        assert_eq!(registry.invoke_bean("auditBean", "missing", &[]), None);

        // Registered method that fails: its error is passed through.
        match registry.invoke_bean("auditBean", "fail", &[]) {
            Some(Err(message)) => assert!(
                message.contains("intentional failure"),
                "expected the method error message, got: {message}"
            ),
            other => panic!("expected Some(Err), got {other:?}"),
        }

        // Registered method that succeeds: value is passed through.
        assert_eq!(
            registry.invoke_bean("auditBean", "echo", &[Value::Bool(true)]),
            Some(Ok(Value::Bool(true)))
        );
    }

    /// The same distinction holds for static-type methods (`T(typeName).m()`).
    #[test]
    fn p1c_invoke_static_distinguishes_missing_from_failing() {
        let registry = ExpressionMethodRegistry::new();
        registry
            .register_static_method("com.acme.Risky", "boom", |_| Err("static boom".to_string()));

        assert_eq!(
            registry.invoke_static("com.acme.Risky", "missing", &[]),
            None
        );
        assert_eq!(registry.invoke_static("com.acme.Other", "boom", &[]), None);
        match registry.invoke_static("com.acme.Risky", "boom", &[]) {
            Some(Err(message)) => assert!(message.contains("static boom")),
            other => panic!("expected Some(Err), got {other:?}"),
        }
    }
}
