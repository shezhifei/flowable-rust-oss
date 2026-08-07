use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};

#[test]
fn test_advanced_events_and_data_associations() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
             targetNamespace="http://flowable.org/bpmn">
    <process id="advancedProcess" isExecutable="true">
        <startEvent id="start" />
        
        <intermediateCatchEvent id="catchCond">
            <conditionalEventDefinition>
                <condition xsi:type="tFormalExpression">${var == 1}</condition>
            </conditionalEventDefinition>
        </intermediateCatchEvent>
        
        <intermediateThrowEvent id="throwLink">
            <linkEventDefinition name="LinkA" />
        </intermediateThrowEvent>
        
        <intermediateCatchEvent id="catchLink">
            <linkEventDefinition name="LinkA" />
        </intermediateCatchEvent>
        
        <endEvent id="endCancel">
            <cancelEventDefinition />
        </endEvent>
        
        <endEvent id="endError">
            <errorEventDefinition errorRef="error1" />
        </endEvent>
        
        <intermediateThrowEvent id="throwComp">
            <compensateEventDefinition activityRef="task1" />
        </intermediateThrowEvent>
        
        <task id="task1">
            <dataInputAssociation id="dia1">
                <sourceRef>varIn</sourceRef>
                <targetRef>taskVarIn</targetRef>
            </dataInputAssociation>
            <dataOutputAssociation id="doa1">
                <sourceRef>taskVarOut</sourceRef>
                <targetRef>varOut</targetRef>
            </dataOutputAssociation>
        </task>
    </process>
</definitions>"#;

    let converter = BpmnXMLConverter::new();
    let bpmn_model = converter.convert_to_bpmn_model(xml);

    let process = bpmn_model
        .main_process
        .as_ref()
        .expect("Should have main process");

    let flow_elements = &process.flow_elements;

    // Helper closure to find element by id
    let find_element = |id: &str| -> &FlowElementEnum {
        flow_elements
            .iter()
            .find(|e| match e {
                FlowElementEnum::IntermediateCatchEvent(ev) => {
                    ev.event.flow_node.flow_element.base_element.id.as_deref() == Some(id)
                }
                FlowElementEnum::IntermediateThrowEvent(ev) => {
                    ev.event.flow_node.flow_element.base_element.id.as_deref() == Some(id)
                }
                FlowElementEnum::EndEvent(ev) => {
                    ev.event.flow_node.flow_element.base_element.id.as_deref() == Some(id)
                }
                FlowElementEnum::Task(task) => {
                    task.activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_deref()
                        == Some(id)
                }
                _ => false,
            })
            .unwrap_or_else(|| panic!("Could not find element {}", id))
    };

    // 1. ConditionalEventDefinition
    if let FlowElementEnum::IntermediateCatchEvent(catch_cond) = find_element("catchCond") {
        let defs = &catch_cond.event.event_definitions;
        assert_eq!(defs.len(), 1);
        if let EventDefinitionEnum::ConditionalEventDefinition(cond) = &defs[0] {
            assert_eq!(cond.condition_expression.as_deref(), Some("${var == 1}"));
        } else {
            panic!("Expected ConditionalEventDefinition");
        }
    } else {
        panic!("catchCond is wrong type");
    }

    // 2. LinkEventDefinition (throw)
    if let FlowElementEnum::IntermediateThrowEvent(throw_link) = find_element("throwLink") {
        let defs = &throw_link.event.event_definitions;
        assert_eq!(defs.len(), 1);
        if let EventDefinitionEnum::LinkEventDefinition(link) = &defs[0] {
            assert_eq!(link.name.as_deref(), Some("LinkA"));
        } else {
            panic!("Expected LinkEventDefinition");
        }
    } else {
        panic!("throwLink is wrong type");
    }

    // 3. CancelEventDefinition
    if let FlowElementEnum::EndEvent(end_cancel) = find_element("endCancel") {
        let defs = &end_cancel.event.event_definitions;
        assert_eq!(defs.len(), 1);
        if let EventDefinitionEnum::CancelEventDefinition(_) = &defs[0] {
            // Check passes
        } else {
            panic!("Expected CancelEventDefinition");
        }
    } else {
        panic!("endCancel is wrong type");
    }

    // 4. ErrorEventDefinition
    if let FlowElementEnum::EndEvent(end_error) = find_element("endError") {
        let defs = &end_error.event.event_definitions;
        assert_eq!(defs.len(), 1);
        if let EventDefinitionEnum::ErrorEventDefinition(err) = &defs[0] {
            assert_eq!(err.error_ref.as_deref(), Some("error1"));
        } else {
            panic!("Expected ErrorEventDefinition");
        }
    } else {
        panic!("endError is wrong type");
    }

    // 5. CompensateEventDefinition
    if let FlowElementEnum::IntermediateThrowEvent(throw_comp) = find_element("throwComp") {
        let defs = &throw_comp.event.event_definitions;
        assert_eq!(defs.len(), 1);
        if let EventDefinitionEnum::CompensateEventDefinition(comp) = &defs[0] {
            assert_eq!(comp.activity_ref.as_deref(), Some("task1"));
        } else {
            panic!("Expected CompensateEventDefinition");
        }
    } else {
        panic!("throwComp is wrong type");
    }

    // 6. Data Associations
    if let FlowElementEnum::Task(task) = find_element("task1") {
        let dia = &task.activity.data_input_associations;
        assert_eq!(dia.len(), 1);
        assert_eq!(dia[0].source_ref.as_deref(), Some("varIn"));
        assert_eq!(dia[0].target_ref.as_deref(), Some("taskVarIn"));

        let doa = &task.activity.data_output_associations;
        assert_eq!(doa.len(), 1);
        assert_eq!(doa[0].source_ref.as_deref(), Some("taskVarOut"));
        assert_eq!(doa[0].target_ref.as_deref(), Some("varOut"));
    } else {
        panic!("task1 is wrong type");
    }
}
