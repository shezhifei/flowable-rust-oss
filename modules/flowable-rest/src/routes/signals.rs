use crate::error::ApiError;
use axum::{Extension, Router, http::StatusCode, routing::post};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::{
    EventSubscriptionKind, RuntimeEventWaitKind, RuntimeStore,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

const SIGNALS_PATH: &str = "/runtime/signals";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new().route(
        &format!("{prefix}{SIGNALS_PATH}"),
        post(signal_event_received),
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignalEventReceivedRequest {
    signal_name: Option<String>,
    #[serde(default)]
    variables: Vec<SignalVariableRequest>,
    #[serde(default)]
    r#async: bool,
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignalVariableRequest {
    name: Option<String>,
    value: Value,
    #[serde(rename = "type")]
    _variable_type: Option<String>,
}

pub(crate) async fn signal_event_received(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: SignalEventReceivedRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let signal_name = request
        .signal_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("signalName is required"))?;

    let variables = parse_signal_variables(request.variables)?;
    let tenant_id = request.tenant_id;

    if request.r#async {
        if !variables.is_empty() {
            return Err(ApiError::bad_request(
                "Async signals cannot take variables as payload",
            ));
        }
        let engine_clone = engine.clone();
        let signal_name_owned = signal_name.to_string();
        let tenant_id_owned = tenant_id.clone();
        tokio::spawn(async move {
            // The 202 is already committed, so a storage failure cannot become a response —
            // but it must not be silent either.
            if let Err(error) = trigger_signal(
                engine_clone,
                &signal_name_owned,
                &[],
                tenant_id_owned.as_deref(),
            ) {
                tracing::error!(
                    error = ?error,
                    signal_name = %signal_name_owned,
                    "async signal trigger failed"
                );
            }
        });
        return Ok(StatusCode::ACCEPTED);
    }

    trigger_signal(engine, signal_name, &variables, tenant_id.as_deref())?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_signal_variables(
    variables: Vec<SignalVariableRequest>,
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

pub(crate) fn trigger_signal(
    engine: Arc<ProcessEngine>,
    signal_name: &str,
    variables: &[(String, Value)],
    tenant_id: Option<&str>,
) -> Result<(), ApiError> {
    let runtime_store = engine.get_runtime_store();
    let runtime_service = engine.get_runtime_service();
    let (mut execution_ids, boundary_process_ids, event_subprocess_process_ids) = {
        let mut session = runtime_store.create_session().map_err(|e| ApiError::InternalServerError(e.to_string()))?;
        let execution_ids: Vec<String> = runtime_store
            .snapshot_event_wait_states(&mut session)
            .into_values()
            .filter(|wait_state| {
                wait_state.wait_kind == RuntimeEventWaitKind::SignalIntermediateCatchEvent
                    && wait_state
                        .event_subscription
                        .as_ref()
                        .is_some_and(|subscription| {
                            subscription.kind == EventSubscriptionKind::Signal
                                && subscription.event_ref == signal_name
                        })
                    && matches_tenant(
                        &runtime_store,
                        &mut session,
                        wait_state.process_instance_id.as_str(),
                        tenant_id,
                    )
            })
            .map(|wait_state| wait_state.execution_id)
            .collect();
        let boundary_process_ids: Vec<String> = runtime_store
            .snapshot_boundary_event_states(&mut session)
            .into_values()
            .filter(|state| {
                state.event_subscription.kind == EventSubscriptionKind::Signal
                    && state.event_subscription.event_ref == signal_name
                    && matches_tenant(
                        &runtime_store,
                        &mut session,
                        &state.process_instance_id,
                        tenant_id,
                    )
            })
            .map(|state| state.process_instance_id)
            .collect();
        let event_subprocess_process_ids: Vec<String> = runtime_store
            .find_event_subprocess_event_subscriptions_by_event_ref(
                &EventSubscriptionKind::Signal,
                signal_name,
                &mut session,
            )
            .into_iter()
            .filter(|subscription| {
                matches_tenant(
                    &runtime_store,
                    &mut session,
                    &subscription.process_instance_id,
                    tenant_id,
                )
            })
            .map(|subscription| subscription.process_instance_id)
            .collect();
        session.rollback().ok();
        (
            execution_ids,
            boundary_process_ids,
            event_subprocess_process_ids,
        )
    };
    execution_ids.sort();

    for execution_id in execution_ids {
        for (name, value) in variables {
            engine.get_variable_service().set_variable(
                execution_id.clone(),
                name.clone(),
                value.clone(),
            )?;
        }
        // Java parity: global signal broadcast does NOT check suspension
        runtime_service
            .trigger_global_signal_intermediate_catch(signal_name.to_string(), execution_id)?;
    }

    for process_instance_id in boundary_process_ids {
        runtime_service
            .trigger_boundary_event_by_signal_ref(signal_name.to_string(), process_instance_id)?;
    }

    for process_instance_id in event_subprocess_process_ids {
        runtime_service
            .trigger_event_subprocess_by_signal(signal_name.to_string(), process_instance_id)?;
    }

    let mut start_subscriptions: Vec<_> = Vec::new();
    for subscription in engine.get_event_start_subscriptions()? {
        if subscription.event_kind != EventSubscriptionKind::Signal
            || subscription.event_ref != signal_name
        {
            continue;
        }
        if matches_definition_tenant(
            &engine,
            &subscription.process_definition_id,
            tenant_id,
        )? {
            start_subscriptions.push(subscription);
        }
    }
    if !start_subscriptions.is_empty() {
        runtime_service.start_process_instance_by_signal(signal_name.to_string())?;
    }

    Ok(())
}

fn matches_tenant(
    runtime_store: &RuntimeStore,
    session: &mut flowable_engine::persistence::DbSession,
    process_instance_id: &str,
    tenant_id: Option<&str>,
) -> bool {
    let Some(filter_tenant) = tenant_id else {
        return true;
    };
    // Storage failure is not a tenant mismatch: sticky-error so the caller
    // can answer 500 instead of silently dropping the row.
    match runtime_store.find_process_instance(process_instance_id, session) {
        Ok(found) => found.and_then(|pi| pi.tenant_id).as_deref() == Some(filter_tenant),
        Err(error) => {
            session.note_write_error(error);
            false
        }
    }
}

fn matches_definition_tenant(
    engine: &ProcessEngine,
    process_definition_id: &str,
    tenant_id: Option<&str>,
) -> Result<bool, ApiError> {
    let Some(filter_tenant) = tenant_id else {
        return Ok(true);
    };
    match engine
        .get_repository_service()
        .get_process_definition(process_definition_id)
    {
        Ok(def) => Ok(def.tenant_id.as_deref() == Some(filter_tenant)),
        Err(flowable_engine::error::FlowableError::NotFound(_)) => Ok(false),
        Err(e) => Err(ApiError::InternalServerError(e.to_string())),
    }
}
