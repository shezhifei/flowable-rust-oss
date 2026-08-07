//! Bridge from BPMN engine `send-event` to this crate's outbound pipeline.
//!
//! Engine cannot depend on this crate (dependency cycle). Instead the engine
//! exposes [`OutboundEventDispatchHook`]; this module is the service-side
//! implementation installed onto `ProcessEngineConfiguration.outbound_event_dispatch`
//! when a [`crate::FlowableEventRegistryService`] is constructed.
//!
//! Java: `DefaultOutboundEventProcessor.java:32-66` (transform + adapter).

use crate::models::{ChannelDefinition, EventPayload};
use crate::pipeline::{dispatch_outbound_event, EventRegistryConfiguration};
use flowable_engine::engine::outbound_event_dispatch::{
    OutboundEventDispatchHook, OutboundEventDispatchRequest,
};
use flowable_engine::error::FlowableError;
use std::sync::Arc;

/// Dispatches via the service's registered transformers and channel adapters.
pub struct ConfigurationBackedOutboundEventDispatch {
    configuration: Arc<EventRegistryConfiguration>,
}

impl ConfigurationBackedOutboundEventDispatch {
    pub fn new(configuration: Arc<EventRegistryConfiguration>) -> Self {
        Self { configuration }
    }
}

impl OutboundEventDispatchHook for ConfigurationBackedOutboundEventDispatch {
    fn dispatch_outbound(
        &self,
        request: &OutboundEventDispatchRequest,
    ) -> Result<(), FlowableError> {
        // dispatch_outbound_event only reads key + configuration (pipeline.rs:929-950).
        let channel = ChannelDefinition {
            id: String::new(),
            deployment_id: String::new(),
            key: request.channel_key.clone(),
            name: request.channel_key.clone(),
            description: None,
            category: None,
            channel_type: "outbound".to_string(),
            resource_name: String::new(),
            version: 1,
            create_time: 0,
            tenant_id: None,
            parent_deployment_id: None,
            configuration: request.channel_configuration.clone(),
        };
        dispatch_outbound_event(
            &self.configuration,
            &channel,
            EventPayload {
                event_type: request.event_type.clone(),
                payload: request.payload.clone(),
                dispatch_token: request.dispatch_token.clone(),
            },
        )
    }
}
