use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use flowable_content_service::{ContentItem, repository as content_repository};
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::db_session::DbSession;
use serde_json::{Number, Value, json};

use crate::field_types;
use crate::models::FormProperty;

#[cfg(test)]
use crate::models::FormEnumValue;

/// Typed submit context for form field lifecycle hooks.
///
/// Carries scope metadata and the command session so handlers can associate
/// content / side effects transactionally. No global service locator.
pub struct FormFieldSubmitContext<'a> {
    pub task_id: Option<&'a str>,
    pub process_instance_id: Option<&'a str>,
    pub scope_id: Option<&'a str>,
    pub scope_type: Option<&'a str>,
    pub scope_definition_id: Option<&'a str>,
    pub tenant_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub session: &'a mut DbSession,
}

/// Typed enrichment context for form field read hooks.
///
/// Content items are resolved by the service and passed in; handlers must not
/// open their own engine sessions.
pub struct FormFieldEnrichContext<'a> {
    pub content_by_id: &'a BTreeMap<String, ContentItem>,
}

/// 表单字段处理器 trait — 扩展点，按 supported_type 分发到对应实现
pub trait FormFieldHandler: Send + Sync {
    /// 返回此 handler 支持处理的字段类型（对应 FormProperty.field_type）
    fn supported_type(&self) -> &str;

    /// 校验字段值
    fn validate(&self, field: &FormProperty, value: &Value) -> Result<(), FlowableError>;

    /// 类型转换/强制
    fn coerce(&self, field: &FormProperty, value: Value) -> Result<Value, FlowableError>;

    /// 返回前端渲染所需的元数据
    fn render_metadata(&self, field: &FormProperty) -> Value;

    /// Submit lifecycle hook (Java `FormFieldHandler.handleFormFieldsOnSubmit` slice).
    ///
    /// Default is a no-op so existing custom handlers keep compiling.
    /// Runs inside the same command session that persists the form instance
    /// (and completes the task when applicable).
    fn handle_submit(
        &self,
        _field: &FormProperty,
        _value: &Value,
        _ctx: &mut FormFieldSubmitContext<'_>,
    ) -> Result<(), FlowableError> {
        Ok(())
    }

    /// Read enrichment hook (Java `FormFieldHandler.enrichFormFields` slice).
    ///
    /// Default returns the stored value unchanged. Implementations may replace
    /// stored ids with metadata in the returned form model without mutating
    /// persisted form values.
    fn enrich_on_read(
        &self,
        _field: &FormProperty,
        value: &Value,
        _ctx: &FormFieldEnrichContext<'_>,
    ) -> Result<Value, FlowableError> {
        Ok(value.clone())
    }
}

// ============================================================
// TextFieldHandler — 处理 "string" / "text" 类型
// ============================================================

pub struct TextFieldHandler;

impl FormFieldHandler for TextFieldHandler {
    fn supported_type(&self) -> &str {
        "string"
    }

    fn validate(&self, field: &FormProperty, value: &Value) -> Result<(), FlowableError> {
        if field.required {
            match value {
                Value::String(s) if s.trim().is_empty() => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                Value::Null => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn coerce(&self, field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
        match value {
            Value::String(_) => Ok(value),
            Value::Bool(b) => Ok(Value::String(b.to_string())),
            Value::Number(n) => Ok(Value::String(n.to_string())),
            other => Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected string, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(&other)
            ))),
        }
    }

    fn render_metadata(&self, _field: &FormProperty) -> Value {
        json!({"type": "string"})
    }
}

// ============================================================
// NumberFieldHandler — 处理 "integer" / "long" / "double" / "float" / "number" / "decimal" 类型
// ============================================================

pub struct NumberFieldHandler;

impl NumberFieldHandler {
    fn is_integer_type(field_type: &str) -> bool {
        matches!(
            normalize_field_type(field_type).as_str(),
            "integer" | "long"
        )
    }
}

impl FormFieldHandler for NumberFieldHandler {
    fn supported_type(&self) -> &str {
        "number"
    }

