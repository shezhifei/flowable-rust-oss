use crate::error::ApiError;
use axum::{Extension, Router, http::StatusCode, routing::post};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::{EventSubscriptionKind, RuntimeEventWaitKind};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

const MESSAGES_PATH: &str = "/runtime/messages";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new().route(
        &format!("{prefix}{MESSAGES_PATH}"),
        post(message_event_received),
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageEventReceivedRequest {
    message_name: Option<String>,
    business_key: Option<String>,
    process_instance_id: Option<String>,
    #[serde(default)]
    variables: Vec<MessageVariableRequest>,
    #[serde(default)]
    transient_variables: Vec<MessageVariableRequest>,
    #[serde(default)]
    r#async: bool,
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageVariableRequest {
    name: Option<String>,
    value: Value,
    #[serde(rename = "type")]
    _variable_type: Option<String>,
}

pub(crate) async fn message_event_received(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: MessageEventReceivedRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let message_name = request
        .message_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("messageName is required"))?;

    let variables = parse_message_variables(request.variables)?;
    let _transient_variables = parse_message_variables(request.transient_variables)?;
    let process_instance_id = request.process_instance_id;
    let business_key = request.business_key;
    let _tenant_id = request.tenant_id;

    if request.r#async {
        let engine_clone = engine.clone();
        let message_name_owned = message_name.to_string();
        let process_instance_id_owned = process_instance_id.clone();
        let business_key_owned = business_key.clone();
        tokio::spawn(async move {
            let _ = correlate_message(
                &engine_clone,
                &message_name_owned,
                process_instance_id_owned.as_deref(),
                business_key_owned.as_deref(),
                &variables,
            );
        });
        return Ok(StatusCode::ACCEPTED);
    }

    let matched = correlate_message(
        &engine,
        message_name,
        process_instance_id.as_deref(),
        business_key.as_deref(),
        &variables,
    )?;

    if matched {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!(
            "No process instance found for message '{message_name}'"
        )))
    }
}

fn parse_message_variables(
    variables: Vec<MessageVariableRequest>,
) -> Result<Vec<(String, Value)>, ApiError> {
    variables
        .into_iter()
        .map(|variable| {
            let name = variable
                .name
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| ApiError::bad_request("Variable name is required."))?;
            Ok((name, variable.value))
        })
        .collect()
}

