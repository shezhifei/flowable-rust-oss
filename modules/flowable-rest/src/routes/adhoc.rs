use crate::error::ApiError;
use axum::{Extension, Json, Router, extract::Path, routing::post};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

const ACTIVATE_TASK_PATH: &str =
    "/runtime/process-instances/:process_instance_id/adhoc-tasks/activate";
const COMPLETE_TASK_PATH: &str =
    "/runtime/process-instances/:process_instance_id/adhoc-tasks/:task_id/complete";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{ACTIVATE_TASK_PATH}"),
            post(activate_adhoc_task),
        )
        .route(
            &format!("{prefix}{COMPLETE_TASK_PATH}"),
            post(complete_adhoc_task),
        )
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateTaskCommand {
    pub task_id: String,
    pub variables: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateTaskResponse {
    pub task_id: String,
    pub process_instance_id: String,
    pub activated: bool,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteTaskCommand {
    pub variables: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteTaskResponse {
    pub task_id: String,
    pub process_instance_id: String,
    pub completed: bool,
    pub message: String,
}

pub async fn activate_adhoc_task(
    Path(process_instance_id): Path<String>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(command): Json<ActivateTaskCommand>,
) -> Result<Json<ActivateTaskResponse>, ApiError> {
    let task_id = command.task_id.trim();
    if task_id.is_empty() {
        return Err(ApiError::BadRequest("taskId is required".to_string()));
    }

    activate_adhoc_task_for_process_instance(&engine, &process_instance_id, task_id)?;

    Ok(Json(ActivateTaskResponse {
        task_id: command.task_id,
        process_instance_id,
        activated: true,
        message: "Task activated successfully".to_string(),
    }))
}

pub async fn complete_adhoc_task(
    Path((process_instance_id, task_id)): Path<(String, String)>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(command): Json<CompleteTaskCommand>,
) -> Result<Json<CompleteTaskResponse>, ApiError> {
    if task_id.trim().is_empty() {
        return Err(ApiError::BadRequest("taskId is required".to_string()));
    }

    complete_active_adhoc_task_for_process_instance(
        &engine,
        &process_instance_id,
        &task_id,
        command.variables,
    )?;

    Ok(Json(CompleteTaskResponse {
        task_id,
        process_instance_id,
        completed: true,
        message: "Task completed successfully".to_string(),
    }))
}

fn activate_adhoc_task_for_process_instance(
    engine: &ProcessEngine,
    process_instance_id: &str,
    task_id: &str,
) -> Result<(), ApiError> {
    let runtime_store = engine.get_runtime_store();
    let (process_instance, mut candidate_execution_ids) = {
        let mut session = runtime_store.create_session().unwrap();
        let process_instance = runtime_store
            .find_process_instance(process_instance_id, &mut session)
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Process instance '{}' was not found",
                    process_instance_id
                ))
            })?;
        let candidate_execution_ids = runtime_store
            .snapshot_executions(&mut session)
            .into_values()
            .filter(|execution| {
                execution.process_instance_id.as_deref() == Some(process_instance_id)
                    && execution.activity_id.is_some()
                    && !execution.is_ended
            })
            .map(|execution| execution.id)
            .collect::<Vec<_>>();
        session.rollback().ok();
        (process_instance, candidate_execution_ids)
    };

    if process_instance.is_ended {
        return Err(ApiError::BadRequest(format!(
            "Cannot activate task for ended process instance '{}'",
            process_instance_id
        )));
    }

    candidate_execution_ids.sort();

    let mut last_not_found = None;
    for execution_id in candidate_execution_ids {
        match engine
            .get_runtime_service()
            .activate_adhoc_task(&execution_id, task_id)
        {
            Ok(()) => return Ok(()),
            Err(FlowableError::NotFound(message)) => last_not_found = Some(message),
            Err(error) => return Err(ApiError::from(error)),
        }
    }

    Err(ApiError::NotFound(last_not_found.unwrap_or_else(|| {
        format!(
            "Waiting ad-hoc subprocess for process instance '{}' and task '{}' was not found",
            process_instance_id, task_id
        )
    })))
}

