use flowable_modeler_service::{decode_bpmn_xml, layout_bpmn};
use std::{env, fs, path::PathBuf};

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
    // Two participants: the only fixture that exercises multi-pool layout and the
    // panel's participant switcher.
    "messageflow.bpmn",
];

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: export_bpmn_render_fixtures <output-directory>");
    fs::create_dir_all(&output).expect("create render fixture output directory");
    for entry in fs::read_dir(&output).expect("read render fixture output directory") {
        let path = entry.expect("read render fixture entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            fs::remove_file(path).expect("remove stale render fixture");
        }
    }

    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../flowable-bpmn-converter/tests/resources/java_fixtures");
    for (index, fixture) in FIXTURES.iter().enumerate() {
        let xml = fs::read_to_string(source.join(fixture))
            .unwrap_or_else(|error| panic!("read {fixture}: {error}"));
        let document =
            decode_bpmn_xml(&xml).unwrap_or_else(|error| panic!("decode {fixture}: {error}"));
        let document = layout_bpmn(&document)
            .unwrap_or_else(|error| panic!("complete layout for {fixture}: {error}"));
        let name = format!("{:02}-{}.json", index + 1, sanitize(fixture));
        let bytes = serde_json::to_vec_pretty(&document)
            .unwrap_or_else(|error| panic!("serialize {fixture}: {error}"));
        fs::write(output.join(&name), bytes)
            .unwrap_or_else(|error| panic!("write {name}: {error}"));
        println!("wrote {name}");
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
