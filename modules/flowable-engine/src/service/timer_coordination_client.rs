use crate::service::issuer_health::{
    IssuerHealthSnapshot, RevocationRegistrySnapshot, RevocationStatusDto,
};
use crate::service::timer_coordination_service::{
    CleanupResponse, DeregisterRequest, Envelope, ReleaseRequest, RevokeRequest, StepDownResponse,
    SuccessResponse, TimerCoordinatorStatusDto, TimerNodeStatusDto, UnrevokeRequest,
    UnrevokeResponse,
};
use std::io::{Read, Write};
use std::net::TcpStream;

pub struct TimerCoordinationClient {
    base_url: String,
    auth_token: Option<String>,
}

impl TimerCoordinationClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            auth_token: None,
        }
    }

    pub fn with_auth(mut self, token: String) -> Self {
        self.auth_token = Some(token);
        self
    }

    fn send_request<Req: serde::Serialize, Res: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&Req>,
    ) -> Result<Res, String> {
        let mut stream =
            TcpStream::connect(&self.base_url).map_err(|e| format!("Connect error: {}", e))?;

        let body_bytes = if let Some(b) = body {
            serde_json::to_vec(b).map_err(|e| format!("Serialize error: {}", e))?
        } else {
            Vec::new()
        };

        let mut req = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\n",
            method, path, self.base_url
        );
        if let Some(token) = &self.auth_token {
            req.push_str(&format!("Authorization: Bearer {}\r\n", token));
        }
        if !body_bytes.is_empty() {
            req.push_str("Content-Type: application/json\r\n");
            req.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
        }
        req.push_str("\r\n");

        stream
            .write_all(req.as_bytes())
            .map_err(|e| format!("Write error: {}", e))?;
        if !body_bytes.is_empty() {
            stream
                .write_all(&body_bytes)
                .map_err(|e| format!("Write body error: {}", e))?;
        }

        let mut response_str = String::new();
        stream
            .read_to_string(&mut response_str)
            .map_err(|e| format!("Read error: {}", e))?;

        let body_start = response_str
            .find("\r\n\r\n")
            .ok_or("Malformed HTTP response")?
            + 4;
        let response_body = &response_str[body_start..];

        let envelope: Envelope<Res> = serde_json::from_str(response_body)
            .map_err(|e| format!("Deserialize envelope error: {} body: {}", e, response_body))?;

        if let Some(err) = envelope.error {
            return Err(format!("[{}] {}", err.code, err.message));
        }

        envelope
            .data
            .ok_or_else(|| "No data in envelope".to_string())
    }

    pub fn get_status(&self) -> Result<TimerCoordinatorStatusDto, String> {
        self.send_request::<(), _>("GET", "/status", None)
    }

    pub fn get_nodes(&self) -> Result<Vec<TimerNodeStatusDto>, String> {
        self.send_request::<(), _>("GET", "/nodes", None)
    }

    pub fn release_leadership(&self, fencing_token: i64) -> Result<bool, String> {
        let req = ReleaseRequest { fencing_token };
        let res: SuccessResponse = self.send_request("POST", "/release", Some(&req))?;
        Ok(res.success)
    }

    pub fn admin_step_down(&self) -> Result<(bool, i64), String> {
        let res: StepDownResponse = self.send_request::<(), _>("POST", "/step-down", None)?;
        Ok((res.success, res.new_fencing_token))
    }

    pub fn deregister_node(&self, node_id: &str) -> Result<bool, String> {
        let req = DeregisterRequest {
            node_id: node_id.to_string(),
        };
        let res: SuccessResponse = self.send_request("POST", "/deregister", Some(&req))?;
        Ok(res.success)
    }

    pub fn cleanup_expired_nodes(&self) -> Result<usize, String> {
        let res: CleanupResponse = self.send_request::<(), _>("POST", "/cleanup", None)?;
        Ok(res.cleaned_count)
    }

    // ── Issuer Health Control Plane ──

    pub fn get_issuer_health(&self) -> Result<Vec<IssuerHealthSnapshot>, String> {
        self.send_request::<(), _>("GET", "/issuer-health", None)
    }

    pub fn get_issuer_health_for_issuer(
        &self,
        issuer: &str,
    ) -> Result<IssuerHealthSnapshot, String> {
        let encoded = url_encode(issuer);
        self.send_request::<(), _>("GET", &format!("/issuer-health/{}", encoded), None)
    }

    // ── Revocation Admin Control Plane ──

    pub fn revoke_token(&self, jti: &str, issuer: &str, reason: &str) -> Result<bool, String> {
        let req = RevokeRequest {
            jti: jti.to_string(),
            issuer: issuer.to_string(),
            reason: reason.to_string(),
            ttl_seconds: 3600,
        };
        let res: SuccessResponse = self.send_request("POST", "/revocation/revoke", Some(&req))?;
        Ok(res.success)
    }

    pub fn revoke_token_with_ttl(
        &self,
        jti: &str,
        issuer: &str,
        reason: &str,
        ttl_seconds: u64,
    ) -> Result<bool, String> {
        let req = RevokeRequest {
            jti: jti.to_string(),
            issuer: issuer.to_string(),
            reason: reason.to_string(),
            ttl_seconds,
        };
        let res: SuccessResponse = self.send_request("POST", "/revocation/revoke", Some(&req))?;
        Ok(res.success)
    }

    pub fn unrevoke_token(&self, jti: &str) -> Result<bool, String> {
        let req = UnrevokeRequest {
            jti: jti.to_string(),
        };
        let res: UnrevokeResponse =
            self.send_request("POST", "/revocation/unrevoke", Some(&req))?;
        Ok(res.success)
    }

    pub fn get_revocation_status(&self, jti: &str) -> Result<RevocationStatusDto, String> {
        let encoded = url_encode(jti);
        self.send_request::<(), _>("GET", &format!("/revocation/status/{}", encoded), None)
    }

    pub fn get_revocation_stats(&self) -> Result<RevocationRegistrySnapshot, String> {
        self.send_request::<(), _>("GET", "/revocation/stats", None)
    }

    // ── Issuer Profile Admin Control Plane ──

    pub fn list_issuer_profiles(
        &self,
    ) -> Result<Vec<crate::service::issuer_profile::IssuerProfile>, String> {
        self.send_request::<(), _>("GET", "/issuer-profiles", None)
    }

    pub fn get_issuer_profile(
        &self,
        profile_id: &str,
    ) -> Result<crate::service::issuer_profile::IssuerProfile, String> {
        let encoded = url_encode(profile_id);
        self.send_request::<(), _>("GET", &format!("/issuer-profiles/{}", encoded), None)
    }

    pub fn create_issuer_profile(
        &self,
        profile: &crate::service::issuer_profile::IssuerProfile,
    ) -> Result<crate::service::issuer_profile::IssuerProfile, String> {
        self.send_request("POST", "/issuer-profiles", Some(profile))
    }

    pub fn update_issuer_profile(
        &self,
        profile: &crate::service::issuer_profile::IssuerProfile,
    ) -> Result<crate::service::issuer_profile::IssuerProfile, String> {
        let encoded = url_encode(&profile.id);
        self.send_request(
            "PUT",
            &format!("/issuer-profiles/{}", encoded),
            Some(profile),
        )
    }

    pub fn delete_issuer_profile(&self, profile_id: &str) -> Result<bool, String> {
        let encoded = url_encode(profile_id);
        let res: SuccessResponse =
            self.send_request::<(), _>("DELETE", &format!("/issuer-profiles/{}", encoded), None)?;
        Ok(res.success)
    }
}

fn url_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(char::from(b));
            }
            _ => {
                result.push('%');
                result.push(char::from(b'0' + (b >> 4)));
                let lo = b & 0x0F;
                if lo < 10 {
                    result.push(char::from(b'0' + lo));
                } else {
                    result.push(char::from(b'A' + lo - 10));
                }
            }
        }
    }
    result
}
