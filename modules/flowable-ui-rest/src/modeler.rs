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
    body::Bytes,
    extract::{Extension, Multipart, Path, Query},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use flowable_engine::{
    engine::{process_engine::ProcessEngine, query::Query as _},
    error::FlowableError,
    repository::model::{RepositoryModel, RepositoryModelBytes},
};
use flowable_modeler_protocol::{BpmnEditorDocument, DmnEditorDocument, FormEditorDocument};
use flowable_modeler_service::{
    ValidationResult, bpmn_thumbnail_png, decode_bpmn_xml, decode_dmn_xml, decode_form_json,
    encode_bpmn_xml, encode_dmn_xml, encode_form_json, layout_bpmn, validate_bpmn, validate_dmn,
    validate_form,
};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    auth::UiAuth,
    error::UiError,
    idm::{GroupRepresentation, ResultListDataRepresentation, UserRepresentation},
};

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
        .route("/modeler-app/rest/editor/layout", post(layout_editor_bpmn))
        .route(
            "/modeler-app/rest/import-process-model",
            post(import_process_model),
        )
        .route(
            "/modeler-app/rest/import-process-model/text",
            post(import_process_model_text),
        )
        // Java mounts the editor api package as its own servlet at `/api/editor`.
        .route(
            "/api/editor/import-process-model",
            post(api_import_process_model),
        )
        .route("/modeler-app/rest/editor-users", get(editor_users))
        .route("/modeler-app/rest/editor-groups", get(editor_groups))
        .route(
            "/modeler-app/rest/models/:model_id/clone",
            post(clone_model),
        )
        .route(
            "/modeler-app/rest/models/:model_id/parent-relations",
            get(get_model_parent_relations),
        )
        .route(
            "/modeler-app/rest/decision-table-models/import-decision-table",
            post(import_decision_table),
        )
        .route(
            "/modeler-app/rest/decision-table-models/import-decision-table-text",
            post(import_decision_table_text),
        );

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

// ---------------------------------------------------------------------------
// Model import, clone and editor sharing
// (Java `ModelsResource`, `ApiModelsResource`, `DecisionTableResource`,
// `EditorUsersResource`, `EditorGroupsResource`, `ModelRelationResource`)
// ---------------------------------------------------------------------------

/// Java `AbstractModel.MODEL_TYPE_BPMN`.
const MODEL_TYPE_BPMN: i32 = 0;
/// Java `AbstractModel.MODEL_TYPE_FORM`.
const MODEL_TYPE_FORM: i32 = 2;
/// Java `AbstractModel.MODEL_TYPE_DECISION_TABLE`.
const MODEL_TYPE_DECISION_TABLE: i32 = 4;

/// Java `EditorUsersResource.MAX_USER_SIZE`.
const MAX_EDITOR_USERS: usize = 100;

/// Java `ModelRepresentation`.
///
/// The Rust repository model has no description/comment/audit columns, so
/// those fields serialize as null. `createdBy`/`lastUpdatedBy` echo the
/// session user, matching Java's `SecurityUtils.getCurrentUserId()`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRepresentation {
    pub id: String,
    pub name: Option<String>,
    pub key: String,
    pub description: Option<String>,
    pub created_by: Option<String>,
    pub last_updated_by: Option<String>,
    /// Millis since the epoch; Jackson serializes `Date` as a numeric timestamp.
    pub last_updated: i64,
    pub latest_version: bool,
    pub version: i32,
    pub comment: Option<String>,
    pub model_type: i32,
    pub tenant_id: Option<String>,
}

/// Java `ModelInformation`.
#[derive(Debug, Serialize)]
pub struct ModelInformation {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub model_type: Option<i32>,
}

