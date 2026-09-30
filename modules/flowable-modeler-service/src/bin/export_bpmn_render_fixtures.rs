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
        .unwrap_or_else(|| {
            fatal("usage: export_bpmn_render_fixtures <output-directory>".to_string())
        });
    fs::create_dir_all(&output)
        .unwrap_or_else(|error| fatal(format!("create render fixture output directory: {error}")));
    for entry in fs::read_dir(&output)
        .unwrap_or_else(|error| fatal(format!("read render fixture output directory: {error}")))
    {
        let path = entry
            .unwrap_or_else(|error| fatal(format!("read render fixture entry: {error}")))
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            fs::remove_file(path)
                .unwrap_or_else(|error| fatal(format!("remove stale render fixture: {error}")));
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

/// Startup-fatal process boundary: print the cause and exit non-zero,
/// matching Java top-level fatal semantics (an exception in `main`
/// propagates to the top level instead of being swallowed).
fn fatal(message: String) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
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
