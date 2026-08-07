//! CMMN-side event-registry correlation key helpers.
//!
//! Mirrors Java `DefaultCorrelationKeyGenerator.generateKey`
//! (`DefaultCorrelationKeyGenerator.java:38-57`) and the power-set key
//! generation / match query in `BaseEventRegistryEventConsumer.java:76-131,156-175`.
//! Kept in cmmn-engine so subscription creation does not depend on flowable-engine
//! (engine already depends on this crate).

use md5::{Digest, Md5};
use serde_json::Value;
use std::collections::BTreeMap;

/// Generate a correlation key from a parameter map.
///
/// Java `DefaultCorrelationKeyGenerator.generateKey` (DefaultCorrelationKeyGenerator.java:38-57):
/// sort keys, append `key=value;` (null → empty string), MD5, hex without leading zeros.
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

/// Power set of correlation parameter instances minus the empty set.
///
/// Java `BaseEventRegistryEventConsumer.generateCorrelationKeys` (:76-131).
pub fn generate_event_correlation_keys(
    params: &BTreeMap<String, Option<String>>,
) -> Vec<String> {
    if params.is_empty() {
        return Vec::new();
    }
    let ordered: Vec<(&String, &Option<String>)> = params.iter().collect();
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

/// Build a correlation parameter map from a JSON event payload object.
///
/// Each object field becomes a correlation parameter; non-object payloads yield empty.
/// Values are stringified like Java's default `CorrelationValueTransformer` +
/// `Object.toString` (null → `""`).
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

pub fn json_value_to_correlation_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

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
    fn known_static_customer_key() {
        let mut params = BTreeMap::new();
        params.insert("customerId".to_string(), Some("testCustomer".to_string()));
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
}
