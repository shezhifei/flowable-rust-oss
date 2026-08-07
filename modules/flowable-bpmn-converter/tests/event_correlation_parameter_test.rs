//! P93: bpmn-converter parses `flowable:eventCorrelationParameter` (and
//! `eventType` / `startEventCorrelationConfiguration`) on start / boundary /
//! intermediateCatch / receiveTask.
//!
//! Java element name is `eventCorrelationParameter` with attributes `name` /
//! `value` (BpmnXMLConstants.ELEMENT_EVENT_CORRELATION_PARAMETER; fixtures under
//! BpmnEventRegistryConsumerTest.*.bpmn20.xml). Generic extension parsing
//! already stores these; this test locks the contract.

use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::FlowElementEnum;

fn attr(el: &flowable_bpmn_model::model::ExtensionElement, name: &str) -> Option<String> {
    el.base_element
        .attributes
        .get(name)
        .and_then(|attrs| attrs.first())
        .and_then(|a| a.value.clone())
}

#[test]
fn parses_event_correlation_parameter_on_start_boundary_catch_receive() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="corrProcess" isExecutable="true">
    <startEvent id="theStart">
      <extensionElements>
        <flowable:eventType>myEvent</flowable:eventType>
        <flowable:eventCorrelationParameter name="customerId" value="testCustomer"/>
        <flowable:startEventCorrelationConfiguration>storeAsUniqueReferenceId</flowable:startEventCorrelationConfiguration>
      </extensionElements>
    </startEvent>
    <sequenceFlow sourceRef="theStart" targetRef="task"/>
    <userTask id="task"/>
    <boundaryEvent id="eventBoundary" attachedToRef="task">
      <extensionElements>
        <flowable:eventType>myEvent</flowable:eventType>
        <flowable:eventCorrelationParameter name="customerId" value="${customerIdVar}"/>
      </extensionElements>
    </boundaryEvent>
    <sequenceFlow sourceRef="task" targetRef="catchEvent"/>
    <intermediateCatchEvent id="catchEvent">
      <extensionElements>
        <flowable:eventType>myEvent</flowable:eventType>
        <flowable:eventCorrelationParameter name="customerId" value="${customerIdVar}"/>
        <flowable:eventCorrelationParameter name="orderId" value="${orderIdVar}"/>
      </extensionElements>
    </intermediateCatchEvent>
    <sequenceFlow sourceRef="catchEvent" targetRef="receive"/>
    <receiveTask id="receive">
      <extensionElements>
        <flowable:eventType>myEvent</flowable:eventType>
        <flowable:eventCorrelationParameter name="customerId" value="${customerIdVar}"/>
      </extensionElements>
    </receiveTask>
    <sequenceFlow sourceRef="receive" targetRef="theEnd"/>
    <endEvent id="theEnd"/>
  </process>
</definitions>"#;

    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.expect("main process");

    // --- startEvent ---
    let FlowElementEnum::StartEvent(start) = process
        .flow_element_map
        .get("theStart")
        .expect("start")
    else {
        panic!("theStart should be StartEvent");
    };
    let start_ext = &start.event.flow_node.flow_element.base_element.extension_elements;
    assert_eq!(
        start_ext
            .get("eventType")
            .and_then(|v| v.first())
            .and_then(|e| e.element_text.as_deref()),
        Some("myEvent")
    );
    let start_corr = start_ext
        .get("eventCorrelationParameter")
        .expect("start correlation params");
    assert_eq!(start_corr.len(), 1);
    assert_eq!(attr(&start_corr[0], "name").as_deref(), Some("customerId"));
    assert_eq!(
        attr(&start_corr[0], "value").as_deref(),
        Some("testCustomer")
    );
    assert_eq!(
        start_ext
            .get("startEventCorrelationConfiguration")
            .and_then(|v| v.first())
            .and_then(|e| e.element_text.as_deref()),
        Some("storeAsUniqueReferenceId")
    );

    // --- boundaryEvent ---
    let FlowElementEnum::BoundaryEvent(boundary) = process
        .flow_element_map
        .get("eventBoundary")
        .expect("boundary")
    else {
        panic!("eventBoundary should be BoundaryEvent");
    };
    let boundary_corr = boundary
        .event
        .flow_node
        .flow_element
        .base_element
        .extension_elements
        .get("eventCorrelationParameter")
        .expect("boundary correlation");
    assert_eq!(boundary_corr.len(), 1);
    assert_eq!(
        attr(&boundary_corr[0], "value").as_deref(),
        Some("${customerIdVar}")
    );

    // --- intermediateCatchEvent ---
    let FlowElementEnum::IntermediateCatchEvent(catch) = process
        .flow_element_map
        .get("catchEvent")
        .expect("catch")
    else {
        panic!("catchEvent should be IntermediateCatchEvent");
    };
    let catch_corr = catch
        .event
        .flow_node
        .flow_element
        .base_element
        .extension_elements
        .get("eventCorrelationParameter")
        .expect("catch correlation");
    assert_eq!(catch_corr.len(), 2);
    assert_eq!(attr(&catch_corr[0], "name").as_deref(), Some("customerId"));
    assert_eq!(attr(&catch_corr[1], "name").as_deref(), Some("orderId"));

    // --- receiveTask ---
    let FlowElementEnum::ReceiveTask(receive) = process
        .flow_element_map
        .get("receive")
        .expect("receive")
    else {
        panic!("receive should be ReceiveTask");
    };
    let receive_corr = receive
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .extension_elements
        .get("eventCorrelationParameter")
        .expect("receive correlation");
    assert_eq!(receive_corr.len(), 1);
    assert_eq!(
        attr(&receive_corr[0], "name").as_deref(),
        Some("customerId")
    );
}