fn model_representation(
    model: &RepositoryModel,
    model_type: i32,
    user_id: Option<&str>,
) -> ModelRepresentation {
    ModelRepresentation {
        id: model.id.clone(),
        name: model.name.clone(),
        key: model.key.clone(),
        description: None,
        created_by: user_id.map(str::to_string),
        last_updated_by: user_id.map(str::to_string),
        last_updated: model.last_update_time,
        latest_version: true,
        version: model.version,
        comment: None,
        model_type,
        tenant_id: model
            .tenant_id
            .clone()
            .filter(|tenant_id| !tenant_id.is_empty()),
    }
}

/// Multipart body of the import endpoints: a `file` part plus optional
/// `name`/`key` text parts overriding the values parsed from the XML.
struct ModelUpload {
    file_name: Option<String>,
    bytes: Vec<u8>,
    name: Option<String>,
    key: Option<String>,
}

async fn read_model_upload(mut multipart: Multipart) -> Result<ModelUpload, UiError> {
    let mut upload = ModelUpload {
        file_name: None,
        bytes: Vec::new(),
        name: None,
        key: None,
    };
    let mut found_file = false;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| UiError::BadRequest(error.to_string()))?
    {
        match field.name() {
            Some("file") => {
                upload.file_name = field.file_name().map(str::to_string);
                upload.bytes = field
                    .bytes()
                    .await
                    .map_err(|error| UiError::BadRequest(error.to_string()))?
                    .to_vec();
                found_file = true;
            }
            Some("name") => {
                upload.name = field.text().await.ok().filter(|value| !value.trim().is_empty());
            }
            Some("key") => {
                upload.key = field.text().await.ok().filter(|value| !value.trim().is_empty());
            }
            _ => {}
        }
    }
    if !found_file {
        return Err(UiError::BadRequest("No file found in POST body".to_string()));
    }
    Ok(upload)
}

/// JSON body of the `/text` import variants: the XML text plus optional
/// overrides. Java's `/text` endpoints take the same multipart body and exist
/// for the IE9 flash uploader; the first-party UI posts JSON instead.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportModelTextRequest {
    xml: String,
    name: Option<String>,
    key: Option<String>,
    file_name: Option<String>,
}

/// Java `ModelsResource.importProcessModel` →
/// `FlowableModelQueryService.importProcessModel`.
async fn import_process_model(
    auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    multipart: Multipart,
) -> Result<Json<ModelRepresentation>, UiError> {
    let upload = read_model_upload(multipart).await?;
    let file_name = required_upload_file_name(
        upload.file_name,
        "Invalid file name, only .bpmn and .bpmn20.xml files are supported",
    )?;
    import_bpmn_model(
        &engine,
        Some(auth.user_id()),
        Some(file_name),
        upload.bytes,
        upload.name,
        upload.key,
    )
    .map(Json)
}

/// Java `ModelsResource.importProcessModelText`.
async fn import_process_model_text(
    auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(request): Json<ImportModelTextRequest>,
) -> Result<Json<ModelRepresentation>, UiError> {
    import_bpmn_model(
        &engine,
        Some(auth.user_id()),
        request.file_name,
        request.xml.into_bytes(),
        request.name,
        request.key,
    )
    .map(Json)
}

/// Java `ApiModelsResource.importProcessModel`.
///
/// `/api/editor/**` matches no rule in `auth::required_access`, so the session
/// is optional here (see the `UiAuth` extractor docs).
async fn api_import_process_model(
    auth: Option<UiAuth>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    multipart: Multipart,
) -> Result<Json<ModelRepresentation>, UiError> {
    let upload = read_model_upload(multipart).await?;
    let file_name = required_upload_file_name(
        upload.file_name,
        "Invalid file name, only .bpmn and .bpmn20.xml files are supported",
    )?;
    import_bpmn_model(
        &engine,
        auth.as_ref().map(UiAuth::user_id),
        Some(file_name),
        upload.bytes,
        upload.name,
        upload.key,
    )
    .map(Json)
}

