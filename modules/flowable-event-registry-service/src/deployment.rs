use crate::models::{
    ChannelDefinition, ChannelDefinitionUpdateRequest, EventDefinition,
    EventDefinitionUpdateRequest, EventRegistryDeployment, EventRegistryDeploymentRequest,
    EventRegistryDeploymentResource, EventRegistryResourceData,
};
use crate::FlowableEventRegistryService;
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::runtime_store::{
    EventRegistryChangeRecord, EventRegistryChannelDefinition,
    EventRegistryDeploymentResource as StoredEventRegistryDeploymentResource,
    EventRegistryEventDefinition,
};
use flowable_event_registry_converter::{
    EventRegistryConverterError, parse_channel_definition as parse_channel_model,
    parse_event_definition as parse_event_model,
};
use flowable_event_registry_model::{ChannelType, EventPayloadField};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

impl FlowableEventRegistryService {
    pub fn deploy(
        &self,
        request: EventRegistryDeploymentRequest,
    ) -> Result<EventRegistryDeployment, FlowableError> {
        if request.name.trim().is_empty() {
            return Err(FlowableError::DeploymentValidationError(
                "Event Registry deployment name is required".to_string(),
            ));
        }

        let mut stored_resources = request
            .resources
            .iter()
            .map(|resource| StoredEventRegistryDeploymentResource {
                resource_name: resource.resource_name.clone(),
                resource: resource.resource.clone(),
            })
            .collect::<Vec<_>>();
        stored_resources.sort_by(|left, right| left.resource_name.cmp(&right.resource_name));

        let mut parsed_resources = request
            .resources
            .into_iter()
            .map(parse_resource)
            .collect::<Result<Vec<_>, _>>()?;
        parsed_resources.sort_by(|left, right| left.resource_name().cmp(right.resource_name()));

        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        let existing_channels = store.list_event_registry_channel_definitions(&mut session);
        let existing_events = store.list_event_registry_event_definitions(&mut session);
        let mut seen_channel_keys = BTreeMap::new();
        let mut seen_event_keys = BTreeMap::new();

        let deployment_id = format!("event-registry-deployment:{}", Uuid::new_v4());
        let deployed_at = store.time_source().now().timestamp_millis();
        let resource_names = parsed_resources
            .iter()
            .map(|resource| resource.resource_name().to_string())
            .collect::<Vec<_>>();

        let deployment = EventRegistryDeployment {
            id: deployment_id.clone(),
            name: request.name,
            deployed_at,
            category: request.category,
            parent_deployment_id: request.parent_deployment_id,
            tenant_id: request.tenant_id,
            resource_names,
            resources: stored_resources,
        };

        let mut channel_definitions = Vec::new();
        let mut event_definitions = Vec::new();

        for resource in parsed_resources {
            match resource {
                ParsedResource::Channel(definition) => {
                    if seen_channel_keys
                        .insert(definition.key.clone(), ())
                        .is_some()
                    {
                        return Err(FlowableError::DeploymentValidationError(format!(
                            "Channel definition key '{}' is declared more than once in deployment",
                            definition.key
                        )));
                    }
                    self.configuration.validate_channel_configuration(
                        &definition.key,
                        &definition.channel_type,
                        &definition.configuration,
                    )?;
                    let version = next_channel_definition_version(
                        &existing_channels,
                        &definition.key,
                        deployment.tenant_id.as_deref(),
                    );

                    channel_definitions.push(EventRegistryChannelDefinition {
                        id: format!("{}:{}", deployment_id, definition.resource_name),
                        deployment_id: deployment_id.clone(),
                        key: definition.key,
                        name: definition.name,
                        description: definition.description,
                        category: deployment.category.clone(),
                        channel_type: definition.channel_type,
                        resource_name: definition.resource_name,
                        version,
                        create_time: deployed_at,
                        tenant_id: deployment.tenant_id.clone(),
                        parent_deployment_id: deployment.parent_deployment_id.clone(),
                        configuration: definition.configuration,
                    });
                }
                ParsedResource::Event(definition) => {
                    if seen_event_keys.insert(definition.key.clone(), ()).is_some() {
                        return Err(FlowableError::DeploymentValidationError(format!(
                            "Event definition key '{}' is declared more than once in deployment",
                            definition.key
                        )));
                    }
                    let version = next_event_definition_version(
                        &existing_events,
                        &definition.key,
                        deployment.tenant_id.as_deref(),
                    );

                    event_definitions.push(EventRegistryEventDefinition {
                        id: format!("{}:{}", deployment_id, definition.resource_name),
                        deployment_id: deployment_id.clone(),
                        key: definition.key,
                        name: definition.name,
                        description: definition.description,
                        category: deployment.category.clone(),
                        event_type: definition.event_type,
                        channel_key: definition.channel_key,
                        resource_name: definition.resource_name,
                        version,
                        tenant_id: deployment.tenant_id.clone(),
                        parent_deployment_id: deployment.parent_deployment_id.clone(),
                        payload: definition.payload,
                    });
                }
            }
        }

        let known_channel_keys = existing_channels
            .iter()
            .map(|item| item.key.clone())
            .chain(channel_definitions.iter().map(|item| item.key.clone()))
            .collect::<Vec<_>>();
        for definition in &event_definitions {
            if !known_channel_keys
                .iter()
                .any(|key| key == &definition.channel_key)
            {
                return Err(FlowableError::DeploymentValidationError(format!(
                    "Event definition '{}' references unknown channel key '{}'",
                    definition.key, definition.channel_key
                )));
            }
        }

        store.insert_event_registry_deployment(deployment.clone(), &mut session);
        for definition in &channel_definitions {
            store.insert_event_registry_channel_definition(definition.clone(), &mut session);
        }
        for definition in &event_definitions {
            store.insert_event_registry_event_definition(definition.clone(), &mut session);
        }

        // Durable change records are written in the same transaction so failed
        // deployments (that never commit) never publish revisions.
        for definition in &channel_definitions {
            let revision = store.next_event_registry_change_revision(&mut session)?;
            store.insert_event_registry_change_record(
                EventRegistryChangeRecord {
                    id: format!("event-registry-change:{}", Uuid::new_v4()),
                    revision,
                    change_type: "deploy".to_string(),
                    entity_type: "channel".to_string(),
                    entity_id: definition.id.clone(),
                    entity_key: definition.key.clone(),
                    tenant_id: definition.tenant_id.clone(),
                    version: Some(definition.version),
                    deployment_id: Some(deployment_id.clone()),
                    created_at: deployed_at,
                },
                &mut session,
            )?;
        }
        for definition in &event_definitions {
            let revision = store.next_event_registry_change_revision(&mut session)?;
            store.insert_event_registry_change_record(
                EventRegistryChangeRecord {
                    id: format!("event-registry-change:{}", Uuid::new_v4()),
                    revision,
                    change_type: "deploy".to_string(),
                    entity_type: "event".to_string(),
                    entity_id: definition.id.clone(),
                    entity_key: definition.key.clone(),
                    tenant_id: definition.tenant_id.clone(),
                    version: Some(definition.version),
                    deployment_id: Some(deployment_id.clone()),
                    created_at: deployed_at,
                },
                &mut session,
            )?;
        }

        session.flush_and_commit()?;

        // Local cache is updated only after commit.
        {
            let mut cache = self.definition_cache.lock().unwrap();
            for definition in channel_definitions {
                cache.register_channel(definition);
            }
            for definition in event_definitions {
                cache.register_event(definition);
            }
        }
        // The local watermark is intentionally left untouched: advancing it here
        // (e.g. to the global max revision) would permanently skip foreign change
        // records committed between our last reconcile and this deployment.
        // Re-applying our own records on the next reconcile is idempotent.

        Ok(deployment)
    }

