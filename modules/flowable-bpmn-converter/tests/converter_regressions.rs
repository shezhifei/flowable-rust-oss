use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use flowable_engine_common::FlowableError;
use serde_json::Value;

/// P86b: historical `<flowable:stringValue>` child element must populate
/// `FieldExtension.string_value` (same as canonical `<flowable:string>`).
#[test]
fn field_extension_accepts_string_value_child_element() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="fieldStringValue" isExecutable="true">
    <serviceTask id="svc" flowable:class="com.example.Delegate">
      <extensionElements>
        <flowable:field name="legacyField">
          <flowable:stringValue>legacy-value</flowable:stringValue>
        </flowable:field>
        <flowable:field name="canonicalField">
          <flowable:string>canonical-value</flowable:string>
        </flowable:field>
        <flowable:field name="attrField" stringValue="attr-value"/>
      </extensionElements>
    </serviceTask>
  </process>
</definitions>"#;

    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.expect("main process");
    let FlowElementEnum::ServiceTask(task) =
        process.flow_element_map.get("svc").expect("service task")
    else {
        panic!("svc should be a service task");
    };

    let fields = &task.task.activity.field_extensions;
    assert_eq!(fields.len(), 3, "{fields:?}");

    let by_name = |name: &str| {
        fields
            .iter()
            .find(|f| f.field_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing field {name}"))
    };

    assert_eq!(
        by_name("legacyField").string_value.as_deref(),
        Some("legacy-value"),
        "child <flowable:stringValue> must be accepted"
    );
    assert_eq!(
        by_name("canonicalField").string_value.as_deref(),
        Some("canonical-value"),
        "child <flowable:string> must still work"
    );
    assert_eq!(
        by_name("attrField").string_value.as_deref(),
        Some("attr-value"),
        "attribute stringValue must still work"
    );
}

#[test]
fn serializes_canonical_compensation_flag() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process" isExecutable="true">
    <userTask id="compTask" isForCompensation="true" />
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let json = converter.to_canonical_contract_value(&model);
    let task = &json["mainProcess"]["flowElementMap"]["compTask"];

    assert_eq!(task["forCompensation"], Value::Bool(true));
    assert!(
        task.get("isForCompensation").is_none(),
        "internal compensation flag should not leak into canonical contract JSON"
    );
}

#[test]
fn preserves_process_and_subprocess_artifacts() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process" isExecutable="true">
    <userTask id="task1" />
    <textAnnotation id="annotation1">
      <text>Process annotation</text>
    </textAnnotation>
    <association id="association1" sourceRef="task1" targetRef="annotation1" />
    <subProcess id="subprocess1">
      <userTask id="task2" />
      <textAnnotation id="annotation2">
        <text>Subprocess annotation</text>
      </textAnnotation>
      <association id="association2" sourceRef="task2" targetRef="annotation2" />
    </subProcess>
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let json = converter.to_canonical_contract_value(&model);

    let process_artifacts = json["mainProcess"]["artifacts"].as_array().unwrap();
    assert_eq!(process_artifacts.len(), 2);
    assert!(
        process_artifacts
            .iter()
            .any(|artifact| artifact["id"] == "annotation1")
    );
    assert!(
        process_artifacts
            .iter()
            .any(|artifact| artifact["id"] == "association1")
    );

    let subprocess_artifacts = json["mainProcess"]["flowElementMap"]["subprocess1"]["artifacts"]
        .as_array()
        .unwrap();
    assert_eq!(subprocess_artifacts.len(), 2);
    assert!(
        subprocess_artifacts
            .iter()
            .any(|artifact| artifact["id"] == "annotation2")
    );
    assert!(
        subprocess_artifacts
            .iter()
            .any(|artifact| artifact["id"] == "association2")
    );
}

#[test]
fn parses_escalation_event_definition_and_top_level_escalation() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <escalation id="approvalEscalation" name="Approval Escalation" escalationCode="APPROVAL_TIMEOUT" />
  <process id="process" isExecutable="true">
    <userTask id="reviewTask" />
    <boundaryEvent id="catchEscalation" attachedToRef="reviewTask" cancelActivity="true">
      <escalationEventDefinition escalationRef="approvalEscalation" />
    </boundaryEvent>
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);

    assert_eq!(model.escalations.len(), 1);
    assert_eq!(
        model.escalations[0].base_element.id.as_deref(),
        Some("approvalEscalation")
    );
    assert_eq!(
        model.escalations[0].escalation_code.as_deref(),
        Some("APPROVAL_TIMEOUT")
    );

    let process = model.main_process.as_ref().unwrap();
    let boundary = process
        .flow_element_map
        .get("catchEscalation")
        .expect("boundary event should be indexed");
    let FlowElementEnum::BoundaryEvent(boundary) = boundary else {
        panic!("catchEscalation should be a boundary event");
    };
    let [EventDefinitionEnum::EscalationEventDefinition(escalation)] =
        boundary.event.event_definitions.as_slice()
    else {
        panic!("boundary should have one escalation event definition");
    };
    assert_eq!(
        escalation.escalation_ref.as_deref(),
        Some("approvalEscalation")
    );
}

