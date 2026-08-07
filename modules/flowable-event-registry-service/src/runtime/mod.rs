pub(crate) mod delivery;

use crate::models::{
    ChannelDefinition, EventDefinition, EventDirection, EventInstanceDelivery,
    EventInstanceRequest, EventInstanceStatus, EventPayload, InboundEventRequest,
    OutboundEventRequest,
};
use crate::pipeline::{
    dispatch_outbound_event, validate_event_payload, InboundEventContext, InboundRawEvent,
};
use crate::query::{
    latest_event_definition_for_tenant_with_policy, EventInstanceDeliveryQuery,
};
use crate::runtime::delivery::{
    clear_delivery_failure, mark_delivery_failed, transition_delivery_status,
};
use crate::FlowableEventRegistryService;
use flowable_engine::error::FlowableError;
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

impl FlowableEventRegistryService {
    /// Process a raw inbound event addressed by channel key through the ADR-6 pipeline:
    /// extraction → filter → tenant → transform → key detection → definition resolution →
    /// payload validation → consumer dispatch.
    pub fn process_inbound_channel_event(
        &self,
        raw: InboundRawEvent,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        let channel = self.load_inbound_channel(&raw.channel_key, raw.tenant_hint.as_deref())?;
        self.run_inbound_pipeline(channel, raw, None)
    }

