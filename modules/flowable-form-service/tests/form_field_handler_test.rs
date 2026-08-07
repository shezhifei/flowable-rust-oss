use flowable_engine::error::FlowableError;
use flowable_form_service::{
    BooleanFieldHandler, DateFieldHandler, FormFieldHandler, NumberFieldHandler,
    OptionFieldHandler, TextFieldHandler, default_handlers,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

// ============================================================
// 辅助函数
// ============================================================

fn make_field(id: &str, field_type: &str, required: bool) -> flowable_form_service::FormProperty {
    flowable_form_service::FormProperty {
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
) -> flowable_form_service::FormProperty {
    flowable_form_service::FormProperty {
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
            .map(|(oid, oname)| flowable_form_service::FormEnumValue {
                id: oid.to_string(),
                name: oname.to_string(),
            })
            .collect(),
    }
}

// ============================================================
// 1. TextFieldHandler 测试
// ============================================================

#[test]
fn test_text_field_handler_validate_valid() {
    let handler = TextFieldHandler;
    let field = make_field("username", "string", false);
    assert!(handler.validate(&field, &json!("alice")).is_ok());
}

#[test]
fn test_text_field_handler_validate_required_empty_string() {
    let handler = TextFieldHandler;
    let field = make_field("username", "string", true);
    let result = handler.validate(&field, &json!(""));
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("is required"));
}

#[test]
fn test_text_field_handler_validate_required_null() {
    let handler = TextFieldHandler;
    let field = make_field("username", "string", true);
    let result = handler.validate(&field, &Value::Null);
    assert!(result.is_err());
}

#[test]
fn test_text_field_handler_coerce_string_passthrough() {
    let handler = TextFieldHandler;
    let field = make_field("username", "string", false);
    let result = handler.coerce(&field, json!("hello")).unwrap();
    assert_eq!(result, json!("hello"));
}

#[test]
fn test_text_field_handler_coerce_bool_to_string() {
    let handler = TextFieldHandler;
    let field = make_field("flag", "text", false);
    let result = handler.coerce(&field, json!(true)).unwrap();
    assert_eq!(result, json!("true"));
}

#[test]
fn test_text_field_handler_coerce_number_to_string() {
    let handler = TextFieldHandler;
    let field = make_field("count", "string", false);
    let result = handler.coerce(&field, json!(42)).unwrap();
    assert_eq!(result, json!("42"));
}

#[test]
fn test_text_field_handler_coerce_rejects_object() {
    let handler = TextFieldHandler;
    let field = make_field("data", "string", false);
    let result = handler.coerce(&field, json!({"key": "val"}));
    assert!(result.is_err());
}

#[test]
fn test_text_field_handler_render_metadata() {
    let handler = TextFieldHandler;
    let field = make_field("username", "string", false);
    let meta = handler.render_metadata(&field);
    assert_eq!(meta, json!({"type": "string"}));
}

// ============================================================
// 2. NumberFieldHandler 测试
// ============================================================

#[test]
fn test_number_field_handler_validate_integer() {
    let handler = NumberFieldHandler;
    let field = make_field("age", "integer", false);
    assert!(handler.validate(&field, &json!(25)).is_ok());
    assert!(handler.validate(&field, &json!("30")).is_ok());
}

#[test]
fn test_number_field_handler_validate_integer_rejects_float_string() {
    let handler = NumberFieldHandler;
    let field = make_field("age", "integer", false);
    let result = handler.validate(&field, &json!("3.14"));
    assert!(result.is_err());
}

#[test]
fn test_number_field_handler_validate_double() {
    let handler = NumberFieldHandler;
    let field = make_field("price", "double", false);
    assert!(handler.validate(&field, &json!(3.5)).is_ok());
    assert!(handler.validate(&field, &json!("2.718")).is_ok());
}

#[test]
fn test_number_field_handler_validate_required_empty() {
    let handler = NumberFieldHandler;
    let field = make_field("amount", "integer", true);
    let result = handler.validate(&field, &json!(""));
    assert!(result.is_err());
}

#[test]
fn test_number_field_handler_coerce_integer() {
    let handler = NumberFieldHandler;
    let field = make_field("amount", "integer", false);
    let result = handler.coerce(&field, json!(42)).unwrap();
    assert_eq!(result, json!(42));
}

#[test]
fn test_number_field_handler_coerce_integer_from_string() {
    let handler = NumberFieldHandler;
    let field = make_field("amount", "long", false);
    let result = handler.coerce(&field, json!("99")).unwrap();
    assert_eq!(result, json!(99));
}

#[test]
fn test_number_field_handler_coerce_double() {
    let handler = NumberFieldHandler;
    let field = make_field("price", "double", false);
    let result = handler.coerce(&field, json!(3.5)).unwrap();
    assert_eq!(result, json!(3.5));
}

#[test]
fn test_number_field_handler_coerce_double_from_string() {
    let handler = NumberFieldHandler;
    let field = make_field("price", "float", false);
    let result = handler.coerce(&field, json!("2.5")).unwrap();
    assert_eq!(result, json!(2.5));
}

#[test]
fn test_number_field_handler_coerce_rejects_invalid_string() {
    let handler = NumberFieldHandler;
    let field = make_field("amount", "integer", false);
    let result = handler.coerce(&field, json!("not-a-number"));
    assert!(result.is_err());
}

#[test]
fn test_number_field_handler_render_metadata() {
    let handler = NumberFieldHandler;
    let field = make_field("amount", "integer", false);
    let meta = handler.render_metadata(&field);
    assert_eq!(meta, json!({"type": "number"}));
}

// ============================================================
// 3. DateFieldHandler 测试
// ============================================================

#[test]
fn test_date_field_handler_validate_valid_date() {
    let handler = DateFieldHandler;
    let field = make_field("startDate", "date", false);
    assert!(handler.validate(&field, &json!("2026-04-26")).is_ok());
}

#[test]
fn test_date_field_handler_validate_valid_iso_datetime() {
    let handler = DateFieldHandler;
    let field = make_field("startDate", "date", false);
    assert!(
        handler
            .validate(&field, &json!("2026-04-26T10:00:00Z"))
            .is_ok()
    );
}

#[test]
fn test_date_field_handler_validate_invalid_format() {
    let handler = DateFieldHandler;
    let field = make_field("startDate", "date", false);
    let result = handler.validate(&field, &json!("not-a-date"));
    assert!(result.is_err());
}

#[test]
fn test_date_field_handler_validate_required_empty() {
    let handler = DateFieldHandler;
    let field = make_field("startDate", "date", true);
    let result = handler.validate(&field, &json!(""));
    assert!(result.is_err());
}

#[test]
fn test_date_field_handler_coerce_preserves_string() {
    let handler = DateFieldHandler;
    let field = make_field("startDate", "date", false);
    let result = handler.coerce(&field, json!("2026-04-26")).unwrap();
    assert_eq!(result, json!("2026-04-26"));
}

#[test]
fn test_date_field_handler_coerce_rejects_non_string() {
    let handler = DateFieldHandler;
    let field = make_field("startDate", "date", false);
    let result = handler.coerce(&field, json!(123));
    assert!(result.is_err());
}

#[test]
fn test_date_field_handler_render_metadata() {
    let handler = DateFieldHandler;
    let field = make_field("startDate", "date", false);
    let meta = handler.render_metadata(&field);
    assert_eq!(meta, json!({"type": "date"}));
}

// ============================================================
// 4. BooleanFieldHandler 测试
// ============================================================

#[test]
fn test_boolean_field_handler_validate_true() {
    let handler = BooleanFieldHandler;
    let field = make_field("approved", "boolean", false);
    assert!(handler.validate(&field, &json!(true)).is_ok());
    assert!(handler.validate(&field, &json!("true")).is_ok());
}

#[test]
fn test_boolean_field_handler_validate_false() {
    let handler = BooleanFieldHandler;
    let field = make_field("approved", "boolean", false);
    assert!(handler.validate(&field, &json!(false)).is_ok());
    assert!(handler.validate(&field, &json!("false")).is_ok());
}

#[test]
fn test_boolean_field_handler_validate_invalid_string() {
    let handler = BooleanFieldHandler;
    let field = make_field("approved", "boolean", false);
    let result = handler.validate(&field, &json!("yes"));
    assert!(result.is_err());
}

#[test]
fn test_boolean_field_handler_validate_required_empty() {
    let handler = BooleanFieldHandler;
    let field = make_field("approved", "boolean", true);
    let result = handler.validate(&field, &json!(""));
    assert!(result.is_err());
}

#[test]
fn test_boolean_field_handler_coerce_string_true() {
    let handler = BooleanFieldHandler;
    let field = make_field("approved", "boolean", false);
    let result = handler.coerce(&field, json!("true")).unwrap();
    assert_eq!(result, json!(true));
}

#[test]
fn test_boolean_field_handler_coerce_string_false() {
    let handler = BooleanFieldHandler;
    let field = make_field("approved", "boolean", false);
    let result = handler.coerce(&field, json!("false")).unwrap();
    assert_eq!(result, json!(false));
}

#[test]
fn test_boolean_field_handler_coerce_case_insensitive() {
    let handler = BooleanFieldHandler;
    let field = make_field("approved", "boolean", false);
    let result = handler.coerce(&field, json!("TRUE")).unwrap();
    assert_eq!(result, json!(true));
    let result = handler.coerce(&field, json!("False")).unwrap();
    assert_eq!(result, json!(false));
}

#[test]
fn test_boolean_field_handler_render_metadata() {
    let handler = BooleanFieldHandler;
    let field = make_field("approved", "boolean", false);
    let meta = handler.render_metadata(&field);
    assert_eq!(meta, json!({"type": "boolean"}));
}

// ============================================================
// 5. OptionFieldHandler 测试
// ============================================================

#[test]
fn test_option_field_handler_validate_valid_option_by_id() {
    let handler = OptionFieldHandler;
    let field = make_field_with_options(
        "color",
        "enum",
        false,
        vec![("red", "Red"), ("blue", "Blue"), ("green", "Green")],
    );
    assert!(handler.validate(&field, &json!("red")).is_ok());
}

#[test]
fn test_option_field_handler_validate_valid_option_by_name() {
    let handler = OptionFieldHandler;
    let field = make_field_with_options(
        "color",
        "dropdown",
        false,
        vec![("red", "Red"), ("blue", "Blue")],
    );
    assert!(handler.validate(&field, &json!("Blue")).is_ok());
}

#[test]
fn test_option_field_handler_validate_invalid_option() {
    let handler = OptionFieldHandler;
    let field = make_field_with_options(
        "color",
        "radio",
        false,
        vec![("red", "Red"), ("blue", "Blue")],
    );
    let result = handler.validate(&field, &json!("yellow"));
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("not in the allowed options"));
}