#[test]
fn parses_service_task_skip_expression() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn">
  <process id="process" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="httpTask" />
    <serviceTask id="httpTask"
                 flowable:type="http"
                 flowable:skipExpression="${skipService}"
                 flowable:resultVariableName="httpResult" />
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let process = model.main_process.as_ref().unwrap();
    let task = process
        .flow_element_map
        .get("httpTask")
        .expect("service task should be indexed");
    let FlowElementEnum::ServiceTask(service_task) = task else {
        panic!("httpTask should be a service task");
    };

    assert_eq!(
        service_task.skip_expression.as_deref(),
        Some("${skipService}")
    );
}

#[test]
fn parses_receive_task_skip_expression() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn">
  <process id="process" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="receiveTask" />
    <receiveTask id="receiveTask"
                 name="Wait for callback"
                 messageRef="callbackMessage"
                 flowable:skipExpression="${skipReceive}" />
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let process = model.main_process.as_ref().unwrap();
    let task = process
        .flow_element_map
        .get("receiveTask")
        .expect("receive task should be indexed");
    let FlowElementEnum::ReceiveTask(receive_task) = task else {
        panic!("receiveTask should be a receive task");
    };

    assert_eq!(receive_task.message_ref.as_deref(), Some("callbackMessage"));
    assert_eq!(
        receive_task.skip_expression.as_deref(),
        Some("${skipReceive}")
    );
}

#[test]
fn preserves_job_category_and_unknown_extensions_on_specialized_elements() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn">
  <process id="process" isExecutable="true">
    <scriptTask id="scriptTask" scriptFormat="groovy">
      <extensionElements>
        <flowable:jobCategory>${categoryValue}</flowable:jobCategory>
        <flowable:customExtension>preserved</flowable:customExtension>
      </extensionElements>
      <script>return true</script>
    </scriptTask>
    <boundaryEvent id="timerBoundary" attachedToRef="scriptTask">
      <extensionElements>
        <flowable:jobCategory>timerCategory</flowable:jobCategory>
      </extensionElements>
      <timerEventDefinition>
        <timeDuration>PT1M</timeDuration>
      </timerEventDefinition>
    </boundaryEvent>
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let process = model.main_process.as_ref().unwrap();

    let FlowElementEnum::ScriptTask(script_task) = process
        .flow_element_map
        .get("scriptTask")
        .expect("script task should be indexed")
    else {
        panic!("scriptTask should be a script task");
    };
    let script_extensions = &script_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .extension_elements;
    let job_category = script_extensions
        .get("jobCategory")
        .and_then(|elements| elements.first())
        .expect("script jobCategory should be preserved");
    assert_eq!(
        job_category.element_text.as_deref(),
        Some("${categoryValue}")
    );
    assert_eq!(job_category.namespace_prefix.as_deref(), Some("flowable"));
    assert_eq!(
        job_category.namespace.as_deref(),
        Some("http://flowable.org/bpmn")
    );
    assert_eq!(
        script_extensions
            .get("customExtension")
            .and_then(|elements| elements.first())
            .and_then(|element| element.element_text.as_deref()),
        Some("preserved")
    );

    let FlowElementEnum::BoundaryEvent(boundary_event) = process
        .flow_element_map
        .get("timerBoundary")
        .expect("boundary event should be indexed")
    else {
        panic!("timerBoundary should be a boundary event");
    };
    assert_eq!(
        boundary_event
            .event
            .flow_node
            .flow_element
            .base_element
            .extension_elements
            .get("jobCategory")
            .and_then(|elements| elements.first())
            .and_then(|element| element.element_text.as_deref()),
        Some("timerCategory")
    );
}

