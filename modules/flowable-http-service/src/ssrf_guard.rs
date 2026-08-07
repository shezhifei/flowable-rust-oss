//! Outbound URL validation to mitigate SSRF on BPMN/CMMN HTTP tasks.
//!
//! **Security deviation from Java Flowable:** Java does not restrict outbound
//! destination URLs for HTTP service tasks. This Rust port rejects non-`http`/
//! `https` schemes and private/link-local/loopback destinations by default.
//! Legitimate internal deployments can opt in via [`OutboundUrlGuardConfig`].
//!
//! **DNS rebinding limitation:** Validation resolves the host and checks every
//! returned address *before* the HTTP client issues the request. DNS answers may
//! change between validation and connect; complete protection would require
//! pinning the resolved address for the connection lifetime (not implemented).

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

    // Literal IP: judge directly without DNS.
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

    // Drop userinfo if present (user:pass@host).
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

    // Ambiguous IPv6 without brackets is rejected by requiring a single colon for port.
    if let Some((host, port_str)) = authority.rsplit_once(':') {
        // If host still contains ':', treat whole authority as IPv6 without brackets.
        if host.contains(':') {
            return Ok((authority.to_string(), None));
        }
        if port_str.is_empty() {
            return Err("Outbound URL has an empty port".to_string());
        }
        // Numeric port only; otherwise host may be something like "example.com:name" — reject.
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
    // unspecified 0.0.0.0/8 (includes 0.0.0.0)
    if octets[0] == 0 {
        return true;
    }
    // loopback 127.0.0.0/8
    if ip.is_loopback() {
        return true;
    }
    // link-local 169.254.0.0/16 (includes cloud metadata 169.254.169.254)
    if ip.is_link_local() {
        return true;
    }
    // private 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
    if ip.is_private() {
        return true;
    }
    false
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_blocked_ipv4(v4);
    }
    // unspecified ::
    if ip.is_unspecified() {
        return true;
    }
    // loopback ::1
    if ip.is_loopback() {
        return true;
    }
    // link-local fe80::/10
    if is_ipv6_link_local(ip) {
        return true;
    }
    // unique local fc00::/7 (fd00::/8 is the commonly used half)
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
    fn rejects_loopback_literal() {
        let err = validate_outbound_url("http://127.0.0.1/admin", &strict()).unwrap_err();
        assert!(err.message.contains("blocked"));
        assert_eq!(err.safe_target.as_deref(), Some("http://127.0.0.1"));
        assert!(!err.message.contains("/admin"));
    }

    #[test]
    fn rejects_metadata_link_local() {
        let err = validate_outbound_url("http://169.254.169.254/latest/meta-data/", &strict())
            .unwrap_err();
        assert!(err.message.contains("blocked"));
        assert_eq!(err.safe_target.as_deref(), Some("http://169.254.169.254"));
        assert!(!err.message.contains("meta-data"));
    }

    #[test]
    fn rejects_private_10_and_192() {
        assert!(validate_outbound_url("http://10.0.0.5:8080/x", &strict()).is_err());
        assert!(validate_outbound_url("http://192.168.1.1/x?q=1", &strict()).is_err());
        assert!(validate_outbound_url("http://172.16.5.1/", &strict()).is_err());
        assert!(validate_outbound_url("http://172.31.255.255/", &strict()).is_err());
    }

    #[test]
    fn allows_public_literal_ip() {
        // TEST-NET-1 documentation range — not private under our policy.
        validate_outbound_url("http://8.8.8.8/resolve", &strict()).unwrap();
        validate_outbound_url("https://1.1.1.1:443/cdn-cgi", &strict()).unwrap();
    }

    #[test]
    fn rejects_non_http_schemes() {
        for url in ["file:///etc/passwd", "gopher://example.com/1", "ftp://example.com/a"] {
            let err = validate_outbound_url(url, &strict()).unwrap_err();
            assert!(
                err.message.contains("not allowed") || err.message.contains("scheme"),
                "unexpected error for {url}: {}",
                err.message
            );
        }
    }

    #[test]
    fn allowlist_single_host_bypasses_private_check() {
        let config = OutboundUrlGuardConfig {
            allow_private_networks: false,
            allowed_private_hosts: vec!["127.0.0.1".to_string()],
        };
        validate_outbound_url("http://127.0.0.1:9/secret?x=1", &config).unwrap();
        // other private still blocked
        assert!(validate_outbound_url("http://10.0.0.1/", &config).is_err());
    }

    #[test]
    fn allow_private_networks_bypasses_all_private() {
        let config = OutboundUrlGuardConfig {
            allow_private_networks: true,
            allowed_private_hosts: vec![],
        };
        validate_outbound_url("http://127.0.0.1/a", &config).unwrap();
        validate_outbound_url("http://10.1.2.3/b", &config).unwrap();
        validate_outbound_url("http://169.254.169.254/c", &config).unwrap();
        // still reject bad schemes
        assert!(validate_outbound_url("file:///tmp", &config).is_err());
    }

    #[test]
    fn safe_url_strips_path_and_query() {
        assert_eq!(
            safe_url_display("https://example.com:8443/path?q=1#frag"),
            "https://example.com:8443"
        );
        assert_eq!(
            safe_url_display("http://[::1]:8080/x"),
            "http://[::1]:8080"
        );
    }

    #[test]
    fn rejects_ipv6_loopback_literal() {
        let err = validate_outbound_url("http://[::1]/", &strict()).unwrap_err();
        assert!(err.message.contains("blocked"));
    }

    #[test]
    fn rejects_unspecified_ipv4() {
        assert!(validate_outbound_url("http://0.0.0.0/", &strict()).is_err());
    }
}