#[test]
fn test_option_field_handler_validate_empty_options_allows_any() {
    let handler = OptionFieldHandler;
    let field = make_field("color", "enum", false);
    assert!(handler.validate(&field, &json!("anything")).is_ok());
}

#[test]
fn test_option_field_handler_validate_required_empty() {
    let handler = OptionFieldHandler;
    let field = make_field_with_options("color", "enum", true, vec![("red", "Red")]);
    let result = handler.validate(&field, &json!(""));
    assert!(result.is_err());
}

#[test]
fn test_option_field_handler_coerce_preserves_string() {
    let handler = OptionFieldHandler;
    let field = make_field_with_options("color", "enum", false, vec![("red", "Red")]);
    let result = handler.coerce(&field, json!("red")).unwrap();
    assert_eq!(result, json!("red"));
}

#[test]
fn test_option_field_handler_render_metadata() {
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
// 6. 未注册类型返回错误
// ============================================================

#[test]
fn test_unknown_type_returns_error() {
    let handlers = default_handlers();
    // upload is a supported default type; unknown custom types stay unregistered.
    assert!(handlers.contains_key("upload"));
    assert!(!handlers.contains_key("custom_type"));
    assert!(!handlers.contains_key("custom_widget"));
}

#[test]
fn test_default_handlers_covers_all_known_types() {
    let handlers = default_handlers();
    let expected_types = [
        "string", "text", "integer", "long", "double", "float", "number", "decimal", "date",
        "boolean", "enum", "dropdown", "radio", "upload",
    ];
    for t in &expected_types {
        assert!(
            handlers.contains_key(*t),
            "Expected handler for type '{}'",
            t
        );
    }
}

// ============================================================
// 7. 自定义 handler 注册和调度
// ============================================================

/// 自定义 handler 用于测试
struct CustomUploadHandler;

impl FormFieldHandler for CustomUploadHandler {
    fn supported_type(&self) -> &str {
        "upload"
    }

    fn validate(
        &self,
        field: &flowable_form_service::FormProperty,
        _value: &Value,
    ) -> Result<(), FlowableError> {
        if field.required {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Field '{}' is required",
                field.name.as_deref().unwrap_or(&field.id)
            )));
        }
        Ok(())
    }

    fn coerce(
        &self,
        _field: &flowable_form_service::FormProperty,
        value: Value,
    ) -> Result<Value, FlowableError> {
        Ok(value)
    }

    fn render_metadata(&self, _field: &flowable_form_service::FormProperty) -> Value {
        json!({"type": "upload"})
    }
}