fn complete_active_adhoc_task_for_process_instance(
    engine: &ProcessEngine,
    process_instance_id: &str,
    task_id: &str,
    variables: Option<HashMap<String, serde_json::Value>>,
) -> Result<(), ApiError> {
    {
        let runtime_store = engine.get_runtime_store();
        let mut session = runtime_store.create_session().unwrap();
        runtime_store
            .find_process_instance(process_instance_id, &mut session)
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Process instance '{}' was not found",
                    process_instance_id
                ))
            })?;
        session.rollback().ok();
    }

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .map_err(ApiError::from)?
        .into_iter()
        .find(|task| !task.is_completed && task.task_definition_key == task_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Open ad-hoc task '{}' was not found for process instance '{}'",
                task_id, process_instance_id
            ))
        })?;

    if let Some(variables) = variables {
        engine
            .get_task_service()
            .complete_task_by_id_with_variables(task.id, variables)
            .map_err(ApiError::from)?;
    } else {
        engine
            .get_task_service()
            .complete_task_by_id(task.id)
            .map_err(ApiError::from)?;
    }

    // P24 engine semantics: completing an ad-hoc task no longer auto-leaves
    // the ad-hoc subprocess (Java parity - RuntimeService#completeAdhocSubProcess
    // is an explicit call). This REST endpoint's contract is "complete the task
    // and finish the subprocess", so explicitly complete the instance's ad-hoc
    // subprocess executions after the task completes.
    let runtime_service = engine.get_runtime_service();
    let adhoc_executions = runtime_service
        .get_adhoc_subprocess_executions(process_instance_id)
        .map_err(ApiError::from)?;
    for execution in adhoc_executions {
        runtime_service
            .complete_adhoc_subprocess(&execution.id)
            .map_err(ApiError::from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_activate_task_command_deserialization() {
        let json = json!({
            "taskId": "task1",
            "variables": {
                "approved": true
            }
        });

        let command: ActivateTaskCommand = serde_json::from_value(json).unwrap();
        assert_eq!(command.task_id, "task1");
        assert!(command.variables.is_some());
        let vars = command.variables.unwrap();
        assert_eq!(vars.get("approved"), Some(&json!(true)));
    }

    #[test]
    fn test_activate_task_command_without_variables() {
        let json = json!({
            "taskId": "task1"
        });

        let command: ActivateTaskCommand = serde_json::from_value(json).unwrap();
        assert_eq!(command.task_id, "task1");
        assert!(command.variables.is_none());
    }

    #[test]
    fn test_complete_task_command_deserialization() {
        let json = json!({
            "variables": {
                "result": "approved"
            }
        });

        let command: CompleteTaskCommand = serde_json::from_value(json).unwrap();
        assert!(command.variables.is_some());
        let vars = command.variables.unwrap();
        assert_eq!(vars.get("result"), Some(&json!("approved")));
    }

    #[test]
    fn test_complete_task_command_without_variables() {
        let json = json!({});

        let command: CompleteTaskCommand = serde_json::from_value(json).unwrap();
        assert!(command.variables.is_none());
    }

    #[test]
    fn test_activate_task_response_serialization() {
        let response = ActivateTaskResponse {
            task_id: "task1".to_string(),
            process_instance_id: "proc1".to_string(),
            activated: true,
            message: "Task activated successfully".to_string(),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["taskId"], "task1");
        assert_eq!(json["processInstanceId"], "proc1");
        assert_eq!(json["activated"], true);
        assert_eq!(json["message"], "Task activated successfully");
    }

    #[test]
    fn test_complete_task_response_serialization() {
        let response = CompleteTaskResponse {
            task_id: "task1".to_string(),
            process_instance_id: "proc1".to_string(),
            completed: true,
            message: "Task completed successfully".to_string(),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["taskId"], "task1");
        assert_eq!(json["processInstanceId"], "proc1");
        assert_eq!(json["completed"], true);
        assert_eq!(json["message"], "Task completed successfully");
    }
}
