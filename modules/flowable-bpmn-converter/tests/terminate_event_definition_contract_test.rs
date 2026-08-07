//! P15 contract: `terminateEventDefinition` parsing.
//!
//! Java reference: `TerminateEventDefinitionParser.java` — element
//! `terminateEventDefinition` (only applied to EndEvent parents), attributes
//! `terminateAll` / `terminateMultiInstance` are true only for the literal
//! string "true" (`BpmnXMLConstants:340-342`, read via
//! `BpmnXMLUtil.getAttributeValue`, which accepts both the
//! flowable-namespaced and un-namespaced attribute).

use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum, TerminateEventDefinition};

fn parse_end_event_terminate(xml: &str, end_event_id: &str) -> Option<TerminateEventDefinition> {
    let converter = BpmnXMLConverter::new();
    let model = converter.try_convert_to_bpmn_model(xml).unwrap();
    let process = model.main_process.as_ref().unwrap();
    let Some(FlowElementEnum::EndEvent(end_event)) = process.flow_element_map.get(end_event_id)
    else {
        panic!("end event '{end_event_id}' not found");
    };
    end_event
        .event
        .event_definitions
        .iter()
        .find_map(|def| match def {
            EventDefinitionEnum::TerminateEventDefinition(t) => Some(t.clone()),
            _ => None,
        })
}

fn process_xml(end_event_body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="terminateParse" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="terminateEnd" />
            <endEvent id="terminateEnd">
                {end_event_body}
            </endEvent>
        </process>
    </definitions>"#
    )
}

/// Self-closing definition without attributes: variant present, both flags
/// default to false.
#[test]
fn terminate_event_definition_defaults_to_false_flags() {
    let def =
        parse_end_event_terminate(&process_xml("<terminateEventDefinition />"), "terminateEnd")
            .expect("terminate event definition must be parsed");
    assert!(!def.terminate_all);
    assert!(!def.terminate_multi_instance);
}

/// Self-closing form with flowable-namespaced attributes set to "true".
#[test]
fn terminate_event_definition_parses_true_attributes() {
    let def = parse_end_event_terminate(
        &process_xml(
            r#"<terminateEventDefinition flowable:terminateAll="true" flowable:terminateMultiInstance="true" />"#,
        ),
        "terminateEnd",
    )
    .expect("terminate event definition must be parsed");
    assert!(def.terminate_all);
    assert!(def.terminate_multi_instance);
}

/// Non-self-closing element form is parsed the same way.
#[test]
fn terminate_event_definition_parses_non_empty_element_form() {
    let def = parse_end_event_terminate(
        &process_xml(
            r#"<terminateEventDefinition flowable:terminateAll="true"></terminateEventDefinition>"#,
        ),
        "terminateEnd",
    )
    .expect("terminate event definition must be parsed");
    assert!(def.terminate_all);
    assert!(!def.terminate_multi_instance);
}

/// Java parity: only the literal string "true" activates the flags
/// (`"true".equals(...)`) — "TRUE" and other values stay false.
#[test]
fn terminate_event_definition_requires_literal_true() {
    let def = parse_end_event_terminate(
        &process_xml(
            r#"<terminateEventDefinition flowable:terminateAll="TRUE" flowable:terminateMultiInstance="yes" />"#,
        ),
        "terminateEnd",
    )
    .expect("terminate event definition must be parsed");
    assert!(!def.terminate_all);
    assert!(!def.terminate_multi_instance);
}

/// Java `TerminateEventDefinitionParser` only applies to EndEvent parents: a
/// terminateEventDefinition on an intermediate throw event is ignored.
#[test]
fn terminate_event_definition_is_ignored_on_non_end_events() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="terminateOnThrow" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="throwEvent" />
            <intermediateThrowEvent id="throwEvent">
                <terminateEventDefinition flowable:terminateAll="true" />
            </intermediateThrowEvent>
            <sequenceFlow id="f2" sourceRef="throwEvent" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let converter = BpmnXMLConverter::new();
    let model = converter.try_convert_to_bpmn_model(xml).unwrap();
    let process = model.main_process.as_ref().unwrap();
    let Some(FlowElementEnum::IntermediateThrowEvent(throw_event)) =
        process.flow_element_map.get("throwEvent")
    else {
        panic!("throw event not found");
    };
    assert!(
        !throw_event
            .event
            .event_definitions
            .iter()
            .any(|def| { matches!(def, EventDefinitionEnum::TerminateEventDefinition(_)) }),
        "terminateEventDefinition must only be parsed for EndEvent parents"
    );
}