    fn validate(&self, field: &FormProperty, value: &Value) -> Result<(), FlowableError> {
        if field.required {
            match value {
                Value::String(s) if s.trim().is_empty() => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                Value::Null => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                _ => {}
            }
        }

        // 验证数值格式
        match value {
            Value::Number(_) => Ok(()),
            Value::String(s) => {
                let trimmed = s.trim();
                if Self::is_integer_type(&field.field_type) {
                    trimmed.parse::<i64>().map_err(|_| {
                        FlowableError::DeploymentValidationError(format!(
                            "Invalid value for field '{}': expected integer",
                            field.name.as_deref().unwrap_or(&field.id)
                        ))
                    })?;
                } else {
                    trimmed.parse::<f64>().map_err(|_| {
                        FlowableError::DeploymentValidationError(format!(
                            "Invalid value for field '{}': expected number",
                            field.name.as_deref().unwrap_or(&field.id)
                        ))
                    })?;
                }
                Ok(())
            }
            other => Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected number, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(other)
            ))),
        }
    }

    fn coerce(&self, field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
        if Self::is_integer_type(&field.field_type) {
            coerce_i64(field, value)
        } else {
            coerce_f64(field, value)
        }
    }

    fn render_metadata(&self, _field: &FormProperty) -> Value {
        json!({"type": "number"})
    }
}

// ============================================================
// DateFieldHandler — 处理 "date" 类型
// ============================================================

pub struct DateFieldHandler;

impl FormFieldHandler for DateFieldHandler {
    fn supported_type(&self) -> &str {
        "date"
    }

    fn validate(&self, field: &FormProperty, value: &Value) -> Result<(), FlowableError> {
        if field.required {
            match value {
                Value::String(s) if s.trim().is_empty() => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                Value::Null => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                _ => {}
            }
        }

        // 验证日期格式：必须为字符串
        match value {
            Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    return Ok(()); // 空字符串允许（非 required 时）
                }
                // 简单日期格式校验：YYYY-MM-DD 或 ISO 8601 格式
                if trimmed.len() >= 10
                    && trimmed.chars().nth(4) == Some('-')
                    && trimmed.chars().nth(7) == Some('-')
                {
                    Ok(())
                } else if trimmed.contains('T') {
                    // ISO 8601 datetime
                    Ok(())
                } else {
                    Err(FlowableError::DeploymentValidationError(format!(
                        "Invalid value for field '{}': expected date string (YYYY-MM-DD or ISO 8601)",
                        field.name.as_deref().unwrap_or(&field.id)
                    )))
                }
            }
            other => Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected date string, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(other)
            ))),
        }
    }

    fn coerce(&self, field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
        match value {
            Value::String(_) => Ok(value),
            other => Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected date string, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(&other)
            ))),
        }
    }

    fn render_metadata(&self, _field: &FormProperty) -> Value {
        json!({"type": "date"})
    }
}

// ============================================================
// BooleanFieldHandler — 处理 "boolean" 类型
// ============================================================

pub struct BooleanFieldHandler;

impl FormFieldHandler for BooleanFieldHandler {
    fn supported_type(&self) -> &str {
        "boolean"
    }

    fn validate(&self, field: &FormProperty, value: &Value) -> Result<(), FlowableError> {
        if field.required {
            match value {
                Value::String(s) if s.trim().is_empty() => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                Value::Null => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                _ => {}
            }
        }

        // 验证布尔值格式
        match value {
            Value::Bool(_) => Ok(()),
            Value::String(s) => {
                let normalized = s.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "true" | "false" => Ok(()),
                    _ => Err(FlowableError::DeploymentValidationError(format!(
                        "Invalid value for field '{}': expected boolean",
                        field.name.as_deref().unwrap_or(&field.id)
                    ))),
                }
            }
            other => Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected boolean, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(other)
            ))),
        }
    }

    fn coerce(&self, field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
        match value {
            Value::Bool(_) => Ok(value),
            Value::String(text) => {
                let normalized = text.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    _ => Err(FlowableError::DeploymentValidationError(format!(
                        "Invalid value for field '{}': expected boolean",
                        field.name.as_deref().unwrap_or(&field.id)
                    ))),
                }
            }
            other => Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected boolean, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(&other)
            ))),
        }
    }

    fn render_metadata(&self, _field: &FormProperty) -> Value {
        json!({"type": "boolean"})
    }
}

// ============================================================
// OptionFieldHandler — 处理 "enum" / "dropdown" / "radio" 类型
// ============================================================

pub struct OptionFieldHandler;