    /// Compatibility adapter: resolve by event type, then route through the channel pipeline.
    pub fn receive_inbound_event(
        &self,
        request: InboundEventRequest,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        let (channel_key, definition_key) = {
            let definitions = self.resolve_event_definitions_by_event_type(&request.event_type)?;

            if definitions.is_empty() {
                return Err(FlowableError::NotFound(format!(
                    "No inbound event definition found for event type '{}'",
                    request.event_type
                )));
            }

            let mut inbound_definitions = Vec::new();
            for item in definitions {
                let channel = self.resolve_latest_channel_definition(
                    &item.channel_key,
                    item.tenant_id.as_deref(),
                )?;
                if channel
                    .map(|channel| channel.channel_type == "inbound")
                    .unwrap_or(false)
                {
                    inbound_definitions.push(item);
                }
            }

            if inbound_definitions.is_empty() {
                return Err(FlowableError::BadRequest(format!(
                    "Event type '{}' is not bound to an inbound channel",
                    request.event_type
                )));
            }

            let policy = self.configuration.tenant_fallback_policy();
            let definition = latest_event_definition_for_tenant_with_policy(
                inbound_definitions,
                request.tenant_id.as_deref(),
                &policy,
            )
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "No inbound event definition found for event type '{}' and tenant '{}'",
                    request.event_type,
                    request.tenant_id.as_deref().unwrap_or("")
                ))
            })?;

            (definition.channel_key.clone(), definition.key.clone())
        };

        let mut headers = BTreeMap::new();
        headers.insert("eventKey".to_string(), definition_key);

        self.process_inbound_channel_event(InboundRawEvent {
            channel_key,
            body: request.event_payload,
            headers,
            tenant_hint: request.tenant_id,
        })
    }

    pub fn receive_event_instance(
        &self,
        request: EventInstanceRequest,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        let event_reference_count = request.event_definition_id.is_some() as u8
            + request.event_definition_key.is_some() as u8;
        if event_reference_count == 0 {
            return Err(FlowableError::ExecutionError(
                "Either eventDefinitionId or eventDefinitionKey is required.".to_string(),
            ));
        }
        if event_reference_count > 1 {
            return Err(FlowableError::ExecutionError(
                "Only one of eventDefinitionId or eventDefinitionKey should be set.".to_string(),
            ));
        }

        let channel_reference_count = request.channel_definition_id.is_some() as u8
            + request.channel_definition_key.is_some() as u8;
        if channel_reference_count == 0 {
            return Err(FlowableError::ExecutionError(
                "Either channelDefinitionId or channelDefinitionKey is required.".to_string(),
            ));
        }
        if channel_reference_count > 1 {
            return Err(FlowableError::ExecutionError(
                "Only one of channelDefinitionId or channelDefinitionKey should be set."
                    .to_string(),
            ));
        }

        let (channel, definition) = {
            let tenant_id = request.tenant_id.as_deref();
            let definition = if let Some(id) = request.event_definition_id.as_deref() {
                self.resolve_event_definition_by_id(id)?
            } else if let Some(key) = request.event_definition_key.as_deref() {
                self.resolve_latest_event_definition(key, tenant_id)?
            } else {
                None
            }
            .ok_or_else(|| FlowableError::NotFound("No event definition found".to_string()))?;

            let channel = if let Some(id) = request.channel_definition_id.as_deref() {
                self.resolve_channel_definition_by_id(id)?
            } else if let Some(key) = request.channel_definition_key.as_deref() {
                self.resolve_latest_channel_definition(key, tenant_id)?
            } else {
                None
            }
            .ok_or_else(|| FlowableError::NotFound("No channel definition found".to_string()))?;

            if channel.channel_type != "inbound" {
                return Err(FlowableError::ExecutionError(format!(
                    "Channel definition '{}' is not inbound",
                    channel.key
                )));
            }
            if definition.channel_key != channel.key {
                return Err(FlowableError::ExecutionError(format!(
                    "Event definition '{}' is not bound to channel '{}'",
                    definition.key, channel.key
                )));
            }
            (channel, definition)
        };

        let mut headers = BTreeMap::new();
        headers.insert("eventKey".to_string(), definition.key.clone());
        self.run_inbound_pipeline(
            channel,
            InboundRawEvent {
                channel_key: definition.channel_key.clone(),
                body: request.event_payload,
                headers,
                tenant_hint: request.tenant_id,
            },
            Some(definition),
        )
    }

    pub fn publish_outbound_event(
        &self,
        request: OutboundEventRequest,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        // definition resolution (cache-backed resolver) → validation → transform → adapter
        let definition = self
            .resolve_latest_event_definition(
                &request.event_definition_key,
                request.tenant_id.as_deref(),
            )?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry event definition '{}' was not found",
                    request.event_definition_key
                ))
            })?;
        let channel = self
            .resolve_latest_channel_definition(
                &definition.channel_key,
                definition.tenant_id.as_deref(),
            )?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry channel '{}' was not found",
                    definition.channel_key
                ))
            })?;

        if channel.channel_type != "outbound" {
            return Err(FlowableError::ExecutionError(format!(
                "Event definition '{}' is not bound to an outbound channel",
                definition.key
            )));
        }

        self.run_outbound_pipeline(definition, channel, request.event_payload, None)
    }

    pub fn create_event_instance_delivery_query(&self) -> EventInstanceDeliveryQuery {
        EventInstanceDeliveryQuery::new(Arc::clone(&self.engine))
    }

    pub fn get_event_instance_delivery(
        &self,
        delivery_id: &str,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        store
            .find_event_registry_event_instance_delivery(delivery_id, &mut session)?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry event delivery '{}' was not found",
                    delivery_id
                ))
            })
    }

    pub fn retry_event_delivery(
        &self,
        delivery_id: &str,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        let delivery = {
            let store = self.engine.get_runtime_store();
            let mut session = store.create_session()?;
            store
                .find_event_registry_event_instance_delivery(delivery_id, &mut session)?
                .ok_or_else(|| {
                    FlowableError::NotFound(format!(
                        "Event Registry event delivery '{}' was not found",
                        delivery_id
                    ))
                })?
        };

        let direction = delivery.direction.clone();
        let status = delivery.status.clone();
        match (direction, status) {
            (
                EventDirection::Outbound,
                EventInstanceStatus::Created | EventInstanceStatus::Failed,
            ) => {
                // Deterministic replay: use the original persisted definition id/version.
                let definition = self
                    .resolve_event_definition_by_id(&delivery.event_definition_id)?
                    .ok_or_else(|| {
                        FlowableError::NotFound(format!(
                            "Event Registry event definition '{}' was not found for retry",
                            delivery.event_definition_id
                        ))
                    })?;
                let channel = self.resolve_retry_channel(&delivery)?;

                self.run_outbound_pipeline(
                    definition,
                    channel,
                    delivery.payload.clone(),
                    Some(delivery),
                )
            }
            (EventDirection::Inbound, EventInstanceStatus::Failed) => {
                self.retry_inbound_delivery(delivery)
            }
            (direction, status) => Err(FlowableError::Conflict(format!(
                "Event delivery '{}' cannot be retried: {:?} delivery in status {:?} is not retryable",
                delivery_id, direction, status
            ))),
        }
    }

    /// Re-runs the inbound consumer for a failed delivery against the original
    /// definition/channel pipeline and persists the outcome.
    fn retry_inbound_delivery(
        &self,
        mut delivery: EventInstanceDelivery,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        let definition = self
            .resolve_event_definition_by_id(&delivery.event_definition_id)?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry event definition '{}' was not found for retry",
                    delivery.event_definition_id
                ))
            })?;
        let channel = self.resolve_retry_channel(&delivery)?;
        if channel.channel_type != "inbound" {
            return Err(FlowableError::BadRequest(format!(
                "Channel '{}' is not bound to an inbound channel",
                channel.key
            )));
        }

        // The persisted payload is already extracted/transformed; re-run
        // validation and consumer dispatch like the original pipeline, without
        // holding a store session across host code.
        let consumer = self.configuration.resolve_consumer(&channel.configuration)?;
        let outcome = validate_event_payload(&definition, &delivery.payload)
            .and_then(|_| consumer.consume(&delivery, &definition));

        let store = self.engine.get_runtime_store();
        let now = store.time_source().now().timestamp_millis();
        if let Err(error) = outcome {
            let persist_result = store
                .create_session()
                .map_err(FlowableError::from)
                .and_then(|mut session| {
                    mark_delivery_failed(
                        &store,
                        &mut session,
                        &mut delivery,
                        error.to_string(),
                        now,
                        true,
                    )?;
                    session.flush_and_commit().map_err(FlowableError::from)
                });
            if let Err(persist_error) = persist_result {
                return Err(FlowableError::Internal(format!(
                    "Inbound event delivery '{}' retry failed ({}) and persisting the Failed status also failed: {}",
                    delivery.id, error, persist_error
                )));
            }
            return Err(error);
        }
        clear_delivery_failure(&mut delivery, Some(now));
        let persist_result = store
            .create_session()
            .map_err(FlowableError::from)
            .and_then(|mut session| {
                transition_delivery_status(
                    &store,
                    &mut session,
                    &mut delivery,
                    EventInstanceStatus::Processed,
                    now,
                )?;
                session.flush_and_commit().map_err(FlowableError::from)
            });
        if let Err(persist_error) = persist_result {
            return Err(FlowableError::Internal(format!(
                "Inbound event delivery '{}' was consumed but persisting the Processed status failed: {}. \
                 The consumer already ran (at-least-once); another retry may run it again.",
                delivery.id, persist_error
            )));
        }
        Ok(delivery)
    }

    /// Loads the channel for a retry: the original channel definition when the
    /// delivery recorded it, otherwise (legacy deliveries) strict resolution by
    /// the delivery's own tenant — never an any-tenant fallback.
    fn resolve_retry_channel(
        &self,
        delivery: &EventInstanceDelivery,
    ) -> Result<ChannelDefinition, FlowableError> {
        if let Some(channel_definition_id) = delivery.channel_definition_id.as_deref() {
            return self
                .resolve_channel_definition_by_id(channel_definition_id)?
                .ok_or_else(|| {
                    FlowableError::NotFound(format!(
                        "Event Registry channel definition '{}' was not found for retry",
                        channel_definition_id
                    ))
                });
        }
        self.resolve_latest_channel_definition(
            &delivery.channel_key,
            delivery.tenant_id.as_deref(),
        )?
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Legacy event delivery '{}' has no recorded channel definition and channel '{}' \
                 could not be resolved for tenant '{}'; the delivery cannot be retried",
                delivery.id,
                delivery.channel_key,
                delivery.tenant_id.as_deref().unwrap_or("")
            ))
        })
    }

    pub fn delete_event_delivery(&self, delivery_id: &str) -> Result<(), FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        store
            .find_event_registry_event_instance_delivery(delivery_id, &mut session)?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry event delivery '{}' was not found",
                    delivery_id
                ))
            })?;

        store.delete_event_registry_event_instance_delivery(delivery_id, &mut session)?;
        session.flush_and_commit()?;
        Ok(())
    }

    fn load_inbound_channel(
        &self,
        channel_key: &str,
        tenant_hint: Option<&str>,
    ) -> Result<ChannelDefinition, FlowableError> {
        let channel = self
            .resolve_latest_channel_definition(channel_key, tenant_hint)?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry inbound channel '{}' was not found for tenant '{}'",
                    channel_key,
                    tenant_hint.unwrap_or("")
                ))
            })?;

        if channel.channel_type != "inbound" {
            return Err(FlowableError::BadRequest(format!(
                "Channel '{}' is not bound to an inbound channel",
                channel_key
            )));
        }
        Ok(channel)
    }

    fn run_inbound_pipeline(
        &self,
        channel: ChannelDefinition,
        raw: InboundRawEvent,
        preloaded_definition: Option<EventDefinition>,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        // Host processors are invoked without holding store sessions/locks.
        let channel_config = channel.configuration.clone();

        let extractor = self.configuration.resolve_payload_extractor(&channel_config)?;
        let mut context = InboundEventContext::from_raw(&raw);
        context.payload = extractor.extract(&raw, &channel_config)?;

        let filter = self.configuration.resolve_filter(&channel_config)?;
        if !filter.retain(&context, &channel_config)? {
            return Err(FlowableError::BadRequest(format!(
                "Inbound event on channel '{}' was filtered out",
                channel.key
            )));
        }

        let tenant_detector = self
            .configuration
            .resolve_tenant_detector(&channel_config)?;
        let detected_tenant = tenant_detector.detect_tenant(&context, &channel_config)?;
        context.tenant_id = detected_tenant.or(raw.tenant_hint.clone());

        let transformer = self
            .configuration
            .resolve_inbound_transformer(&channel_config)?;
        context.payload = transformer.transform(&context, &channel_config)?;

        let key_detector = self.configuration.resolve_key_detector(&channel_config)?;
        let event_key = key_detector.detect_event_key(&context, &channel_config)?;
        context.event_key = Some(event_key.clone());

        let definition = if let Some(definition) = preloaded_definition {
            definition
        } else {
            self.resolve_inbound_event_definition(
                &event_key,
                &channel.key,
                context.tenant_id.as_deref(),
            )?
        };

        if definition.channel_key != channel.key {
            return Err(FlowableError::ExecutionError(format!(
                "Event definition '{}' is not bound to channel '{}'",
                definition.key, channel.key
            )));
        }

        validate_event_payload(&definition, &context.payload)?;

        let consumer = self.configuration.resolve_consumer(&channel_config)?;

        // Persist Received before consumer dispatch.
        let mut delivery = {
            let store = self.engine.get_runtime_store();
            let mut session = store.create_session()?;
            let now = store.time_source().now().timestamp_millis();
            let delivery = EventInstanceDelivery {
                id: format!("event-instance:{}", Uuid::new_v4()),
                event_definition_id: definition.id.clone(),
                event_definition_key: definition.key.clone(),
                event_type: definition.event_type.clone(),
                channel_key: definition.channel_key.clone(),
                direction: EventDirection::Inbound,
                status: EventInstanceStatus::Received,
                status_history: vec![EventInstanceStatus::Received],
                last_error: None,
                retry_count: 0,
                last_retry_at: None,
                last_failure_at: None,
                next_retry_at: None,
                dispatch_token: None,
                channel_definition_id: Some(channel.id.clone()),
                // Java EventInstanceImpl carries the *detected* runtime tenant
                // (DefaultInboundEventProcessingPipeline.java:148-150), not the
                // definition's storage tenant — critical when fallback resolves a
                // default-tenant definition for a non-default event tenant.
                tenant_id: context.tenant_id.clone(),
                payload: context.payload.clone(),
                created_at: now,
                updated_at: now,
            };
            store.insert_event_registry_event_instance_delivery(delivery.clone(), &mut session)?;
            session.flush_and_commit()?;
            delivery
        };

        // Invoke host consumer without holding a store session.
        if let Err(error) = consumer.consume(&delivery, &definition) {
            let store = self.engine.get_runtime_store();
            let now = store.time_source().now().timestamp_millis();
            let message = error.to_string();
            let persist_result = store
                .create_session()
                .map_err(FlowableError::from)
                .and_then(|mut session| {
                    mark_delivery_failed(&store, &mut session, &mut delivery, message, now, false)?;
                    session.flush_and_commit().map_err(FlowableError::from)
                });
            if let Err(persist_error) = persist_result {
                return Err(FlowableError::Internal(format!(
                    "Inbound event delivery '{}' failed ({}) and persisting the Failed status also failed: {}",
                    delivery.id, error, persist_error
                )));
            }
            return Err(error);
        }

        let store = self.engine.get_runtime_store();
        let now = store.time_source().now().timestamp_millis();
        let persist_result = store
            .create_session()
            .map_err(FlowableError::from)
            .and_then(|mut session| {
                transition_delivery_status(
                    &store,
                    &mut session,
                    &mut delivery,
                    EventInstanceStatus::Processed,
                    now,
                )?;
                session.flush_and_commit().map_err(FlowableError::from)
            });
        if let Err(persist_error) = persist_result {
            return Err(FlowableError::Internal(format!(
                "Inbound event delivery '{}' was consumed but persisting the Processed status failed: {}. \
                 The consumer already ran (at-least-once); a retry may run it again.",
                delivery.id, persist_error
            )));
        }
        Ok(delivery)
    }

    /// Outbound ADR-6 pipeline: definition already resolved → validation → transform → adapter.
    /// When `existing` is set (retry), the original delivery id/definition id/dispatch token are reused.
    fn run_outbound_pipeline(
        &self,
        definition: EventDefinition,
        channel: ChannelDefinition,
        payload: serde_json::Value,
        existing: Option<EventInstanceDelivery>,
    ) -> Result<EventInstanceDelivery, FlowableError> {
        // Validation before any external I/O and without holding a write session open for host code.
        validate_event_payload(&definition, &payload)?;

        let is_retry = existing.is_some();
        let mut delivery = if let Some(existing) = existing {
            existing
        } else {
            let store = self.engine.get_runtime_store();
            let mut session = store.create_session()?;
            let now = store.time_source().now().timestamp_millis();
            let delivery = EventInstanceDelivery {
                id: format!("event-instance:{}", Uuid::new_v4()),
                event_definition_id: definition.id.clone(),
                event_definition_key: definition.key.clone(),
                event_type: definition.event_type.clone(),
                channel_key: definition.channel_key.clone(),
                direction: EventDirection::Outbound,
                status: EventInstanceStatus::Created,
                status_history: vec![EventInstanceStatus::Created],
                last_error: None,
                retry_count: 0,
                last_retry_at: None,
                last_failure_at: None,
                next_retry_at: None,
                dispatch_token: Some(format!("dispatch:{}", Uuid::new_v4())),
                channel_definition_id: Some(channel.id.clone()),
                tenant_id: definition.tenant_id.clone(),
                payload: payload.clone(),
                created_at: now,
                updated_at: now,
            };
            // Persist Created + dispatch token before external I/O.
            store.insert_event_registry_event_instance_delivery(delivery.clone(), &mut session)?;
            session.flush_and_commit()?;
            delivery
        };

        if delivery.dispatch_token.is_none() || delivery.channel_definition_id.is_none() {
            if delivery.dispatch_token.is_none() {
                delivery.dispatch_token = Some(format!("dispatch:{}", Uuid::new_v4()));
            }
            // Legacy deliveries persisted before the original channel id was
            // recorded: backfill with the channel used for this replay.
            if delivery.channel_definition_id.is_none() {
                delivery.channel_definition_id = Some(channel.id.clone());
            }
            let store = self.engine.get_runtime_store();
            let mut session = store.create_session()?;
            store.update_event_registry_event_instance_delivery(delivery.clone(), &mut session)?;
            session.flush_and_commit()?;
        }

        // Host transform + adapter without holding store locks.
        let dispatch_result = dispatch_outbound_event(
            &self.configuration,
            &channel,
            EventPayload {
                event_type: definition.event_type.clone(),
                payload: delivery.payload.clone(),
                dispatch_token: delivery.dispatch_token.clone(),
            },
        );

        let store = self.engine.get_runtime_store();
        let now = store.time_source().now().timestamp_millis();

        if let Err(error) = dispatch_result {
            let message = error.to_string();
            let persist_result = store
                .create_session()
                .map_err(FlowableError::from)
                .and_then(|mut session| {
                    mark_delivery_failed(
                        &store,
                        &mut session,
                        &mut delivery,
                        message,
                        now,
                        is_retry,
                    )?;
                    session.flush_and_commit().map_err(FlowableError::from)
                });
            if let Err(persist_error) = persist_result {
                return Err(FlowableError::Internal(format!(
                    "Outbound event delivery '{}' failed ({}) and persisting the Failed status also failed: {}",
                    delivery.id, error, persist_error
                )));
            }
            return Err(error);
        }

        if is_retry {
            clear_delivery_failure(&mut delivery, Some(now));
        }
        let persist_result = store
            .create_session()
            .map_err(FlowableError::from)
            .and_then(|mut session| {
                transition_delivery_status(
                    &store,
                    &mut session,
                    &mut delivery,
                    EventInstanceStatus::Published,
                    now,
                )?;
                session.flush_and_commit().map_err(FlowableError::from)
            });
        if let Err(persist_error) = persist_result {
            return Err(FlowableError::Internal(format!(
                "Outbound event delivery '{}' was dispatched to channel '{}' but persisting the Published status failed: {}. \
                 The external system already received the event (at-least-once); a retry may re-dispatch it and \
                 receivers should deduplicate on dispatch token '{}'.",
                delivery.id,
                delivery.channel_key,
                persist_error,
                delivery.dispatch_token.as_deref().unwrap_or("<none>")
            )));
        }
        Ok(delivery)
    }
}
