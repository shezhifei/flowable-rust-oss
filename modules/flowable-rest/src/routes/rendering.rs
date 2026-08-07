use crate::{common::parse_query, error::ApiError};
use axum::{
    Extension, Router,
    extract::Path,
    http::{HeaderMap, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use flowable_image_generator::svg_to_png_bytes;
use serde::Deserialize;
use std::sync::Arc;

pub type DynRenderingApi = Arc<dyn RenderingApi>;

pub trait RenderingApi: Send + Sync {
    fn render_process_definition_image(
        &self,
        process_definition_id: &str,
        request: RenderingRequest,
    ) -> Result<String, ApiError>;
    fn render_process_instance_diagram(
        &self,
        process_instance_id: &str,
        request: RenderingRequest,
    ) -> Result<String, ApiError>;
    fn render_decision_table_image(
        &self,
        decision_table_id: &str,
        request: RenderingRequest,
    ) -> Result<String, ApiError>;
    fn render_case_definition_image(
        &self,
        case_definition_id: &str,
        request: RenderingRequest,
    ) -> Result<String, ApiError>;
    fn render_case_instance_diagram(
        &self,
        case_instance_id: &str,
        request: RenderingRequest,
    ) -> Result<String, ApiError>;
    fn render_app_definition_image(
        &self,
        app_definition_id: &str,
        request: RenderingRequest,
    ) -> Result<String, ApiError>;
}

#[derive(Debug, Clone, Default)]
pub struct RenderingRequest {
    format: RenderingFormat,
    /// BPMN activity/element ids to highlight in process diagrams.
    pub highlight_activity_ids: Vec<String>,
    /// Sequence-flow ids to highlight in process diagrams.
    pub highlight_flow_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum RenderingFormat {
    Png,
    #[default]
    Svg,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct RenderingQueryParams {
    renderer: Option<String>,
    /// Comma-separated or repeated activity ids to highlight.
    #[serde(default, deserialize_with = "deserialize_id_list")]
    highlight_activity_ids: Vec<String>,
    /// Comma-separated or repeated sequence-flow ids to highlight.
    #[serde(default, deserialize_with = "deserialize_id_list")]
    highlight_flow_ids: Vec<String>,
}

/// Accepts either a single comma-separated string or repeated query keys.
fn deserialize_id_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

    struct IdListVisitor;

    impl<'de> Visitor<'de> for IdListVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a comma-separated string or list of ids")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(split_ids(value))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(split_ids(&value))
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut ids = Vec::new();
            while let Some(entry) = seq.next_element::<String>()? {
                ids.extend(split_ids(&entry));
            }
            Ok(ids)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Vec::new())
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Vec::new())
        }
    }

    deserializer.deserialize_any(IdListVisitor)
}