#[test]
fn parses_call_activity_binding_business_key_and_expression_mappings() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn">
  <process id="process" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="callActivity" />
    <callActivity id="callActivity"
                  calledElement="${childKey}"
                  flowable:calledElementBinding="deployment"
                  flowable:calledElementType="key"
                  flowable:businessKey="${attrBusinessKey}"
                  flowable:inheritBusinessKey="true"
                  flowable:processInstanceName="${childName}"
                  flowable:processInstanceIdVariableName="childInstanceId"
                  flowable:fallbackToDefaultTenant="true"
                  flowable:useLocalScopeForOutParameters="true"
                  flowable:completeAsync="true">
      <extensionElements>
        <flowable:in sourceExpression="${parentInput}" targetExpression="childInput" />
        <flowable:out sourceExpression="${childResult}" target="parentResult" transient="true" />
      </extensionElements>
    </callActivity>
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let process = model.main_process.as_ref().unwrap();
    let element = process
        .flow_element_map
        .get("callActivity")
        .expect("call activity should be indexed");
    let FlowElementEnum::CallActivity(call_activity) = element else {
        panic!("callActivity should parse as CallActivity");
    };

    assert_eq!(call_activity.called_element.as_deref(), Some("${childKey}"));
    assert_eq!(call_activity.called_element_type.as_deref(), Some("key"));
    assert!(call_activity.same_deployment);
    assert!(call_activity.inherit_business_key);
    assert_eq!(
        call_activity.business_key.as_deref(),
        Some("${attrBusinessKey}")
    );
    assert_eq!(
        call_activity.process_instance_name.as_deref(),
        Some("${childName}")
    );
    assert_eq!(
        call_activity.process_instance_id_variable_name.as_deref(),
        Some("childInstanceId")
    );
    assert_eq!(call_activity.fallback_to_default_tenant, Some(true));
    assert!(call_activity.use_local_scope_for_out_parameters);
    assert!(call_activity.complete_async);
    assert_eq!(call_activity.in_parameters.len(), 1);
    assert_eq!(
        call_activity.in_parameters[0].source_expression.as_deref(),
        Some("${parentInput}")
    );
    assert_eq!(
        call_activity.in_parameters[0].target_expression.as_deref(),
        Some("childInput")
    );
    assert_eq!(call_activity.out_parameters.len(), 1);
    assert_eq!(
        call_activity.out_parameters[0].source_expression.as_deref(),
        Some("${childResult}")
    );
    assert!(call_activity.out_parameters[0].transient);
}

#[test]
fn malformed_xml_returns_invalid_bpmn_xml_error() {
    let converter = BpmnXMLConverter::new();

    let err = converter
        .try_convert_to_bpmn_model(
            r#"<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"><process id="broken">"#,
        )
        .expect_err("malformed XML must be rejected instead of returning a partial model");

    assert!(matches!(err, FlowableError::InvalidBpmnXml { .. }));
}

#[test]
fn timer_event_definition_parses_flowable_end_date_on_time_cycle() {
    // Java TimeCycleParser: ATTRIBUTE_END_DATE on the timeCycle element.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn">
  <process id="process" isExecutable="true">
    <startEvent id="timerStart">
      <timerEventDefinition>
        <timeCycle flowable:endDate="2026-12-12T00:00:05Z">R10/PT24H</timeCycle>
      </timerEventDefinition>
    </startEvent>
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let process = model.main_process.as_ref().expect("main process");
    let start = process
        .flow_elements
        .iter()
        .find_map(|fe| match fe {
            FlowElementEnum::StartEvent(se) => Some(se),
            _ => None,
        })
        .expect("start event");
    let timer = start
        .event
        .event_definitions
        .iter()
        .find_map(|d| match d {
            EventDefinitionEnum::TimerEventDefinition(t) => Some(t),
            _ => None,
        })
        .expect("timer def");
    assert_eq!(timer.time_cycle.as_deref(), Some("R10/PT24H"));
    assert_eq!(
        timer.end_date.as_deref(),
        Some("2026-12-12T00:00:05Z"),
        "flowable:endDate on timeCycle must populate TimerEventDefinition.end_date"
    );
}

/// P64 guard: Java `TimerEventDefinitionParser` reads the business calendar from
/// the `flowable:businessCalendarName` attribute on `<timerEventDefinition>`, and
/// `BpmnXMLConverter` also accepts a nested `<calendar>` element. Both must reach
/// `TimerEventDefinition.calendar_name` verbatim — the raw expression, not a
/// resolved name (ADR-2) — because timer scheduling selects the calendar from it.
#[test]
fn business_calendar_name_attribute_populates_timer_calendar_name() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn">
  <process id="process" isExecutable="true">
    <startEvent id="timerStart">
      <timerEventDefinition flowable:businessCalendarName="${calendarSelector}">
        <timeDuration>PT1H</timeDuration>
      </timerEventDefinition>
    </startEvent>
  </process>
</definitions>"#;

    let timer_calendar = extract_start_timer_calendar_name(xml);
    assert_eq!(
        timer_calendar.as_deref(),
        Some("${calendarSelector}"),
        "flowable:businessCalendarName must be captured raw, without evaluating the expression"
    );
}