fn required_upload_file_name(
    file_name: Option<String>,
    message: &str,
) -> Result<String, UiError> {
    file_name
        .filter(|name| !name.is_empty())
        .ok_or_else(|| UiError::BadRequest(message.to_string()))
}

/// Validates the BPMN XML, derives name/key from the main process and creates
/// a repository model whose source is the XML text. Java stores converted Oryx
/// editor JSON instead; the Rust modeler protocol keeps the XML as the source.
fn import_bpmn_model(
    engine: &Arc<ProcessEngine>,
    user_id: Option<&str>,
    file_name: Option<String>,
    bytes: Vec<u8>,
    name_override: Option<String>,
    key_override: Option<String>,
) -> Result<ModelRepresentation, UiError> {
    let display_name = file_name.clone().unwrap_or_else(|| "request".to_string());
    if let Some(name) = file_name.as_deref() {
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".bpmn") || lower.ends_with(".bpmn20.xml")) {
            return Err(UiError::BadRequest(format!(
                "Invalid file name, only .bpmn and .bpmn20.xml files are supported not {name}"
            )));
        }
    }
    let xml = std::str::from_utf8(&bytes).map_err(|error| {
        UiError::BadRequest(format!(
            "Import failed for {display_name}, error message {error}"
        ))
    })?;
    let document = decode_bpmn_xml(xml).map_err(|error| {
        UiError::BadRequest(format!(
            "Import failed for {display_name}, error message {error}"
        ))
    })?;
    // Java: `CollectionUtils.isEmpty(bpmnModel.getProcesses())` → 400.
    let process = document.model.processes.first().ok_or_else(|| {
        UiError::BadRequest(format!("No process found in definition {display_name}"))
    })?;
    let key = key_override
        .or_else(|| process.base_element.id.clone())
        .ok_or_else(|| {
            UiError::BadRequest(format!("No process id found in definition {display_name}"))
        })?;
    // Java: the name falls back to the process id.
    let name = name_override
        .or_else(|| process.name.clone())
        .unwrap_or_else(|| key.clone());
    let resource_name = file_name.or_else(|| Some(format!("{key}.bpmn20.xml")));
    create_model_with_source(
        engine,
        user_id,
        name,
        key,
        MODEL_TYPE_BPMN,
        resource_name,
        bytes,
    )
}

/// Java `DecisionTableResource.importDecisionTable` →
/// `FlowableDecisionTableService.importDecisionTable`.
async fn import_decision_table(
    auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    multipart: Multipart,
) -> Result<Json<ModelRepresentation>, UiError> {
    let upload = read_model_upload(multipart).await?;
    let file_name = required_upload_file_name(
        upload.file_name,
        "Invalid file name, only .dmn or .xml files are supported",
    )?;
    import_dmn_model(
        &engine,
        Some(auth.user_id()),
        Some(file_name),
        upload.bytes,
        upload.name,
        upload.key,
    )
    .map(Json)
}

/// Java `DecisionTableResource.importDecisionTableText`.
async fn import_decision_table_text(
    auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(request): Json<ImportModelTextRequest>,
) -> Result<Json<ModelRepresentation>, UiError> {
    import_dmn_model(
        &engine,
        Some(auth.user_id()),
        request.file_name,
        request.xml.into_bytes(),
        request.name,
        request.key,
    )
    .map(Json)
}

