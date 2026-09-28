// Pre-existing `unwrap()` call(s), grandfathered by the workspace clippy ratchet
// (`[workspace.lints.clippy] unwrap_used = "warn"` in the root Cargo.toml). These
// sites predate the ratchet and were NOT individually audited against Java. The
// exemption is scoped with `cfg_attr(test, ...)`, so it covers only this file's
// `#[cfg(test)]` code; a NEW unwrap() in production code is still surfaced.
// Do not add more without an audit note.
#![cfg_attr(test, allow(clippy::unwrap_used))]

//! Dual-version API detection for the Flowable REST surface.
//!
//! The router serves both 6.8-era and 8.x clients; handlers read
//! [`ApiVersion`] from request extensions to adapt response shapes.
//! Detection runs outermost — ahead of Basic auth — so even rejected
//! requests carry version context, and the resolved version is echoed in a
//! response header so contract tests can assert it without touching the body.

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};

/// Request header selecting the targeted API generation. Absent or
/// unrecognized values fall back to [`ApiVersion::V8_0`].
pub const API_VERSION_HEADER: &str = "X-Flowable-API-Version";

const ECHO_HEADER_NAME: HeaderName = HeaderName::from_static("x-flowable-api-version");

/// The Flowable REST API generation a request targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiVersion {
    V6_8,
    V8_0,
}

impl ApiVersion {
    /// Canonical header value for this generation.
    pub fn as_str(self) -> &'static str {
        match self {
            ApiVersion::V6_8 => "6.8",
            ApiVersion::V8_0 => "8.0",
        }
    }

    /// Resolves the version from request headers. Only `6.x` values select
    /// legacy semantics; anything else (missing, empty, unknown) defaults to
    /// modern 8.x behavior so existing clients are unaffected.
    pub fn detect(headers: &HeaderMap) -> Self {
        let Some(raw) = headers.get(API_VERSION_HEADER) else {
            return ApiVersion::V8_0;
        };
        match raw.to_str() {
            Ok(value) if value.starts_with("6.") => ApiVersion::V6_8,
            _ => ApiVersion::V8_0,
        }
    }
}

/// Inserts [`ApiVersion`] into the request extensions and echoes it back as
/// `x-flowable-api-version` on the response.
pub async fn version_detection_middleware(mut req: Request, next: Next) -> Response {
    let api_version = ApiVersion::detect(req.headers());
    req.extensions_mut().insert(api_version);

    let mut response = next.run(req).await;
    if let Ok(value) = HeaderValue::from_str(api_version.as_str()) {
        response.headers_mut().insert(ECHO_HEADER_NAME, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router, body::Body, extract::Extension, http::Request as HttpRequest, routing::get,
    };
    use tower::ServiceExt;

    fn detect_from(header: Option<&str>) -> ApiVersion {
        let mut headers = HeaderMap::new();
        if let Some(value) = header {
            headers.insert(API_VERSION_HEADER, HeaderValue::from_str(value).unwrap());
        }
        ApiVersion::detect(&headers)
    }

    #[test]
    fn missing_header_defaults_to_v8() {
        assert_eq!(detect_from(None), ApiVersion::V8_0);
    }

    #[test]
    fn six_prefix_selects_legacy_version() {
        assert_eq!(detect_from(Some("6.8")), ApiVersion::V6_8);
        assert_eq!(detect_from(Some("6.9")), ApiVersion::V6_8);
        assert_eq!(detect_from(Some("6")), ApiVersion::V8_0);
    }

    #[test]
    fn other_values_default_to_v8() {
        assert_eq!(detect_from(Some("8.0")), ApiVersion::V8_0);
        assert_eq!(detect_from(Some("7.5")), ApiVersion::V8_0);
        assert_eq!(detect_from(Some("")), ApiVersion::V8_0);
        assert_eq!(detect_from(Some("legacy")), ApiVersion::V8_0);
    }

    #[tokio::test]
    async fn echoes_resolved_version_and_exposes_extension() {
        async fn probe(Extension(version): Extension<ApiVersion>) -> String {
            version.as_str().to_string()
        }

        let app = Router::new()
            .route("/probe", get(probe))
            .layer(axum::middleware::from_fn(version_detection_middleware));

        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/probe")
                    .header(API_VERSION_HEADER, "6.8")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response
                .headers()
                .get(ECHO_HEADER_NAME)
                .map(HeaderValue::as_bytes),
            Some(b"6.8".as_slice())
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"6.8");

        let fallback = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/probe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            fallback
                .headers()
                .get(ECHO_HEADER_NAME)
                .map(HeaderValue::as_bytes),
            Some(b"8.0".as_slice())
        );
    }
}
