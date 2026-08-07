//! Tenant fallback policy shared by definition resolution and consumer matching.
//!
//! Java sources (verified):
//! - Config defaults: `AbstractEngineConfiguration.java:321-329`
//!   - `fallbackToDefaultTenant` defaults to `false`
//!   - `defaultTenantProvider` defaults to `NO_TENANT_ID` (`""`)
//! - Event definition resolve: `GetEventModelCmd.java:82-90`
//! - Channel definition resolve: `GetChannelModelCmd.java:82-90`
//! - Inbound event-def resolve: `DefaultInboundEventProcessingPipeline.java:120-136`
//! - Consumer subscription match: `BaseEventRegistryEventConsumer.java:177-265`
//!
//! Empty / null event tenant: Java skips the tenant branch entirely
//! (`BaseEventRegistryEventConsumer.java:177-178`), so subscriptions are not
//! filtered by tenant. Empty subscription tenant is **not** a wildcard for
//! non-empty event tenants unless fallback is on and the default tenant is
//! empty (definition-level only).

/// Java `AbstractEngineConfiguration.NO_TENANT_ID` — empty string.
pub const NO_TENANT_ID: &str = "";

/// Snapshot of Event Registry tenant-fallback configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TenantFallbackPolicy {
    /// Java `fallbackToDefaultTenant` (`AbstractEngineConfiguration.java:324`).
    pub fallback_to_default_tenant: bool,
    /// Resolved default tenant id. Empty string means Java `NO_TENANT_ID`
    /// (tenantless / without-tenant lookup).
    pub default_tenant: String,
}

impl Default for TenantFallbackPolicy {
    fn default() -> Self {
        Self {
            // AbstractEngineConfiguration.java:324 — boolean field default false.
            fallback_to_default_tenant: false,
            // AbstractEngineConfiguration.java:329 — provider returns NO_TENANT_ID.
            default_tenant: NO_TENANT_ID.to_string(),
        }
    }
}

impl TenantFallbackPolicy {
    pub fn new(fallback_to_default_tenant: bool, default_tenant: impl Into<String>) -> Self {
        Self {
            fallback_to_default_tenant,
            default_tenant: default_tenant.into(),
        }
    }

    pub fn with_fallback(mut self) -> Self {
        self.fallback_to_default_tenant = true;
        self
    }

    pub fn default_tenant(mut self, default_tenant: impl Into<String>) -> Self {
        self.default_tenant = default_tenant.into();
        self
    }

    /// Non-empty request tenant, treating blank as absent (Java `NO_TENANT_ID`).
    pub fn normalize_tenant<'a>(&self, tenant_id: Option<&'a str>) -> Option<&'a str> {
        tenant_id.filter(|t| !t.is_empty())
    }

    /// Whether the default tenant is Java `NO_TENANT_ID` (tenantless).
    pub fn default_is_tenantless(&self) -> bool {
        self.default_tenant.is_empty()
    }
}

/// Resolve a definition for `request_tenant` with optional fallback.
///
/// Order (GetEventModelCmd.java:82-90 / GetChannelModelCmd.java:82-90):
/// 1. exact tenant (when request has a non-empty tenant)
/// 2. if miss and `fallbackToDefaultTenant`:
///    - non-empty default tenant → lookup that tenant
///    - empty default tenant → tenantless lookup (`findLatest*ByKey`)
/// 3. when request has no tenant → tenantless lookup only
pub fn resolve_definition_with_fallback<T, F>(
    request_tenant: Option<&str>,
    policy: &TenantFallbackPolicy,
    mut lookup_exact_tenant: F,
) -> Option<T>
where
    F: FnMut(Option<&str>) -> Option<T>,
{
    let tenant = policy.normalize_tenant(request_tenant);

    if let Some(tenant) = tenant {
        if let Some(hit) = lookup_exact_tenant(Some(tenant)) {
            return Some(hit);
        }
        if !policy.fallback_to_default_tenant {
            return None;
        }
        // GetEventModelCmd.java:85-89
        if policy.default_is_tenantless() {
            lookup_exact_tenant(None)
        } else {
            lookup_exact_tenant(Some(policy.default_tenant.as_str()))
        }
    } else {
        // No / empty tenant on the request → tenantless latest by key.
        lookup_exact_tenant(None)
    }
}

/// Consumer subscription tenant match.
///
/// Java `BaseEventRegistryEventConsumer.findEventSubscriptions` (:177-265):
/// - event tenant empty → no tenant filter (match all)
/// - fallback off → exact tenant only
/// - fallback on + empty default:
///   - instance-level (process/case instance present) → exact only (:198-201)
///   - definition-level → exact **or** empty-tenant (:203-252; dedup by
///     definition key is applied separately when needed)
/// - fallback on + non-empty default → tenant in {event, default} (:258)
pub fn subscription_matches_event_tenant(
    event_tenant: Option<&str>,
    subscription_tenant: Option<&str>,
    is_instance_level: bool,
    policy: &TenantFallbackPolicy,
) -> bool {
    let event_tenant = policy.normalize_tenant(event_tenant);
    let Some(event_t) = event_tenant else {
        // BaseEventRegistryEventConsumer.java:177-178 — skip tenant branch.
        return true;
    };

    let sub_t = subscription_tenant.filter(|t| !t.is_empty());

    if !policy.fallback_to_default_tenant {
        // :262-263 exact only
        return sub_t == Some(event_t);
    }

    if policy.default_is_tenantless() {
        // :186-255 cleaning path
        if is_instance_level {
            // :198-201 instance-level requires exact tenant equality
            return sub_t == Some(event_t);
        }
        // definition-level: exact tenant or empty-tenant (:203-227 + :231-252)
        sub_t == Some(event_t) || sub_t.is_none()
    } else {
        // :258 tenantIds(event, default)
        sub_t == Some(event_t) || sub_t == Some(policy.default_tenant.as_str())
    }
}