impl OptionFieldHandler {
    fn selected_value(value: &Value) -> Option<&str> {
        match value {
            Value::String(value) => Some(value.as_str()),
            Value::Object(value) => value
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| value.get("name").and_then(Value::as_str)),
            _ => None,
        }
    }
}

impl FormFieldHandler for OptionFieldHandler {
    fn supported_type(&self) -> &str {
        "enum"
    }

    fn validate(&self, field: &FormProperty, value: &Value) -> Result<(), FlowableError> {
        if field.required {
            match value {
                Value::String(s) if s.trim().is_empty() => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                Value::Null => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                _ => {}
            }
        }

        // 验证值在允许的选项列表中
        match Self::selected_value(value) {
            Some(selected) => {
                let trimmed = selected.trim();
                if trimmed.is_empty() {
                    return Ok(()); // 空字符串允许（非 required 时）
                }
                if field.enum_values.is_empty() {
                    return Ok(()); // 无选项列表时不校验
                }
                let valid = field
                    .enum_values
                    .iter()
                    .any(|opt| opt.id == trimmed || opt.name == trimmed);
                if valid {
                    Ok(())
                } else {
                    Err(FlowableError::DeploymentValidationError(format!(
                        "Invalid value for field '{}': '{}' is not in the allowed options",
                        field.name.as_deref().unwrap_or(&field.id),
                        trimmed
                    )))
                }
            }
            None => Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected option string or object with an id/name, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(value)
            ))),
        }
    }

    fn coerce(&self, field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
        Self::selected_value(&value)
            .map(|selected| Value::String(selected.to_string()))
            .ok_or_else(|| {
                FlowableError::DeploymentValidationError(format!(
                    "Invalid value for field '{}': expected option string or object with an id/name, got {}",
                    field.name.as_deref().unwrap_or(&field.id),
                    json_type_name(&value)
                ))
            })
    }

    fn render_metadata(&self, field: &FormProperty) -> Value {
        let options: Vec<Value> = field
            .enum_values
            .iter()
            .map(|opt| {
                json!({
                    "id": opt.id,
                    "name": opt.name
                })
            })
            .collect();
        json!({
            "type": "option",
            "options": options
        })
    }
}

// ============================================================
// IdentityFieldHandler — Flowable 6.8 "people" / "functional-group"
// ============================================================

/// Coerces the Java wire representation (`{"id": ...}`) to the identity id
/// stored in the process variable. A bare string id is accepted for REST
/// clients that already resolved the identity picker selection.
pub struct IdentityFieldHandler;

impl IdentityFieldHandler {
    fn identity_id(value: &Value) -> Option<&str> {
        match value {
            Value::String(value) => Some(value.as_str()),
            Value::Object(value) => value.get("id").and_then(Value::as_str),
            _ => None,
        }
        .map(str::trim)
        .filter(|value| !value.is_empty())
    }
}

impl FormFieldHandler for IdentityFieldHandler {
    fn supported_type(&self) -> &str {
        field_types::PEOPLE
    }

    fn validate(&self, field: &FormProperty, value: &Value) -> Result<(), FlowableError> {
        if Self::identity_id(value).is_some() {
            return Ok(());
        }
        if !field.required && matches!(value, Value::Null) {
            return Ok(());
        }
        Err(FlowableError::DeploymentValidationError(format!(
            "Invalid value for field '{}': expected an identity id or object with a non-empty id",
            field.name.as_deref().unwrap_or(&field.id)
        )))
    }

    fn coerce(&self, field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
        if !field.required && matches!(value, Value::Null) {
            return Ok(Value::Null);
        }
        Self::identity_id(&value)
            .map(|id| Value::String(id.to_string()))
            .ok_or_else(|| {
                FlowableError::DeploymentValidationError(format!(
                    "Invalid value for field '{}': expected an identity id or object with a non-empty id",
                    field.name.as_deref().unwrap_or(&field.id)
                ))
            })
    }

    fn render_metadata(&self, field: &FormProperty) -> Value {
        json!({ "type": normalize_field_type(&field.field_type) })
    }
}

// ============================================================
// UploadFieldHandler — 处理 "upload" 类型（Java DefaultFormFieldHandler）
// ============================================================

