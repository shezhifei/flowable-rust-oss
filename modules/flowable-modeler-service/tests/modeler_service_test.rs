use flowable_form_service::{
    BaseFormField, ExpressionFormField, FormContainer, FormFieldModel, FormModel, FormOption,
    FormOutcome, LayoutDefinition, OptionFormField,
};
use flowable_modeler_protocol::{DmnEditorDocument, FormEditorDocument};
use flowable_modeler_service::{
    bpmn_thumbnail_png, decode_bpmn_xml, decode_dmn_xml, decode_form_json, encode_bpmn_xml,
    encode_dmn_xml, encode_form_json, layout_bpmn, validate_bpmn, validate_dmn, validate_form,
};
use serde_json::json;

const BPMN: &str = r#"<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="https://flowable.org/test"><process id="leave" isExecutable="true"><startEvent id="start"/><userTask id="review"/><endEvent id="end"/><sequenceFlow id="f1" sourceRef="start" targetRef="review"/><sequenceFlow id="f2" sourceRef="review" targetRef="end"/></process></definitions>"#;
const DMN: &str = r#"<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="definitions" name="Eligibility" namespace="https://flowable.org/test"><decision id="decision" name="Eligibility"><decisionTable id="table" hitPolicy="FIRST"><input id="input"><inputExpression id="expression" typeRef="integer"><text>age</text></inputExpression></input><output id="output" name="result" typeRef="string"/><rule id="adult"><inputEntry id="adult-input"><text>&gt;= 18</text></inputEntry><outputEntry id="adult-output"><text>"adult"</text></outputEntry></rule></decisionTable></decision></definitions>"#;
const DMN_UNSUPPORTED_UNARY: &str = r#"<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="definitions" name="Invalid unary" namespace="https://flowable.org/test"><decision id="decision" name="Invalid unary"><decisionTable id="table" hitPolicy="FIRST"><input id="input"><inputExpression id="expression" typeRef="string"><text>value</text></inputExpression></input><output id="output" name="result" typeRef="string"/><rule id="rule"><inputEntry id="rule-input"><text>starts with("missing-placeholder")</text></inputEntry><outputEntry id="rule-output"><text>"invalid"</text></outputEntry></rule></decisionTable></decision></definitions>"#;
const DMN_INVALID_COLLECT_TYPE: &str = r#"<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="definitions" name="Invalid collect" namespace="https://flowable.org/test"><decision id="decision" name="Invalid collect"><decisionTable id="table" hitPolicy="COLLECT" aggregation="COUNT"><input id="input"><inputExpression id="expression" typeRef="integer"><text>age</text></inputExpression></input><output id="output" name="result" typeRef="string"/><rule id="rule"><inputEntry id="rule-input"><text>-</text></inputEntry><outputEntry id="rule-output"><text>"one"</text></outputEntry></rule></decisionTable></decision></definitions>"#;

#[test]
fn bpmn_boundary_produces_valid_xml_layout_and_png() {
    let document = decode_bpmn_xml(BPMN).unwrap();
    assert!(validate_bpmn(&document).valid);
    assert!(encode_bpmn_xml(&document).unwrap().contains("<userTask"));
    let laid_out = layout_bpmn(&document).unwrap();
    assert!(!laid_out.model.location_map.is_empty());
    assert_eq!(
        &bpmn_thumbnail_png(&laid_out).unwrap()[..8],
        b"\x89PNG\r\n\x1a\n"
    );
}

#[test]
fn dmn_boundary_roundtrips_editor_json_and_xml() {
    let document = decode_dmn_xml(DMN).unwrap();
    let editor_json = serde_json::to_vec(&document).unwrap();
    let restored: DmnEditorDocument = serde_json::from_slice(&editor_json).unwrap();
    assert!(validate_dmn(&restored).valid);
    assert_eq!(
        decode_dmn_xml(&encode_dmn_xml(&restored).unwrap())
            .unwrap()
            .model,
        document.model
    );
}

#[test]
fn dmn_validation_rejects_unsupported_unary_tests_before_persistence() {
    let document = decode_dmn_xml(DMN_UNSUPPORTED_UNARY).unwrap();

    let result = validate_dmn(&document);

    assert!(!result.valid);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(
        result.errors[0].message,
        "Unsupported DMN unary test: unsupported string function unary test \
         'starts with(\"missing-placeholder\")'; only contains(?, \"literal\"), starts with(?, \
         \"literal\"), ends with(?, \"literal\"), and matches(?, \"regex\") are supported"
    );
}

#[test]
fn dmn_validation_rejects_collect_output_type_mismatches() {
    let document = decode_dmn_xml(DMN_INVALID_COLLECT_TYPE).unwrap();

    let result = validate_dmn(&document);

    assert!(!result.valid);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(
        result.errors[0].message,
        "HitPolicy: COLLECT has aggregation: Count needs output type number"
    );
}

#[test]
fn five_representative_forms_roundtrip_editor_json() {
    for model in representative_forms() {
        let document = FormEditorDocument::new(model.clone());
        assert!(validate_form(&document).valid, "{}", model.key);
        let json = encode_form_json(&document).unwrap();
        assert_eq!(decode_form_json(&json).unwrap().model, model);
    }
}

