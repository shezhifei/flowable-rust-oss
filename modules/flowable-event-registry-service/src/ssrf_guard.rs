//! Outbound URL validation to mitigate SSRF on event-registry REST channels.
//!
//! **Security deviation from Java Flowable:** Java does not restrict outbound
//! destination URLs for event-registry REST channel adapters. This Rust port
//! rejects non-`http`/`https` schemes and private/link-local/loopback destinations
//! by default. Legitimate internal deployments can opt in via
//! [`OutboundUrlGuardConfig`].
//!
//! **DNS rebinding limitation:** Validation resolves the host and checks every
//! returned address *before* the HTTP client issues the request. DNS answers may
//! change between validation and connect; complete protection would require
//! pinning the resolved address for the connection lifetime (not implemented).
//!
//! Kept as a small local module (duplicate of `flowable-http-service::ssrf_guard`)
//! to avoid introducing a new crate dependency edge.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

/// Configuration escape hatches for environments that must reach private hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundUrlGuardConfig {
    /// When `true`, skip private/loopback/link-local network checks (default `false`).
    pub allow_private_networks: bool,
    /// Explicit host names or IP literals that are allowed even when private.
    /// Matching is case-insensitive against the URL host (not resolved names).
    pub allowed_private_hosts: Vec<String>,
}

impl Default for OutboundUrlGuardConfig {
    fn default() -> Self {
        Self {
            allow_private_networks: false,
            allowed_private_hosts: Vec::new(),
        }
    }
}

/// Validation failure for an outbound URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundUrlGuardError {
    pub message: String,
    /// Safe target for diagnostics: `scheme://host[:port]` only (no path/query).
    pub safe_target: Option<String>,
}

impl std::fmt::Display for OutboundUrlGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OutboundUrlGuardError {}

/// Validate an outbound URL against the SSRF guard policy.
pub fn validate_outbound_url(
    url: &str,
    config: &OutboundUrlGuardConfig,
) -> Result<(), OutboundUrlGuardError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(OutboundUrlGuardError {
            message: "Outbound URL is empty".to_string(),
            safe_target: None,
        });
    }

    // Reject disallowed schemes early — even when the URL has no host
    // (e.g. `file:///etc/passwd`).
    if let Some(scheme) = scheme_of(trimmed) {
        if scheme != "http" && scheme != "https" {
            return Err(OutboundUrlGuardError {
                message: format!(
                    "Outbound URL scheme '{scheme}' is not allowed (only http/https). \
                     Denied by SSRF guard (security deviation from Java)."
                ),
                safe_target: Some(format!("{scheme}://")),
            });
        }
    }

    let parsed = parse_http_url(trimmed).map_err(|message| OutboundUrlGuardError {
        message,
        safe_target: safe_url_for_error(trimmed).ok(),
    })?;

    let safe_target = Some(format_safe_target(&parsed));

    if host_is_allowlisted(&parsed.host, config) {
        return Ok(());
    }

    if config.allow_private_networks {
        return Ok(());
    }

    if let Ok(ip) = parsed.host.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(blocked_ip_error(&parsed, ip));
        }
        return Ok(());
    }

    // Hostname: resolve and require every address to be non-blocked.
    // Known limitation: DNS may change after this check (rebinding).
    let port = parsed.port.unwrap_or(if parsed.scheme == "https" { 443 } else { 80 });
    let addrs: Vec<SocketAddr> = match (parsed.host.as_str(), port).to_socket_addrs() {
        Ok(iter) => iter.collect(),
        Err(error) => {
            return Err(OutboundUrlGuardError {
                message: format!(
                    "Outbound URL host '{}' could not be resolved for SSRF checks: {}. \
                     Configure DNS or use an allowlisted host.",
                    parsed.host, error
                ),
                safe_target,
            });
        }
    };

    if addrs.is_empty() {
        return Err(OutboundUrlGuardError {
            message: format!(
                "Outbound URL host '{}' resolved to no addresses (SSRF guard)",
                parsed.host
            ),
            safe_target,
        });
    }

    for addr in addrs {
        let ip = addr.ip();
        if is_blocked_ip(ip) {
            return Err(blocked_ip_error(&parsed, ip));
        }
    }

    Ok(())
}

/// Strip path/query/fragment so error messages cannot be used for path blind probing.
pub fn safe_url_for_error(url: &str) -> Result<String, ()> {
    let parsed = parse_http_url(url.trim()).map_err(|_| ())?;
    Ok(format_safe_target(&parsed))
}

/// Best-effort safe display; falls back to a fixed placeholder when parsing fails.
pub fn safe_url_display(url: &str) -> String {
    safe_url_for_error(url).unwrap_or_else(|_| "<invalid-url>".to_string())
}

struct ParsedUrl {
    scheme: String,
    host: String,
    port: Option<u16>,
}

fn format_safe_target(parsed: &ParsedUrl) -> String {
    match parsed.port {
        Some(port) => {
            if parsed.host.contains(':') && !parsed.host.starts_with('[') {
                format!("{}://[{}]:{}", parsed.scheme, parsed.host, port)
            } else {
                format!("{}://{}:{}", parsed.scheme, parsed.host, port)
            }
        }
        None => {
            if parsed.host.contains(':') && !parsed.host.starts_with('[') {
                format!("{}://[{}]", parsed.scheme, parsed.host)
            } else {
                format!("{}://{}", parsed.scheme, parsed.host)
            }
        }
    }
}

fn scheme_of(url: &str) -> Option<String> {
    let scheme_sep = url.find("://")?;
    let scheme = url[..scheme_sep].to_ascii_lowercase();
    if scheme.is_empty() {
        None
    } else {
        Some(scheme)
    }
}

