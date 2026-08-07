use flowable_form_service::{FormFieldModel, FormOption, FormOutcome, LayoutDefinition};
use serde_json::{Value, json};

// ============================================================================
// Test 1: FormOption serialization/deserialization
// ============================================================================
#[test]
fn test_option_serialization_deserialization() {
    let option = FormOption {
        id: "opt1".to_string(),
        name: "Option One".to_string(),
    };

    let json_str = serde_json::to_string(&option).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed["id"], "opt1");
    assert_eq!(parsed["name"], "Option One");

    let roundtrip: FormOption = serde_json::from_str(&json_str).unwrap();
    assert_eq!(roundtrip.id, "opt1");
    assert_eq!(roundtrip.name, "Option One");
}

// ============================================================================
// Test 2: OptionFormField JSON round-trip (with options list)
// ============================================================================
#[test]
fn test_option_form_field_json_roundtrip() {
    let json_input = json!({
        "id": "color",
        "name": "Favorite Color",
        "type": "dropdown",
        "fieldType": "OptionFormField",
        "optionType": "static",
        "hasEmptyValue": true,
        "options": [
            { "id": "red", "name": "Red" },
            { "id": "blue", "name": "Blue" },
            { "id": "green", "name": "Green" }
        ],
        "optionsExpression": null,
        "readable": true,
        "writable": true,
        "required": true
    });

    let field: FormFieldModel = serde_json::from_value(json_input.clone()).unwrap();

    match &field {
        FormFieldModel::OptionField(opt_field) => {
            assert_eq!(opt_field.base.id, "color");
            assert_eq!(opt_field.base.name.as_deref(), Some("Favorite Color"));
            assert_eq!(opt_field.base.field_type.as_deref(), Some("dropdown"));
            assert_eq!(opt_field.option_type.as_deref(), Some("static"));
            assert!(opt_field.has_empty_value);
            assert_eq!(opt_field.options.len(), 3);
            assert_eq!(opt_field.options[0].id, "red");
            assert_eq!(opt_field.options[1].name, "Blue");
            assert_eq!(opt_field.base.required, Some(true));
        }
        other => panic!("Expected OptionField variant, got {:?}", other),
    }

    // Serialize back and verify
    let serialized = serde_json::to_value(&field).unwrap();
    assert_eq!(serialized["id"], "color");
    assert_eq!(serialized["fieldType"], "OptionFormField");
    assert_eq!(serialized["options"].as_array().unwrap().len(), 3);
}

// ============================================================================
// Test 3: ExpressionFormField JSON round-trip
// ============================================================================
#[test]
fn test_expression_form_field_json_roundtrip() {
    let json_input = json!({
        "id": "fullName",
        "name": "Full Name",
        "type": "string",
        "fieldType": "ExpressionFormField",
        "expression": "${firstName} ${lastName}",
        "readable": true,
        "writable": false,
        "required": false
    });

    let field: FormFieldModel = serde_json::from_value(json_input.clone()).unwrap();

    match &field {
        FormFieldModel::ExpressionField(expr_field) => {
            assert_eq!(expr_field.base.id, "fullName");
            assert_eq!(expr_field.base.name.as_deref(), Some("Full Name"));
            assert_eq!(expr_field.expression, "${firstName} ${lastName}");
            assert_eq!(expr_field.base.writable, Some(false));
        }
        other => panic!("Expected ExpressionField variant, got {:?}", other),
    }

    // Serialize back
    let serialized = serde_json::to_value(&field).unwrap();
    assert_eq!(serialized["id"], "fullName");
    assert_eq!(serialized["fieldType"], "ExpressionFormField");
    assert_eq!(serialized["expression"], "${firstName} ${lastName}");
}

// ============================================================================
// Test 4: FormContainer nested field parsing
// ============================================================================
#[test]
fn test_form_container_nested_field_parsing() {
    let json_input = json!({
        "id": "addressSection",
        "name": "Address",
        "type": "container",
        "fieldType": "Container",
        "fields": [
            [
                {
                    "id": "street",
                    "name": "Street",
                    "type": "string",
                    "fieldType": "BaseField"
                },
                {
                    "id": "city",
                    "name": "City",
                    "type": "string",
                    "fieldType": "BaseField"
                }
            ],
            [
                {
                    "id": "zipCode",
                    "name": "Zip Code",
                    "type": "string",
                    "fieldType": "BaseField"
                }
            ]
        ]
    });

    let field: FormFieldModel = serde_json::from_value(json_input.clone()).unwrap();

    match &field {
        FormFieldModel::Container(container) => {
            assert_eq!(container.base.id, "addressSection");
            assert_eq!(container.base.name.as_deref(), Some("Address"));
            assert_eq!(container.fields.len(), 2); // 2 rows

            // Row 0: 2 columns
            assert_eq!(container.fields[0].len(), 2);
            match &container.fields[0][0] {
                FormFieldModel::BaseField(bf) => {
                    assert_eq!(bf.id, "street");
                }
                other => panic!("Expected BaseField, got {:?}", other),
            }
            match &container.fields[0][1] {
                FormFieldModel::BaseField(bf) => {
                    assert_eq!(bf.id, "city");
                }
                other => panic!("Expected BaseField, got {:?}", other),
            }

            // Row 1: 1 column
            assert_eq!(container.fields[1].len(), 1);
            match &container.fields[1][0] {
                FormFieldModel::BaseField(bf) => {
                    assert_eq!(bf.id, "zipCode");
                }
                other => panic!("Expected BaseField, got {:?}", other),
            }
        }
        other => panic!("Expected Container variant, got {:?}", other),
    }

    // Serialize back
    let serialized = serde_json::to_value(&field).unwrap();
    assert_eq!(serialized["fieldType"], "Container");
    assert_eq!(serialized["fields"].as_array().unwrap().len(), 2);
}