#[test]
fn form_validation_reports_all_nested_duplicate_and_required_field_errors() {
    let duplicate = base_field("duplicate", "text");
    let document = FormEditorDocument::new(FormModel {
        key: String::new(),
        name: String::new(),
        description: None,
        fields: vec![
            FormFieldModel::BaseField(duplicate.clone()),
            FormFieldModel::Container(FormContainer {
                base: non_writable_field("container", "container"),
                fields: vec![vec![
                    FormFieldModel::BaseField(duplicate),
                    FormFieldModel::BaseField(base_field("", "text")),
                ]],
            }),
        ],
        outcomes: Vec::new(),
        outcome_variable_name: None,
        layout: None,
    });

    let result = validate_form(&document);
    assert!(!result.valid);
    assert_eq!(result.errors.len(), 4);
    assert!(
        result
            .errors
            .iter()
            .any(|issue| issue.element_id.as_deref() == Some("duplicate"))
    );
}

fn representative_forms() -> Vec<FormModel> {
    vec![
        form("empty", Vec::new()),
        form(
            "text",
            vec![FormFieldModel::BaseField(base_field("summary", "text"))],
        ),
        form(
            "options",
            vec![FormFieldModel::OptionField(OptionFormField {
                base: base_field("priority", "dropdown"),
                option_type: Some("dropdown".into()),
                has_empty_value: true,
                options: vec![
                    FormOption {
                        id: "low".into(),
                        name: "Low".into(),
                    },
                    FormOption {
                        id: "high".into(),
                        name: "High".into(),
                    },
                ],
                options_expression: None,
            })],
        ),
        form(
            "expression",
            vec![FormFieldModel::ExpressionField(ExpressionFormField {
                base: non_writable_field("manager", "expression"),
                expression: "${managerName}".into(),
            })],
        ),
        FormModel {
            key: "container".into(),
            name: "Container form".into(),
            description: Some("Nested form projection".into()),
            fields: vec![FormFieldModel::Container(FormContainer {
                base: non_writable_field("details", "container"),
                fields: vec![vec![FormFieldModel::BaseField(base_field(
                    "notes",
                    "multi-line-text",
                ))]],
            })],
            outcomes: vec![FormOutcome {
                id: Some("approve".into()),
                name: Some("Approve".into()),
            }],
            outcome_variable_name: Some("decision".into()),
            layout: Some(json!({ "columns": 2 })),
        },
    ]
}

fn form(key: &str, fields: Vec<FormFieldModel>) -> FormModel {
    FormModel {
        key: key.into(),
        name: format!("{key} form"),
        description: None,
        fields,
        outcomes: Vec::new(),
        outcome_variable_name: None,
        layout: None,
    }
}

fn base_field(id: &str, field_type: &str) -> BaseFormField {
    BaseFormField {
        id: id.into(),
        name: Some(id.into()),
        field_type: Some(field_type.into()),
        value: None,
        readable: Some(true),
        writable: Some(true),
        required: Some(false),
        read_only: Some(false),
        placeholder: None,
        params: None,
        layout: Some(LayoutDefinition {
            row: Some(0),
            col: Some(0),
            col_span: Some(1),
        }),
        date_pattern: None,
        enum_values: Vec::new(),
    }
}

#[test]
fn form_validation_reports_stable_recursive_contract_errors_before_save() {
    let mut invalid_layout = base_field("layout", "text");
    invalid_layout.layout = Some(LayoutDefinition {
        row: Some(-1),
        col: Some(0),
        col_span: Some(0),
    });
    let document = FormEditorDocument::new(FormModel {
        key: "invalid-contract".into(),
        name: "Invalid contract".into(),
        description: None,
        fields: vec![
            FormFieldModel::BaseField(BaseFormField {
                field_type: Some("dropdown".into()),
                ..base_field("wrongVariant", "text")
            }),
            FormFieldModel::OptionField(OptionFormField {
                base: base_field("badOptions", "radio-buttons"),
                option_type: None,
                has_empty_value: false,
                options: vec![FormOption {
                    id: String::new(),
                    name: String::new(),
                }],
                options_expression: Some("${dynamicOptions}".into()),
            }),
            FormFieldModel::ExpressionField(ExpressionFormField {
                base: base_field("badExpression", "expression"),
                expression: String::new(),
            }),
            FormFieldModel::Container(FormContainer {
                base: non_writable_field("nested", "container"),
                fields: vec![vec![FormFieldModel::BaseField(invalid_layout)]],
            }),
        ],
        outcomes: Vec::new(),
        outcome_variable_name: None,
        layout: None,
    });

    let result = validate_form(&document);
    assert!(!result.valid);
    for code in [
        "flowable-form-field-variant-incompatible",
        "flowable-form-field-options-invalid",
        "flowable-form-dynamic-options-unsupported",
        "flowable-form-field-writeability-incompatible",
        "flowable-form-field-expression-invalid",
        "flowable-form-field-layout-invalid",
    ] {
        assert!(
            result
                .errors
                .iter()
                .any(|issue| issue.message.contains(code)),
            "missing validation code {code}: {:?}",
            result.errors
        );
    }
}

fn non_writable_field(id: &str, field_type: &str) -> BaseFormField {
    let mut field = base_field(id, field_type);
    field.writable = Some(false);
    field.read_only = Some(true);
    field
}
