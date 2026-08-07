use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::FlowElementEnum;

/// P105 — `<sendTask>` must parse into the `SendTask` model variant instead of
/// being silently dropped (the pre-P105 `_ => return None` fallthrough caused
/// the node to vanish and left sequence flows dangling).
#[test]
fn send_task_type_mail_parses_into_send_task_variant() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="sendMailProcess" isExecutable="true">
    <sendTask id="sendTask1" name="Notify Ops" flowable:type="mail">
      <extensionElements>
        <flowable:field name="to" stringValue="ops@example.flowable.local" />
        <flowable:field name="subject" stringValue="Deployment finished" />
        <flowable:field name="text" expression="${bodyText}" />
      </extensionElements>
    </sendTask>
  </process>
</definitions>"#;

    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.expect("main process");
    let FlowElementEnum::SendTask(task) = process
        .flow_element_map
        .get("sendTask1")
        .expect("send task")
    else {
        panic!("sendTask1 should be a SendTask variant");
    };

    assert_eq!(
        task.service_task.task_type.as_deref(),
        Some("mail"),
        "flowable:type must populate SendTask.service_task.task_type"
    );
    assert_eq!(
        task.service_task.task.activity.flow_node.flow_element.name.as_deref(),
        Some("Notify Ops")
    );
    let fields = &task.service_task.task.activity.field_extensions;
    assert_eq!(fields.len(), 3, "{fields:?}");
    assert_eq!(fields[0].field_name.as_deref(), Some("to"));
    assert_eq!(fields[0].string_value.as_deref(), Some("ops@example.flowable.local"));
    assert_eq!(fields[1].field_name.as_deref(), Some("subject"));
    assert_eq!(fields[2].field_name.as_deref(), Some("text"));
    assert_eq!(fields[2].expression.as_deref(), Some("${bodyText}"));
}

/// P105 — dmn sendTask reuses the service-task field-extension parsing path.
#[test]
fn send_task_type_dmn_parses_decision_table_reference_key() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="sendDmnProcess" isExecutable="true">
    <sendTask id="sendTask1" flowable:type="dmn">
      <extensionElements>
        <flowable:field name="decisionTableReferenceKey">
          <flowable:string>loanEligibility</flowable:string>
        </flowable:field>
      </extensionElements>
    </sendTask>
  </process>
</definitions>"#;

    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.expect("main process");
    let FlowElementEnum::SendTask(task) = process
        .flow_element_map
        .get("sendTask1")
        .expect("send task")
    else {
        panic!("sendTask1 should be a SendTask variant");
    };

    assert_eq!(task.service_task.task_type.as_deref(), Some("dmn"));
    assert_eq!(
        task.service_task
            .task
            .activity
            .field_extensions
            .first()
            .and_then(|f| f.field_name.as_deref()),
        Some("decisionTableReferenceKey")
    );
}

/// P105 — a sendTask without `type` is still parsed (Java only warns at parse
/// and passes through at runtime; SendTaskParseHandler.java:54-56).
#[test]
fn send_task_without_type_still_parses() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="sendNoTypeProcess" isExecutable="true">
    <sendTask id="sendTask1" name="Plain Send" />
  </process>
</definitions>"#;

    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.expect("main process");
    let FlowElementEnum::SendTask(task) = process
        .flow_element_map
        .get("sendTask1")
        .expect("send task")
    else {
        panic!("sendTask1 should be a SendTask variant");
    };
    assert!(task.service_task.task_type.is_none());
    assert!(task.operation_ref.is_none());
}

/// P105 — the webservice marker (`implementation="##WebService"`) is captured on
/// the model; deployment validation rejects it with a clear error (deviation).
#[test]
fn send_task_webservice_marker_is_captured() {
    let xml = r###"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://example.flowable.local/services">
  <process id="sendWebServiceProcess" isExecutable="true">
    <sendTask id="sendTask1" name="Invoke WS"
              implementation="##WebService"
              operationRef="tns:myOperation" />
  </process>
</definitions>"###;

    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.expect("main process");
    let FlowElementEnum::SendTask(task) = process
        .flow_element_map
        .get("sendTask1")
        .expect("send task")
    else {
        panic!("sendTask1 should be a SendTask variant");
    };
    assert_eq!(
        task.service_task.implementation_type.as_deref(),
        Some("webservice")
    );
    assert_eq!(task.operation_ref.as_deref(), Some("tns:myOperation"));
}

/// P105 — `<manualTask>` must parse into the `ManualTask` variant (the previous
/// silent fallthrough dropped manual tasks from deployed models).
#[test]
fn manual_task_parses_into_manual_task_variant() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
  <process id="manualTaskProcess" isExecutable="true">
    <manualTask id="manualTask1" name="Review Manually" />
  </process>
</definitions>"#;

    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.expect("main process");
    let FlowElementEnum::ManualTask(task) = process
        .flow_element_map
        .get("manualTask1")
        .expect("manual task")
    else {
        panic!("manualTask1 should be a ManualTask variant");
    };
    assert_eq!(
        task.task.activity.flow_node.flow_element.name.as_deref(),
        Some("Review Manually")
    );
}

/// P105 — a mixed process containing both sendTask and manualTask keeps every
/// node in the flow-element map (core observable: no silently dropped nodes).
#[test]
fn mixed_send_and_manual_tasks_are_all_kept() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="mixedProcess" isExecutable="true">
    <startEvent id="startEvent1" />
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendTask1" />
    <sendTask id="sendTask1" flowable:type="mail">
      <extensionElements>
        <flowable:to>ops@example.flowable.local</flowable:to>
        <flowable:subject>Mixed</flowable:subject>
        <flowable:text>body</flowable:text>
      </extensionElements>
    </sendTask>
    <sequenceFlow id="flow2" sourceRef="sendTask1" targetRef="manualTask1" />
    <manualTask id="manualTask1" name="Review" />
    <sequenceFlow id="flow3" sourceRef="manualTask1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;

    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.expect("main process");

    assert!(
        matches!(
            process.flow_element_map.get("sendTask1"),
            Some(FlowElementEnum::SendTask(_))
        ),
        "sendTask1 must be present as a SendTask"
    );
    assert!(
        matches!(
            process.flow_element_map.get("manualTask1"),
            Some(FlowElementEnum::ManualTask(_))
        ),
        "manualTask1 must be present as a ManualTask"
    );
    assert!(
        matches!(
            process.flow_element_map.get("startEvent1"),
            Some(FlowElementEnum::StartEvent(_))
        ) && matches!(
            process.flow_element_map.get("endEvent1"),
            Some(FlowElementEnum::EndEvent(_))
        ),
        "start/end events must also be present"
    );
}