/// DMN counterpart of [`import_bpmn_model`]: key comes from the first decision,
/// the name from the DMN definition.
fn import_dmn_model(
    engine: &Arc<ProcessEngine>,
    user_id: Option<&str>,
    file_name: Option<String>,
    bytes: Vec<u8>,
    name_override: Option<String>,
    key_override: Option<String>,
) -> Result<ModelRepresentation, UiError> {
    let display_name = file_name.clone().unwrap_or_else(|| "request".to_string());
    if let Some(name) = file_name.as_deref() {
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".dmn") || lower.ends_with(".xml")) {
            return Err(UiError::BadRequest(format!(
                "Invalid file name, only .dmn or .xml files are supported not {name}"
            )));
        }
    }
    let xml = std::str::from_utf8(&bytes)
        .map_err(|error| UiError::BadRequest(format!("Could not import decision table model: {error}")))?;
    let document = decode_dmn_xml(xml).map_err(|error| {
        UiError::BadRequest(format!("Could not import decision table model: {error}"))
    })?;
    // Java: `dmnDefinition.getDecisions().size() == 0` → error.
    let decision = document
        .model
        .decisions
        .first()
        .ok_or_else(|| UiError::BadRequest(format!("No decisions found in {display_name}")))?;
    let key = key_override.unwrap_or_else(|| decision.id.clone());
    let name = name_override
        .or_else(|| document.model.name.clone())
        .unwrap_or_else(|| key.clone());
    let resource_name = file_name.or_else(|| Some(format!("{key}.dmn")));
    create_model_with_source(
        engine,
        user_id,
        name,
        key,
        MODEL_TYPE_DECISION_TABLE,
        resource_name,
        bytes,
    )
}

fn create_model_with_source(
    engine: &Arc<ProcessEngine>,
    user_id: Option<&str>,
    name: String,
    key: String,
    model_type: i32,
    resource_name: Option<String>,
    bytes: Vec<u8>,
) -> Result<ModelRepresentation, UiError> {
    let repository = engine.get_repository_service();
    let model = repository
        .create_repository_model(RepositoryModel {
            id: String::new(),
            name: Some(name),
            key,
            category: None,
            version: 1,
            meta_info: None,
            deployment_id: None,
            resource_name,
            process_definition_id: None,
            tenant_id: None,
            create_time: 0,
            last_update_time: 0,
            source_content_type: XML_CONTENT_TYPE.to_string(),
            source_extra_content_type: JSON_CONTENT_TYPE.to_string(),
        })
        .map_err(repository_error)?;
    repository
        .update_repository_model_source(&model.id, XML_CONTENT_TYPE.to_string(), bytes)
        .map_err(repository_error)?;
    Ok(model_representation(&model, model_type, user_id))
}

#[derive(Debug, Deserialize)]
pub struct EditorFilterQuery {
    filter: Option<String>,
}

/// Java `EditorUsersResource.getUsers`.
async fn editor_users(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Query(query): Query<EditorFilterQuery>,
) -> Result<Json<ResultListDataRepresentation<UserRepresentation>>, UiError> {
    let users = engine
        .get_identity_service()
        .create_user_query()
        .list()
        .map_err(repository_error)?;
    let filter = query.filter.unwrap_or_default().to_lowercase();
    let data: Vec<UserRepresentation> = users
        .into_iter()
        .filter(|user| {
            filter.is_empty()
                || user.id.to_lowercase().contains(&filter)
                || user
                    .first_name
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase().contains(&filter))
                || user
                    .last_name
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase().contains(&filter))
        })
        .take(MAX_EDITOR_USERS)
        .map(|user| {
            // Java's `UserRepresentation.getFullName`: `first + " " + last`
            // with each null half replaced by the empty string.
            let full_name = format!(
                "{} {}",
                user.first_name.clone().unwrap_or_default(),
                user.last_name.clone().unwrap_or_default()
            );
            UserRepresentation {
                id: user.id,
                first_name: user.first_name,
                last_name: user.last_name,
                email: user.email,
                full_name,
                tenant_id: user.tenant_id.filter(|tenant_id| !tenant_id.is_empty()),
                groups: Vec::new(),
                privileges: Vec::new(),
            }
        })
        .collect();
    let size = data.len() as i32;
    Ok(Json(ResultListDataRepresentation {
        size,
        total: i64::from(size),
        start: 0,
        data,
    }))
}

