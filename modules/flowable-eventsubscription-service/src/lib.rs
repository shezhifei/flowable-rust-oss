use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::runtime_service::EventSubscriptionQuery;
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::runtime_store::RuntimeEventWaitState;
use std::sync::Arc;

pub struct FlowableEventSubscriptionService {
    engine: Arc<ProcessEngine>,
}

impl FlowableEventSubscriptionService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        Self { engine }
    }

    pub fn create_event_subscription_query(&self) -> EventSubscriptionQuery {
        self.engine
            .get_runtime_service()
            .create_event_subscription_query()
    }

    pub fn get_event_subscriptions_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Result<Vec<RuntimeEventWaitState>, FlowableError> {
        self.engine
            .get_event_subscription_service()
            .get_event_subscriptions_by_process_instance_id(process_instance_id)
    }
}