/// Default upload field handler.
///
/// Submit: parse string or list of content item ids, trim/dedupe, require every
/// item exists, reject cross-tenant ownership, associate task/process/scope/
/// field/tenant on the command session.
///
/// Enrich: replace stored ids with content metadata in the returned form model
/// without changing persisted form values.
pub struct UploadFieldHandler;

impl UploadFieldHandler {
    /// Parse upload value into ordered unique content item ids.
    pub fn parse_content_item_ids(value: &Value) -> Result<Vec<String>, FlowableError> {
        let mut ids = Vec::new();
        let mut seen = BTreeSet::new();

        match value {
            Value::Null => {}
            Value::String(text) => {
                for part in text.split(',') {
                    let trimmed = part.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if seen.insert(trimmed.to_string()) {
                        ids.push(trimmed.to_string());
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    match item {
                        Value::String(s) => {
                            let trimmed = s.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            if seen.insert(trimmed.to_string()) {
                                ids.push(trimmed.to_string());
                            }
                        }
                        other => {
                            return Err(FlowableError::DeploymentValidationError(format!(
                                "Upload field value array items must be strings, got {}",
                                json_type_name(other)
                            )));
                        }
                    }
                }
            }
            other => {
                return Err(FlowableError::DeploymentValidationError(format!(
                    "Upload field value must be a string or array of strings, got {}",
                    json_type_name(other)
                )));
            }
        }
        Ok(ids)
    }
}

impl FormFieldHandler for UploadFieldHandler {
    fn supported_type(&self) -> &str {
        "upload"
    }

    fn validate(&self, field: &FormProperty, value: &Value) -> Result<(), FlowableError> {
        if field.required {
            match value {
                Value::Null => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                Value::String(s) if s.trim().is_empty() => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                Value::Array(items) if items.is_empty() => {
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "Field '{}' is required",
                        field.name.as_deref().unwrap_or(&field.id)
                    )));
                }
                _ => {}
            }
        }
        // Format check (empty allowed for non-required).
        let _ = Self::parse_content_item_ids(value)?;
        Ok(())
    }

    fn coerce(&self, field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
        // Normalize to a comma-joined string of unique trimmed ids (Java stores
        // upload variables as a comma-separated string of content item ids).
        let ids = Self::parse_content_item_ids(&value).map_err(|err| {
            FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': {}",
                field.name.as_deref().unwrap_or(&field.id),
                err
            ))
        })?;
        Ok(Value::String(ids.join(",")))
    }

    fn render_metadata(&self, _field: &FormProperty) -> Value {
        json!({"type": "upload"})
    }

    fn handle_submit(
        &self,
        field: &FormProperty,
        value: &Value,
        ctx: &mut FormFieldSubmitContext<'_>,
    ) -> Result<(), FlowableError> {
        let ids = Self::parse_content_item_ids(value)?;
        if ids.is_empty() {
            return Ok(());
        }

        for id in &ids {
            // Strict, symmetric ownership check + guarded claim (P1 tenant fix):
            // form submits may only claim same-tenant, unowned content items.
            content_repository::claim_content_item_for_field_in_session(
                ctx.session,
                id,
                ctx.task_id,
                ctx.process_instance_id,
                ctx.scope_id,
                ctx.scope_type,
                Some(field.id.as_str()),
                ctx.tenant_id,
            )
            .map_err(|e| match e {
                content_repository::ContentClaimError::NotFound => FlowableError::NotFound(format!(
                    "Content item '{id}' referenced by upload field '{}' was not found",
                    field.id
                )),
                content_repository::ContentClaimError::TenantMismatch { item_tenant } => {
                    FlowableError::BadRequest(format!(
                        "Content item '{id}' belongs to tenant '{}', cannot associate with tenant '{}'",
                        item_tenant.unwrap_or_default(),
                        ctx.tenant_id.unwrap_or_default()
                    ))
                }
                content_repository::ContentClaimError::AlreadyAssociated => {
                    FlowableError::Conflict(format!(
                        "Content item '{id}' referenced by upload field '{}' is already associated with another task, process or scope",
                        field.id
                    ))
                }
                content_repository::ContentClaimError::ConcurrentClaim => {
                    FlowableError::Conflict(format!(
                        "Content item '{id}' referenced by upload field '{}' was claimed by a concurrent submission",
                        field.id
                    ))
                }
                content_repository::ContentClaimError::Storage(err) => {
                    FlowableError::Internal(format!("Database error: {err}"))
                }
            })?;
        }
        Ok(())
    }

    fn enrich_on_read(
        &self,
        _field: &FormProperty,
        value: &Value,
        ctx: &FormFieldEnrichContext<'_>,
    ) -> Result<Value, FlowableError> {
        let ids = Self::parse_content_item_ids(value)?;
        if ids.is_empty() {
            return Ok(value.clone());
        }
        let mut items = Vec::new();
        for id in ids {
            if let Some(item) = ctx.content_by_id.get(&id) {
                items.push(serde_json::to_value(item).unwrap_or(Value::Null));
            }
        }
        Ok(Value::Array(items))
    }
}

