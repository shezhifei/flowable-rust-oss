//! Event-registry correlation key generation and subscription matching.
//!
//! Java counterparts (line numbers verified 2026-08-02):
//! - `CorrelationUtil.java:30-67` — build parameter map from
//!   `eventCorrelationParameter` extension elements, then `generateKey`
//! - `DefaultCorrelationKeyGenerator.java:38-57` — sorted `key=value;` MD5 hex
//! - `BaseEventRegistryEventConsumer.java:156-175` — match query:
//!   `configuration IS NULL OR configuration IN (keys)` (empty keys → only NULL)
//! - `BpmnEventRegistryEventConsumer.java:125-219` — `storeAsUniqueReferenceId`
//!   dedup via process-instance referenceId count
//!
//! **P93 first-phase cuts (documented for follow-up):**
//! - No tenant fallback branch (`BaseEventRegistryEventConsumer:177-265`)
//! - No distributed lock for unique reference (count-only; lock path :139-193)
//!
//! P98: `generate_event_correlation_keys` now emits the full power set minus
//! empty (`BaseEventRegistryEventConsumer.generateCorrelationKeys:76-131`);
//! the subscription side keeps the single full-parameter key
//! (`CorrelationUtil.java:30-67`, untouched).
//!
//! Correlation value stringification (verified 2026-08-02): the default
//! `CorrelationValueTransformer` (`transformValue`/`transformRawValue`) is
//! identity; `DefaultCorrelationKeyGenerator` then stringifies via
//! `Object.toString` (null → `""`). Rust `json_value_to_correlation_string`
//! is equivalent for STRING/BOOLEAN/INTEGER/LONG values; JSON-object payloads
//! and Java `Double` scientific notation are residual deviations (unchanged
//! here — values reach key generation already stringified).

use crate::persistence::db_session::DbSession;
use crate::persistence::runtime_store::RuntimeStore;
use crate::runtime::process_instance::ProcessInstance;
use flowable_bpmn_model::model::{BaseElement, ExtensionElement};
use flowable_engine_common::el::expression::{Expression, SimpleExpression};
use flowable_engine_common::el::variable_container::VariableContainer;
use indexmap::IndexMap;
use md5::{Digest, Md5};
use serde_json::Value;
use std::collections::BTreeMap;

/// Java `BpmnXMLConstants.ELEMENT_EVENT_CORRELATION_PARAMETER`.
pub const ELEMENT_EVENT_CORRELATION_PARAMETER: &str = "eventCorrelationParameter";
/// Java `BpmnXMLConstants.ELEMENT_TRIGGER_EVENT_CORRELATION_PARAMETER`
/// (send-event triggerable receive-side correlation;
/// `SendEventTaskActivityBehavior.java:140`).
pub const ELEMENT_TRIGGER_EVENT_CORRELATION_PARAMETER: &str =
    "triggerEventCorrelationParameter";
/// Java `BpmnXMLConstants.ELEMENT_EVENT_TYPE`.
pub const ELEMENT_EVENT_TYPE: &str = "eventType";
/// Java `BpmnXMLConstants.START_EVENT_CORRELATION_CONFIGURATION`.
pub const START_EVENT_CORRELATION_CONFIGURATION: &str = "startEventCorrelationConfiguration";
/// Java `BpmnXMLConstants.START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID`.
pub const START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID: &str = "storeAsUniqueReferenceId";
/// Java `BpmnXMLConstants.START_EVENT_CORRELATION_MANUAL`.
pub const START_EVENT_CORRELATION_MANUAL: &str = "manualSubscription";
/// Java `ReferenceTypes.EVENT_PROCESS`.
pub const REFERENCE_TYPE_EVENT_PROCESS: &str = "event-to-bpmn-2.0-process";