    pub fn get_channel_definition(
        &self,
        channel_definition_id: &str,
    ) -> Result<ChannelDefinition, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        store
            .find_event_registry_channel_definition(channel_definition_id, &mut session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry channel definition '{}' was not found",
                    channel_definition_id
                ))
            })
    }

    pub fn get_deployment(
        &self,
        deployment_id: &str,
    ) -> Result<EventRegistryDeployment, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        store
            .find_event_registry_deployment(deployment_id, &mut session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry deployment '{}' was not found",
                    deployment_id
                ))
            })
    }

    pub fn delete_deployment(&self, deployment_id: &str) -> Result<(), FlowableError> {
        self.get_deployment(deployment_id)?;
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        let channels = store
            .list_event_registry_channel_definitions(&mut session)
            .into_iter()
            .filter(|definition| definition.deployment_id == deployment_id)
            .collect::<Vec<_>>();
        let events = store
            .list_event_registry_event_definitions(&mut session)
            .into_iter()
            .filter(|definition| definition.deployment_id == deployment_id)
            .collect::<Vec<_>>();
        let now = store.time_source().now().timestamp_millis();

        store.delete_event_registry_deployment(deployment_id, &mut session);

        for definition in &channels {
            let revision = store.next_event_registry_change_revision(&mut session)?;
            store.insert_event_registry_change_record(
                EventRegistryChangeRecord {
                    id: format!("event-registry-change:{}", Uuid::new_v4()),
                    revision,
                    change_type: "delete".to_string(),
                    entity_type: "channel".to_string(),
                    entity_id: definition.id.clone(),
                    entity_key: definition.key.clone(),
                    tenant_id: definition.tenant_id.clone(),
                    version: Some(definition.version),
                    deployment_id: Some(deployment_id.to_string()),
                    created_at: now,
                },
                &mut session,
            )?;
        }
        for definition in &events {
            let revision = store.next_event_registry_change_revision(&mut session)?;
            store.insert_event_registry_change_record(
                EventRegistryChangeRecord {
                    id: format!("event-registry-change:{}", Uuid::new_v4()),
                    revision,
                    change_type: "delete".to_string(),
                    entity_type: "event".to_string(),
                    entity_id: definition.id.clone(),
                    entity_key: definition.key.clone(),
                    tenant_id: definition.tenant_id.clone(),
                    version: Some(definition.version),
                    deployment_id: Some(deployment_id.to_string()),
                    created_at: now,
                },
                &mut session,
            )?;
        }
        let revision = store.next_event_registry_change_revision(&mut session)?;
        store.insert_event_registry_change_record(
            EventRegistryChangeRecord {
                id: format!("event-registry-change:{}", Uuid::new_v4()),
                revision,
                change_type: "delete".to_string(),
                entity_type: "deployment".to_string(),
                entity_id: deployment_id.to_string(),
                entity_key: deployment_id.to_string(),
                tenant_id: None,
                version: None,
                deployment_id: Some(deployment_id.to_string()),
                created_at: now,
            },
            &mut session,
        )?;

        session.flush_and_commit()?;

        {
            let mut cache = self.definition_cache.lock().unwrap();
            for definition in &channels {
                cache.unregister_channel_id(&definition.id);
            }
            for definition in &events {
                cache.unregister_event_id(&definition.id);
            }
            // Best-effort reload of the surviving latest versions: the delete
            // already committed, and reconcile repairs the cache from the
            // change log if this session fails.
            if let Ok(mut reload_session) = store.create_session() {
                let remaining_channels =
                    store.list_event_registry_channel_definitions(&mut reload_session);
                let remaining_events =
                    store.list_event_registry_event_definitions(&mut reload_session);
                for definition in &channels {
                    if let Some(previous) = remaining_channels
                        .iter()
                        .filter(|candidate| {
                            candidate.key == definition.key
                                && candidate.tenant_id == definition.tenant_id
                        })
                        .max_by_key(|candidate| candidate.version)
                        .cloned()
                    {
                        cache.register_channel(previous);
                    }
                }
                for definition in &events {
                    if let Some(previous) = remaining_events
                        .iter()
                        .filter(|candidate| {
                            candidate.key == definition.key
                                && candidate.tenant_id == definition.tenant_id
                        })
                        .max_by_key(|candidate| candidate.version)
                        .cloned()
                    {
                        cache.register_event(previous);
                    }
                }
            }
        }
        // The local watermark is intentionally left untouched: setting it to this
        // delete's revision would permanently skip foreign change records with
        // lower revisions that were not reconciled yet. Re-applying our own
        // delete records on the next reconcile is idempotent.

        Ok(())
    }

    pub fn get_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<EventRegistryResourceData>, FlowableError> {
        let deployment = self.get_deployment(deployment_id)?;
        Ok(deployment
            .resources
            .into_iter()
            .map(|resource| {
                EventRegistryResourceData::new(
                    deployment_id.to_string(),
                    EventRegistryDeploymentResource {
                        resource_name: resource.resource_name,
                        resource: resource.resource,
                    },
                    deployment.deployed_at,
                )
            })
            .collect())
    }

    pub fn get_deployment_resource(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<EventRegistryResourceData, FlowableError> {
        self.get_deployment_resources(deployment_id)?
            .into_iter()
            .find(|resource| resource.resource_name == resource_name)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry deployment resource '{}' was not found in deployment '{}'",
                    resource_name, deployment_id
                ))
            })
    }

    pub fn get_channel_definition_resource_data(
        &self,
        channel_definition_id: &str,
    ) -> Result<EventRegistryResourceData, FlowableError> {
        let definition = self.get_channel_definition(channel_definition_id)?;
        self.get_deployment_resource(&definition.deployment_id, &definition.resource_name)
    }

    pub fn get_event_definition_resource_data(
        &self,
        event_definition_id: &str,
    ) -> Result<EventRegistryResourceData, FlowableError> {
        let definition = self.get_event_definition(event_definition_id)?;
        self.get_deployment_resource(&definition.deployment_id, &definition.resource_name)
    }

    pub fn get_event_definition(
        &self,
        event_definition_id: &str,
    ) -> Result<EventDefinition, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        store
            .find_event_registry_event_definition(event_definition_id, &mut session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry event definition '{}' was not found",
                    event_definition_id
                ))
            })
    }

    pub fn update_channel_definition(
        &self,
        channel_definition_id: &str,
        request: ChannelDefinitionUpdateRequest,
    ) -> Result<ChannelDefinition, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        let mut definition = store
            .find_event_registry_channel_definition(channel_definition_id, &mut session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry channel definition '{}' was not found",
                    channel_definition_id
                ))
            })?;

        if let Some(name) = request.name {
            if name.trim().is_empty() {
                return Err(FlowableError::DeploymentValidationError(
                    "Channel definition name cannot be empty".to_string(),
                ));
            }
            definition.name = name;
        }

        if let Some(configuration) = request.configuration {
            self.configuration.validate_channel_configuration(
                &definition.key,
                &definition.channel_type,
                &configuration,
            )?;
            definition.configuration = configuration;
        }

        store.update_event_registry_channel_definition(definition.clone(), &mut session);
        // Publish the update in the same transaction so other instances
        // reconcile it through the change log.
        let now = store.time_source().now().timestamp_millis();
        let revision = store.next_event_registry_change_revision(&mut session)?;
        store.insert_event_registry_change_record(
            EventRegistryChangeRecord {
                id: format!("event-registry-change:{}", Uuid::new_v4()),
                revision,
                change_type: "update".to_string(),
                entity_type: "channel".to_string(),
                entity_id: definition.id.clone(),
                entity_key: definition.key.clone(),
                tenant_id: definition.tenant_id.clone(),
                version: Some(definition.version),
                deployment_id: Some(definition.deployment_id.clone()),
                created_at: now,
            },
            &mut session,
        )?;
        session.flush_and_commit()?;
        // Replace the local cache body only after commit. The local watermark
        // is left untouched; re-applying our own record on the next reconcile
        // is idempotent and never skips foreign records.
        self.definition_cache
            .lock()
            .unwrap()
            .register_channel(definition.clone());
        Ok(definition)
    }

    pub fn update_event_definition(
        &self,
        event_definition_id: &str,
        request: EventDefinitionUpdateRequest,
    ) -> Result<EventDefinition, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        let mut definition = store
            .find_event_registry_event_definition(event_definition_id, &mut session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Event Registry event definition '{}' was not found",
                    event_definition_id
                ))
            })?;

        if let Some(name) = request.name {
            if name.trim().is_empty() {
                return Err(FlowableError::DeploymentValidationError(
                    "Event definition name cannot be empty".to_string(),
                ));
            }
            definition.name = name;
        }

        if let Some(payload) = request.payload {
            definition.payload = payload;
        }

        store.update_event_registry_event_definition(definition.clone(), &mut session);
        // Publish the update in the same transaction so other instances
        // reconcile it through the change log.
        let now = store.time_source().now().timestamp_millis();
        let revision = store.next_event_registry_change_revision(&mut session)?;
        store.insert_event_registry_change_record(
            EventRegistryChangeRecord {
                id: format!("event-registry-change:{}", Uuid::new_v4()),
                revision,
                change_type: "update".to_string(),
                entity_type: "event".to_string(),
                entity_id: definition.id.clone(),
                entity_key: definition.key.clone(),
                tenant_id: definition.tenant_id.clone(),
                version: Some(definition.version),
                deployment_id: Some(definition.deployment_id.clone()),
                created_at: now,
            },
            &mut session,
        )?;
        session.flush_and_commit()?;
        // Replace the local cache body only after commit. The local watermark
        // is left untouched; re-applying our own record on the next reconcile
        // is idempotent and never skips foreign records.
        self.definition_cache
            .lock()
            .unwrap()
            .register_event(definition.clone());
        Ok(definition)
    }
}

