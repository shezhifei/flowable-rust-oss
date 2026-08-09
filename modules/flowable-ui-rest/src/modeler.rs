//! First-party modeler REST and static application surface.
//!
//! HTTP/auth/repository concerns live here. Typed conversion, validation,
//! layout, and thumbnail generation remain in `flowable-modeler-service`.

use std::{
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{Extension, Path},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use flowable_engine::{
    engine::process_engine::ProcessEngine, error::FlowableError,
    repository::model::RepositoryModelBytes,
};
use flowable_modeler_protocol::{BpmnEditorDocument, DmnEditorDocument, FormEditorDocument};
use flowable_modeler_service::{
    ValidationResult, bpmn_thumbnail_png, decode_bpmn_xml, decode_dmn_xml, decode_form_json,
    encode_bpmn_xml, encode_dmn_xml, encode_form_json, layout_bpmn, validate_bpmn, validate_dmn,
    validate_form,
};
use tower_http::services::{ServeDir, ServeFile};

use crate::{auth::UiAuth, error::UiError};

const JSON_CONTENT_TYPE: &str = "application/json";
const XML_CONTENT_TYPE: &str = "application/xml";

pub fn router() -> Router {
    let static_dir = configured_static_dir();
    router_with_static_dir(static_dir.as_deref())
}

/// Modeler router with an explicit optional distribution directory.
///
/// Exposed for contract tests and embedders. A missing directory simply omits
/// static routes while keeping every REST route available.
pub fn router_with_static_dir(static_dir: Option<&FsPath>) -> Router {
    let rest = Router::new()
        .route(
            "/modeler-app/rest/models/:model_id/editor/bpmn-json",
            get(get_bpmn_editor).put(put_bpmn_editor),
        )
        .route(
            "/modeler-app/rest/models/:model_id/editor/dmn-json",
            get(get_dmn_editor).put(put_dmn_editor),
        )
        .route(
            "/modeler-app/rest/form-models/:model_id/editor/form-json",
            get(get_form_editor).put(put_form_editor),
        )
        .route(
            "/modeler-app/rest/models/:model_id/validate",
            post(validate_stored_model),
        )
        .route(
            "/modeler-app/rest/models/:model_id/thumbnail",
            get(get_bpmn_thumbnail),
        )
        .route("/modeler-app/rest/editor/layout", post(layout_editor_bpmn));

    match static_dir.filter(|directory| directory.is_dir()) {
        Some(directory) => {
            let index = directory.join("index.html");
            let service = ServeDir::new(directory)
                .append_index_html_on_directories(true)
                .fallback(ServeFile::new(index));
            rest.nest_service("/modeler-app", service)
        }
        None => rest,
    }
}

fn configured_static_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FLOWABLE_MODELER_STATIC_DIR") {
        return Some(PathBuf::from(path));
    }
    Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/modeler/dist"))
}