#[test]
fn test_custom_handler_registration() {
    let mut custom_handlers: BTreeMap<String, Arc<dyn FormFieldHandler>> = BTreeMap::new();
    custom_handlers.insert("upload".to_string(), Arc::new(CustomUploadHandler));

    // 合并默认 handler 和自定义 handler
    let mut handlers = default_handlers();
    handlers.extend(custom_handlers);

    assert!(handlers.contains_key("upload"));
    assert!(handlers.contains_key("string")); // 默认 handler 仍然存在
}

#[test]
fn test_custom_handler_dispatch() {
    let handler = CustomUploadHandler;
    let field = make_field("attachment", "upload", false);

    // validate
    assert!(handler.validate(&field, &json!("file.pdf")).is_ok());

    // coerce
    let result = handler.coerce(&field, json!("file.pdf")).unwrap();
    assert_eq!(result, json!("file.pdf"));

    // render_metadata
    let meta = handler.render_metadata(&field);
    assert_eq!(meta, json!({"type": "upload"}));
}

#[test]
fn test_custom_handler_overrides_default() {
    // 自定义 string handler 覆盖默认
    struct CustomStringHandler;

    impl FormFieldHandler for CustomStringHandler {
        fn supported_type(&self) -> &str {
            "string"
        }

        fn validate(
            &self,
            _field: &flowable_form_service::FormProperty,
            _value: &Value,
        ) -> Result<(), FlowableError> {
            Ok(())
        }

        fn coerce(
            &self,
            _field: &flowable_form_service::FormProperty,
            value: Value,
        ) -> Result<Value, FlowableError> {
            // 自定义行为：总是返回大写
            match value {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                other => Ok(other),
            }
        }

        fn render_metadata(&self, _field: &flowable_form_service::FormProperty) -> Value {
            json!({"type": "custom_string"})
        }
    }

    let mut handlers = default_handlers();
    handlers.insert("string".to_string(), Arc::new(CustomStringHandler));

    let handler = handlers.get("string").unwrap();
    let field = make_field("name", "string", false);

    // 自定义 coerce 行为
    let result = handler.coerce(&field, json!("hello")).unwrap();
    assert_eq!(result, json!("HELLO"));

    // 自定义 render_metadata
    let meta = handler.render_metadata(&field);
    assert_eq!(meta, json!({"type": "custom_string"}));
}

