use crate::service::claim_mapping::ClaimMapping;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum RolloutState {
    #[default]
    Active,
    Deprecated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimValidation {
    #[serde(default = "default_validate_exp")]
    pub validate_exp: bool,
    #[serde(default = "default_validate_nbf")]
    pub validate_nbf: bool,
    #[serde(default = "default_validate_iat")]
    pub validate_iat: bool,
    #[serde(default = "default_reject_empty")]
    pub reject_empty_claims: bool,
}

fn default_validate_exp() -> bool {
    true
}
fn default_validate_nbf() -> bool {
    false
}
fn default_validate_iat() -> bool {
    false
}
fn default_reject_empty() -> bool {
    true
}

impl Default for ClaimValidation {
    fn default() -> Self {
        Self {
            validate_exp: true,
            validate_nbf: false,
            validate_iat: false,
            reject_empty_claims: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoleMapping {
    pub external_role: String,
    pub internal_role: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuerProfile {
    pub id: String,
    pub issuer: String,
    pub audience: String,
    #[serde(default)]
    pub mapping: ClaimMappingConfig,
    #[serde(default)]
    pub validation: ClaimValidation,
    #[serde(default)]
    pub role_mappings: Vec<RoleMapping>,
    #[serde(default)]
    pub required_tenant: bool,
    #[serde(default = "default_rollout_state")]
    pub rollout_state: RolloutState,
    #[serde(default)]
    pub jwks_uri: Option<String>,
    #[serde(default = "default_algorithms")]
    pub allowed_algorithms: Vec<String>,
    #[serde(default = "default_jwks_cache_ttl_seconds")]
    pub jwks_cache_ttl_seconds: u64,
    #[serde(default)]
    pub jwks_refresh_policy: JwksRefreshPolicy,
    #[serde(default)]
    pub version: i64,
}

/// Per-issuer JWKS refresh and backoff policy.
///
/// Controls how the JWKS cache handles refresh, retry, stale-key
/// tolerance, and negative-cache behavior for a specific issuer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JwksRefreshPolicy {
    /// Minimum seconds between refresh attempts for the same issuer.
    #[serde(default = "default_min_refresh_interval_seconds")]
    pub min_refresh_interval_seconds: u64,

    /// Backoff multiplier for retry after failed refresh.
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,

    /// Maximum retry delay in seconds (caps exponential backoff).
    #[serde(default = "default_max_retry_delay_seconds")]
    pub max_retry_delay_seconds: u64,

    /// If true, allow stale cached keys to be used when a refresh
    /// fails, but only within stale_tolerance_seconds of expiry.
    #[serde(default)]
    pub allow_stale_on_failure: bool,

    /// Maximum seconds past cache expiry that stale keys are tolerated.
    #[serde(default = "default_stale_tolerance_seconds")]
    pub stale_tolerance_seconds: u64,

    /// Duration in seconds to negatively cache an unknown kid so that
    /// repeated misses do not hammer the JWKS endpoint.
    #[serde(default = "default_negative_cache_seconds")]
    pub negative_cache_seconds: u64,
}

fn default_min_refresh_interval_seconds() -> u64 {
    30
}
fn default_backoff_multiplier() -> f64 {
    2.0
}
fn default_max_retry_delay_seconds() -> u64 {
    300
}
fn default_stale_tolerance_seconds() -> u64 {
    60
}
fn default_negative_cache_seconds() -> u64 {
    30
}

impl Default for JwksRefreshPolicy {
    fn default() -> Self {
        Self {
            min_refresh_interval_seconds: default_min_refresh_interval_seconds(),
            backoff_multiplier: default_backoff_multiplier(),
            max_retry_delay_seconds: default_max_retry_delay_seconds(),
            allow_stale_on_failure: false,
            stale_tolerance_seconds: default_stale_tolerance_seconds(),
            negative_cache_seconds: default_negative_cache_seconds(),
        }
    }
}

impl JwksRefreshPolicy {
    pub fn min_refresh_interval(&self) -> Duration {
        Duration::from_secs(self.min_refresh_interval_seconds)
    }

    pub fn max_retry_delay(&self) -> Duration {
        Duration::from_secs(self.max_retry_delay_seconds)
    }

    pub fn stale_tolerance(&self) -> Duration {
        Duration::from_secs(self.stale_tolerance_seconds)
    }

    pub fn negative_cache_duration(&self) -> Duration {
        Duration::from_secs(self.negative_cache_seconds)
    }
}

fn default_algorithms() -> Vec<String> {
    vec!["RS256".to_string()]
}

fn default_jwks_cache_ttl_seconds() -> u64 {
    3600
}

fn default_rollout_state() -> RolloutState {
    RolloutState::Active
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimMappingConfig {
    #[serde(default = "default_actor_id_claim")]
    pub actor_id_claim: String,
    #[serde(default = "default_subject_claim")]
    pub subject_claim: String,
    #[serde(default = "default_issuer_claim")]
    pub issuer_claim: String,
    #[serde(default)]
    pub tenant_id_claim: Option<String>,
    #[serde(default = "default_role_claim")]
    pub role_claim: String,
}

fn default_actor_id_claim() -> String {
    "sub".to_string()
}
fn default_subject_claim() -> String {
    "sub".to_string()
}
fn default_issuer_claim() -> String {
    "iss".to_string()
}
fn default_role_claim() -> String {
    "role".to_string()
}

impl Default for ClaimMappingConfig {
    fn default() -> Self {
        Self {
            actor_id_claim: default_actor_id_claim(),
            subject_claim: default_subject_claim(),
            issuer_claim: default_issuer_claim(),
            tenant_id_claim: None,
            role_claim: default_role_claim(),
        }
    }
}

impl From<&ClaimMappingConfig> for ClaimMapping {
    fn from(config: &ClaimMappingConfig) -> Self {
        ClaimMapping {
            actor_id_claim: config.actor_id_claim.clone(),
            subject_claim: config.subject_claim.clone(),
            issuer_claim: config.issuer_claim.clone(),
            tenant_id_claim: config.tenant_id_claim.clone(),
            role_claim: config.role_claim.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ValidatedClaims {
    pub issuer: String,
    pub audience: String,
    pub subject: String,
    pub actor_id: String,
    pub tenant_id: Option<String>,
    pub roles: HashSet<String>,
    pub profile_id: String,
    pub exp: Option<i64>,
    pub nbf: Option<i64>,
    pub iat: Option<i64>,
}

impl IssuerProfile {
    pub fn validate_token(
        &self,
        claims: &HashMap<String, Value>,
        now_seconds: i64,
    ) -> Result<ValidatedClaims, String> {
        let validation = &self.validation;

        let issuer = self
            .extract_string(claims, &self.mapping.issuer_claim)
            .ok_or_else(|| format!("Missing required claim: {}", self.mapping.issuer_claim))?;

        if issuer != self.issuer {
            return Err(format!(
                "Issuer mismatch: expected '{}', got '{}'",
                self.issuer, issuer
            ));
        }

        let audience = self
            .extract_string(claims, "aud")
            .ok_or_else(|| "Missing required claim: aud".to_string())?;

        if audience != self.audience {
            return Err(format!(
                "Audience mismatch: expected '{}', got '{}'",
                self.audience, audience
            ));
        }

        if validation.validate_exp {
            if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
                if now_seconds >= exp {
                    return Err("Token has expired".to_string());
                }
            } else if validation.reject_empty_claims {
                return Err("Missing required claim: exp".to_string());
            }
        }

        if validation.validate_nbf {
            if let Some(nbf) = claims.get("nbf").and_then(|v| v.as_i64()) {
                if now_seconds < nbf {
                    return Err("Token is not yet valid".to_string());
                }
            } else if validation.reject_empty_claims {
                return Err("Missing required claim: nbf".to_string());
            }
        }

        if validation.validate_iat {
            if let Some(iat) = claims.get("iat").and_then(|v| v.as_i64()) {
                if iat > now_seconds {
                    return Err("Token issued in the future".to_string());
                }
            } else if validation.reject_empty_claims {
                return Err("Missing required claim: iat".to_string());
            }
        }

        let actor_id = self
            .extract_string(claims, &self.mapping.actor_id_claim)
            .ok_or_else(|| format!("Missing required claim: {}", self.mapping.actor_id_claim))?;

        if validation.reject_empty_claims && actor_id.is_empty() {
            return Err(format!(
                "Empty value for required claim: {}",
                self.mapping.actor_id_claim
            ));
        }

        let subject = self
            .extract_string(claims, &self.mapping.subject_claim)
            .ok_or_else(|| format!("Missing required claim: {}", self.mapping.subject_claim))?;

        if validation.reject_empty_claims && subject.is_empty() {
            return Err(format!(
                "Empty value for required claim: {}",
                self.mapping.subject_claim
            ));
        }

        let tenant_id = if let Some(ref claim_name) = self.mapping.tenant_id_claim {
            let val = self.extract_string(claims, claim_name);
            if self.required_tenant && val.is_none() {
                return Err(format!(
                    "Tenant claim '{}' is required but missing",
                    claim_name
                ));
            }
            val
        } else {
            None
        };

        let raw_role = self
            .extract_string(claims, &self.mapping.role_claim)
            .ok_or_else(|| format!("Missing required claim: {}", self.mapping.role_claim))?;

        if validation.reject_empty_claims && raw_role.is_empty() {
            return Err(format!(
                "Empty value for required claim: {}",
                self.mapping.role_claim
            ));
        }

        let mut roles = HashSet::new();
        let mapped_role = self.apply_role_mapping(&raw_role);
        roles.insert(mapped_role);

        for additional_role_claim in &["roles", "capabilities", "groups"] {
            if let Some(v) = claims.get(*additional_role_claim)
                && let Some(arr) = v.as_array()
            {
                for item in arr {
                    if let Some(role_str) = item.as_str() {
                        let mapped = self.apply_role_mapping(role_str);
                        roles.insert(mapped);
                    }
                }
            }
        }

        if self.required_tenant && tenant_id.is_none() {
            return Err("Tenant is required but not present in token".to_string());
        }

        let exp = claims.get("exp").and_then(|v| v.as_i64());
        let nbf = claims.get("nbf").and_then(|v| v.as_i64());
        let iat = claims.get("iat").and_then(|v| v.as_i64());

        Ok(ValidatedClaims {
            issuer,
            audience,
            subject,
            actor_id,
            tenant_id,
            roles,
            profile_id: self.id.clone(),
            exp,
            nbf,
            iat,
        })
    }

    fn extract_string(&self, claims: &HashMap<String, Value>, key: &str) -> Option<String> {
        claims
            .get(key)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    fn apply_role_mapping(&self, external_role: &str) -> String {
        for mapping in &self.role_mappings {
            if mapping.external_role == external_role {
                return mapping.internal_role.clone();
            }
        }
        external_role.to_string()
    }

    pub fn is_active(&self) -> bool {
        self.rollout_state == RolloutState::Active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_profile() -> IssuerProfile {
        IssuerProfile {
            id: "test-profile".to_string(),
            issuer: "https://test-issuer.example.com".to_string(),
            audience: "flowable-timer".to_string(),
            mapping: ClaimMappingConfig::default(),
            validation: ClaimValidation::default(),
            role_mappings: vec![RoleMapping {
                external_role: "superadmin".to_string(),
                internal_role: "admin".to_string(),
            }],
            required_tenant: false,
            rollout_state: RolloutState::Active,
            jwks_uri: None,
            allowed_algorithms: vec!["RS256".to_string()],
            jwks_cache_ttl_seconds: default_jwks_cache_ttl_seconds(),
            jwks_refresh_policy: JwksRefreshPolicy::default(),
            version: 0,
        }
    }

    #[test]
    fn test_exact_issuer_match() {
        let profile = create_test_profile();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut claims = HashMap::new();
        claims.insert(
            "iss".to_string(),
            Value::String("https://test-issuer.example.com".to_string()),
        );
        claims.insert(
            "aud".to_string(),
            Value::String("flowable-timer".to_string()),
        );
        claims.insert("sub".to_string(), Value::String("user-123".to_string()));
        claims.insert("role".to_string(), Value::String("admin".to_string()));
        claims.insert("exp".to_string(), Value::Number((now + 3600).into()));

        let result = profile.validate_token(&claims, now);
        assert!(result.is_ok());
    }

    #[test]
    fn test_issuer_mismatch_rejected() {
        let profile = create_test_profile();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut claims = HashMap::new();
        claims.insert(
            "iss".to_string(),
            Value::String("https://wrong-issuer.example.com".to_string()),
        );
        claims.insert(
            "aud".to_string(),
            Value::String("flowable-timer".to_string()),
        );
        claims.insert("sub".to_string(), Value::String("user-123".to_string()));
        claims.insert("role".to_string(), Value::String("admin".to_string()));
        claims.insert("exp".to_string(), Value::Number((now + 3600).into()));

        let result = profile.validate_token(&claims, now);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Issuer mismatch"));
    }

    #[test]
    fn test_audience_mismatch_rejected() {
        let profile = create_test_profile();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut claims = HashMap::new();
        claims.insert(
            "iss".to_string(),
            Value::String("https://test-issuer.example.com".to_string()),
        );
        claims.insert(
            "aud".to_string(),
            Value::String("wrong-audience".to_string()),
        );
        claims.insert("sub".to_string(), Value::String("user-123".to_string()));
        claims.insert("role".to_string(), Value::String("admin".to_string()));
        claims.insert("exp".to_string(), Value::Number((now + 3600).into()));

        let result = profile.validate_token(&claims, now);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Audience mismatch"));
    }

    #[test]
    fn test_expired_token_rejected() {
        let profile = create_test_profile();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut claims = HashMap::new();
        claims.insert(
            "iss".to_string(),
            Value::String("https://test-issuer.example.com".to_string()),
        );
        claims.insert(
            "aud".to_string(),
            Value::String("flowable-timer".to_string()),
        );
        claims.insert("sub".to_string(), Value::String("user-123".to_string()));
        claims.insert("role".to_string(), Value::String("admin".to_string()));
        claims.insert("exp".to_string(), Value::Number((now - 3600).into()));

        let result = profile.validate_token(&claims, now);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_not_yet_valid_token_rejected() {
        let mut profile = create_test_profile();
        profile.validation.validate_nbf = true;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut claims = HashMap::new();
        claims.insert(
            "iss".to_string(),
            Value::String("https://test-issuer.example.com".to_string()),
        );
        claims.insert(
            "aud".to_string(),
            Value::String("flowable-timer".to_string()),
        );
        claims.insert("sub".to_string(), Value::String("user-123".to_string()));
        claims.insert("role".to_string(), Value::String("admin".to_string()));
        claims.insert("nbf".to_string(), Value::Number((now + 3600).into()));
        claims.insert("exp".to_string(), Value::Number((now + 7200).into()));

        let result = profile.validate_token(&claims, now);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not yet valid"));
    }

    #[test]
    fn test_role_mapping() {
        let profile = create_test_profile();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut claims = HashMap::new();
        claims.insert(
            "iss".to_string(),
            Value::String("https://test-issuer.example.com".to_string()),
        );
        claims.insert(
            "aud".to_string(),
            Value::String("flowable-timer".to_string()),
        );
        claims.insert("sub".to_string(), Value::String("user-123".to_string()));
        claims.insert("role".to_string(), Value::String("superadmin".to_string()));
        claims.insert("exp".to_string(), Value::Number((now + 3600).into()));

        let result = profile.validate_token(&claims, now).unwrap();
        assert!(result.roles.contains("admin"));
        assert!(!result.roles.contains("superadmin"));
    }

    #[test]
    fn test_required_tenant_missing() {
        let mut profile = create_test_profile();
        profile.required_tenant = true;
        profile.mapping.tenant_id_claim = Some("tenant".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut claims = HashMap::new();
        claims.insert(
            "iss".to_string(),
            Value::String("https://test-issuer.example.com".to_string()),
        );
        claims.insert(
            "aud".to_string(),
            Value::String("flowable-timer".to_string()),
        );
        claims.insert("sub".to_string(), Value::String("user-123".to_string()));
        claims.insert("role".to_string(), Value::String("admin".to_string()));
        claims.insert("exp".to_string(), Value::Number((now + 3600).into()));

        let result = profile.validate_token(&claims, now);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Tenant"));
    }

    #[test]
    fn test_empty_role_rejected() {
        let mut profile = create_test_profile();
        profile.validation.reject_empty_claims = true;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut claims = HashMap::new();
        claims.insert(
            "iss".to_string(),
            Value::String("https://test-issuer.example.com".to_string()),
        );
        claims.insert(
            "aud".to_string(),
            Value::String("flowable-timer".to_string()),
        );
        claims.insert("sub".to_string(), Value::String("user-123".to_string()));
        claims.insert("role".to_string(), Value::String("".to_string()));
        claims.insert("exp".to_string(), Value::Number((now + 3600).into()));

        let result = profile.validate_token(&claims, now);
        assert!(result.is_err());
    }
}
