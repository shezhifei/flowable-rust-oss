use flowable_bpmn_converter::{BpmnXMLConverter, write_bpmn_model};
use flowable_bpmn_model::BpmnModel;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Converter models copied from Java, under `tests/resources/java_fixtures`.
///
/// Every fixture in that directory is in the gate — nothing is skipped, and
/// `semantic_value` below is strict, so any writer construct one of these models
/// carries has to survive the round-trip.
const JAVA_FIXTURES: &[&str] = &[
    "BoundaryTimerEventTest.testBoundaryTimerEvent.bpmn20.xml",
    "BusinessRuleTaskTest.testBusinessRuleTask.bpmn20.xml",
    "InclusiveGatewayTest.testDecisionFunctionality.bpmn20.xml",
    "ReceiveTaskTest.testWaitStateBehavior.bpmn20.xml",
    "activityWithDataAssociations.bpmn",
    "adhocsubprocess.bpmn",
    "asyncendeventmodel.bpmn",
    "boundaryErrorEventWithInParameters.bpmn",
    "callactivity.bpmn",
    "callactivityFallbackValueFalse.bpmn",
    "callactivityNoFallbackValue.bpmn",
    "callactivity_attributes.bpmn",
    "conditionaltest.bpmn",
    "customextensionsmodel.bpmn",
    "dataobjectmodel.bpmn",
    "datastore.bpmn",
    "eventgatewaymodel.bpmn",
    "extensions.bpmn20.xml",
    "extensionsXmlLocation.bpmn20.xml",
    "externalWorkerServiceTask.bpmn",
    "formAwareServiceTask.bpmn",
    "formPropertiesProcess.bpmn",
    "httpServiceTaskWithParallelInSameTransactionModel.bpmn",
    "message.bpmn",
    "messageflow.bpmn",
    "multiInstanceVariableAggregationsModel.bpmn",
    "multiinstancemodel.bpmn",
    "notexecutablemodel.bpmn",
    "parallelgatewaymodel.bpmn",
    "pool-with-extensions.bpmn",
    "pools.bpmn",
    "scopedmodel.bpmn",
    "script-task-do-not-include-variables.xml",
    "script-task-input-parameters.xml",
    "servicetaskmodel.bpmn",
    "signalExpressionTest.bpmn",
    "signaltest.bpmn",
    "simplemodel.bpmn",
    "subprocessmodel-noDI.bpmn",
    "subprocessmodel.bpmn",
    "subprocessmodel_with_extensions.bpmn",
    "subprocessmultidiagrammodel.bpmn",
    "usertaskmodel.bpmn",
    "valueddataobjectmodel.bpmn",
];

/// Engine differential-test models, read in place at `differential/fixtures`
/// rather than copied here so the two suites cannot drift apart. These carry no
/// diagram interchange, which is exactly the shape the modeler produces for a
/// model it has never laid out.
///
/// `timers/empty_timer.bpmn20.xml` is left out: an unconfigured
/// `<timerEventDefinition/>` has no id in the XML and no timer field for
/// `normalize` to key off, so each parse invents a different generated id and
/// there is nothing stable to compare. Nothing is lost in the round-trip there.
const ENGINE_FIXTURES: &[&str] = &[
    "boundary/error_one_shot.bpmn20.xml",
    "boundary/interrupt_keeps_event_sub_timer.bpmn20.xml",
    "boundary/non_interrupt_message.bpmn20.xml",
    "gateways/exclusive_no_outgoing.bpmn20.xml",
    "gateways/inclusive_join_boundary.bpmn20.xml",
    "gateways/inclusive_join_terminate.bpmn20.xml",
    "gateways/parallel_join.bpmn20.xml",
    "http/async_retry.bpmn20.xml",
    "http/automatic_async_retry.bpmn20.xml",
    "http/cancel_job.bpmn20.xml",
    "http/fail_status.bpmn20.xml",
    "http/handled_status.bpmn20.xml",
    "http/ignore_exception.bpmn20.xml",
    "http/nested_unrecoverable.bpmn20.xml",
    "http/request_response.bpmn20.xml",
    "http/smoke_user_task.bpmn20.xml",
    "http/uncaught_handled_status.bpmn20.xml",
    "http/unlock_owned_jobs.bpmn20.xml",
    "http/unrecoverable.bpmn20.xml",
    "task_queries/candidate_or_assigned.bpmn20.xml",
    "task_queries/exclude_assigned.bpmn20.xml",
    "task_queries/group_candidate_task.bpmn20.xml",
    "task_queries/or_query_tasks.bpmn20.xml",
    "tasks/simple_task.bpmn20.xml",
    "timers/async_user_task.bpmn20.xml",
    "timers/calendar_name_duration.bpmn20.xml",
    "timers/complete_async_call.bpmn20.xml",
    "timers/duration_timer.bpmn20.xml",
    "timers/repeat_timer_cycle.bpmn20.xml",
    "variables/parallel_tasks.bpmn20.xml",
    "variables/simple_task.bpmn20.xml",
];