/// Definition-key dedup for definition-level subscriptions when fallback is on and
/// default tenant is tenantless (BaseEventRegistryEventConsumer.java:203-253).
///
/// 1. Collect definition keys of tenant-exact (non-empty tenant) definition-level subs.
/// 2. Drop tenantless definition-level subs whose key is already in that set.
/// 3. Keep all instance-level subs and all other definition-level subs.
///
/// Dedup unit is definition **key** (cross-version), not definition id.
///
/// `classify(sub) -> (is_definition_level, tenant_id)`:
///   - is_definition_level = true when no process/case instance binding
///   - tenant_id = subscription tenant (None/empty = tenantless)
///
/// `definition_key(sub) -> Option<String>` resolves scope/process definition id → key.
pub fn dedup_definition_level_subscriptions_by_key<T, C, K>(
    subscriptions: Vec<T>,
    mut classify: C,
    mut definition_key: K,
) -> Vec<T>
where
    C: FnMut(&T) -> (bool, Option<String>),
    K: FnMut(&T) -> Option<String>,
{
    // First pass: collect keys covered by tenant-exact definition-level subs.
    let mut tenant_keys: Vec<String> = Vec::new();
    for sub in &subscriptions {
        let (is_definition_level, tenant) = classify(sub);
        if !is_definition_level {
            continue;
        }
        let tenant_nonempty = tenant.as_deref().filter(|t| !t.is_empty()).is_some();
        if tenant_nonempty {
            if let Some(key) = definition_key(sub) {
                if !tenant_keys.contains(&key) {
                    tenant_keys.push(key);
                }
            }
        }
    }

    // Second pass: keep everything except tenantless def-level whose key is covered.
    let mut out = Vec::with_capacity(subscriptions.len());
    for sub in subscriptions {
        let (is_definition_level, tenant) = classify(&sub);
        if is_definition_level {
            let tenantless = tenant.as_deref().filter(|t| !t.is_empty()).is_none();
            if tenantless {
                if let Some(key) = definition_key(&sub) {
                    if tenant_keys.contains(&key) {
                        // Drop: same key already has a tenant-exact subscription
                        // (BaseEventRegistryEventConsumer.java:231-252).
                        continue;
                    }
                }
            }
        }
        out.push(sub);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_drops_tenantless_same_key_keeps_different_key() {
        #[derive(Debug, Clone, PartialEq)]
        struct Sub {
            id: &'static str,
            tenant: Option<&'static str>,
            key: &'static str,
            instance: bool,
        }
        let subs = vec![
            Sub {
                id: "t-exact",
                tenant: Some("T1"),
                key: "shared",
                instance: false,
            },
            Sub {
                id: "global-same",
                tenant: None,
                key: "shared",
                instance: false,
            },
            Sub {
                id: "global-other",
                tenant: None,
                key: "other",
                instance: false,
            },
            Sub {
                id: "inst",
                tenant: Some("T1"),
                key: "shared",
                instance: true,
            },
        ];
        let cleaned = dedup_definition_level_subscriptions_by_key(
            subs,
            |s| (!s.instance, s.tenant.map(str::to_string)),
            |s| Some(s.key.to_string()),
        );
        let ids: Vec<_> = cleaned.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["t-exact", "global-other", "inst"]);
    }

    #[test]
    fn definition_exact_preferred_over_fallback() {
        let policy = TenantFallbackPolicy::default().with_fallback();
        let result = resolve_definition_with_fallback(Some("tenant-a"), &policy, |t| match t {
            Some("tenant-a") => Some("exact"),
            None => Some("fallback"),
            _ => None,
        });
        assert_eq!(result, Some("exact"));
    }

    #[test]
    fn definition_falls_back_to_tenantless_when_enabled() {
        let policy = TenantFallbackPolicy::default().with_fallback();
        let result = resolve_definition_with_fallback(Some("tenant-b"), &policy, |t| match t {
            Some("tenant-b") => None,
            None => Some("global"),
            _ => None,
        });
        assert_eq!(result, Some("global"));
    }

    #[test]
    fn definition_no_fallback_when_switch_off() {
        let policy = TenantFallbackPolicy::default();
        let result = resolve_definition_with_fallback(Some("tenant-b"), &policy, |t| match t {
            Some("tenant-b") => None,
            None => Some("global"),
            _ => None,
        });
        assert_eq!(result, None);
    }

    #[test]
    fn empty_event_tenant_matches_all_subscriptions() {
        let policy = TenantFallbackPolicy::default();
        assert!(subscription_matches_event_tenant(
            None,
            Some("tenant-a"),
            true,
            &policy
        ));
        assert!(subscription_matches_event_tenant(Some(""), None, true, &policy));
    }

    #[test]
    fn instance_level_exact_only_even_with_fallback() {
        let policy = TenantFallbackPolicy::default().with_fallback();
        assert!(subscription_matches_event_tenant(
            Some("tenant-a"),
            Some("tenant-a"),
            true,
            &policy
        ));
        assert!(!subscription_matches_event_tenant(
            Some("tenant-a"),
            None,
            true,
            &policy
        ));
    }

    #[test]
    fn definition_level_accepts_empty_tenant_when_fallback_on() {
        let policy = TenantFallbackPolicy::default().with_fallback();
        assert!(subscription_matches_event_tenant(
            Some("tenant-a"),
            None,
            false,
            &policy
        ));
        let policy_off = TenantFallbackPolicy::default();
        assert!(!subscription_matches_event_tenant(
            Some("tenant-a"),
            None,
            false,
            &policy_off
        ));
    }
}
