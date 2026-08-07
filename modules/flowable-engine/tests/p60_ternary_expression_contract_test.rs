use flowable_engine::el::expression::{Expression, SimpleExpression};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;
use serde_json::{Value, json};
use std::collections::HashMap;

fn evaluate(expression: &str, variables: &[(&str, Value)]) -> Option<Value> {
    let execution = Execution {
        variables: variables
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect::<HashMap<_, _>>(),
        ..Default::default()
    };
    SimpleExpression::new(expression.to_string()).get_value(&execution)
}

#[test]
fn ternary_expression_selects_values_with_java_precedence_and_associativity() {
    assert_eq!(evaluate("${true ? 'yes' : 'no'}", &[]), Some(json!("yes")));
    assert_eq!(evaluate("${false || true ? 1 : 2}", &[]), Some(json!(1)));
    assert_eq!(
        evaluate("${false ? 0 : true ? 2 : 3}", &[]),
        Some(json!(2))
    );
    assert_eq!(
        evaluate("${amount > 10 ? amount * 2 : amount + 1}", &[("amount", json!(12))]),
        Some(json!(24))
    );
}

#[test]
fn ternary_expression_only_evaluates_the_selected_branch() {
    assert_eq!(
        evaluate("${false ? missingVariable : 'fallback'}", &[]),
        Some(json!("fallback"))
    );
    assert_eq!(
        evaluate("${true ? 'selected' : missingVariable}", &[]),
        Some(json!("selected"))
    );
}

#[test]
fn ternary_boolean_result_drives_exclusive_gateway() {
    let engine = ProcessEngine::new("default".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p60TernaryGateway">
    <startEvent id="start" />
    <sequenceFlow id="toGateway" sourceRef="start" targetRef="gateway" />
    <exclusiveGateway id="gateway" />
    <sequenceFlow id="toApproved" sourceRef="gateway" targetRef="approvedEnd">
      <conditionExpression><![CDATA[${approved ? true : false}]]></conditionExpression>
    </sequenceFlow>
    <sequenceFlow id="toRejected" sourceRef="gateway" targetRef="rejectedEnd">
      <conditionExpression><![CDATA[${approved ? false : true}]]></conditionExpression>
    </sequenceFlow>
    <endEvent id="approvedEnd" />
    <endEvent id="rejectedEnd" />
  </process>
</definitions>"#;
    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("P60 ternary gateway".to_string())
                .add_string("p60-ternary.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let definition_id = repository.get_process_definition_ids().unwrap()[0].clone();

    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id)
                .variable("approved".to_string(), Value::Bool(true)),
        )
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);

    assert_eq!(
        executions
            .get(&instance.id)
            .and_then(|execution| execution.activity_id.as_deref()),
        Some("approvedEnd")
    );
}