/// Every fixture as a `(label, path)` pair.
fn fixtures() -> Vec<(String, PathBuf)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let java_root = manifest.join("tests/resources/java_fixtures");
    // `modules/flowable-bpmn-converter` → workspace root → `differential`.
    let engine_root = manifest.join("../../differential/fixtures");
    JAVA_FIXTURES
        .iter()
        .map(|name| ((*name).to_string(), java_root.join(name)))
        .chain(
            ENGINE_FIXTURES
                .iter()
                .map(|name| (format!("differential/{name}"), engine_root.join(name))),
        )
        .collect()
}

#[test]
fn editor_json_and_xml_roundtrip_preserves_representative_models() {
    let all = fixtures();
    assert!(
        all.len() >= 50,
        "need ≥50 round-trip fixtures, have {}",
        all.len()
    );

    let converter = BpmnXMLConverter::new();
    // Collect every mismatch instead of stopping at the first: with this many
    // fixtures a single panic hides the shape of a regression.
    let mut failures: Vec<String> = Vec::new();
    for (label, path) in &all {
        let original_xml = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("cannot read fixture {}: {error}", path.display()));
        let original = converter.try_convert_to_bpmn_model(&original_xml).unwrap();

        // Simulate the exact wire boundary used by the browser before writing XML.
        let editor_json = serde_json::to_vec(&original).unwrap();
        let editor_model: BpmnModel = serde_json::from_slice(&editor_json).unwrap();
        let written_xml = write_bpmn_model(&editor_model).unwrap();
        let reparsed = match converter.try_convert_to_bpmn_model(&written_xml) {
            Ok(model) => model,
            Err(error) => {
                failures.push(format!(
                    "{label} generated invalid XML: {error:?}\n{written_xml}"
                ));
                continue;
            }
        };

        let expected = semantic_value(&converter, &original);
        let actual = semantic_value(&converter, &reparsed);
        if actual != expected {
            failures.push(format!(
                "semantic mismatch for {label}\n{}\n{written_xml}",
                first_difference(&expected, &actual, String::new())
                    .unwrap_or_else(|| "difference is not in a shared key".to_string())
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} fixtures failed to round-trip:\n\n{}",
        failures.len(),
        all.len(),
        failures.join("\n\n")
    );
}

fn semantic_value(converter: &BpmnXMLConverter, model: &BpmnModel) -> Value {
    let mut value = converter.to_canonical_contract_value(model);
    normalize(&mut value);
    value
}

/// The first `path: expected != actual` leaf under `expected`, for a readable
/// failure message when two large contract values disagree.
fn first_difference(expected: &Value, actual: &Value, path: String) -> Option<String> {
    if expected == actual {
        return None;
    }
    match (expected, actual) {
        (Value::Object(left), Value::Object(right)) => {
            for (key, value) in left {
                let child = format!("{path}/{key}");
                match right.get(key) {
                    Some(other) => {
                        if let Some(found) = first_difference(value, other, child) {
                            return Some(found);
                        }
                    }
                    None => return Some(format!("{child}: missing after round-trip")),
                }
            }
            for key in right.keys() {
                if !left.contains_key(key) {
                    return Some(format!("{path}/{key}: added by round-trip"));
                }
            }
            None
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                return Some(format!(
                    "{path}: length {} became {}",
                    left.len(),
                    right.len()
                ));
            }
            for (index, (value, other)) in left.iter().zip(right).enumerate() {
                if let Some(found) = first_difference(value, other, format!("{path}[{index}]")) {
                    return Some(found);
                }
            }
            None
        }
        _ => Some(format!("{path}: {expected} became {actual}")),
    }
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
