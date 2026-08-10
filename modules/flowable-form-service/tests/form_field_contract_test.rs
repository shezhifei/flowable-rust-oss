mod test_support;

use flowable_form_service::{
    AMOUNT, BOOLEAN, CONTAINER, DATE, DECIMAL, DROPDOWN, EXPRESSION, FLOWABLE_6_8_FIELD_TYPES,
    FUNCTIONAL_GROUP, FormDeploymentRequest, FormDeploymentResource, FormFieldCategory,
    FormFieldModel, FormFieldVariant, FormModel, FormSubmissionProperty, FormSubmissionRequest,
    FormSubmissionResult, HEADLINE, HEADLINE_WITH_LINE, HORIZONTAL_LINE, HYPERLINK, INTEGER,
    MULTI_LINE_TEXT, PEOPLE, RADIO_BUTTONS, SINGLE_LINE_TEXT, SPACER, UPLOAD,
    flowable_6_8_field_capability, runtime_handler_type, validate_form_model,
};
use serde_json::{Value, json};
use test_support::{deploy_runtime_process, runtime_fixture};

#[test]
fn exposes_the_exact_flowable_6_8_wire_constants_and_capabilities() {
    assert_eq!(
        FLOWABLE_6_8_FIELD_TYPES,
        &[
            SINGLE_LINE_TEXT,
            MULTI_LINE_TEXT,
            INTEGER,
            DECIMAL,
            AMOUNT,
            DATE,
            BOOLEAN,
            RADIO_BUTTONS,
            DROPDOWN,
            UPLOAD,
            EXPRESSION,
            PEOPLE,
            FUNCTIONAL_GROUP,
            CONTAINER,
            HYPERLINK,
            SPACER,
            HORIZONTAL_LINE,
            HEADLINE,
            HEADLINE_WITH_LINE,
        ]
    );

    let radio = flowable_6_8_field_capability(RADIO_BUTTONS).unwrap();
    assert_eq!(radio.category, FormFieldCategory::Option);
    assert_eq!(radio.required_variant, FormFieldVariant::OptionFormField);
    assert_eq!(runtime_handler_type(RADIO_BUTTONS), Some("radio"));
    assert_eq!(runtime_handler_type(MULTI_LINE_TEXT), Some("text"));
    assert_eq!(runtime_handler_type(AMOUNT), Some("decimal"));
    assert_eq!(runtime_handler_type(EXPRESSION), None);
    assert_eq!(runtime_handler_type(HEADLINE), None);
    assert!(flowable_6_8_field_capability("checkbox").is_none());
}

#[test]
fn unknown_wire_type_roundtrips_losslessly_but_is_rejected_for_save() {
    let source = json!({
        "key": "imported",
        "name": "Imported",
        "fields": [{
            "fieldType": "BaseField",
            "id": "vendor",
            "type": "vendor-widget-v7",
            "writable": true
        }]
    });
    let model: FormModel = serde_json::from_value(source).unwrap();
    let serialized = serde_json::to_value(&model).unwrap();
    assert_eq!(serialized["fields"][0]["type"], "vendor-widget-v7");

    let issues = validate_form_model(&model);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "flowable-form-field-type-unsupported");
    assert_eq!(issues[0].element_id.as_deref(), Some("vendor"));
}

#[test]
fn the_boundary_validator_rejects_dynamic_options_and_writable_display_fields() {
    // These checks belong to the modeler boundary: deployment is lenient like
    // Java 6.8, so the validator is what stops an unusable definition early.
    let dynamic: FormModel = serde_json::from_value(json!({
        "key": "dynamic",
        "name": "Dynamic",
        "fields": [{
            "fieldType": "OptionFormField",
            "id": "choice",
            "type": "dropdown",
            "writable": true,
            "optionsExpression": "${choices}"
        }]
    }))
    .unwrap();
    let issues = validate_form_model(&dynamic);
    assert!(
        issues
            .iter()
            .any(|issue| issue.code == "flowable-form-dynamic-options-unsupported"),
        "expected a dynamic-options issue, got {issues:?}"
    );

    let writable_display: FormModel = serde_json::from_value(json!({
        "key": "display",
        "name": "Display",
        "fields": [{
            "fieldType": "BaseField",
            "id": "heading",
            "type": "headline",
            "writable": true
        }]
    }))
    .unwrap();
    let issues = validate_form_model(&writable_display);
    assert!(
        issues
            .iter()
            .any(|issue| issue.code == "flowable-form-field-writeability-incompatible"),
        "expected a writeability issue, got {issues:?}"
    );
}