fn parse_http_url(url: &str) -> Result<ParsedUrl, String> {
    let scheme_sep = url
        .find("://")
        .ok_or_else(|| "Outbound URL is missing a scheme (expected http:// or https://)".to_string())?;
    let scheme = url[..scheme_sep].to_ascii_lowercase();
    if scheme.is_empty() {
        return Err("Outbound URL scheme is empty".to_string());
    }
    let rest = &url[scheme_sep + 3..];
    if rest.is_empty() {
        return Err("Outbound URL is missing a host".to_string());
    }

    let authority = match rest.find('@') {
        Some(idx) => &rest[idx + 1..],
        None => rest,
    };

    let authority_end = authority
        .find(['/', '?', '#'])
        .unwrap_or(authority.len());
    let authority = &authority[..authority_end];
    if authority.is_empty() {
        return Err("Outbound URL is missing a host".to_string());
    }

    let (host, port) = split_host_port(authority)?;
    if host.is_empty() {
        return Err("Outbound URL is missing a host".to_string());
    }

    Ok(ParsedUrl {
        scheme,
        host: host.to_ascii_lowercase(),
        port,
    })
}

fn split_host_port(authority: &str) -> Result<(String, Option<u16>), String> {
    if authority.starts_with('[') {
        let end = authority
            .find(']')
            .ok_or_else(|| "Outbound URL has an invalid IPv6 host".to_string())?;
        let host = authority[1..end].to_string();
        let after = &authority[end + 1..];
        if after.is_empty() {
            return Ok((host, None));
        }
        if let Some(port_str) = after.strip_prefix(':') {
            let port = port_str
                .parse::<u16>()
                .map_err(|_| format!("Outbound URL has an invalid port '{port_str}'"))?;
            return Ok((host, Some(port)));
        }
        return Err("Outbound URL has an invalid IPv6 authority".to_string());
    }

    if let Some((host, port_str)) = authority.rsplit_once(':') {
        if host.contains(':') {
            return Ok((authority.to_string(), None));
        }
        if port_str.is_empty() {
            return Err("Outbound URL has an empty port".to_string());
        }
        if port_str.chars().all(|c| c.is_ascii_digit()) {
            let port = port_str
                .parse::<u16>()
                .map_err(|_| format!("Outbound URL has an invalid port '{port_str}'"))?;
            return Ok((host.to_string(), Some(port)));
        }
    }

    Ok((authority.to_string(), None))
}

fn host_is_allowlisted(host: &str, config: &OutboundUrlGuardConfig) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    config
        .allowed_private_hosts
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host))
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    if octets[0] == 0 {
        return true;
    }
    if ip.is_loopback() {
        return true;
    }
    if ip.is_link_local() {
        return true;
    }
    if ip.is_private() {
        return true;
    }
    false
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_blocked_ipv4(v4);
    }
    if ip.is_unspecified() {
        return true;
    }
    if ip.is_loopback() {
        return true;
    }
    if is_ipv6_link_local(ip) {
        return true;
    }
    if is_ipv6_unique_local(ip) {
        return true;
    }
    false
}

fn is_ipv6_link_local(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    (segments[0] & 0xffc0) == 0xfe80
}

fn is_ipv6_unique_local(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    (segments[0] & 0xfe00) == 0xfc00
}

fn blocked_ip_error(parsed: &ParsedUrl, ip: IpAddr) -> OutboundUrlGuardError {
    OutboundUrlGuardError {
        message: format!(
            "Outbound URL target resolves to blocked address {} (private/loopback/link-local). \
             Denied by SSRF guard (security deviation from Java). \
             Set allow_private_networks=true or add the host to allowed_private_hosts \
             for legitimate internal endpoints.",
            ip
        ),
        safe_target: Some(format_safe_target(parsed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strict() -> OutboundUrlGuardConfig {
        OutboundUrlGuardConfig::default()
    }

    #[test]
    fn rejects_loopback_and_metadata_literals() {
        assert!(validate_outbound_url("http://127.0.0.1/x", &strict()).is_err());
        assert!(validate_outbound_url("http://169.254.169.254/latest", &strict()).is_err());
        assert!(validate_outbound_url("http://10.1.1.1/", &strict()).is_err());
        assert!(validate_outbound_url("http://192.168.0.2/", &strict()).is_err());
    }

    #[test]
    fn allows_public_ip() {
        validate_outbound_url("https://8.8.8.8/dns", &strict()).unwrap();
    }

    #[test]
    fn rejects_file_and_gopher() {
        assert!(validate_outbound_url("file:///etc/passwd", &strict()).is_err());
        assert!(validate_outbound_url("gopher://127.0.0.1/1", &strict()).is_err());
    }

    #[test]
    fn allowlist_and_allow_private_escape_hatches() {
        let allowlist = OutboundUrlGuardConfig {
            allowed_private_hosts: vec!["127.0.0.1".into()],
            ..Default::default()
        };
        validate_outbound_url("http://127.0.0.1:1/secret", &allowlist).unwrap();

        let open = OutboundUrlGuardConfig {
            allow_private_networks: true,
            ..Default::default()
        };
        validate_outbound_url("http://10.0.0.1/", &open).unwrap();
    }

    #[test]
    fn error_omits_path_and_query() {
        let err =
            validate_outbound_url("http://192.168.1.1/secret?token=abc", &strict()).unwrap_err();
        assert_eq!(err.safe_target.as_deref(), Some("http://192.168.1.1"));
        assert!(!err.message.contains("secret"));
        assert!(!err.message.contains("token"));
        assert!(!err.to_string().contains("token=abc"));
    }
}