#[test]
fn calendar_child_element_populates_timer_calendar_name() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process" isExecutable="true">
    <startEvent id="timerStart">
      <timerEventDefinition>
        <calendar>customCalendar</calendar>
        <timeDuration>PT1H</timeDuration>
      </timerEventDefinition>
    </startEvent>
  </process>
</definitions>"#;

    let timer_calendar = extract_start_timer_calendar_name(xml);
    assert_eq!(
        timer_calendar.as_deref(),
        Some("customCalendar"),
        "<calendar> element must populate TimerEventDefinition.calendar_name"
    );
}

#[test]
fn timer_without_calendar_leaves_calendar_name_absent() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process" isExecutable="true">
    <startEvent id="timerStart">
      <timerEventDefinition>
        <timeDuration>PT1H</timeDuration>
      </timerEventDefinition>
    </startEvent>
  </process>
</definitions>"#;

    assert_eq!(
        extract_start_timer_calendar_name(xml),
        None,
        "an unmodelled calendar must stay None so the kind default calendar applies"
    );
}

/// Java `StartEvent` carries `formProperties` just like `UserTask`, and the
/// `formPropertiesProcess.bpmn` fixture declares eight of them on its start
/// event. The Rust model dropped them on the floor: `parse_extensions_into_event`
/// read `flowable:formProperty` only to discard it, so a start-event form
/// definition disappeared on every parse and could never be written back.
#[test]
fn start_event_extension_elements_carry_form_properties() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="startForm" isExecutable="true">
    <startEvent id="start">
      <extensionElements>
        <flowable:formProperty id="amount" name="Amount" type="long" variable="amount"
                               required="true" writable="false"/>
        <flowable:formProperty id="kind" name="Kind" type="enum">
          <flowable:value id="sick" name="Sick leave"/>
          <flowable:value id="paid" name="Paid leave"/>
        </flowable:formProperty>
      </extensionElements>
    </startEvent>
  </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let process = model.main_process.as_ref().expect("main process");
    let FlowElementEnum::StartEvent(start) =
        process.flow_element_map.get("start").expect("start event")
    else {
        panic!("start should be a start event");
    };

    let properties = &start.form_properties;
    assert_eq!(properties.len(), 2, "{properties:?}");

    let amount = &properties[0];
    assert_eq!(amount.base_element.id.as_deref(), Some("amount"));
    assert_eq!(amount.name.as_deref(), Some("Amount"));
    assert_eq!(amount.property_type.as_deref(), Some("long"));
    assert_eq!(amount.variable.as_deref(), Some("amount"));
    assert!(amount.required, "required=\"true\" must be honoured");
    assert!(amount.readable, "readable defaults to true when unspecified");
    assert!(!amount.writeable, "writable=\"false\" must be honoured");

    let kind = &properties[1];
    assert_eq!(kind.property_type.as_deref(), Some("enum"));
    assert_eq!(
        kind.form_values
            .iter()
            .map(|value| value.base_element.id.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["sick", "paid"],
        "enum options must survive the parse"
    );

    // The writer has to emit them back, otherwise the modeler loses the form on save.
    let xml_out = flowable_bpmn_converter::write_bpmn_model(&model).expect("write");
    assert!(
        xml_out.contains(r#"id="amount""#) && xml_out.contains(r#"variable="amount""#),
        "start-event form properties must round-trip through the writer: {xml_out}"
    );
    let reparsed = converter.convert_to_bpmn_model(&xml_out);
    let reparsed_process = reparsed.main_process.as_ref().expect("main process");
    let FlowElementEnum::StartEvent(reparsed_start) = reparsed_process
        .flow_element_map
        .get("start")
        .expect("start event")
    else {
        panic!("start should be a start event");
    };
    assert_eq!(reparsed_start.form_properties.len(), 2);
    assert!(
        !reparsed_start.form_properties[0].writeable,
        "the writer emits writable=\"false\"; the parser must read that spelling back"
    );
}

fn extract_start_timer_calendar_name(xml: &str) -> Option<String> {
    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(xml);
    let process = model.main_process.as_ref().expect("main process");
    let start = process
        .flow_elements
        .iter()
        .find_map(|fe| match fe {
            FlowElementEnum::StartEvent(se) => Some(se),
            _ => None,
        })
        .expect("start event");
    start
        .event
        .event_definitions
        .iter()
        .find_map(|d| match d {
            EventDefinitionEnum::TimerEventDefinition(t) => Some(t),
            _ => None,
        })
        .expect("timer def")
        .calendar_name
        .clone()
}
