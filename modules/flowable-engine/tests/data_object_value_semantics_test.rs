//! P24 sub-item 4: data object value semantics — converter-typed, no EL eval.

// The 3.14 below is the literal value in the BPMN fixture being round-tripped,
// not an approximation of pi; substituting `f64::consts::PI` would break the
// assertion it exists to make.
#![allow(clippy::approx_constant)]

use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

const TYPED_DATA_OBJECTS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="typedDataObjects" isExecutable="true">
    <dataObject id="doLong" name="longVar" itemSubjectRef="xsd:long">
      <extensionElements>
        <flowable:value>42</flowable:value>
      </extensionElements>
    </dataObject>
    <dataObject id="doDouble" name="doubleVar" itemSubjectRef="xsd:double">
      <extensionElements>
        <flowable:value>3.14</flowable:value>
      </extensionElements>
    </dataObject>
    <dataObject id="doBool" name="boolVar" itemSubjectRef="xsd:boolean">
      <extensionElements>
        <flowable:value>true</flowable:value>
      </extensionElements>
    </dataObject>
    <dataObject id="doDate" name="dateVar" itemSubjectRef="xsd:datetime">
      <extensionElements>
        <flowable:value>2020-01-15T10:30:00</flowable:value>
      </extensionElements>
    </dataObject>
    <dataObject id="doExpr" name="exprVar" itemSubjectRef="xsd:string">
      <extensionElements>
        <flowable:value>${shouldNotEvaluate}</flowable:value>
      </extensionElements>
    </dataObject>
    <startEvent id="start"/>
    <sequenceFlow sourceRef="start" targetRef="task"/>
    <userTask id="task" name="Task"/>
    <sequenceFlow sourceRef="task" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>"#;

#[test]
fn test_converter_types_data_object_values() {
    let model = BpmnXMLConverter::new().convert_to_bpmn_model(TYPED_DATA_OBJECTS_XML);
    let process = model.main_process.as_ref().unwrap();
    let by_name = |n: &str| {
        process
            .data_objects
            .iter()
            .find(|d| d.name.as_deref() == Some(n))
            .unwrap()
    };

    assert_eq!(by_name("longVar").value, Some(json!(42)));
    assert_eq!(by_name("doubleVar").value, Some(json!(3.14)));
    assert_eq!(by_name("boolVar").value, Some(json!(true)));
    assert_eq!(
        by_name("dateVar").value,
        Some(json!("2020-01-15T10:30:00"))
    );
    // Expressions are NOT evaluated — stored as literal string.
    assert_eq!(
        by_name("exprVar").value,
        Some(json!("${shouldNotEvaluate}"))
    );
}

#[test]
fn test_runtime_copies_typed_values_without_el() {
    let engine = ProcessEngine::new("default".to_string());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    repo.deploy(
        repo.create_deployment().add_string(
            "typed-do.bpmn20.xml".to_string(),
            TYPED_DATA_OBJECTS_XML.to_string(),
        ),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime.start_process_instance_by_id(def_id, None).unwrap();

    let vars = runtime.get_variables(pi.id.clone()).unwrap();
    assert_eq!(vars.get("longVar"), Some(&json!(42)));
    assert_eq!(vars.get("doubleVar"), Some(&json!(3.14)));
    assert_eq!(vars.get("boolVar"), Some(&json!(true)));
    assert_eq!(vars.get("dateVar"), Some(&json!("2020-01-15T10:30:00")));
    assert_eq!(
        vars.get("exprVar"),
        Some(&json!("${shouldNotEvaluate}")),
        "must not evaluate ${{...}} at process start"
    );

    let dos = runtime.get_data_objects(pi.id.clone()).unwrap();
    assert_eq!(dos.get("longVar").map(|d| &d.value), Some(&json!(42)));
    assert_eq!(
        dos.get("exprVar").map(|d| &d.value),
        Some(&json!("${shouldNotEvaluate}"))
    );
}