#[derive(Debug)]
enum ParsedResource {
    Channel(ParsedChannelDefinition),
    Event(ParsedEventDefinition),
}

impl ParsedResource {
    fn resource_name(&self) -> &str {
        match self {
            Self::Channel(definition) => &definition.resource_name,
            Self::Event(definition) => &definition.resource_name,
        }
    }
}

#[derive(Debug)]
struct ParsedChannelDefinition {
    key: String,
    name: String,
    description: Option<String>,
    channel_type: String,
    resource_name: String,
    configuration: Value,
}

#[derive(Debug)]
struct ParsedEventDefinition {
    key: String,
    name: String,
    description: Option<String>,
    event_type: String,
    channel_key: String,
    resource_name: String,
    payload: Value,
}

fn parse_resource(
    resource: EventRegistryDeploymentResource,
) -> Result<ParsedResource, FlowableError> {
    let value: Value = serde_json::from_str(&resource.resource).map_err(|error| {
        FlowableError::DeploymentValidationError(format!(
            "Event Registry resource '{}' is not valid JSON: {}",
            resource.resource_name, error
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        FlowableError::DeploymentValidationError(format!(
            "Event Registry resource '{}' must be a JSON object",
            resource.resource_name
        ))
    })?;

    if resource.resource_name.ends_with(".channel") {
        parse_channel_definition(object, &resource.resource_name).map(ParsedResource::Channel)
    } else if resource.resource_name.ends_with(".event") {
        parse_event_definition(object, &resource.resource_name).map(ParsedResource::Event)
    } else {
        Err(FlowableError::DeploymentValidationError(format!(
            "Unsupported Event Registry resource '{}'",
            resource.resource_name
        )))
    }
}

fn parse_channel_definition(
    object: &Map<String, Value>,
    resource_name: &str,
) -> Result<ParsedChannelDefinition, FlowableError> {
    let channel_type = required_string(object, "channelType", resource_name)?;
    if channel_type != "inbound" && channel_type != "outbound" {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Unsupported channelType '{}' in '{}'",
            channel_type, resource_name
        )));
    }

    // Adapter type presence is required; registry membership is validated against
    // the engine-local configuration during deploy.
    let _adapter_type = required_string(object, "type", resource_name)?;

    validate_resource_name_field(object, resource_name)?;

    let mut normalized = serde_json::Map::new();
    normalized.insert(
        "key".to_string(),
        Value::String(required_string(object, "key", resource_name)?),
    );
    normalized.insert(
        "name".to_string(),
        Value::String(required_string(object, "name", resource_name)?),
    );
    if let Some(description) = optional_string(object, "description") {
        normalized.insert("description".to_string(), Value::String(description));
    }
    normalized.insert(
        "channelType".to_string(),
        Value::String(channel_type.clone()),
    );
    normalized.insert(
        "resourceName".to_string(),
        Value::String(resource_name.to_string()),
    );

    let mut configuration = object.clone();
    for field in ["key", "name", "description", "channelType", "resourceName"] {
        configuration.remove(field);
    }
    normalized.insert("configuration".to_string(), Value::Object(configuration));

    let model = parse_channel_model(&Value::Object(normalized).to_string())
        .map_err(converter_error_to_flowable_error)?;

    Ok(ParsedChannelDefinition {
        key: model.key,
        name: model.name.unwrap_or_default(),
        description: model.description,
        channel_type: match model.channel_type {
            ChannelType::Inbound => "inbound".to_string(),
            ChannelType::Outbound => "outbound".to_string(),
        },
        resource_name: model
            .resource_name
            .unwrap_or_else(|| resource_name.to_string()),
        configuration: Value::Object(model.configuration),
    })
}

