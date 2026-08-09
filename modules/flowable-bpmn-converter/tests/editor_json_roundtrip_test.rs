use flowable_bpmn_converter::{BpmnXMLConverter, write_bpmn_model};
use flowable_bpmn_model::BpmnModel;
use serde_json::Value;
use std::{fs, path::Path};

const FIXTURES: &[&str] = &[
    "simplemodel.bpmn",
    "usertaskmodel.bpmn",
    "servicetaskmodel.bpmn",
    "BoundaryTimerEventTest.testBoundaryTimerEvent.bpmn20.xml",
    "pools.bpmn",
    "conditionaltest.bpmn",
    "callactivity_attributes.bpmn",
    "multiinstancemodel.bpmn",
    "subprocessmodel_with_extensions.bpmn",
    "BusinessRuleTaskTest.testBusinessRuleTask.bpmn20.xml",
    "message.bpmn",
    "signaltest.bpmn",
    "asyncendeventmodel.bpmn",
    "boundaryErrorEventWithInParameters.bpmn",
    "httpServiceTaskWithParallelInSameTransactionModel.bpmn",
    "dataobjectmodel.bpmn",
    "script-task-input-parameters.xml",
    "eventgatewaymodel.bpmn",
    "adhocsubprocess.bpmn",
    "externalWorkerServiceTask.bpmn",
];

#[test]
fn editor_json_and_xml_roundtrip_preserves_representative_models() {
    let converter = BpmnXMLConverter::new();
    for fixture in FIXTURES {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/resources/java_fixtures")
            .join(fixture);
        let original_xml = fs::read_to_string(&path).unwrap();
        let original = converter.try_convert_to_bpmn_model(&original_xml).unwrap();

        // Simulate the exact wire boundary used by the browser before writing XML.
        let editor_json = serde_json::to_vec(&original).unwrap();
        let editor_model: BpmnModel = serde_json::from_slice(&editor_json).unwrap();
        let written_xml = write_bpmn_model(&editor_model).unwrap();
        let reparsed = converter
            .try_convert_to_bpmn_model(&written_xml)
            .unwrap_or_else(|error| {
                panic!("{fixture} generated invalid XML: {error:?}\n{written_xml}")
            });

        let expected = semantic_value(&converter, &original);
        let actual = semantic_value(&converter, &reparsed);
        assert_eq!(
            actual, expected,
            "semantic mismatch for {fixture}\n{written_xml}"
        );
    }
}

fn semantic_value(converter: &BpmnXMLConverter, model: &BpmnModel) -> Value {
    let mut value = converter.to_canonical_contract_value(model);
    normalize(&mut value);
    value
}

fn normalize(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let generated_event_definition_id = object.contains_key("timeDate")
                || object.contains_key("timeDuration")
                || object.contains_key("timeCycle")
                || object.contains_key("errorCode")
                || object.contains_key("signalRef")
                || object.contains_key("messageRef");
            if generated_event_definition_id {
                object.remove("id");
            }
            if object.contains_key("fieldName") {
                object.remove("id");
            }
            if object.contains_key("transient")
                && (object.contains_key("source")
                    || object.contains_key("sourceExpression")
                    || object.contains_key("target"))
            {
                object.remove("id");
            }
            for key in [
                "xmlRowNumber",
                "xmlColumnNumber",
                "mainProcess",
                "flowElementMap",
                "artifactMap",
                "incomingFlows",
                "outgoingFlows",
                "namespaces",
                "edgeMap",
            ] {
                object.remove(key);
            }
            for child in object.values_mut() {
                normalize(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize(item);
            }
        }
        _ => {}
    }
}
