use flowable_engine::engine::process_engine::ProcessEngine;

fn deploy_error(process_body: &str) -> String {
    let engine = ProcessEngine::new("default".to_string());
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <error id="p62Error" errorCode="P62" />
  <process id="p62InvalidProcess">{process_body}</process>
</definitions>"#
    );
    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("P62 invalid structure".to_string())
                .add_string("p62-invalid.bpmn20.xml".to_string(), xml),
        )
        .expect_err("structurally invalid model must be rejected")
        .to_string()
}

#[test]
fn rejects_sequence_flow_references_outside_the_current_scope() {
    let message = deploy_error(
        r#"
        <startEvent id="start" />
        <sequenceFlow id="invalidSource" sourceRef="missing" targetRef="end" />
        <sequenceFlow id="invalidTarget" sourceRef="start" targetRef="missing" />
        <endEvent id="end" />"#,
    );

    assert!(message.contains("flowable-seq-flow-invalid-src"), "{message}");
    assert!(message.contains("flowable-seq-flow-invalid-target"), "{message}");
}

#[test]
fn rejects_java_exclusive_gateway_error_rules() {
    let no_outgoing = deploy_error(
        r#"
        <startEvent id="start" />
        <sequenceFlow id="toGateway" sourceRef="start" targetRef="gateway" />
        <exclusiveGateway id="gateway" />"#,
    );
    assert!(
        no_outgoing.contains("flowable-exclusive-gateway-no-outgoing-seq-flow"),
        "{no_outgoing}"
    );

    let single_condition = deploy_error(
        r#"
        <startEvent id="start" />
        <sequenceFlow id="toGateway" sourceRef="start" targetRef="gateway" />
        <exclusiveGateway id="gateway" />
        <sequenceFlow id="only" sourceRef="gateway" targetRef="end">
          <conditionExpression><![CDATA[${true}]]></conditionExpression>
        </sequenceFlow>
        <endEvent id="end" />"#,
    );
    assert!(
        single_condition
            .contains("flowable-exclusive-gateway-condition-not-allowed-on-single-seq-flow"),
        "{single_condition}"
    );

    let conditioned_default = deploy_error(
        r#"
        <startEvent id="start" />
        <sequenceFlow id="toGateway" sourceRef="start" targetRef="gateway" />
        <exclusiveGateway id="gateway" default="defaultFlow" />
        <sequenceFlow id="conditional" sourceRef="gateway" targetRef="firstEnd">
          <conditionExpression><![CDATA[${true}]]></conditionExpression>
        </sequenceFlow>
        <sequenceFlow id="defaultFlow" sourceRef="gateway" targetRef="defaultEnd">
          <conditionExpression><![CDATA[${false}]]></conditionExpression>
        </sequenceFlow>
        <endEvent id="firstEnd" />
        <endEvent id="defaultEnd" />"#,
    );
    assert!(
        conditioned_default.contains("flowable-exclusive-gateway-condition-on-seq-flow"),
        "{conditioned_default}"
    );
}

#[test]
fn rejects_multiple_none_starts_and_invalid_start_event_definitions() {
    let multiple = deploy_error(
        r#"
        <startEvent id="firstStart" />
        <startEvent id="secondStart" />
        <endEvent id="end" />"#,
    );
    assert!(
        multiple.contains("flowable-start-event-multiple-found"),
        "{multiple}"
    );

    let invalid_definition = deploy_error(
        r#"
        <startEvent id="errorStart">
          <errorEventDefinition errorRef="p62Error" />
        </startEvent>
        <endEvent id="end" />"#,
    );
    assert!(
        invalid_definition.contains("flowable-start-event-invalid-event-definition"),
        "{invalid_definition}"
    );
}

#[test]
fn rejects_complex_gateway_deployment_until_execution_semantics_are_supported() {
    let message = deploy_error(
        r#"
        <startEvent id="start"/>
        <sequenceFlow id="toGateway" sourceRef="start" targetRef="gateway"/>
        <complexGateway id="gateway">
          <activationCondition><![CDATA[${arrivals >= 2}]]></activationCondition>
        </complexGateway>
        <sequenceFlow id="toEnd" sourceRef="gateway" targetRef="end"/>
        <endEvent id="end"/>"#,
    );

    assert!(
        message.contains("flowable-complex-gateway-not-supported"),
        "{message}"
    );
}

#[test]
fn accepts_structurally_valid_process() {
    let engine = ProcessEngine::new("default".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p62ValidProcess">
    <startEvent id="start" />
    <sequenceFlow id="toGateway" sourceRef="start" targetRef="gateway" />
    <exclusiveGateway id="gateway" default="defaultFlow" />
    <sequenceFlow id="conditional" sourceRef="gateway" targetRef="firstEnd">
      <conditionExpression><![CDATA[${approved}]]></conditionExpression>
    </sequenceFlow>
    <sequenceFlow id="defaultFlow" sourceRef="gateway" targetRef="defaultEnd" />
    <endEvent id="firstEnd" />
    <endEvent id="defaultEnd" />
  </process>
</definitions>"#;
    let repository = engine.get_repository_service();

    repository
        .deploy(
            repository
                .create_deployment()
                .name("P62 valid structure".to_string())
                .add_string("p62-valid.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
}
