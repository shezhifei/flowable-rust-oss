use crate::models::{EventInstanceDelivery, EventInstanceStatus};
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::db_session::DbSession;
use flowable_engine::persistence::runtime_store::RuntimeStore;

pub(crate) fn transition_delivery_status(
    store: &RuntimeStore,
    session: &mut DbSession,
    delivery: &mut EventInstanceDelivery,
    new_status: EventInstanceStatus,
    timestamp: i64,
) -> Result<(), FlowableError> {
    delivery.status = new_status.clone();
    delivery.updated_at = timestamp;
    delivery.status_history.push(new_status);
    store.update_event_registry_event_instance_delivery(delivery.clone(), session)?;
    Ok(())
}

pub(crate) fn mark_delivery_failed(
    store: &RuntimeStore,
    session: &mut DbSession,
    delivery: &mut EventInstanceDelivery,
    error_message: String,
    timestamp: i64,
    retry_attempted: bool,
) -> Result<(), FlowableError> {
    if retry_attempted {
        delivery.retry_count = delivery.retry_count.saturating_add(1);
        delivery.last_retry_at = Some(timestamp);
    }
    delivery.status = EventInstanceStatus::Failed;
    delivery.updated_at = timestamp;
    delivery.last_error = Some(error_message);
    delivery.last_failure_at = Some(timestamp);
    delivery.next_retry_at = Some(timestamp);

    if delivery.status_history.last() != Some(&EventInstanceStatus::Failed) {
        delivery.status_history.push(EventInstanceStatus::Failed);
    }

    store.update_event_registry_event_instance_delivery(delivery.clone(), session)?;
    Ok(())
}

pub(crate) fn clear_delivery_failure(
    delivery: &mut EventInstanceDelivery,
    retry_timestamp: Option<i64>,
) {
    if let Some(timestamp) = retry_timestamp {
        delivery.retry_count = delivery.retry_count.saturating_add(1);
        delivery.last_retry_at = Some(timestamp);
    }
    delivery.last_error = None;
    delivery.last_failure_at = None;
    delivery.next_retry_at = None;
}