async fn get_bpmn_editor(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Json<BpmnEditorDocument>, UiError> {
    let source = model_source(&engine, &model_id)?;
    let xml = stored_text(&source, &model_id)?;
    decode_bpmn_xml(xml)
        .map(Json)
        .map_err(|error| stored_model_error(&model_id, error))
}

async fn put_bpmn_editor(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
    Json(document): Json<BpmnEditorDocument>,
) -> Result<StatusCode, UiError> {
    let xml = encode_bpmn_xml(&document).map_err(client_model_error)?;
    update_model_source(&engine, &model_id, XML_CONTENT_TYPE, xml.into_bytes())?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_dmn_editor(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Json<DmnEditorDocument>, UiError> {
    let source = model_source(&engine, &model_id)?;
    let xml = stored_text(&source, &model_id)?;
    decode_dmn_xml(xml)
        .map(Json)
        .map_err(|error| stored_model_error(&model_id, error))
}

async fn put_dmn_editor(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
    Json(document): Json<DmnEditorDocument>,
) -> Result<StatusCode, UiError> {
    let xml = encode_dmn_xml(&document).map_err(client_model_error)?;
    update_model_source(&engine, &model_id, XML_CONTENT_TYPE, xml.into_bytes())?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_form_editor(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Json<FormEditorDocument>, UiError> {
    let source = model_source(&engine, &model_id)?;
    decode_form_json(&source.bytes)
        .map(Json)
        .map_err(|error| stored_model_error(&model_id, error))
}

async fn put_form_editor(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
    Json(document): Json<FormEditorDocument>,
) -> Result<StatusCode, UiError> {
    let validation = validate_form(&document);
    if !validation.valid {
        return Err(validation_error(&validation));
    }
    let json = encode_form_json(&document).map_err(client_model_error)?;
    update_model_source(&engine, &model_id, JSON_CONTENT_TYPE, json)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_stored_model(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Json<ValidationResult>, UiError> {
    let repository = engine.get_repository_service();
    let model = repository
        .get_repository_model(&model_id)
        .map_err(repository_error)?;
    let source = repository
        .get_repository_model_source(&model_id)
        .map_err(repository_error)?;

    let result = match detect_model_kind(model.resource_name.as_deref(), &source) {
        StoredModelKind::Bpmn => {
            let xml = stored_text(&source, &model_id)?;
            match decode_bpmn_xml(xml) {
                Ok(document) => validate_bpmn(&document),
                Err(error) => ValidationResult::invalid(error.to_string()),
            }
        }
        StoredModelKind::Dmn => {
            let xml = stored_text(&source, &model_id)?;
            match decode_dmn_xml(xml) {
                Ok(document) => validate_dmn(&document),
                Err(error) => ValidationResult::invalid(error.to_string()),
            }
        }
        StoredModelKind::Form => match decode_form_json(&source.bytes) {
            Ok(document) => validate_form(&document),
            Err(error) => ValidationResult::invalid(error.to_string()),
        },
    };
    Ok(Json(result))
}

async fn get_bpmn_thumbnail(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Response, UiError> {
    let source = model_source(&engine, &model_id)?;
    let xml = stored_text(&source, &model_id)?;
    let document = decode_bpmn_xml(xml).map_err(|error| stored_model_error(&model_id, error))?;
    let png =
        bpmn_thumbnail_png(&document).map_err(|error| stored_model_error(&model_id, error))?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        png,
    )
        .into_response())
}

async fn layout_editor_bpmn(
    _auth: UiAuth,
    Json(document): Json<BpmnEditorDocument>,
) -> Result<Json<BpmnEditorDocument>, UiError> {
    layout_bpmn(&document).map(Json).map_err(client_model_error)
}

fn model_source(
    engine: &Arc<ProcessEngine>,
    model_id: &str,
) -> Result<RepositoryModelBytes, UiError> {
    engine
        .get_repository_service()
        .get_repository_model_source(model_id)
        .map_err(repository_error)
}

fn update_model_source(
    engine: &Arc<ProcessEngine>,
    model_id: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<(), UiError> {
    engine
        .get_repository_service()
        .update_repository_model_source(model_id, content_type.to_string(), bytes)
        .map_err(repository_error)
}

fn stored_text<'a>(source: &'a RepositoryModelBytes, model_id: &str) -> Result<&'a str, UiError> {
    std::str::from_utf8(&source.bytes).map_err(|error| {
        UiError::Internal(format!(
            "Stored source for model '{model_id}' is not UTF-8: {error}"
        ))
    })
}

fn client_model_error(error: impl std::fmt::Display) -> UiError {
    UiError::BadRequest(error.to_string())
}

fn stored_model_error(model_id: &str, error: impl std::fmt::Display) -> UiError {
    UiError::Internal(format!(
        "Stored source for model '{model_id}' is invalid: {error}"
    ))
}

fn validation_error(result: &ValidationResult) -> UiError {
    let message = result
        .errors
        .iter()
        .map(|issue| issue.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    UiError::BadRequest(message)
}

fn repository_error(error: FlowableError) -> UiError {
    match error.primary_error() {
        FlowableError::NotFound(message) => UiError::NotFound(Some(message.clone())),
        FlowableError::BadRequest(message) | FlowableError::DeploymentValidationError(message) => {
            UiError::BadRequest(message.clone())
        }
        FlowableError::Forbidden(message) => UiError::Forbidden(message.clone()),
        FlowableError::Conflict(message) => UiError::Conflict {
            message: message.clone(),
            message_key: None,
        },
        _ => UiError::Internal(error.to_string()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoredModelKind {
    Bpmn,
    Dmn,
    Form,
}

fn detect_model_kind(
    resource_name: Option<&str>,
    source: &RepositoryModelBytes,
) -> StoredModelKind {
    let resource_name = resource_name.unwrap_or_default().to_ascii_lowercase();
    if resource_name.ends_with(".form") || resource_name.ends_with(".form.json") {
        return StoredModelKind::Form;
    }
    if resource_name.ends_with(".dmn") || resource_name.ends_with(".dmn.xml") {
        return StoredModelKind::Dmn;
    }
    if source.content_type.to_ascii_lowercase().contains("json")
        || source
            .bytes
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            == Some(b'{')
    {
        return StoredModelKind::Form;
    }
    let source_text = String::from_utf8_lossy(&source.bytes).to_ascii_lowercase();
    if source_text.contains("spec/dmn") || source_text.contains("<decision") {
        StoredModelKind::Dmn
    } else {
        StoredModelKind::Bpmn
    }
}