/// Java `EditorGroupsResource.getGroups`.
async fn editor_groups(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Query(query): Query<EditorFilterQuery>,
) -> Result<Json<ResultListDataRepresentation<GroupRepresentation>>, UiError> {
    let mut groups = engine
        .get_identity_service()
        .create_group_query()
        .list()
        .map_err(repository_error)?;
    // Java: `groupQuery.orderByGroupName().asc()`.
    groups.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    let filter = query.filter.unwrap_or_default().to_lowercase();
    let data: Vec<GroupRepresentation> = groups
        .into_iter()
        .filter(|group| filter.is_empty() || group.name.to_lowercase().contains(&filter))
        .map(|group| GroupRepresentation {
            id: Some(group.id),
            name: Some(group.name),
            group_type: group.group_type,
        })
        .collect();
    let size = data.len() as i32;
    Ok(Json(ResultListDataRepresentation {
        size,
        total: i64::from(size),
        start: 0,
        data,
    }))
}

/// Body of the clone endpoint; mirrors the relevant `ModelRepresentation`
/// fields. An empty body clones with a `-copy` key and `" (copy)"` name suffix.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CloneModelRequest {
    name: Option<String>,
    key: Option<String>,
}

/// Java `ModelsResource.duplicateModel`. The source bytes are copied verbatim,
/// so a cloned BPMN model keeps the original process id in its XML; Java
/// rewrites it to the new key in the editor JSON.
async fn clone_model(
    auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
    body: Bytes,
) -> Result<Json<ModelRepresentation>, UiError> {
    let repository = engine.get_repository_service();
    let original = repository
        .get_repository_model(&model_id)
        .map_err(repository_error)?;
    let source = repository
        .get_repository_model_source(&model_id)
        .map_err(repository_error)?;
    let request: CloneModelRequest = if body.is_empty() {
        CloneModelRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|error| UiError::BadRequest(error.to_string()))?
    };
    // Java strips spaces from the key and rejects duplicates with 409.
    let key = request
        .key
        .unwrap_or_else(|| format!("{}-copy", original.key))
        .replace(' ', "");
    let name = request.name.unwrap_or_else(|| {
        format!(
            "{} (copy)",
            original.name.clone().unwrap_or_else(|| original.key.clone())
        )
    });
    let models = repository.get_repository_models().map_err(repository_error)?;
    if models.iter().any(|model| model.key == key) {
        return Err(UiError::Conflict {
            message: format!("Provided model key already exists: {key}"),
            message_key: None,
        });
    }
    let model_type = match detect_model_kind(original.resource_name.as_deref(), &source) {
        StoredModelKind::Bpmn => MODEL_TYPE_BPMN,
        StoredModelKind::Dmn => MODEL_TYPE_DECISION_TABLE,
        StoredModelKind::Form => MODEL_TYPE_FORM,
    };
    let created = repository
        .create_repository_model(RepositoryModel {
            id: String::new(),
            name: Some(name),
            key,
            version: 1,
            deployment_id: None,
            process_definition_id: None,
            create_time: 0,
            last_update_time: 0,
            ..original
        })
        .map_err(repository_error)?;
    repository
        .update_repository_model_source(&created.id, source.content_type, source.bytes)
        .map_err(repository_error)?;
    if let Ok(extra) = repository.get_repository_model_source_extra(&model_id) {
        repository
            .update_repository_model_source_extra(&created.id, extra.content_type, extra.bytes)
            .map_err(repository_error)?;
    }
    Ok(Json(model_representation(
        &created,
        model_type,
        Some(auth.user_id()),
    )))
}

/// Java `ModelRelationResource.getModelRelations` →
/// `ModelRelationService.findParentModels`: the models that reference this one.
/// The Rust engine has no model-relation store, so after Java's 404 check this
/// returns an empty list.
async fn get_model_parent_relations(
    _auth: UiAuth,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Json<Vec<ModelInformation>>, UiError> {
    engine
        .get_repository_service()
        .get_repository_model(&model_id)
        .map_err(repository_error)?;
    Ok(Json(Vec::new()))
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