/// Generate a correlation key from a parameter map.
///
/// Java `DefaultCorrelationKeyGenerator.generateKey` (DefaultCorrelationKeyGenerator.java:38-57):
/// sort keys, append `key=value;` (null value → empty string), MD5, hex without
/// leading zeros (`String.format("%x", new BigInteger(1, bytes))`).
pub fn generate_correlation_key(params: &BTreeMap<String, Option<String>>) -> String {
    let mut sb = String::new();
    for (key, value) in params {
        let value = value.as_deref().unwrap_or("");
        sb.push_str(key);
        sb.push('=');
        sb.push_str(value);
        sb.push(';');
    }
    let digest = Md5::digest(sb.as_bytes());
    bytes_to_java_hex(digest.as_slice())
}

/// Generate all correlation keys for the event parameters: the power set of the
/// parameter subsets minus the empty set (2^n - 1 keys for n parameters).
///
/// Java `BaseEventRegistryEventConsumer.generateCorrelationKeys` (76-131):
/// n=1 special-cases to the single full-parameter key; n=2 emits the full key
/// plus both single-parameter keys; n>=3 enumerates bit masks 1..2^n-1 where
/// each set bit includes that parameter. Subset membership alone determines the
/// key value (keys are sorted inside `generate_correlation_key`), so the
/// enumeration order only affects list order, which is irrelevant to matching.
///
/// Order is deterministic: the input is a `BTreeMap`, so subsets are enumerated
/// in sorted key order (Java uses payload-instance order, which does not change
/// any key value). Java has no cap on n and we stay faithful; the only guard is
/// an explicit panic when 2^n overflows `usize` (n >= 64), which Java would
/// otherwise turn into an OOM-sized `HashSet`.
pub fn generate_event_correlation_keys(
    params: &BTreeMap<String, Option<String>>,
) -> Vec<String> {
    if params.is_empty() {
        return Vec::new();
    }
    let ordered: Vec<(&String, &Option<String>)> = params.iter().collect();
    // 2^n subsets; emit all but the empty set (counter starts at 1, Java :117).
    let subset_count = 1usize
        .checked_shl(ordered.len() as u32)
        .expect("correlation power-set overflow: too many parameters (2^n must fit usize)");
    let mut keys = Vec::with_capacity(subset_count - 1);
    for counter in 1..subset_count {
        let mut subset = BTreeMap::new();
        for (i, (key, value)) in ordered.iter().enumerate() {
            if (counter & (1usize << i)) != 0 {
                subset.insert((*key).clone(), (*value).clone());
            }
        }
        keys.push(generate_correlation_key(&subset));
    }
    keys
}

/// Java match semantics (`BaseEventRegistryEventConsumer.findEventSubscriptions:163-174`):
/// - empty keys → only subscriptions with `configuration IS NULL`
/// - non-empty keys → `configuration IS NULL OR configuration IN (keys)`
pub fn matches_subscription_configuration(
    subscription_configuration: Option<&str>,
    correlation_keys: &[String],
) -> bool {
    if correlation_keys.is_empty() {
        return subscription_configuration.is_none();
    }
    match subscription_configuration {
        None => true,
        Some(cfg) => correlation_keys.iter().any(|k| k == cfg),
    }
}

/// Filter items that match event-registry correlation configuration.
pub fn filter_by_configuration<'a, T, F>(
    items: impl IntoIterator<Item = &'a T>,
    correlation_keys: &[String],
    configuration_of: F,
) -> Vec<&'a T>
where
    F: Fn(&T) -> Option<&str>,
{
    items
        .into_iter()
        .filter(|item| matches_subscription_configuration(configuration_of(item), correlation_keys))
        .collect()
}

