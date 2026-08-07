//! P134/P124: mixed JUEL composite templates for mail text/html/textVar/htmlVar.
//!
//! Java: `ExpressionManager.createExpression` produces composite ValueExpressions
//! for mixed text; mail bodies use that via `BaseMailActivityDelegate.java:94-105`.
//!
//! P138: mail resultVariableName is a Rust-only super-set (Java mail has no
//! consumer). Assertions re-point to outbox records.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::MailOutboxStatus;
use serde_json::json;

#[test]
fn mail_task_composite_juel_on_text_and_html_fields() {
    let process_engine = ProcessEngine::new("p134-mail-composite".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    // In the BPMN XML, `$` + `{` must appear as literal expression markers.
    // Rust raw string: write them as consecutive characters.
    // (b) keep resultVariableName — assert variable is absent (P138 Java parity).
    let xml = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailCompositeProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="mailTask" />
            <serviceTask id="mailTask" flowable:type="mail" flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>Composite mail</flowable:subject>
                    <flowable:text>Hello "#,
        "${gender}",
        r#", your order "#,
        "${orderId}",
        r#"!</flowable:text>
                    <flowable:html><![CDATA[<p>Hi "#,
        "${gender}",
        r#"</p>]]></flowable:html>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="f2" sourceRef="mailTask" targetRef="after" />
            <userTask id="after" />
        </process>
    </definitions>"#
    );

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("mail_composite.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("gender".to_string(), json!("Mx"))
                .variable("orderId".to_string(), json!("ORD-9")),
        )
        .unwrap();

    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(pi.id.clone())
            .unwrap()
            .len(),
        1
    );

    assert!(
        runtime_service
            .get_variable(pi.id.clone(), "mailResult".to_string())
            .unwrap()
            .is_none(),
        "mail resultVariableName must not write a process variable (P138 Java parity)"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    assert_eq!(outbox[0].body, "Hello Mx, your order ORD-9!");
    assert_eq!(outbox[0].html_body.as_deref(), Some("<p>Hi Mx</p>"));
}

#[test]
fn mail_task_text_var_html_var_composite_and_escape() {
    let process_engine = ProcessEngine::new("p134-mail-var-composite".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailVarComposite" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="mailTask" />
            <serviceTask id="mailTask" flowable:type="mail" flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>Var composite</flowable:subject>
                    <flowable:textVar>bodyTemplate</flowable:textVar>
                    <flowable:htmlVar>htmlTemplate</flowable:htmlVar>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="f2" sourceRef="mailTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string(
                    "mail_var_composite.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();
    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // bodyTemplate: mixed expansion + escaped \${literal}
    let body_template = format!(
        "Hello {}; show {}",
        "${gender}",
        r"\${literal}"
    );
    let html_template = format!("<b>{}</b>", "${gender}");

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

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    assert_eq!(outbox[0].body, "Hello Mx; show ${literal}");
    assert_eq!(outbox[0].html_body.as_deref(), Some("<b>Mx</b>"));
}
