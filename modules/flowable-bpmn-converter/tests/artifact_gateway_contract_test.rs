use flowable_bpmn_converter::{BpmnXMLConverter, write_bpmn_model};
use flowable_bpmn_model::BpmnModel;
use serde_json::Value;

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
             targetNamespace="https://flowable.org/modeler-c1-contract">
  <process id="artifactProcess" name="Artifact process" isExecutable="true">
    <startEvent id="start"/>
    <complexGateway id="decision" name="Complex decision" default="fallback">
      <activationCondition xsi:type="tFormalExpression"><![CDATA[${approvedCount >= 2}]]></activationCondition>
    </complexGateway>
    <endEvent id="end"/>
    <sequenceFlow id="toDecision" sourceRef="start" targetRef="decision"/>
    <sequenceFlow id="fallback" sourceRef="decision" targetRef="end"/>
    <textAnnotation id="note" textFormat="text/markdown">
      <text>**Two** approvals are required.</text>
    </textAnnotation>
    <group id="reviewGroup" categoryValueRef="reviewCategory"/>
    <association id="noteLink" sourceRef="note" targetRef="decision" associationDirection="One"/>
    <subProcess id="nested" name="Nested review">
      <task id="nestedTask"/>
      <textAnnotation id="nestedNote"><text>Nested note</text></textAnnotation>
      <association id="nestedLink" sourceRef="nestedNote" targetRef="nestedTask"/>
    </subProcess>
  </process>
</definitions>"#;

#[test]
fn complex_gateway_and_artifacts_survive_editor_json_and_xml_roundtrip() {
    let converter = BpmnXMLConverter::new();
    let original = converter.try_convert_to_bpmn_model(XML).unwrap();
    assert_contract(&serde_json::to_value(&original).unwrap());

    let editor_json = serde_json::to_vec(&original).unwrap();
    let editor_model: BpmnModel = serde_json::from_slice(&editor_json).unwrap();
    let written = write_bpmn_model(&editor_model).unwrap();
    assert!(written.contains("<complexGateway"));
    assert!(written.contains("<activationCondition"));
    assert!(written.contains("<textAnnotation"));
    assert!(written.contains("<group"));
    assert!(written.contains("associationDirection=\"One\""));

    let reparsed = converter.try_convert_to_bpmn_model(&written).unwrap();
    assert_contract(&serde_json::to_value(&reparsed).unwrap());
}

fn assert_contract(model: &Value) {
    let process = &model["mainProcess"];
    let elements = process["flowElements"].as_array().unwrap();
    let complex = elements
        .iter()
        .find(|element| element["elementType"] == "complexGateway")
        .expect("complex gateway should be preserved");
    assert_eq!(complex["id"], "decision");
    assert_eq!(complex["defaultFlow"], "fallback");
    assert_eq!(complex["activationCondition"], "${approvedCount >= 2}");

    let artifacts = process["artifacts"].as_array().unwrap();
    let annotation = artifacts
        .iter()
        .find(|artifact| artifact["artifactType"] == "textAnnotation")
        .expect("text annotation should be preserved");
    assert_eq!(annotation["id"], "note");
    assert_eq!(annotation["textFormat"], "text/markdown");
    assert_eq!(annotation["text"], "**Two** approvals are required.");

    let group = artifacts
        .iter()
        .find(|artifact| artifact["artifactType"] == "group")
        .expect("group should be preserved");
    assert_eq!(group["categoryValueRef"], "reviewCategory");

    let association = artifacts
        .iter()
        .find(|artifact| artifact["artifactType"] == "association")
        .expect("association should be preserved");
    assert_eq!(association["associationDirection"], "ONE");

    let nested = elements
        .iter()
        .find(|element| element["id"] == "nested")
        .expect("nested subprocess should be preserved");
    assert_eq!(nested["artifacts"].as_array().unwrap().len(), 2);
}
