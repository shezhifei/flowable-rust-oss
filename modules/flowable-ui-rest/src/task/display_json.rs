//! Runtime display-json endpoints (Java `RuntimeDisplayJsonClientResource`).
//!
//! The task app's process-diagram view (`display/displaymodel.js`) fetches
//! these directly. The response is assembled with the same builder the admin
//! app uses so both apps emit the identical display shape; `admin` keeps its
//! copy private, so the source file is included here via `#[path]` until the
//! builder is hoisted to a shared location.

#[path = "../admin/display_json.rs"]
mod builder;

use axum::{Json, extract::Extension, extract::Path, response::IntoResponse};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query as EngineQuery;
use serde_json::Value;
use std::sync::Arc;

use super::TaskError;

/// `GET /app/rest/process-definitions/:process_definition_id/model-json`
pub(super) async fn process_definition_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let model = engine
        .get_repository_service()
        .get_bpmn_model(&process_definition_id)
        .map_err(TaskError::from_engine)?;
    Ok(Json(builder::build_process_definition_display(
        model.as_ref(),
    )))
}

/// `GET /app/rest/process-instances/:process_instance_id/model-json`
pub(super) async fn process_instance_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let pi = super::list_all_process_instances(&engine)?
        .into_iter()
        .find(|p| p.id == process_instance_id)
        .ok_or_else(|| TaskError::not_found(format!("Process instance {process_instance_id}")))?;
    runtime_display(&engine, &pi.process_definition_id, &process_instance_id)
}

/// `GET /app/rest/process-instances/history/:process_instance_id/model-json`
pub(super) async fn process_instance_history_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let historic = historic_process_instance(&engine, &process_instance_id)?;
    history_display(
        &engine,
        &historic.process_definition_id,
        &process_instance_id,
    )
}

/// `GET /app/rest/process-instances/debugger/:process_instance_id/model-json`
///
/// Java `getDebuggerModelJSON`: runtime highlighting while the instance is
/// active, falling back to the historic view once it has ended.
pub(super) async fn process_instance_debugger_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let pi = super::list_all_process_instances(&engine)?
        .into_iter()
        .find(|p| p.id == process_instance_id);
    if let Some(pi) = pi {
        return runtime_display(&engine, &pi.process_definition_id, &process_instance_id);
    }
    let historic = historic_process_instance(&engine, &process_instance_id)?;
    history_display(
        &engine,
        &historic.process_definition_id,
        &process_instance_id,
    )
}

fn cmmn_engine(
    engine: &ProcessEngine,
) -> Result<std::sync::Arc<flowable_cmmn_engine::CmmnEngine>, TaskError> {
    engine
        .get_config()
        .cmmn_engine
        .clone()
        .ok_or_else(|| TaskError::bad_request("CMMN engine is not configured on this process engine"))
}

/// `GET /app/rest/case-definitions/:case_definition_id/model-json`
///
/// Java `CaseInstanceDisplayJsonClientResource.getModelJSONForCaseDefinition`.
pub(super) async fn case_definition_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(case_definition_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    let cmmn = cmmn_engine(&engine)?;
    let definition = cmmn
        .repository_service()
        .get_case_definition(&case_definition_id)
        .map_err(|e| TaskError::from_engine(e))?;
    Ok(Json(builder::build_case_definition_display(
        &definition.model,
        &std::collections::HashMap::new(),
    )))
}

/// `GET /app/rest/case-instances/:case_instance_id/model-json`
///
/// Java runtime case diagram with plan-item highlighting.
pub(super) async fn case_instance_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(case_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    case_display(&engine, &case_instance_id)
}

/// `GET /app/rest/case-instances/history/:case_instance_id/model-json`
///
/// Java historic case diagram (same builder; plan items drive highlighting).
pub(super) async fn case_instance_history_model_json(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(case_instance_id): Path<String>,
) -> Result<impl IntoResponse, TaskError> {
    case_display(&engine, &case_instance_id)
}

fn case_display(
    engine: &ProcessEngine,
    case_instance_id: &str,
) -> Result<Json<Value>, TaskError> {
    let cmmn = cmmn_engine(engine)?;
    let case_definition_id = match cmmn.runtime_service().get_case_instance(case_instance_id) {
        Ok(instance) => instance.case_definition_id,
        Err(_) => cmmn
            .history_service()
            .get_historic_case_instance(case_instance_id)
            .map_err(|e| TaskError::from_engine(e))?
            .case_definition_id,
    };
    let definition = cmmn
        .repository_service()
        .get_case_definition(&case_definition_id)
        .map_err(|e| TaskError::from_engine(e))?;
    let plan_item_instances = cmmn
        .runtime_service()
        .create_plan_item_instance_query()
        .case_instance_id(case_instance_id.to_string())
        .include_ended()
        .list()
        .unwrap_or_default();
    let mut completed = Vec::new();
    let mut current = Vec::new();
    let mut available = Vec::new();
    for item in &plan_item_instances {
        if item.ended_at.is_some() || item.occurred_at.is_some() {
            completed.push(item.plan_item_definition_id.clone());
        }
        if item.state.eq_ignore_ascii_case("active") {
            current.push(item.plan_item_definition_id.clone());
        }
        if item.state.eq_ignore_ascii_case("available") {
            available.push(item.plan_item_definition_id.clone());
        }
    }
    Ok(Json(builder::build_case_instance_display(
        &definition.model,
        &std::collections::HashMap::new(),
        &completed,
        &current,
        &available,
    )))
}

fn runtime_display(
    engine: &ProcessEngine,
    process_definition_id: &str,
    process_instance_id: &str,
) -> Result<Json<Value>, TaskError> {
    let model = engine
        .get_repository_service()
        .get_bpmn_model(process_definition_id)
        .map_err(TaskError::from_engine)?;
    let (completed, current) = activity_sets(engine, process_instance_id);
    Ok(Json(builder::build_process_instance_display(
        model.as_ref(),
        &completed,
        &current,
    )))
}

fn history_display(
    engine: &ProcessEngine,
    process_definition_id: &str,
    process_instance_id: &str,
) -> Result<Json<Value>, TaskError> {
    let model = engine
        .get_repository_service()
        .get_bpmn_model(process_definition_id)
        .map_err(TaskError::from_engine)?;
    let (completed, _) = activity_sets(engine, process_instance_id);
    Ok(Json(builder::build_history_display(
        model.as_ref(),
        &completed,
    )))
}

fn historic_process_instance(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Result<flowable_engine::history::historic_entities::HistoricProcessInstance, TaskError> {
    engine
        .get_history_service()
        .create_historic_process_instance_query()
        .process_instance_id(process_instance_id.to_string())
        .single_result()
        .map_err(TaskError::from_engine)?
        .ok_or_else(|| TaskError::not_found(format!("Process instance {process_instance_id}")))
}

/// Java: historic activity instances split on end time — ended ones are
/// completed, still-open ones are current.
fn activity_sets(engine: &ProcessEngine, process_instance_id: &str) -> (Vec<String>, Vec<String>) {
    let activities = engine
        .get_history_service()
        .create_historic_activity_instance_query()
        .process_instance_id(process_instance_id.to_string())
        .list()
        .unwrap_or_default();
    let mut completed = Vec::new();
    let mut current = Vec::new();
    for activity in activities {
        if activity.end_time.is_some() {
            completed.push(activity.activity_id);
        } else {
            current.push(activity.activity_id);
        }
    }
    (completed, current)
}
