//! P112 — converter parses `flowable:historyLevel` process extension element.
//!
//! Java reads `process.getExtensionElements().get("historyLevel")` text
//! (`DefaultHistoryConfigurationSettings.getProcessDefinitionHistoryLevel:68-73`).
//! The converter already stores unknown flowable extension elements generically;
//! this contract pins the historyLevel key + element text.

use flowable_bpmn_converter::BpmnXMLConverter;

#[test]
fn converter_parses_process_history_level_extension() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="oneTaskProcess" name="The One Task Process" isExecutable="true">
            <extensionElements>
                <flowable:historyLevel>activity</flowable:historyLevel>
            </extensionElements>
            <startEvent id="theStart" />
            <sequenceFlow id="flow1" sourceRef="theStart" targetRef="theTask" />
            <userTask id="theTask" name="my task" />
            <sequenceFlow id="flow2" sourceRef="theTask" targetRef="theEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#;

    let model = BpmnXMLConverter::new()
        .try_convert_to_bpmn_model(xml)
        .expect("convert");
    let process = model
        .processes
        .first()
        .expect("one process");
    let history = process
        .base_element
        .extension_elements
        .get("historyLevel")
        .and_then(|v| v.first())
        .expect("historyLevel extension");
    assert_eq!(history.element_text.as_deref(), Some("activity"));
}

#[test]
fn converter_parses_all_history_level_keys() {
    for key in ["none", "instance", "task", "activity", "audit", "full"] {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                         xmlns:flowable="http://flowable.org/bpmn"
                         targetNamespace="Examples">
                <process id="p" isExecutable="true">
                    <extensionElements>
                        <flowable:historyLevel>{key}</flowable:historyLevel>
                    </extensionElements>
                    <startEvent id="s" />
                    <endEvent id="e" />
                    <sequenceFlow id="f" sourceRef="s" targetRef="e" />
                </process>
            </definitions>"#
        );
        let model = BpmnXMLConverter::new()
            .try_convert_to_bpmn_model(&xml)
            .unwrap_or_else(|e| panic!("convert {key}: {e:?}"));
        let text = model
            .processes[0]
            .base_element
            .extension_elements
            .get("historyLevel")
            .and_then(|v| v.first())
            .and_then(|e| e.element_text.as_deref());
        assert_eq!(text, Some(key), "key={key}");
    }
}