fn parse_event_definition(
    object: &Map<String, Value>,
    resource_name: &str,
) -> Result<ParsedEventDefinition, FlowableError> {
    validate_resource_name_field(object, resource_name)?;

    let mut normalized = serde_json::Map::new();
    normalized.insert(
        "key".to_string(),
        Value::String(required_string(object, "key", resource_name)?),
    );
    normalized.insert(
        "name".to_string(),
        Value::String(required_string(object, "name", resource_name)?),
    );
    if let Some(description) = optional_string(object, "description") {
        normalized.insert("description".to_string(), Value::String(description));
    }
    normalized.insert(
        "eventType".to_string(),
        Value::String(required_string(object, "eventType", resource_name)?),
    );
    normalized.insert(
        "channelKey".to_string(),
        Value::String(required_string(object, "channelKey", resource_name)?),
    );
    normalized.insert(
        "resourceName".to_string(),
        Value::String(resource_name.to_string()),
    );
    normalized.insert(
        "payload".to_string(),
        object
            .get("payload")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    let model = parse_event_model(&Value::Object(normalized).to_string())
        .map_err(converter_error_to_flowable_error)?;

    Ok(ParsedEventDefinition {
        key: model.key,
        name: model.name.unwrap_or_default(),
        description: model.description,
        event_type: model.event_type,
        channel_key: model.channel_key,
        resource_name: model
            .resource_name
            .unwrap_or_else(|| resource_name.to_string()),
        payload: Value::Array(
            model
                .payload
                .into_iter()
                .map(payload_field_to_json)
                .collect(),
        ),
    })
}

fn required_string(
    object: &Map<String, Value>,
    field: &str,
    resource_name: &str,
) -> Result<String, FlowableError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            FlowableError::DeploymentValidationError(format!(
                "Event Registry resource '{}' is missing string field '{}'",
                resource_name, field
            ))
        })
}

