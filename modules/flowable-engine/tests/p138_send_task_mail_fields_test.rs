//! P138 (G12) — `<sendTask flowable:type="mail">` exercises the full P124/P134
//! mail field surface (textVar/htmlVar indirection, mixed JUEL composite
//! templates, `\${` escape).
//!
//! The sendTask mail path dispatches into the shared
//! `execute_mail_service_task` helper (`send_task_activity_behavior.rs`),
//! which contains the same `resolve_mail_body_field` textVar/htmlVar logic as
//! serviceTask mail (`service_task_activity_behavior.rs`). Existing sendTask
//! tests (`send_task_runtime_test.rs`) only use static literal fields, and the
//! P134 composite tests only use serviceTask — this file closes the crossed
//! gap. Java: `SendTaskParseHandler.java:37-56` assigns MailActivityBehavior
//! for type=mail, so the full `BaseMailActivityDelegate` field surface applies
//! to sendTask identically.
//!
//! P138: mail has no resultVariableName consumer in Java
//! (`DefaultActivityBehaviorFactory.java:242-244`); sendTask already discarded
//! the send payload. Assertions use outbox records, not process variables.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::MailOutboxStatus;
use serde_json::json;

/// sendTask mail: inline `text`/`html` fields with mixed JUEL composite
/// templates expand `${...}` segments and honor the `\${` escape, and the
/// process continues past the sendTask.
#[test]
fn send_task_mail_inline_composite_juel() {
    let process_engine = ProcessEngine::new("p138-sendtask-inline".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    // (b) keep resultVariableName — assert variable is absent (P138 Java parity).
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="sendTaskInlineComposite" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="mailTask" />
            <sendTask id="mailTask" flowable:type="mail" flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>Inline composite</flowable:subject>
                    <flowable:text>Hello ${gender}; show \${literal}</flowable:text>
                    <flowable:html><![CDATA[<b>${gender}</b> done]]></flowable:html>
                </extensionElements>
            </sendTask>
            <sequenceFlow id="f2" sourceRef="mailTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("send_task_inline_composite.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("gender".to_string(), json!("Mx")),
        )
        .unwrap();

    assert!(
        runtime_service
            .get_variable(pi.id.clone(), "mailResult".to_string())
            .unwrap()
            .is_none(),
        "sendTask mail must not write resultVariableName (Java DefaultActivityBehaviorFactory.java:242-244)"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    assert_eq!(outbox[0].body, "Hello Mx; show ${literal}");
    assert_eq!(outbox[0].html_body.as_deref(), Some("<b>Mx</b> done"));

    let stored = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .expect("process instance should be in runtime store");
    assert!(stored.is_ended, "sendTask mail should pass through to the end event");
}

/// sendTask mail: `textVar`/`htmlVar` indirect the body through process
/// variables, whose contents are themselves composite JUEL templates.
#[test]
fn send_task_mail_text_var_html_var_composite() {
    let process_engine = ProcessEngine::new("p138-sendtask-var".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="sendTaskVarComposite" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="mailTask" />
            <sendTask id="mailTask" flowable:type="mail" flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>Var composite</flowable:subject>
                    <flowable:textVar>bodyTemplate</flowable:textVar>
                    <flowable:htmlVar>htmlTemplate</flowable:htmlVar>
                </extensionElements>
            </sendTask>
            <sequenceFlow id="f2" sourceRef="mailTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("send_task_var_composite.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let body_template = format!("Hello {}; show {}", "${gender}", r"\${literal}");
    let html_template = format!("<b>{}</b> done", "${gender}");

    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("gender".to_string(), json!("Mx"))
                .variable("bodyTemplate".to_string(), json!(body_template))
                .variable("htmlTemplate".to_string(), json!(html_template)),
        )
        .unwrap();

    assert!(
        runtime_service
            .get_variable(pi.id, "mailResult".to_string())
            .unwrap()
            .is_none(),
        "sendTask mail must not write resultVariableName (P138 Java parity)"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    assert_eq!(outbox[0].body, "Hello Mx; show ${literal}");
    assert_eq!(outbox[0].html_body.as_deref(), Some("<b>Mx</b> done"));
}