// ============================================================================
// Test 5: FormOutcome parsing
// ============================================================================
#[test]
fn test_form_outcome_parsing() {
    let json_input = json!([
        { "id": "submit", "name": "Submit" },
        { "id": "cancel", "name": "Cancel" },
        { "id": "save", "name": "Save Draft" }
    ]);

    let outcomes: Vec<FormOutcome> = serde_json::from_value(json_input).unwrap();

    assert_eq!(outcomes.len(), 3);
    assert_eq!(outcomes[0].id.as_deref(), Some("submit"));
    assert_eq!(outcomes[0].name.as_deref(), Some("Submit"));
    assert_eq!(outcomes[1].id.as_deref(), Some("cancel"));
    assert_eq!(outcomes[1].name.as_deref(), Some("Cancel"));
    assert_eq!(outcomes[2].id.as_deref(), Some("save"));
    assert_eq!(outcomes[2].name.as_deref(), Some("Save Draft"));

    // Test with null id/name
    let json_null = json!([
        { "id": null, "name": null }
    ]);
    let outcomes_null: Vec<FormOutcome> = serde_json::from_value(json_null).unwrap();
    assert_eq!(outcomes_null[0].id, None);
    assert_eq!(outcomes_null[0].name, None);
}

// ============================================================================
// Test 6: Complete form definition JSON parsing
// (with outcomes, containers, option fields)
// ============================================================================
#[test]
fn test_complete_form_definition_json_parsing() {
    let json_input = json!({
        "key": "employeeOnboarding",
        "name": "Employee Onboarding",
        "description": "New employee onboarding form",
        "resourceName": "employee-onboarding.form",
        "outcomes": [
            { "id": "submit", "name": "Submit" },
            { "id": "cancel", "name": "Cancel" }
        ],
        "outcomeVariableName": "formOutcome",
        "layout": {
            "columns": 2
        },
        "fields": [
            {
                "id": "personalInfo",
                "name": "Personal Information",
                "type": "container",
                "fieldType": "Container",
                "fields": [
                    [
                        {
                            "id": "firstName",
                            "name": "First Name",
                            "type": "string",
                            "fieldType": "BaseField",
                            "required": true
                        },
                        {
                            "id": "lastName",
                            "name": "Last Name",
                            "type": "string",
                            "fieldType": "BaseField",
                            "required": true
                        }
                    ]
                ]
            },
            {
                "id": "department",
                "name": "Department",
                "type": "dropdown",
                "fieldType": "OptionFormField",
                "optionType": "static",
                "hasEmptyValue": false,
                "options": [
                    { "id": "eng", "name": "Engineering" },
                    { "id": "hr", "name": "Human Resources" },
                    { "id": "sales", "name": "Sales" }
                ],
                "required": true
            },
            {
                "id": "startDate",
                "name": "Start Date",
                "type": "date",
                "fieldType": "BaseField",
                "datePattern": "yyyy-MM-dd"
            }
        ]
    });

    // Parse the fields array as Vec<FormFieldModel>
    let fields_array = json_input["fields"].as_array().unwrap();
    let parsed_fields: Vec<FormFieldModel> = fields_array
        .iter()
        .map(|f| serde_json::from_value(f.clone()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(parsed_fields.len(), 3);

    // Field 0: Container
    match &parsed_fields[0] {
        FormFieldModel::Container(container) => {
            assert_eq!(container.base.id, "personalInfo");
            assert_eq!(container.fields.len(), 1);
            assert_eq!(container.fields[0].len(), 2);
        }
        other => panic!("Expected Container, got {:?}", other),
    }

    // Field 1: OptionFormField
    match &parsed_fields[1] {
        FormFieldModel::OptionField(opt_field) => {
            assert_eq!(opt_field.base.id, "department");
            assert_eq!(opt_field.options.len(), 3);
            assert_eq!(opt_field.options[1].name, "Human Resources");
        }
        other => panic!("Expected OptionField, got {:?}", other),
    }

    // Field 2: BaseField
    match &parsed_fields[2] {
        FormFieldModel::BaseField(bf) => {
            assert_eq!(bf.id, "startDate");
            assert_eq!(bf.date_pattern.as_deref(), Some("yyyy-MM-dd"));
        }
        other => panic!("Expected BaseField, got {:?}", other),
    }

    // Parse outcomes
    let outcomes: Vec<FormOutcome> =
        serde_json::from_value(json_input["outcomes"].clone()).unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].id.as_deref(), Some("submit"));

    // Verify outcomeVariableName
    assert_eq!(json_input["outcomeVariableName"], "formOutcome");

    // Verify layout
    assert_eq!(json_input["layout"]["columns"], 2);
}

