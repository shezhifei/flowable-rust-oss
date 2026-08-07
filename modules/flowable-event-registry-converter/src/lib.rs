use flowable_event_registry_model::{ChannelDefinition, EventDefinition};
use serde_json::{Map, Value};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventRegistryConverterError {
    InvalidJson {
        entity: &'static str,
        message: String,
    },
    UnsupportedField {
        entity: &'static str,
        field: String,
    },
    UnsupportedShape {
        entity: &'static str,
        message: String,
    },
}

impl Display for EventRegistryConverterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson { entity, message } => {
                write!(f, "invalid {entity} json: {message}")
            }
            Self::UnsupportedField { entity, field } => {
                write!(f, "unsupported {entity} json field `{field}`")
            }
            Self::UnsupportedShape { entity, message } => {
                write!(f, "unsupported {entity} json shape: {message}")
            }
        }
    }
}

impl Error for EventRegistryConverterError {}

pub fn parse_channel_definition(
    input: &str,
) -> Result<ChannelDefinition, EventRegistryConverterError> {
    let value = parse_json(input, "channel definition")?;
    validate_channel_definition_value(&value)?;
    serde_json::from_value(value).map_err(|error| EventRegistryConverterError::InvalidJson {
        entity: "channel definition",
        message: error.to_string(),
    })
}

pub fn parse_event_definition(input: &str) -> Result<EventDefinition, EventRegistryConverterError> {
    let value = parse_json(input, "event definition")?;
    validate_event_definition_value(&value)?;
    serde_json::from_value(value).map_err(|error| EventRegistryConverterError::InvalidJson {
        entity: "event definition",
        message: error.to_string(),
    })
}

pub fn channel_definition_to_json(
    definition: &ChannelDefinition,
) -> Result<String, EventRegistryConverterError> {
    serde_json::to_string(definition).map_err(|error| EventRegistryConverterError::InvalidJson {
        entity: "channel definition",
        message: error.to_string(),
    })
}

pub fn event_definition_to_json(
    definition: &EventDefinition,
) -> Result<String, EventRegistryConverterError> {
    serde_json::to_string(definition).map_err(|error| EventRegistryConverterError::InvalidJson {
        entity: "event definition",
        message: error.to_string(),
    })
}

fn parse_json(input: &str, entity: &'static str) -> Result<Value, EventRegistryConverterError> {
    serde_json::from_str(input).map_err(|error| EventRegistryConverterError::InvalidJson {
        entity,
        message: error.to_string(),
    })
}

fn validate_channel_definition_value(value: &Value) -> Result<(), EventRegistryConverterError> {
    let object = expect_object(value, "channel definition")?;
    reject_unknown_fields(
        object,
        &[
            "id",
            "key",
            "name",
            "description",
            "channelType",
            "resourceName",
            "configuration",
        ],
        "channel definition",
    )?;

    match object.get("configuration") {
        Some(Value::Object(_)) => Ok(()),
        Some(_) => Err(EventRegistryConverterError::UnsupportedShape {
            entity: "channel definition",
            message: "configuration must be a JSON object".to_string(),
        }),
        None => Err(EventRegistryConverterError::UnsupportedShape {
            entity: "channel definition",
            message: "missing required field `configuration`".to_string(),
        }),
    }
}

fn validate_event_definition_value(value: &Value) -> Result<(), EventRegistryConverterError> {
    let object = expect_object(value, "event definition")?;
    reject_unknown_fields(
        object,
        &[
            "id",
            "key",
            "name",
            "description",
            "eventType",
            "channelKey",
            "payload",
            "resourceName",
        ],
        "event definition",
    )?;

    match object.get("payload") {
        Some(Value::Array(entries)) => {
            for entry in entries {
                let payload = expect_object(entry, "event payload entry")?;
                reject_unknown_fields(
                    payload,
                    &["name", "type", "required"],
                    "event payload entry",
                )?;
            }
            Ok(())
        }
        Some(_) => Err(EventRegistryConverterError::UnsupportedShape {
            entity: "event definition",
            message: "payload must be an array".to_string(),
        }),
        None => Err(EventRegistryConverterError::UnsupportedShape {
            entity: "event definition",
            message: "missing required field `payload`".to_string(),
        }),
    }
}

fn expect_object<'a>(
    value: &'a Value,
    entity: &'static str,
) -> Result<&'a Map<String, Value>, EventRegistryConverterError> {
    value
        .as_object()
        .ok_or_else(|| EventRegistryConverterError::UnsupportedShape {
            entity,
            message: "top-level JSON value must be an object".to_string(),
        })
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed_fields: &[&str],
    entity: &'static str,
) -> Result<(), EventRegistryConverterError> {
    for field in object.keys() {
        if !allowed_fields.iter().any(|allowed| allowed == field) {
            return Err(EventRegistryConverterError::UnsupportedField {
                entity,
                field: field.clone(),
            });
        }
    }

    Ok(())
}