fn optional_string(object: &Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn validate_resource_name_field(
    object: &Map<String, Value>,
    resource_name: &str,
) -> Result<(), FlowableError> {
    if let Some(value) = object.get("resourceName").and_then(Value::as_str)
        && value != resource_name
    {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Event Registry resource '{}' declared mismatched resourceName '{}'",
            resource_name, value
        )));
    }

    Ok(())
}

fn payload_field_to_json(field: EventPayloadField) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("name".to_string(), Value::String(field.name));
    object.insert("type".to_string(), Value::String(field.field_type));
    if let Some(required) = field.required {
        object.insert("required".to_string(), Value::Bool(required));
    }
    Value::Object(object)
}

fn converter_error_to_flowable_error(error: EventRegistryConverterError) -> FlowableError {
    FlowableError::DeploymentValidationError(error.to_string())
}

fn next_event_definition_version(
    existing_events: &[EventRegistryEventDefinition],
    key: &str,
    tenant_id: Option<&str>,
) -> i32 {
    existing_events
        .iter()
        .filter(|definition| definition.key == key && definition.tenant_id.as_deref() == tenant_id)
        .map(|definition| definition.version)
        .max()
        .unwrap_or(0)
        + 1
}

fn next_channel_definition_version(
    existing_channels: &[EventRegistryChannelDefinition],
    key: &str,
    tenant_id: Option<&str>,
) -> i32 {
    existing_channels
        .iter()
        .filter(|definition| definition.key == key && definition.tenant_id.as_deref() == tenant_id)
        .map(|definition| definition.version)
        .max()
        .unwrap_or(0)
        + 1
}