// ============================================================
// 默认 handler 集合
// ============================================================

/// 返回默认的 handler 集合，覆盖当前 `coerce_property_value()` 支持的所有类型。
pub fn default_handlers() -> BTreeMap<String, Arc<dyn FormFieldHandler>> {
    let mut map: BTreeMap<String, Arc<dyn FormFieldHandler>> = BTreeMap::new();

    let text_handler: Arc<dyn FormFieldHandler> = Arc::new(TextFieldHandler);
    for field_type in &[
        "string",
        field_types::SINGLE_LINE_TEXT,
        field_types::MULTI_LINE_TEXT,
    ] {
        map.insert((*field_type).to_string(), text_handler.clone());
    }

    let number_handler: Arc<dyn FormFieldHandler> = Arc::new(NumberFieldHandler);
    for t in &[
        field_types::INTEGER,
        "long",
        "double",
        "float",
        "number",
        field_types::DECIMAL,
        field_types::AMOUNT,
    ] {
        map.insert(t.to_string(), number_handler.clone());
    }

    let date_handler: Arc<dyn FormFieldHandler> = Arc::new(DateFieldHandler);
    map.insert("date".to_string(), date_handler);

    let bool_handler: Arc<dyn FormFieldHandler> = Arc::new(BooleanFieldHandler);
    map.insert("boolean".to_string(), bool_handler);

    let option_handler: Arc<dyn FormFieldHandler> = Arc::new(OptionFieldHandler);
    for t in &[
        "enum",
        field_types::DROPDOWN,
        "radio",
        field_types::RADIO_BUTTONS,
    ] {
        map.insert(t.to_string(), option_handler.clone());
    }

    let upload_handler: Arc<dyn FormFieldHandler> = Arc::new(UploadFieldHandler);
    map.insert("upload".to_string(), upload_handler);

    let identity_handler: Arc<dyn FormFieldHandler> = Arc::new(IdentityFieldHandler);
    map.insert(field_types::PEOPLE.to_string(), identity_handler.clone());
    map.insert(field_types::FUNCTIONAL_GROUP.to_string(), identity_handler);

    map
}

// ============================================================
// 内部辅助函数
// ============================================================

fn normalize_field_type(field_type: &str) -> String {
    field_type.trim().to_ascii_lowercase()
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn coerce_i64(field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
    let number = match value {
        Value::Number(number) => number.as_i64().ok_or_else(|| {
            FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected integer",
                field.name.as_deref().unwrap_or(&field.id)
            ))
        })?,
        Value::String(text) => text.trim().parse::<i64>().map_err(|_| {
            FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected integer",
                field.name.as_deref().unwrap_or(&field.id)
            ))
        })?,
        other => {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected integer, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(&other)
            )));
        }
    };

    Ok(Value::Number(Number::from(number)))
}