fn split_ids(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

impl TryFrom<RenderingQueryParams> for RenderingRequest {
    type Error = ApiError;

    fn try_from(value: RenderingQueryParams) -> Result<Self, Self::Error> {
        let format = match value.renderer.as_deref().map(normalize_renderer) {
            None => RenderingFormat::Svg,
            Some(Ok(format)) => format,
            Some(Err(error)) => return Err(error),
        };
        Ok(Self {
            format,
            highlight_activity_ids: value.highlight_activity_ids,
            highlight_flow_ids: value.highlight_flow_ids,
        })
    }
}

impl RenderingRequest {
    fn from_query_and_headers(
        query: RenderingQueryParams,
        headers: &HeaderMap,
    ) -> Result<Self, ApiError> {
        let renderer_was_requested = query.renderer.is_some();
        let mut request = RenderingRequest::try_from(query)?;
        if !renderer_was_requested && accepts_png(headers) && !accepts_svg(headers) {
            request.format = RenderingFormat::Png;
        }
        if request.format == RenderingFormat::Svg && !accepts_svg(headers) {
            return Err(ApiError::bad_request(
                "Accept header does not allow image/svg+xml rendering",
            ));
        }
        Ok(request)
    }
}

fn normalize_renderer(renderer: &str) -> Result<RenderingFormat, ApiError> {
    match renderer.trim().to_ascii_lowercase().as_str() {
        "svg" | "svg+xml" | "svg xml" | "image/svg" | "image/svg+xml" | "image/svg xml" => {
            Ok(RenderingFormat::Svg)
        }
        "png" | "image/png" => Ok(RenderingFormat::Png),
        other => Err(unsupported_renderer(other)),
    }
}

fn unsupported_renderer(renderer: &str) -> ApiError {
    ApiError::bad_request(format!(
        "Unsupported renderer '{renderer}'. The rendering endpoint produces image/png; renderer=svg is also supported as an extension."
    ))
}

fn accepts_png(headers: &HeaderMap) -> bool {
    let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    accept.split(',').map(str::trim).any(|media_range| {
        let media_type = media_range
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        matches!(media_type.as_str(), "*/*" | "image/*" | "image/png")
    })
}

fn accepts_svg(headers: &HeaderMap) -> bool {
    let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    accept.split(',').map(str::trim).any(|media_range| {
        let media_type = media_range
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        matches!(
            media_type.as_str(),
            "*/*" | "image/*" | "image/svg" | "image/svg+xml"
        )
    })
}

pub fn router(api: DynRenderingApi) -> Router {
    router_with_prefix("", api)
}

fn router_with_prefix(prefix: &str, api: DynRenderingApi) -> Router {
    Router::new()
        .route(
            &format!("{prefix}/repository/process-definitions/:process_definition_id/image"),
            get(get_process_definition_image),
        )
        .route(
            &format!("{prefix}/runtime/process-instances/:process_instance_id/diagram"),
            get(get_process_instance_diagram),
        )
        .route(
            &format!("{prefix}/dmn-repository/decision-tables/:decision_table_id/image"),
            get(get_decision_table_image),
        )
        .route(
            &format!("{prefix}/dmn-repository/decisions/:decision_id/image"),
            get(get_decision_image),
        )
        .route(
            &format!("{prefix}/cmmn-repository/case-definitions/:case_definition_id/image"),
            get(get_case_definition_image),
        )
        .route(
            &format!("{prefix}/cmmn-runtime/case-instances/:case_instance_id/diagram"),
            get(get_case_instance_diagram),
        )
        .route(
            &format!("{prefix}/app-repository/app-definitions/:app_definition_id/image"),
            get(get_app_definition_image),
        )
        .layer(Extension(api))
}

pub async fn get_process_definition_image(
    Extension(api): Extension<DynRenderingApi>,
    Path(process_definition_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    let request = RenderingRequest::from_query_and_headers(
        parse_query::<RenderingQueryParams>(&uri)?,
        &headers,
    )?;
    image_response(
        api.render_process_definition_image(&process_definition_id, request.clone())?,
        request,
    )
}

pub async fn get_process_instance_diagram(
    Extension(api): Extension<DynRenderingApi>,
    Path(process_instance_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    let request = RenderingRequest::from_query_and_headers(
        parse_query::<RenderingQueryParams>(&uri)?,
        &headers,
    )?;
    image_response(
        api.render_process_instance_diagram(&process_instance_id, request.clone())?,
        request,
    )
}

pub async fn get_decision_table_image(
    Extension(api): Extension<DynRenderingApi>,
    Path(decision_table_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    let request = RenderingRequest::from_query_and_headers(
        parse_query::<RenderingQueryParams>(&uri)?,
        &headers,
    )?;
    image_response(
        api.render_decision_table_image(&decision_table_id, request.clone())?,
        request,
    )
}

pub async fn get_decision_image(
    Extension(api): Extension<DynRenderingApi>,
    Path(decision_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    let request = RenderingRequest::from_query_and_headers(
        parse_query::<RenderingQueryParams>(&uri)?,
        &headers,
    )?;
    image_response(
        api.render_decision_table_image(&decision_id, request.clone())?,
        request,
    )
}

pub async fn get_case_definition_image(
    Extension(api): Extension<DynRenderingApi>,
    Path(case_definition_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    let request = RenderingRequest::from_query_and_headers(
        parse_query::<RenderingQueryParams>(&uri)?,
        &headers,
    )?;
    image_response(
        api.render_case_definition_image(&case_definition_id, request.clone())?,
        request,
    )
}

pub async fn get_case_instance_diagram(
    Extension(api): Extension<DynRenderingApi>,
    Path(case_instance_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    let request = RenderingRequest::from_query_and_headers(
        parse_query::<RenderingQueryParams>(&uri)?,
        &headers,
    )?;
    image_response(
        api.render_case_instance_diagram(&case_instance_id, request.clone())?,
        request,
    )
}

pub async fn get_app_definition_image(
    Extension(api): Extension<DynRenderingApi>,
    Path(app_definition_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    let request = RenderingRequest::from_query_and_headers(
        parse_query::<RenderingQueryParams>(&uri)?,
        &headers,
    )?;
    image_response(
        api.render_app_definition_image(&app_definition_id, request.clone())?,
        request,
    )
}

fn image_response(svg: String, request: RenderingRequest) -> Result<Response, ApiError> {
    match request.format {
        RenderingFormat::Svg => {
            Ok(([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response())
        }
        RenderingFormat::Png => Ok((
            [(header::CONTENT_TYPE, "image/png")],
            svg_to_png_bytes(&svg)?,
        )
            .into_response()),
    }
}