fn correlate_message(
    engine: &Arc<ProcessEngine>,
    message_name: &str,
    process_instance_id: Option<&str>,
    business_key: Option<&str>,
    variables: &[(String, Value)],
) -> Result<bool, ApiError> {
    let runtime_store = engine.get_runtime_store();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let mut matched_any = false;

    let (mut execution_ids, receive_task_ids, boundary_process_ids, event_subprocess_ids) = {
        let mut session = runtime_store.create_session().unwrap();
        let event_wait_states = runtime_store.snapshot_event_wait_states(&mut session);
        let execution_ids: Vec<String> = event_wait_states
            .values()
            .filter(|wait_state| {
                matches!(
                    wait_state.wait_kind,
                    RuntimeEventWaitKind::MessageIntermediateCatchEvent
                        | RuntimeEventWaitKind::ReceiveTask
                ) && wait_state
                    .event_subscription
                    .as_ref()
                    .is_some_and(|subscription| {
                        subscription.kind == EventSubscriptionKind::Message
                            && subscription.event_ref == message_name
                    })
            })
            .filter(|wait_state| {
                filter_by_process_instance(
                    &runtime_store,
                    &wait_state.process_instance_id,
                    process_instance_id,
                    business_key,
                )
            })
            .map(|wait_state| wait_state.execution_id.clone())
            .collect();
        let receive_task_ids: Vec<String> = event_wait_states
            .values()
            .filter(|wait_state| {
                wait_state.wait_kind == RuntimeEventWaitKind::ReceiveTask
                    && wait_state.event_subscription.is_none()
            })
            .filter(|wait_state| {
                filter_by_process_instance(
                    &runtime_store,
                    &wait_state.process_instance_id,
                    process_instance_id,
                    business_key,
                )
            })
            .map(|wait_state| wait_state.process_instance_id.clone())
            .collect();
        let boundary_process_ids: Vec<String> = runtime_store
            .snapshot_boundary_event_states(&mut session)
            .into_values()
            .filter(|state| {
                state.event_subscription.kind == EventSubscriptionKind::Message
                    && state.event_subscription.event_ref == message_name
            })
            .filter(|state| {
                filter_by_process_instance(
                    &runtime_store,
                    &state.process_instance_id,
                    process_instance_id,
                    business_key,
                )
            })
            .map(|state| state.process_instance_id)
            .collect();
        let event_subprocess_ids: Vec<String> = runtime_store
            .find_event_subprocess_event_subscriptions_by_event_ref(
                &EventSubscriptionKind::Message,
                message_name,
                &mut session,
            )
            .into_iter()
            .filter(|subscription| {
                filter_by_process_instance(
                    &runtime_store,
                    &subscription.process_instance_id,
                    process_instance_id,
                    business_key,
                )
            })
            .map(|subscription| subscription.process_instance_id)
            .collect();
        session.rollback().ok();
        (
            execution_ids,
            receive_task_ids,
            boundary_process_ids,
            event_subprocess_ids,
        )
    };
    execution_ids.sort();
    execution_ids.dedup();

    for execution_id in &execution_ids {
        for (name, value) in variables {
            engine.get_variable_service().set_variable(
                execution_id.clone(),
                name.clone(),
                value.clone(),
            )?;
        }
        runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
            message_name.to_string(),
            execution_id.clone(),
        );
        matched_any = true;
    }

    for pid in &receive_task_ids {
        for (name, value) in variables {
            engine
                .get_variable_service()
                .set_variable(pid.clone(), name.clone(), value.clone())?;
        }
        let _ = task_service.wake_up_message_by_message_ref(pid.clone(), message_name.to_string());
        matched_any = true;
    }

    for pid in &boundary_process_ids {
        for (name, value) in variables {
            engine
                .get_variable_service()
                .set_variable(pid.clone(), name.clone(), value.clone())?;
        }
        runtime_service
            .trigger_boundary_event_by_message_ref(message_name.to_string(), pid.clone());
        matched_any = true;
    }

    for pid in &event_subprocess_ids {
        for (name, value) in variables {
            engine
                .get_variable_service()
                .set_variable(pid.clone(), name.clone(), value.clone())?;
        }
        runtime_service.trigger_event_subprocess_by_message(message_name.to_string(), pid.clone());
        matched_any = true;
    }

    // 5. Process start subscriptions
    let start_subscriptions: Vec<_> = engine
        .get_event_start_subscriptions()
        .into_iter()
        .filter(|subscription| {
            subscription.event_kind == EventSubscriptionKind::Message
                && subscription.event_ref == message_name
        })
        .collect();

    for _subscription in &start_subscriptions {
        let instance =
            runtime_service.start_process_instance_by_message(message_name.to_string())?;
        for (name, value) in variables {
            engine.get_variable_service().set_variable(
                instance.id.clone(),
                name.clone(),
                value.clone(),
            )?;
        }
        matched_any = true;
    }

    Ok(matched_any)
}

fn filter_by_process_instance(
    store: &flowable_engine::persistence::runtime_store::RuntimeStore,
    actual_process_instance_id: &str,
    filter_process_instance_id: Option<&str>,
    filter_business_key: Option<&str>,
) -> bool {
    if let Some(target_pid) = filter_process_instance_id
        && actual_process_instance_id != target_pid
    {
        return false;
    }
    if let Some(target_bk) = filter_business_key {
        let mut session = store.create_session().unwrap();
        if let Some(instance) =
            store.find_process_instance(actual_process_instance_id, &mut session)
            && instance.business_key.as_deref() != Some(target_bk)
        {
            return false;
        }
    }
    true
}