#[test]
fn runtime_aliases_identity_values_and_nested_fields_close_the_java_wire_loop() {
    let (engine, service) = runtime_fixture("form-6-8-wire-runtime");
    service
        .deploy(FormDeploymentRequest {
            name: "Flowable 6.8 form".into(),
            resources: vec![FormDeploymentResource {
                resource_name: "flowable-68.form".into(),
                resource: json!({
                    "key": "flowable68",
                    "name": "Flowable 6.8",
                    "fields": [
                        {
                            "fieldType": "Container",
                            "id": "details",
                            "type": "container",
                            "readOnly": true,
                            "writable": false,
                            "fields": [[
                                {
                                    "fieldType": "BaseField",
                                    "id": "notes",
                                    "name": "Notes",
                                    "type": "multi-line-text",
                                    "writable": true,
                                    "required": true,
                                    "layout": { "row": 0, "col": 0, "colSpan": 1 }
                                },
                                {
                                    "fieldType": "BaseField",
                                    "id": "total",
                                    "name": "Total",
                                    "type": "amount",
                                    "writable": true,
                                    "required": true
                                }
                            ]]
                        },
                        {
                            "fieldType": "OptionFormField",
                            "id": "choice",
                            "name": "Choice",
                            "type": "radio-buttons",
                            "writable": true,
                            "required": true,
                            "options": [
                                { "id": "approve", "name": "Approve" },
                                { "id": "reject", "name": "Reject" }
                            ]
                        },
                        {
                            "fieldType": "BaseField",
                            "id": "requester",
                            "name": "Requester",
                            "type": "people",
                            "writable": true,
                            "required": true
                        },
                        {
                            "fieldType": "BaseField",
                            "id": "team",
                            "name": "Team",
                            "type": "functional-group",
                            "writable": true,
                            "required": true
                        },
                        {
                            "fieldType": "ExpressionFormField",
                            "id": "requesterSummary",
                            "name": "Requester summary",
                            "type": "expression",
                            "readOnly": true,
                            "writable": false,
                            "expression": "${requester} / ${team}"
                        },
                        {
                            "fieldType": "BaseField",
                            "id": "heading",
                            "name": "Details",
                            "type": "headline",
                            "readOnly": true,
                            "writable": false
                        }
                    ]
                })
                .to_string(),
            }],
        })
        .unwrap();
    let process_definition_id =
        deploy_runtime_process(&engine, "flowable68Process", "flowable68", "flowable68");

    let start_form = service.get_start_form_data(&process_definition_id).unwrap();
    assert_eq!(
        start_form
            .form_properties
            .iter()
            .map(|field| field.id.as_str())
            .collect::<Vec<_>>(),
        vec!["notes", "total", "choice", "requester", "team"]
    );
    assert_eq!(
        start_form
            .form_properties
            .iter()
            .find(|field| field.id == "notes")
            .unwrap()
            .field_type,
        MULTI_LINE_TEXT
    );
    let nested_layout = start_form
        .form_fields
        .as_ref()
        .and_then(|fields| fields.first())
        .and_then(|field| match field {
            FormFieldModel::Container(container) => container.fields.first(),
            _ => None,
        })
        .and_then(|row| row.first())
        .and_then(|field| match field {
            FormFieldModel::BaseField(field) => field.layout.as_ref(),
            _ => None,
        })
        .unwrap();
    assert_eq!(nested_layout.row, Some(0));
    assert_eq!(nested_layout.col, Some(0));
    assert_eq!(nested_layout.col_span, Some(1));

    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id),
            task_id: None,
            business_key: None,
            outcome: None,
            properties: vec![
                property("notes", json!("line one\nline two")),
                property("total", json!("42.75")),
                property("choice", json!({ "id": "approve", "name": "Approve" })),
                property(
                    "requester",
                    json!({ "id": "alice", "displayName": "Alice" }),
                ),
                property("team", json!({ "id": "finance", "name": "Finance" })),
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(instance) => instance,
        other => panic!("expected process instance, got {other:?}"),
    };
    let variables = engine
        .get_variable_service()
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(variables.get("notes"), Some(&json!("line one\nline two")));
    assert_eq!(variables.get("total"), Some(&json!(42.75)));
    assert_eq!(variables.get("choice"), Some(&json!("approve")));
    assert_eq!(variables.get("requester"), Some(&json!("alice")));
    assert_eq!(variables.get("team"), Some(&json!("finance")));

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id)
        .unwrap()
        .pop()
        .unwrap();
    let task_form = service.get_task_form_data(&task.id).unwrap();
    let fields = task_form.form_fields.unwrap();
    let expression_value = fields.iter().find_map(|field| match field {
        FormFieldModel::ExpressionField(field) if field.base.id == "requesterSummary" => {
            field.base.value.clone()
        }
        _ => None,
    });
    assert_eq!(
        expression_value,
        Some(Value::String("alice / finance".into()))
    );
}

fn property(id: &str, value: Value) -> FormSubmissionProperty {
    FormSubmissionProperty {
        id: id.to_string(),
        value,
    }
}