// ============================================================================
// Test 7: Flat field (no fieldType) is rejected with structured error
// ============================================================================
#[test]
fn flat_field_without_field_type_is_rejected() {
    let json_input = json!({
        "id": "requester",
        "name": "Requester",
        "type": "string",
        "required": true
    });

    let result: Result<FormFieldModel, _> = serde_json::from_value(json_input);
    assert!(
        result.is_err(),
        "Field without fieldType should be rejected"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("missing required 'fieldType'"),
        "Error should mention missing fieldType, got: {}",
        err_msg
    );
}

#[test]
fn unknown_field_type_is_rejected() {
    let json_input = json!({
        "id": "custom",
        "name": "Custom",
        "type": "string",
        "fieldType": "CustomWidget"
    });

    let result: Result<FormFieldModel, _> = serde_json::from_value(json_input);
    assert!(result.is_err(), "Unknown fieldType should be rejected");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unknown form field fieldType"),
        "Error should mention unknown fieldType, got: {}",
        err_msg
    );
}

// ============================================================================
// Additional: LayoutDefinition serialization
// ============================================================================
#[test]
fn test_layout_definition_serialization() {
    let layout = LayoutDefinition {
        row: Some(0),
        col: Some(1),
        col_span: Some(2),
    };

    let json_str = serde_json::to_string(&layout).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed["row"], 0);
    assert_eq!(parsed["col"], 1);
    assert_eq!(parsed["colSpan"], 2);

    let roundtrip: LayoutDefinition = serde_json::from_str(&json_str).unwrap();
    assert_eq!(roundtrip.row, Some(0));
    assert_eq!(roundtrip.col, Some(1));
    assert_eq!(roundtrip.col_span, Some(2));
}

// ============================================================================
// Additional: BaseFormField with all optional fields
// ============================================================================
#[test]
fn test_base_form_field_with_all_fields() {
    let json_input = json!({
        "id": "email",
        "name": "Email Address",
        "type": "string",
        "fieldType": "BaseField",
        "readable": true,
        "writable": true,
        "required": true,
        "readOnly": false,
        "placeholder": "Enter your email",
        "params": {
            "maxLength": "255",
            "pattern": "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$"
        },
        "layout": {
            "row": 0,
            "col": 0,
            "colSpan": 1
        },
        "datePattern": null,
        "enumValues": []
    });

    let field: FormFieldModel = serde_json::from_value(json_input).unwrap();

    match &field {
        FormFieldModel::BaseField(bf) => {
            assert_eq!(bf.id, "email");
            assert_eq!(bf.placeholder.as_deref(), Some("Enter your email"));
            assert_eq!(bf.read_only, Some(false));
            assert!(bf.params.is_some());
            let params = bf.params.as_ref().unwrap();
            assert_eq!(params.get("maxLength").unwrap(), "255");
            assert!(bf.layout.is_some());
            let layout = bf.layout.as_ref().unwrap();
            assert_eq!(layout.row, Some(0));
            assert_eq!(layout.col, Some(0));
            assert_eq!(layout.col_span, Some(1));
        }
        other => panic!("Expected BaseField, got {:?}", other),
    }
}

// ============================================================================
// Additional: Deeply nested containers
// ============================================================================
#[test]
fn test_deeply_nested_containers() {
    let json_input = json!({
        "id": "outerContainer",
        "name": "Outer",
        "type": "container",
        "fieldType": "Container",
        "fields": [
            [
                {
                    "id": "innerContainer",
                    "name": "Inner",
                    "type": "container",
                    "fieldType": "Container",
                    "fields": [
                        [
                            {
                                "id": "deepField",
                                "name": "Deep Field",
                                "type": "string",
                                "fieldType": "BaseField"
                            }
                        ]
                    ]
                }
            ]
        ]
    });

    let field: FormFieldModel = serde_json::from_value(json_input).unwrap();

    match &field {
        FormFieldModel::Container(outer) => {
            assert_eq!(outer.base.id, "outerContainer");
            assert_eq!(outer.fields.len(), 1);
            assert_eq!(outer.fields[0].len(), 1);

            match &outer.fields[0][0] {
                FormFieldModel::Container(inner) => {
                    assert_eq!(inner.base.id, "innerContainer");
                    assert_eq!(inner.fields.len(), 1);
                    assert_eq!(inner.fields[0].len(), 1);

                    match &inner.fields[0][0] {
                        FormFieldModel::BaseField(bf) => {
                            assert_eq!(bf.id, "deepField");
                        }
                        other => panic!("Expected BaseField, got {:?}", other),
                    }
                }
                other => panic!("Expected Container, got {:?}", other),
            }
        }
        other => panic!("Expected Container, got {:?}", other),
    }
}
