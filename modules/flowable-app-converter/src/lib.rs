use flowable_app_model::{AppDefinition, AppPage, AppReferenceType, AppResourceReference};
use serde_json::{Map, Value};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppConverterError {
    InvalidJson {
        entity: &'static str,
        message: String,
    },
    MissingField {
        entity: &'static str,
        field: &'static str,
    },
    UnsupportedField {
        entity: &'static str,
        field: String,
    },
    UnsupportedValue {
        entity: &'static str,
        field: &'static str,
        value: String,
    },
    UnsupportedShape {
        entity: &'static str,
        message: String,
    },
}

impl Display for AppConverterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson { entity, message } => {
                write!(f, "invalid {entity} json: {message}")
            }
            Self::MissingField { entity, field } => {
                write!(f, "missing required {entity} field `{field}`")
            }
            Self::UnsupportedField { entity, field } => {
                write!(f, "unsupported {entity} json field `{field}`")
            }
            Self::UnsupportedValue {
                entity,
                field,
                value,
            } => {
                write!(
                    f,
                    "unsupported {entity} value `{value}` for field `{field}`"
                )
            }
            Self::UnsupportedShape { entity, message } => {
                write!(f, "unsupported {entity} json shape: {message}")
            }
        }
    }
}

impl Error for AppConverterError {}

pub fn parse_app_definition(input: &str) -> Result<AppDefinition, AppConverterError> {
    let value = parse_json(input, "app definition")?;
    validate_app_definition_value(&value)?;
    serde_json::from_value(value).map_err(|error| AppConverterError::InvalidJson {
        entity: "app definition",
        message: error.to_string(),
    })
}

pub fn parse_app_page(input: &str) -> Result<AppPage, AppConverterError> {
    let value = parse_json(input, "app page")?;
    validate_app_page_value(&value)?;
    serde_json::from_value(value).map_err(|error| AppConverterError::InvalidJson {
        entity: "app page",
        message: error.to_string(),
    })
}

pub fn parse_app_resource_reference(
    input: &str,
) -> Result<AppResourceReference, AppConverterError> {
    let value = parse_json(input, "app resource reference")?;
    validate_app_resource_reference_value(&value)?;
    serde_json::from_value(value).map_err(|error| AppConverterError::InvalidJson {
        entity: "app resource reference",
        message: error.to_string(),
    })
}

pub fn app_definition_to_json(definition: &AppDefinition) -> Result<String, AppConverterError> {
    serde_json::to_string(definition).map_err(|error| AppConverterError::InvalidJson {
        entity: "app definition",
        message: error.to_string(),
    })
}

pub fn app_page_to_json(page: &AppPage) -> Result<String, AppConverterError> {
    serde_json::to_string(page).map_err(|error| AppConverterError::InvalidJson {
        entity: "app page",
        message: error.to_string(),
    })
}

pub fn app_resource_reference_to_json(
    reference: &AppResourceReference,
) -> Result<String, AppConverterError> {
    serde_json::to_string(reference).map_err(|error| AppConverterError::InvalidJson {
        entity: "app resource reference",
        message: error.to_string(),
    })
}

fn parse_json(input: &str, entity: &'static str) -> Result<Value, AppConverterError> {
    serde_json::from_str(input).map_err(|error| AppConverterError::InvalidJson {
        entity,
        message: error.to_string(),
    })
}

fn validate_app_definition_value(value: &Value) -> Result<(), AppConverterError> {
    let object = expect_object(value, "app definition")?;
    reject_unknown_fields(
        object,
        &[
            "id",
            "key",
            "name",
            "description",
            "category",
            "theme",
            "icon",
            "usersAccess",
            "groupsAccess",
            "landingPage",
            "pages",
            "references",
        ],
        "app definition",
    )?;

    require_string_field(object, "key", "app definition")?;
    validate_optional_string_field(object, "id", "app definition")?;
    validate_optional_string_field(object, "name", "app definition")?;
    validate_optional_string_field(object, "description", "app definition")?;
    validate_optional_string_field(object, "category", "app definition")?;
    validate_optional_string_field(object, "theme", "app definition")?;
    validate_optional_string_field(object, "icon", "app definition")?;
    validate_optional_string_field(object, "usersAccess", "app definition")?;
    validate_optional_string_field(object, "groupsAccess", "app definition")?;
    validate_optional_string_field(object, "landingPage", "app definition")?;

    match object.get("pages") {
        Some(Value::Array(entries)) => {
            for entry in entries {
                validate_app_page_value(entry)?;
            }
        }
        Some(_) => {
            return Err(AppConverterError::UnsupportedShape {
                entity: "app definition",
                message: "field `pages` must be an array".to_string(),
            });
        }
        None => {}
    }

    match object.get("references") {
        Some(Value::Array(entries)) => {
            for entry in entries {
                validate_app_resource_reference_value(entry)?;
            }
        }
        Some(_) => {
            return Err(AppConverterError::UnsupportedShape {
                entity: "app definition",
                message: "field `references` must be an array".to_string(),
            });
        }
        None => {}
    }

    Ok(())
}

