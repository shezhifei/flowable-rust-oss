use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use axum::{Extension, Json, Router, extract::Path, http::Uri, routing::get};
use flowable_engine::engine::process_engine::ProcessEngine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const EVENT_SUBSCRIPTIONS_PATH: &str = "/runtime/event-subscriptions";
const EVENT_SUBSCRIPTION_PATH: &str = "/runtime/event-subscriptions/:event_subscription_id";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{EVENT_SUBSCRIPTIONS_PATH}"),
            get(list_event_subscriptions),
        )
        .route(
            &format!("{prefix}{EVENT_SUBSCRIPTION_PATH}"),
            get(get_event_subscription),
        )
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EventSubscriptionListQuery {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    #[serde(rename = "eventType")]
    event_type: Option<String>,
    #[serde(rename = "eventName")]
    event_name: Option<String>,
    #[serde(rename = "activityId")]
    activity_id: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "withoutProcessInstanceId")]
    without_process_instance_id: Option<bool>,
    configuration: Option<String>,
    #[serde(rename = "withoutConfiguration")]
    without_configuration: Option<bool>,
}

impl EventSubscriptionListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

/// P110: local row view for the `event_subscriptions` REST surface.
///
/// Java `EventSubscriptionCollectionResource.java:99-147` exposes
/// activityId/executionId/processInstanceId/configuration (+ their
/// `without*` variants) as query filters. The engine's `EventSubscription`
/// query model only surfaces id/eventName/eventType, so the REST layer reads
/// the persisted row directly — extras columns (execution_id,
/// process_instance_id, event_name, event_kind) plus the wait-state JSON
/// (activity_id, configuration) — mirroring `VariableInstanceQueryCmd` in
/// `variable_service.rs:830-855`.
struct EventSubscriptionRow {
    id: String,
    event_name: Option<String>,
    event_kind: Option<String>,
    execution_id: Option<String>,
    process_instance_id: Option<String>,
    activity_id: Option<String>,
    configuration: Option<String>,
}

fn load_event_subscriptions(
    engine: &Arc<ProcessEngine>,
) -> Result<Vec<EventSubscriptionRow>, ApiError> {
    let store = engine.get_runtime_store();
    let mut session = store
        .create_session()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let rows = session
        .find_raw_all("event_subscriptions")
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let _ = session.rollback();
    rows.into_iter()
        .map(|row| {
            let data = serde_json::from_str::<serde_json::Value>(row.data.as_ref()).ok();
            Ok(EventSubscriptionRow {
                id: row.id,
                event_name: row.extras.get("event_name").cloned().flatten(),
                event_kind: row.extras.get("event_kind").cloned().flatten(),
                execution_id: row.extras.get("execution_id").cloned().flatten(),
                process_instance_id: row.extras.get("process_instance_id").cloned().flatten(),
                activity_id: data
                    .as_ref()
                    .and_then(|value| value.get("activity_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                configuration: data
                    .as_ref()
                    .and_then(|value| value.get("configuration"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventSubscriptionResponse {
    id: String,
    url: String,
    event_type: String,
    event_name: String,
    activity_id: Option<String>,
    execution_id: Option<String>,
    execution_url: Option<String>,
    process_instance_id: Option<String>,
    process_instance_url: Option<String>,
    process_definition_id: Option<String>,
    process_definition_url: Option<String>,
    scope_id: Option<String>,
    scope_type: Option<String>,
    sub_scope_id: Option<String>,
    scope_definition_id: Option<String>,
    created: Option<String>,
    configuration: Option<String>,
    tenant_id: Option<String>,
}

fn to_event_subscription_response(subscription: EventSubscriptionRow) -> EventSubscriptionResponse {
    // Java `RestResponseFactory.createEventSubscriptionResponse`
    // (RestResponseFactory.java:1304-1336): executionUrl/processInstanceUrl are
    // only populated when the corresponding id is present.
    let execution_url = subscription
        .execution_id
        .as_ref()
        .map(|id| format!("/runtime/executions/{id}"));
    let process_instance_url = subscription
        .process_instance_id
        .as_ref()
        .map(|id| format!("/runtime/process-instances/{id}"));
    EventSubscriptionResponse {
        url: format!("/runtime/event-subscriptions/{}", subscription.id),
        id: subscription.id,
        event_type: subscription.event_kind.unwrap_or_default(),
        event_name: subscription.event_name.unwrap_or_default(),
        activity_id: subscription.activity_id,
        execution_id: subscription.execution_id,
        execution_url,
        process_instance_id: subscription.process_instance_id,
        process_instance_url,
        process_definition_id: None,
        process_definition_url: None,
        scope_id: None,
        scope_type: None,
        sub_scope_id: None,
        scope_definition_id: None,
        created: None,
        configuration: subscription.configuration,
        tenant_id: None,
    }
}

pub(crate) async fn list_event_subscriptions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<EventSubscriptionResponse>>, ApiError> {
    let query: EventSubscriptionListQuery = parse_query(&uri)?;
    let mut subscriptions = load_event_subscriptions(&engine)?;

    // Java `EventSubscriptionCollectionResource.java:90-148`: every parameter
    // is an equality filter on the event subscription entity.
    if let Some(id) = query.id.as_deref() {
        subscriptions.retain(|subscription| subscription.id == id);
    }
    if let Some(event_type) = query.event_type.as_deref() {
        subscriptions.retain(|subscription| subscription.event_kind.as_deref() == Some(event_type));
    }
    if let Some(event_name) = query.event_name.as_deref() {
        subscriptions.retain(|subscription| subscription.event_name.as_deref() == Some(event_name));
    }
    if let Some(activity_id) = query.activity_id.as_deref() {
        subscriptions.retain(|subscription| {
            subscription.activity_id.as_deref() == Some(activity_id)
        });
    }
    if let Some(execution_id) = query.execution_id.as_deref() {
        subscriptions.retain(|subscription| {
            subscription.execution_id.as_deref() == Some(execution_id)
        });
    }
    if let Some(process_instance_id) = query.process_instance_id.as_deref() {
        subscriptions.retain(|subscription| {
            subscription.process_instance_id.as_deref() == Some(process_instance_id)
        });
    }
    // Java :108 `query.withoutProcessInstanceId()` — rows with no
    // process instance id. Current rows always carry one, so this matches
    // nothing, but the filter is kept for Java parity.
    if query.without_process_instance_id.unwrap_or(false) {
        subscriptions.retain(|subscription| subscription.process_instance_id.is_none());
    }
    if let Some(configuration) = query.configuration.as_deref() {
        subscriptions.retain(|subscription| {
            subscription.configuration.as_deref() == Some(configuration)
        });
    }
    // Java :147 `query.withoutConfiguration()` — rows with no configuration
    // value (the common case for BPMN message/signal/timer subscriptions).
    if query.without_configuration.unwrap_or(false) {
        subscriptions.retain(|subscription| subscription.configuration.is_none());
    }

    subscriptions.sort_by(|left, right| left.id.cmp(&right.id));
    let response = subscriptions
        .into_iter()
        .map(to_event_subscription_response)
        .collect();

    Ok(Json(query.paging().paginate(response)))
}

pub(crate) async fn get_event_subscription(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(event_subscription_id): Path<String>,
) -> Result<Json<EventSubscriptionResponse>, ApiError> {
    let subscription = load_event_subscriptions(&engine)?
        .into_iter()
        .find(|subscription| subscription.id == event_subscription_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Event subscription '{}' was not found",
                event_subscription_id
            ))
        })?;

    Ok(Json(to_event_subscription_response(subscription)))
}
