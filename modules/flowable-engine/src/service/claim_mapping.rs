use crate::service::principal::Principal;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct ClaimMapping {
    pub actor_id_claim: String,
    pub subject_claim: String,
    pub issuer_claim: String,
    pub tenant_id_claim: Option<String>,
    pub role_claim: String,
}

impl ClaimMapping {
    pub fn map_claims(&self, claims: &HashMap<String, Value>) -> Result<Principal, String> {
        let actor_id = Self::extract_string(claims, &self.actor_id_claim).ok_or_else(|| {
            format!(
                "Missing required claim for actor_id: {}",
                self.actor_id_claim
            )
        })?;

        let subject = Self::extract_string(claims, &self.subject_claim)
            .ok_or_else(|| format!("Missing required claim for subject: {}", self.subject_claim))?;

        let issuer = Self::extract_string(claims, &self.issuer_claim)
            .ok_or_else(|| format!("Missing required claim for issuer: {}", self.issuer_claim))?;

        let role = Self::extract_string(claims, &self.role_claim)
            .ok_or_else(|| format!("Missing required claim for role: {}", self.role_claim))?;

        let tenant_id = if let Some(ref claim_name) = self.tenant_id_claim {
            Self::extract_string(claims, claim_name)
        } else {
            None
        };

        Ok(Principal::new(&actor_id, &subject, &issuer, tenant_id).with_role(&role))
    }

    fn extract_string(claims: &HashMap<String, Value>, key: &str) -> Option<String> {
        claims
            .get(key)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }
}