fn coerce_f64(field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
    let number = match value {
        Value::Number(number) => number.as_f64().ok_or_else(|| {
            FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected number",
                field.name.as_deref().unwrap_or(&field.id)
            ))
        })?,
        Value::String(text) => text.trim().parse::<f64>().map_err(|_| {
            FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected number",
                field.name.as_deref().unwrap_or(&field.id)
            ))
        })?,
        other => {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Invalid value for field '{}': expected number, got {}",
                field.name.as_deref().unwrap_or(&field.id),
                json_type_name(&other)
            )));
        }
    };

    let number = Number::from_f64(number).ok_or_else(|| {
        FlowableError::DeploymentValidationError(format!(
            "Invalid value for field '{}': expected finite number",
            field.name.as_deref().unwrap_or(&field.id)
        ))
    })?;
    Ok(Value::Number(number))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_field(id: &str, field_type: &str, required: bool) -> FormProperty {
        FormProperty {
            id: id.to_string(),
            name: Some(id.to_string()),
            field_type: field_type.to_string(),
            value: None,
            readable: true,
            writable: true,
            required,
            date_pattern: None,
            enum_values: vec![],
        }
    }

    fn make_field_with_options(
        id: &str,
        field_type: &str,
        required: bool,
        options: Vec<(&str, &str)>,
    ) -> FormProperty {
        FormProperty {
            id: id.to_string(),
            name: Some(id.to_string()),
            field_type: field_type.to_string(),
            value: None,
            readable: true,
            writable: true,
            required,
            date_pattern: None,
            enum_values: options
                .into_iter()
                .map(|(oid, oname)| FormEnumValue {
                    id: oid.to_string(),
                    name: oname.to_string(),
                })
                .collect(),
        }
    }

    // ============================================================
    // TextFieldHandler 测试
    // ============================================================

    #[test]
    fn test_text_handler_validate_valid_string() {
        let handler = TextFieldHandler;
        let field = make_field("name", "string", false);
        assert!(handler.validate(&field, &json!("hello")).is_ok());
    }

    #[test]
    fn test_text_handler_validate_required_empty() {
        let handler = TextFieldHandler;
        let field = make_field("name", "string", true);
        let result = handler.validate(&field, &json!(""));
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("is required"));
    }

    #[test]
    fn test_text_handler_validate_required_null() {
        let handler = TextFieldHandler;
        let field = make_field("name", "string", true);
        let result = handler.validate(&field, &Value::Null);
        assert!(result.is_err());
    }

    #[test]
    fn test_text_handler_coerce_string() {
        let handler = TextFieldHandler;
        let field = make_field("name", "string", false);
        let result = handler.coerce(&field, json!("hello")).unwrap();
        assert_eq!(result, json!("hello"));
    }

    #[test]
    fn test_text_handler_coerce_bool_to_string() {
        let handler = TextFieldHandler;
        let field = make_field("name", "string", false);
        let result = handler.coerce(&field, json!(true)).unwrap();
        assert_eq!(result, json!("true"));
    }

    #[test]
    fn test_text_handler_coerce_number_to_string() {
        let handler = TextFieldHandler;
        let field = make_field("name", "string", false);
        let result = handler.coerce(&field, json!(42)).unwrap();
        assert_eq!(result, json!("42"));
    }

    #[test]
    fn test_text_handler_render_metadata() {
        let handler = TextFieldHandler;
        let field = make_field("name", "string", false);
        let meta = handler.render_metadata(&field);
        assert_eq!(meta, json!({"type": "string"}));
    }

    // ============================================================
    // NumberFieldHandler 测试
    // ============================================================

    #[test]
    fn test_number_handler_validate_integer() {
        let handler = NumberFieldHandler;
        let field = make_field("amount", "integer", false);
        assert!(handler.validate(&field, &json!(100)).is_ok());
        assert!(handler.validate(&field, &json!("200")).is_ok());
    }

    #[test]
    fn test_number_handler_validate_integer_rejects_float_string() {
        let handler = NumberFieldHandler;
        let field = make_field("amount", "integer", false);
        let result = handler.validate(&field, &json!("3.14"));
        assert!(result.is_err());
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_number_handler_validate_double() {
        let handler = NumberFieldHandler;
        let field = make_field("price", "double", false);
        assert!(handler.validate(&field, &json!(3.14)).is_ok());
        assert!(handler.validate(&field, &json!("2.718")).is_ok());
    }

    #[test]
    fn test_number_handler_validate_required_empty() {
        let handler = NumberFieldHandler;
        let field = make_field("amount", "integer", true);
        let result = handler.validate(&field, &json!(""));
        assert!(result.is_err());
    }

    #[test]
    fn test_number_handler_coerce_integer() {
        let handler = NumberFieldHandler;
        let field = make_field("amount", "integer", false);
        let result = handler.coerce(&field, json!(42)).unwrap();
        assert_eq!(result, json!(42));
    }

    #[test]
    fn test_number_handler_coerce_integer_from_string() {
        let handler = NumberFieldHandler;
        let field = make_field("amount", "long", false);
        let result = handler.coerce(&field, json!("99")).unwrap();
        assert_eq!(result, json!(99));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_number_handler_coerce_double() {
        let handler = NumberFieldHandler;
        let field = make_field("price", "double", false);
        let result = handler.coerce(&field, json!(3.14)).unwrap();
        assert_eq!(result, json!(3.14));
    }

    #[test]
    fn test_number_handler_coerce_double_from_string() {
        let handler = NumberFieldHandler;
        let field = make_field("price", "float", false);
        let result = handler.coerce(&field, json!("2.5")).unwrap();
        assert_eq!(result, json!(2.5));
    }

    #[test]
    fn test_number_handler_render_metadata() {
        let handler = NumberFieldHandler;
        let field = make_field("amount", "integer", false);
        let meta = handler.render_metadata(&field);
        assert_eq!(meta, json!({"type": "number"}));
    }

    // ============================================================
    // DateFieldHandler 测试
    // ============================================================

    #[test]
    fn test_date_handler_validate_valid_date() {
        let handler = DateFieldHandler;
        let field = make_field("startDate", "date", false);
        assert!(handler.validate(&field, &json!("2026-04-26")).is_ok());
        assert!(
            handler
                .validate(&field, &json!("2026-04-26T10:00:00Z"))
                .is_ok()
        );
    }

    #[test]
    fn test_date_handler_validate_invalid_format() {
        let handler = DateFieldHandler;
        let field = make_field("startDate", "date", false);
        let result = handler.validate(&field, &json!("not-a-date"));
        assert!(result.is_err());
    }

    #[test]
    fn test_date_handler_validate_required_empty() {
        let handler = DateFieldHandler;
        let field = make_field("startDate", "date", true);
        let result = handler.validate(&field, &json!(""));
        assert!(result.is_err());
    }

    #[test]
    fn test_date_handler_coerce_preserves_string() {
        let handler = DateFieldHandler;
        let field = make_field("startDate", "date", false);
        let result = handler.coerce(&field, json!("2026-04-26")).unwrap();
        assert_eq!(result, json!("2026-04-26"));
    }

    #[test]
    fn test_date_handler_coerce_rejects_non_string() {
        let handler = DateFieldHandler;
        let field = make_field("startDate", "date", false);
        let result = handler.coerce(&field, json!(123));
        assert!(result.is_err());
    }

    #[test]
    fn test_date_handler_render_metadata() {
        let handler = DateFieldHandler;
        let field = make_field("startDate", "date", false);
        let meta = handler.render_metadata(&field);
        assert_eq!(meta, json!({"type": "date"}));
    }

    // ============================================================
    // BooleanFieldHandler 测试
    // ============================================================

    #[test]
    fn test_boolean_handler_validate_true() {
        let handler = BooleanFieldHandler;
        let field = make_field("approved", "boolean", false);
        assert!(handler.validate(&field, &json!(true)).is_ok());
        assert!(handler.validate(&field, &json!("true")).is_ok());
    }

    #[test]
    fn test_boolean_handler_validate_false() {
        let handler = BooleanFieldHandler;
        let field = make_field("approved", "boolean", false);
        assert!(handler.validate(&field, &json!(false)).is_ok());
        assert!(handler.validate(&field, &json!("false")).is_ok());
    }

    #[test]
    fn test_boolean_handler_validate_invalid() {
        let handler = BooleanFieldHandler;
        let field = make_field("approved", "boolean", false);
        let result = handler.validate(&field, &json!("yes"));
        assert!(result.is_err());
    }

    #[test]
    fn test_boolean_handler_validate_required_empty() {
        let handler = BooleanFieldHandler;
        let field = make_field("approved", "boolean", true);
        let result = handler.validate(&field, &json!(""));
        assert!(result.is_err());
    }

    #[test]
    fn test_boolean_handler_coerce_string_true() {
        let handler = BooleanFieldHandler;
        let field = make_field("approved", "boolean", false);
        let result = handler.coerce(&field, json!("true")).unwrap();
        assert_eq!(result, json!(true));
    }

    #[test]
    fn test_boolean_handler_coerce_string_false() {
        let handler = BooleanFieldHandler;
        let field = make_field("approved", "boolean", false);
        let result = handler.coerce(&field, json!("false")).unwrap();
        assert_eq!(result, json!(false));
    }

    #[test]
    fn test_boolean_handler_coerce_case_insensitive() {
        let handler = BooleanFieldHandler;
        let field = make_field("approved", "boolean", false);
        let result = handler.coerce(&field, json!("TRUE")).unwrap();
        assert_eq!(result, json!(true));
    }

    #[test]
    fn test_boolean_handler_render_metadata() {
        let handler = BooleanFieldHandler;
        let field = make_field("approved", "boolean", false);
        let meta = handler.render_metadata(&field);
        assert_eq!(meta, json!({"type": "boolean"}));
    }

    // ============================================================
    // OptionFieldHandler 测试
    // ============================================================

    #[test]
    fn test_option_handler_validate_valid_option() {
        let handler = OptionFieldHandler;
        let field = make_field_with_options(
            "color",
            "enum",
            false,
            vec![("red", "Red"), ("blue", "Blue"), ("green", "Green")],
        );
        assert!(handler.validate(&field, &json!("red")).is_ok());
        assert!(handler.validate(&field, &json!("Blue")).is_ok()); // by name
    }

    #[test]
    fn test_option_handler_validate_invalid_option() {
        let handler = OptionFieldHandler;
        let field = make_field_with_options(
            "color",
            "dropdown",
            false,
            vec![("red", "Red"), ("blue", "Blue")],
        );
        let result = handler.validate(&field, &json!("yellow"));
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("not in the allowed options"));
    }

    #[test]
    fn test_option_handler_validate_empty_options_allows_any() {
        let handler = OptionFieldHandler;
        let field = make_field("color", "radio", false);
        // 无选项列表时允许任意值
        assert!(handler.validate(&field, &json!("anything")).is_ok());
    }

    #[test]
    fn test_option_handler_validate_required_empty() {
        let handler = OptionFieldHandler;
        let field = make_field_with_options("color", "enum", true, vec![("red", "Red")]);
        let result = handler.validate(&field, &json!(""));
        assert!(result.is_err());
    }

    #[test]
    fn test_option_handler_coerce_preserves_string() {
        let handler = OptionFieldHandler;
        let field = make_field_with_options("color", "enum", false, vec![("red", "Red")]);
        let result = handler.coerce(&field, json!("red")).unwrap();
        assert_eq!(result, json!("red"));
    }

    #[test]
    fn test_option_handler_render_metadata() {
        let handler = OptionFieldHandler;
        let field = make_field_with_options(
            "color",
            "dropdown",
            false,
            vec![("red", "Red"), ("blue", "Blue")],
        );
        let meta = handler.render_metadata(&field);
        assert_eq!(
            meta,
            json!({
                "type": "option",
                "options": [
                    {"id": "red", "name": "Red"},
                    {"id": "blue", "name": "Blue"}
                ]
            })
        );
    }

    // ============================================================
    // default_handlers 测试
    // ============================================================

    #[test]
    fn test_default_handlers_covers_all_known_types() {
        let handlers = default_handlers();
        assert!(handlers.contains_key("string"));
        assert!(handlers.contains_key("text"));
        assert!(handlers.contains_key("integer"));
        assert!(handlers.contains_key("long"));
        assert!(handlers.contains_key("double"));
        assert!(handlers.contains_key("float"));
        assert!(handlers.contains_key("number"));
        assert!(handlers.contains_key("decimal"));
        assert!(handlers.contains_key("date"));
        assert!(handlers.contains_key("boolean"));
        assert!(handlers.contains_key("enum"));
        assert!(handlers.contains_key("dropdown"));
        assert!(handlers.contains_key("radio"));
        assert!(handlers.contains_key("upload"));
    }

    #[test]
    fn test_default_handlers_unknown_type_missing() {
        let handlers = default_handlers();
        assert!(!handlers.contains_key("file"));
        assert!(!handlers.contains_key("custom_type"));
        assert!(!handlers.contains_key("custom_widget"));
    }

    #[test]
    fn test_supported_type_values() {
        assert_eq!(TextFieldHandler.supported_type(), "string");
        assert_eq!(NumberFieldHandler.supported_type(), "number");
        assert_eq!(DateFieldHandler.supported_type(), "date");
        assert_eq!(BooleanFieldHandler.supported_type(), "boolean");
        assert_eq!(OptionFieldHandler.supported_type(), "enum");
        assert_eq!(UploadFieldHandler.supported_type(), "upload");
    }
}