/// Extract first extension element text (e.g. `eventType`, correlation config).
pub fn extension_element_text(
    extensions: &IndexMap<String, Vec<ExtensionElement>>,
    name: &str,
) -> Option<String> {
    extensions
        .get(name)
        .and_then(|list| list.first())
        .and_then(|el| el.element_text.as_ref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Attribute value from an extension element (local name, no namespace).
fn extension_attr(el: &ExtensionElement, name: &str) -> Option<String> {
    el.base_element
        .attributes
        .get(name)
        .and_then(|attrs| attrs.first())
        .and_then(|a| a.value.clone())
}

/// Collect correlation-parameter name→value map from extension elements named
/// `element_name` (Java `CorrelationUtil.getCorrelationKey(elementName, …)`).
///
/// Java `CorrelationUtil.java:37-60`:
/// - attributes `name` and `value`
/// - when `variable_scope` is None (deploy-time): store raw value expression
/// - when present: evaluate expression against the scope
pub fn correlation_parameters_from_extensions_named(
    extensions: &IndexMap<String, Vec<ExtensionElement>>,
    element_name: &str,
    variable_scope: Option<&dyn VariableContainer>,
) -> BTreeMap<String, Option<String>> {
    let mut params = BTreeMap::new();
    let Some(elements) = extensions.get(element_name) else {
        return params;
    };
    for el in elements {
        let Some(name) = extension_attr(el, "name").filter(|s| !s.is_empty()) else {
            continue;
        };
        let value_expression = extension_attr(el, "value");
        let value = match value_expression {
            Some(expr) if !expr.is_empty() => {
                if let Some(scope) = variable_scope {
                    Some(evaluate_correlation_value(&expr, scope))
                } else {
                    // Deploy-time path: CorrelationUtil.java:53-54
                    Some(expr)
                }
            }
            _ => None,
        };
        params.insert(name, value);
    }
    params
}

/// Collect `eventCorrelationParameter` name→value map from extension elements.
pub fn correlation_parameters_from_extensions(
    extensions: &IndexMap<String, Vec<ExtensionElement>>,
    variable_scope: Option<&dyn VariableContainer>,
) -> BTreeMap<String, Option<String>> {
    correlation_parameters_from_extensions_named(
        extensions,
        ELEMENT_EVENT_CORRELATION_PARAMETER,
        variable_scope,
    )
}

/// Build a correlation parameter map from a JSON event payload object.
///
/// Each object field becomes a correlation parameter; non-object payloads yield empty.
/// Mirrors CMMN `correlation_params_from_payload` /
/// Java `BaseEventRegistryEventConsumer.generateCorrelationKeys` input.
pub fn correlation_params_from_payload(payload: &Value) -> BTreeMap<String, Option<String>> {
    let mut params = BTreeMap::new();
    let Some(object) = payload.as_object() else {
        return params;
    };
    for (key, value) in object {
        params.insert(key.clone(), Some(json_value_to_correlation_string(value)));
    }
    params
}

fn evaluate_correlation_value(expression: &str, scope: &dyn VariableContainer) -> String {
    // Prefer UEL evaluation for `${...}`; plain literals pass through
    // (Java ExpressionManager also returns plain strings for non-UEL text).
    if expression.starts_with("${") && expression.ends_with('}') {
        if let Some(value) = SimpleExpression::new(expression.to_string()).get_value(scope) {
            return json_value_to_correlation_string(&value);
        }
    }
    expression.to_string()
}

fn json_value_to_correlation_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Correlation key for a named extension element list, or `None` when empty.
///
/// Java `CorrelationUtil.getCorrelationKey(elementName, …)` (CorrelationUtil.java:34-66).
pub fn correlation_key_from_extensions_named(
    extensions: &IndexMap<String, Vec<ExtensionElement>>,
    element_name: &str,
    variable_scope: Option<&dyn VariableContainer>,
) -> Option<String> {
    let params =
        correlation_parameters_from_extensions_named(extensions, element_name, variable_scope);
    if params.is_empty() {
        return None;
    }
    Some(generate_correlation_key(&params))
}

/// Correlation key for a flow element's `eventCorrelationParameter` map, or
/// `None` when no such extensions are present.
///
/// Java `CorrelationUtil.getCorrelationKey` (CorrelationUtil.java:34-66).
pub fn correlation_key_from_extensions(
    extensions: &IndexMap<String, Vec<ExtensionElement>>,
    variable_scope: Option<&dyn VariableContainer>,
) -> Option<String> {
    correlation_key_from_extensions_named(
        extensions,
        ELEMENT_EVENT_CORRELATION_PARAMETER,
        variable_scope,
    )
}

/// Convenience: correlation key from a base element (flow element extensions).
pub fn correlation_key_from_base_element(
    base: &BaseElement,
    variable_scope: Option<&dyn VariableContainer>,
) -> Option<String> {
    correlation_key_from_extensions(&base.extension_elements, variable_scope)
}

/// Convenience: correlation key for send-event triggerable receive side.
/// Java: `SendEventTaskActivityBehavior.java:140`
/// (`ELEMENT_TRIGGER_EVENT_CORRELATION_PARAMETER`).
pub fn trigger_event_correlation_key_from_base_element(
    base: &BaseElement,
    variable_scope: Option<&dyn VariableContainer>,
) -> Option<String> {
    correlation_key_from_extensions_named(
        &base.extension_elements,
        ELEMENT_TRIGGER_EVENT_CORRELATION_PARAMETER,
        variable_scope,
    )
}

/// Whether the start event requests unique-reference-id correlation.
///
/// Java `BpmnEventRegistryEventConsumer.getStartCorrelationConfiguration:272-296`
/// + compare to `storeAsUniqueReferenceId` (:125).
pub fn is_store_as_unique_reference_id(
    extensions: &IndexMap<String, Vec<ExtensionElement>>,
) -> bool {
    extension_element_text(extensions, START_EVENT_CORRELATION_CONFIGURATION)
        .as_deref()
        == Some(START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID)
}

/// Whether the start event uses manual subscription (skip auto-register).
///
/// Java `EventSubscriptionManager.insertEventRegistryEvent:226-230`.
pub fn is_manual_subscription(extensions: &IndexMap<String, Vec<ExtensionElement>>) -> bool {
    extension_element_text(extensions, START_EVENT_CORRELATION_CONFIGURATION).as_deref()
        == Some(START_EVENT_CORRELATION_MANUAL)
}

/// Count non-ended process instances matching unique event-registry reference.
///
/// Java `BpmnEventRegistryEventConsumer.countProcessInstances:213-225`
/// (processDefinitionKey + referenceId + referenceType EVENT_PROCESS [+ tenant]).
///
/// Distributed lock path intentionally omitted (single-engine first phase).
pub fn count_process_instances_for_unique_reference(
    store: &RuntimeStore,
    session: &mut DbSession,
    process_definition_key: &str,
    reference_id: &str,
    tenant_id: Option<&str>,
) -> u64 {
    store
        .snapshot_process_instances(session)
        .values()
        .filter(|pi| {
            !pi.is_ended
                && pi.process_definition_key == process_definition_key
                && pi.reference_id.as_deref() == Some(reference_id)
                && pi.reference_type.as_deref() == Some(REFERENCE_TYPE_EVENT_PROCESS)
                && tenant_matches(pi, tenant_id)
        })
        .count() as u64
}

fn tenant_matches(pi: &ProcessInstance, tenant_id: Option<&str>) -> bool {
    match tenant_id {
        Some(t) if !t.is_empty() => pi.tenant_id.as_deref() == Some(t),
        _ => true,
    }
}

/// Whether a unique-correlation start should be skipped because an instance
/// already exists for the full-parameter correlation key.
pub fn should_skip_unique_start(
    store: &RuntimeStore,
    session: &mut DbSession,
    process_definition_key: &str,
    full_correlation_key: &str,
    tenant_id: Option<&str>,
) -> bool {
    count_process_instances_for_unique_reference(
        store,
        session,
        process_definition_key,
        full_correlation_key,
        tenant_id,
    ) > 0
}

/// Hex of MD5 matching Java `String.format("%x", new BigInteger(1, bytes))`
/// (no leading-zero padding).
fn bytes_to_java_hex(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    let trimmed = hex.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlation_key_is_order_independent() {
        let mut a = BTreeMap::new();
        a.insert("someKey".to_string(), Some("value".to_string()));
        a.insert("otherKey".to_string(), Some("other value".to_string()));
        let mut b = BTreeMap::new();
        b.insert("otherKey".to_string(), Some("other value".to_string()));
        b.insert("someKey".to_string(), Some("value".to_string()));
        assert_eq!(generate_correlation_key(&a), generate_correlation_key(&b));
    }

    #[test]
    fn null_and_empty_value_produce_same_key() {
        let mut a = BTreeMap::new();
        a.insert("someKey".to_string(), Some("value".to_string()));
        a.insert("noValue".to_string(), None);
        let mut b = BTreeMap::new();
        b.insert("someKey".to_string(), Some("value".to_string()));
        b.insert("noValue".to_string(), Some(String::new()));
        assert_eq!(generate_correlation_key(&a), generate_correlation_key(&b));
    }

    #[test]
    fn known_static_customer_key() {
        let mut params = BTreeMap::new();
        params.insert("customerId".to_string(), Some("testCustomer".to_string()));
        // MD5("customerId=testCustomer;")
        assert_eq!(
            generate_correlation_key(&params),
            "3fee3f81db181da99e46b23aeca15764"
        );
    }

    #[test]
    fn match_null_config_always_when_keys_present() {
        assert!(matches_subscription_configuration(
            None,
            &["abc".to_string()]
        ));
    }

    #[test]
    fn match_empty_keys_requires_null_config() {
        assert!(matches_subscription_configuration(None, &[]));
        assert!(!matches_subscription_configuration(Some("x"), &[]));
    }

    #[test]
    fn match_in_keys() {
        let keys = vec!["a".to_string(), "b".to_string()];
        assert!(matches_subscription_configuration(Some("b"), &keys));
        assert!(!matches_subscription_configuration(Some("c"), &keys));
    }

    /// Subset-key expectations are reproducible from a raw parameter map
    /// (`generate_correlation_key` sorts keys internally, so order in `pairs`
    /// is irrelevant).
    fn key_for(pairs: &[(&str, &str)]) -> String {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), Some(v.to_string()));
        }
        generate_correlation_key(&m)
    }

    #[test]
    fn event_keys_empty_params_yield_empty_vec() {
        assert!(generate_event_correlation_keys(&BTreeMap::new()).is_empty());
    }

    #[test]
    fn event_keys_single_param_yields_single_full_key() {
        let mut params = BTreeMap::new();
        params.insert("a".to_string(), Some("1".to_string()));
        // Java n=1 special case (:83-87): only the full key.
        assert_eq!(
            generate_event_correlation_keys(&params),
            vec![generate_correlation_key(&params)]
        );
    }

    #[test]
    fn event_keys_two_params_yield_three_subset_keys() {
        let mut params = BTreeMap::new();
        params.insert("a".to_string(), Some("1".to_string()));
        params.insert("b".to_string(), Some("2".to_string()));
        // Bitmask order over sorted keys: {a}, {b}, {a,b}.
        let expected = [
            key_for(&[("a", "1")]),
            key_for(&[("b", "2")]),
            key_for(&[("a", "1"), ("b", "2")]),
        ];
        assert_eq!(generate_event_correlation_keys(&params), expected);
    }

    #[test]
    fn event_keys_three_params_yield_seven_subset_keys() {
        let mut params = BTreeMap::new();
        params.insert("a".to_string(), Some("1".to_string()));
        params.insert("b".to_string(), Some("2".to_string()));
        params.insert("c".to_string(), Some("3".to_string()));
        // Bitmask order 1..=7 over sorted keys a, b, c.
        let expected = [
            key_for(&[("a", "1")]),
            key_for(&[("b", "2")]),
            key_for(&[("a", "1"), ("b", "2")]),
            key_for(&[("c", "3")]),
            key_for(&[("a", "1"), ("c", "3")]),
            key_for(&[("b", "2"), ("c", "3")]),
            key_for(&[("a", "1"), ("b", "2"), ("c", "3")]),
        ];
        assert_eq!(generate_event_correlation_keys(&params), expected);
    }
}
