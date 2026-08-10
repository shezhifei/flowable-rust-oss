use flowable_bpmn_model::{
    ArtifactEnum, Association, BoundaryEvent, BpmnModel, ComplexGateway, ExclusiveGateway,
    FlowElementEnum, Group, Process, ServiceTask, StartEvent, SubProcess, Task, TextAnnotation,
    UserTask,
};
use flowable_form_service::{BaseFormField, FormFieldModel};
use flowable_modeler_protocol::{BpmnEditorDocument, ProtocolVersion, editor_protocol_schema_json};

#[test]
fn bpmn_element_discriminators_preserve_concrete_variants() {
    let elements = vec![
        FlowElementEnum::Task(Task::default()),
        FlowElementEnum::UserTask(UserTask::default()),
        FlowElementEnum::ServiceTask(ServiceTask::default()),
        FlowElementEnum::StartEvent(StartEvent::default()),
        FlowElementEnum::ExclusiveGateway(ExclusiveGateway::default()),
        FlowElementEnum::ComplexGateway(ComplexGateway::default()),
        FlowElementEnum::BoundaryEvent(BoundaryEvent::default()),
        FlowElementEnum::SubProcess(SubProcess::default()),
    ];
    let model = BpmnModel {
        processes: vec![Process {
            flow_elements: elements,
            ..Process::default()
        }],
        ..BpmnModel::default()
    };

    let json = serde_json::to_value(BpmnEditorDocument::new(model)).unwrap();
    let encoded_types: Vec<_> = json["model"]["processes"][0]["flowElements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|element| element["elementType"].as_str().unwrap())
        .collect();
    assert_eq!(
        encoded_types,
        [
            "task",
            "userTask",
            "serviceTask",
            "startEvent",
            "exclusiveGateway",
            "complexGateway",
            "boundaryEvent",
            "subProcess",
        ]
    );

    let decoded: BpmnEditorDocument = serde_json::from_value(json).unwrap();
    let decoded = &decoded.model.processes[0].flow_elements;
    assert!(matches!(decoded[0], FlowElementEnum::Task(_)));
    assert!(matches!(decoded[1], FlowElementEnum::UserTask(_)));
    assert!(matches!(decoded[2], FlowElementEnum::ServiceTask(_)));
    assert!(matches!(decoded[3], FlowElementEnum::StartEvent(_)));
    assert!(matches!(decoded[4], FlowElementEnum::ExclusiveGateway(_)));
    assert!(matches!(decoded[5], FlowElementEnum::ComplexGateway(_)));
    assert!(matches!(decoded[6], FlowElementEnum::BoundaryEvent(_)));
    assert!(matches!(decoded[7], FlowElementEnum::SubProcess(_)));
}

#[test]
fn bpmn_artifact_discriminators_preserve_every_editable_variant() {
    let artifacts = vec![
        ArtifactEnum::Association(Association::default()),
        ArtifactEnum::TextAnnotation(TextAnnotation::default()),
        ArtifactEnum::Group(Group::default()),
    ];

    let json = serde_json::to_value(&artifacts).unwrap();
    let encoded_types = json
        .as_array()
        .unwrap()
        .iter()
        .map(|artifact| artifact["artifactType"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(encoded_types, ["association", "textAnnotation", "group"]);

    let decoded: Vec<ArtifactEnum> = serde_json::from_value(json).unwrap();
    assert!(matches!(decoded[0], ArtifactEnum::Association(_)));
    assert!(matches!(decoded[1], ArtifactEnum::TextAnnotation(_)));
    assert!(matches!(decoded[2], ArtifactEnum::Group(_)));
}

#[test]
fn bpmn_projection_keeps_the_canonical_model_shape() {
    let json = serde_json::to_value(BpmnEditorDocument::new(BpmnModel::default())).unwrap();
    let model = json["model"].as_object().unwrap();

    assert_eq!(json["schemaVersion"], "1.0");
    assert!(model.contains_key("mainProcess"));

    let process_json = serde_json::to_value(Process::default()).unwrap();
    let process = process_json.as_object().unwrap();
    assert!(process.contains_key("flowElementMap"));
    assert!(process.contains_key("artifactMap"));
}

#[test]
fn form_fields_keep_their_manual_field_type_discriminator() {
    let field = FormFieldModel::BaseField(BaseFormField {
        id: "requestReason".to_string(),
        name: Some("Reason".to_string()),
        field_type: Some("text".to_string()),
        value: None,
        readable: Some(true),
        writable: Some(true),
        required: Some(true),
        read_only: Some(false),
        placeholder: None,
        params: None,
        layout: None,
        date_pattern: None,
        enum_values: Vec::new(),
    });

    let json = serde_json::to_value(&field).unwrap();
    assert_eq!(json["fieldType"], "BaseField");
    let decoded: FormFieldModel = serde_json::from_value(json).unwrap();
    assert!(matches!(decoded, FormFieldModel::BaseField(_)));
}

#[test]
fn generated_schema_contains_all_editor_roots_and_discriminators() {
    let schema = editor_protocol_schema_json().unwrap();
    assert!(schema.contains("BpmnEditorDocument"));
    assert!(schema.contains("DmnEditorDocument"));
    assert!(schema.contains("FormEditorDocument"));
    assert!(schema.contains("elementType"));
    assert!(schema.contains("artifactType"));
    assert!(schema.contains("complexGateway"));
    assert!(schema.contains("textAnnotation"));
    assert!(schema.contains("eventDefinitionType"));
    assert!(schema.contains("fieldType"));
    assert!(schema.contains("OptionFormField"));
    assert!(schema.contains("ExpressionFormField"));
    let schema_value: serde_json::Value = serde_json::from_str(&schema).unwrap();
    let first_flow_variant = &schema_value["$defs"]["FlowElementEnum"]["oneOf"][0];
    assert!(first_flow_variant.get("$ref").is_none());
    assert_eq!(
        first_flow_variant["allOf"][0]["$ref"],
        "#/$defs/SequenceFlow"
    );
    assert_eq!(
        first_flow_variant["allOf"][1]["properties"]["elementType"]["const"],
        "sequenceFlow"
    );
    assert_eq!(ProtocolVersion::default(), ProtocolVersion::V1);
}