// ============================================================
// 8. Handler 的 required 字段校验
// ============================================================

#[test]
fn test_required_field_validation_across_all_handlers() {
    let handlers: Vec<(&str, Arc<dyn FormFieldHandler>)> = vec![
        ("string", Arc::new(TextFieldHandler)),
        ("integer", Arc::new(NumberFieldHandler)),
        ("date", Arc::new(DateFieldHandler)),
        ("boolean", Arc::new(BooleanFieldHandler)),
        ("enum", Arc::new(OptionFieldHandler)),
    ];

    for (type_name, handler) in &handlers {
        let field = make_field("test_field", type_name, true);

        // 空字符串应失败
        let result = handler.validate(&field, &json!(""));
        assert!(
            result.is_err(),
            "Handler for '{}' should reject empty string when required",
            type_name
        );

        // Null 应失败
        let result = handler.validate(&field, &Value::Null);
        assert!(
            result.is_err(),
            "Handler for '{}' should reject null when required",
            type_name
        );
    }
}

#[test]
fn test_non_required_field_allows_empty() {
    let handlers: Vec<(&str, Arc<dyn FormFieldHandler>)> = vec![
        ("string", Arc::new(TextFieldHandler)),
        ("integer", Arc::new(NumberFieldHandler)),
        ("date", Arc::new(DateFieldHandler)),
        ("boolean", Arc::new(BooleanFieldHandler)),
        ("enum", Arc::new(OptionFieldHandler)),
    ];

    for (type_name, handler) in &handlers {
        let field = make_field("test_field", type_name, false);

        // 非 required 时，空字符串应通过（对于 string/date/enum）
        // 对于 number/boolean，空字符串可能被 validate 拒绝（格式错误），但不应是 "required" 错误
        let result = handler.validate(&field, &json!(""));
        if let Err(err) = result {
            let err_msg = format!("{}", err);
            assert!(
                !err_msg.contains("is required"),
                "Handler for '{}' should not report 'required' error for non-required field",
                type_name
            );
        }
    }
}

#[test]
fn test_supported_type_values() {
    assert_eq!(TextFieldHandler.supported_type(), "string");
    assert_eq!(NumberFieldHandler.supported_type(), "number");
    assert_eq!(DateFieldHandler.supported_type(), "date");
    assert_eq!(BooleanFieldHandler.supported_type(), "boolean");
    assert_eq!(OptionFieldHandler.supported_type(), "enum");
}
