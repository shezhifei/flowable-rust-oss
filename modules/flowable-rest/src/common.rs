use crate::error::ApiError;
use axum::http::Uri;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct PagingQuery {
    #[serde(default)]
    pub start: usize,
    #[serde(default)]
    pub size: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct PagedResponse<T> {
    pub start: usize,
    pub size: usize,
    pub total: usize,
    /// Java `DataResponse` sort/order echo. Only populated by endpoints that
    /// resolve an effective sort; omitted from the JSON otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    pub data: Vec<T>,
}

impl PagingQuery {
    pub fn paginate<T>(self, items: Vec<T>) -> PagedResponse<T> {
        let total = items.len();
        let start = self.start.min(total);
        let data: Vec<T> = match self.size {
            Some(size) => items.into_iter().skip(start).take(size).collect(),
            None => items.into_iter().skip(start).collect(),
        };

        PagedResponse {
            start,
            size: data.len(),
            total,
            sort: None,
            order: None,
            data,
        }
    }
}

pub fn parse_query<T>(uri: &Uri) -> Result<T, ApiError>
where
    T: DeserializeOwned + Default,
{
    match uri.query() {
        Some(raw_query) if !raw_query.is_empty() => serde_urlencoded::from_str(raw_query)
            .map_err(|err| ApiError::bad_request(format!("Invalid query parameters: {err}"))),
        _ => Ok(T::default()),
    }
}

pub fn parse_rfc3339_datetime(value: &str, field: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(|_| ApiError::bad_request(format!("Could not parse '{field}' as an instant")))
}

pub fn absolute_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{base}{path}")
}