fn validate_app_page_value(value: &Value) -> Result<(), AppConverterError> {
    let object = expect_object(value, "app page")?;
    reject_unknown_fields(
        object,
        &[
            "id",
            "name",
            "description",
            "pageType",
            "definitionKey",
            "icon",
            "order",
        ],
        "app page",
    )?;

    require_string_field(object, "id", "app page")?;
    validate_optional_string_field(object, "name", "app page")?;
    validate_optional_string_field(object, "description", "app page")?;
    let page_type = require_string_field(object, "pageType", "app page")?;
    if !matches!(page_type, "process" | "decision" | "case" | "event") {
        return Err(AppConverterError::UnsupportedValue {
            entity: "app page",
            field: "pageType",
            value: page_type.to_string(),
        });
    }
    require_string_field(object, "definitionKey", "app page")?;
    validate_optional_string_field(object, "icon", "app page")?;
    validate_optional_i32_field(object, "order", "app page")?;

    Ok(())
}

fn validate_app_resource_reference_value(value: &Value) -> Result<(), AppConverterError> {
    let object = expect_object(value, "app resource reference")?;
    reject_unknown_fields(
        object,
        &[
            "id",
            "name",
            "description",
            "referenceType",
            "definitionKey",
            "definitionId",
            "tenantId",
        ],
        "app resource reference",
    )?;

    validate_optional_string_field(object, "id", "app resource reference")?;
    validate_optional_string_field(object, "name", "app resource reference")?;
    validate_optional_string_field(object, "description", "app resource reference")?;
    let reference_type = require_string_field(object, "referenceType", "app resource reference")?;
    match reference_type {
        "bpmn" | "dmn" | "cmmn" | "eventRegistry" => {}
        other => {
            return Err(AppConverterError::UnsupportedValue {
                entity: "app resource reference",
                field: "referenceType",
                value: other.to_string(),
            });
        }
    }
    require_string_field(object, "definitionKey", "app resource reference")?;
    validate_optional_string_field(object, "definitionId", "app resource reference")?;
    validate_optional_string_field(object, "tenantId", "app resource reference")?;

    Ok(())
}

fn expect_object<'a>(
    value: &'a Value,
    entity: &'static str,
) -> Result<&'a Map<String, Value>, AppConverterError> {
    value
        .as_object()
        .ok_or_else(|| AppConverterError::UnsupportedShape {
            entity,
            message: "top-level JSON value must be an object".to_string(),
        })
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed_fields: &[&str],
    entity: &'static str,
) -> Result<(), AppConverterError> {
    for field in object.keys() {
        if !allowed_fields.iter().any(|allowed| allowed == field) {
            return Err(AppConverterError::UnsupportedField {
                entity,
                field: field.clone(),
            });
        }
    }

    Ok(())
}

fn validate_optional_string_field(
    object: &Map<String, Value>,
    field: &'static str,
    entity: &'static str,
) -> Result<(), AppConverterError> {
    if let Some(value) = object.get(field) {
        expect_non_empty_string(value, field, entity)?;
    }

    Ok(())
}

fn validate_optional_i32_field(
    object: &Map<String, Value>,
    field: &'static str,
    entity: &'static str,
) -> Result<(), AppConverterError> {
    if let Some(value) = object.get(field) {
        match value.as_i64().and_then(|value| i32::try_from(value).ok()) {
            Some(_) => {}
            None => {
                return Err(AppConverterError::UnsupportedShape {
                    entity,
                    message: format!("field `{field}` must be a 32-bit integer"),
                });
            }
        }
    }

    Ok(())
}

fn require_string_field<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
    entity: &'static str,
) -> Result<&'a str, AppConverterError> {
    let value = object
        .get(field)
        .ok_or(AppConverterError::MissingField { entity, field })?;
    expect_non_empty_string(value, field, entity)
}

fn expect_non_empty_string<'a>(
    value: &'a Value,
    field: &'static str,
    entity: &'static str,
) -> Result<&'a str, AppConverterError> {
    let string = value
        .as_str()
        .ok_or_else(|| AppConverterError::UnsupportedShape {
            entity,
            message: format!("field `{field}` must be a non-empty string"),
        })?;
    if string.trim().is_empty() {
        return Err(AppConverterError::UnsupportedShape {
            entity,
            message: format!("field `{field}` must be a non-empty string"),
        });
    }

    Ok(string)
}

pub fn page_type_to_reference_type(page: &AppPage) -> AppReferenceType {
    page.page_type.into()
}
