use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::MailOutboxStatus;
use serde_json::json;

#[test]
fn mail_task_executes_owned_runtime_and_stores_send_record() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    // (b) keep resultVariableName and assert the variable is NOT written —
    // Java DefaultActivityBehaviorFactory.java:234-239 has no resultVariable
    // consumer for type=mail (P138 alignment).
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailRuntimeProcess" name="Mail Runtime Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1"
                         name="Send Mail"
                         flowable:type="mail"
                         flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local; audit@example.flowable.local</flowable:to>
                    <flowable:from>noreply@example.flowable.local</flowable:from>
                    <flowable:subject>Deployment finished</flowable:subject>
                    <flowable:text>Process deployment completed successfully.</flowable:text>
                    <flowable:html>Process deployment completed successfully.</flowable:html>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review Mail Result" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail Runtime Deployment".to_string())
        .add_string("mailRuntimeProcess.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail Runtime Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "Mail task should continue into the user task"
    );
    assert_eq!(tasks[0].name, "Review Mail Result");

    // P138 differential: resultVariableName is ignored for mail (Java parity).
    assert!(
        runtime_service
            .get_variable(process_instance.id.clone(), "mailResult".to_string())
            .unwrap()
            .is_none(),
        "mail resultVariableName must not write a process variable (Java has no consumer)"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(
        outbox[0].recipient,
        "ops@example.flowable.local,audit@example.flowable.local"
    );
    assert_eq!(outbox[0].recipients.len(), 2);
    assert_eq!(outbox[0].recipients[0], "ops@example.flowable.local");
    assert_eq!(outbox[0].recipients[1], "audit@example.flowable.local");
    assert_eq!(outbox[0].subject, "Deployment finished");
    assert_eq!(
        outbox[0].body,
        "Process deployment completed successfully."
    );
    assert_eq!(
        outbox[0].html_body.as_deref(),
        Some("Process deployment completed successfully.")
    );
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    // Dropped: mailResult["service"]/"transport" — Rust-only super-set metadata,
    // no outbox field (P138 cut).
}

/// P51 S2 — Java BaseMailActivityDelegate: cc/bcc/charset + field EL evaluation.
#[test]
fn mail_task_evaluates_el_fields_and_captures_cc_bcc_charset() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailElProcess" name="Mail EL Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1"
                         name="Send Mail With EL"
                         flowable:type="mail"
                         flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>${toAddress}</flowable:to>
                    <flowable:cc>${ccAddress}</flowable:cc>
                    <flowable:bcc>secret@example.flowable.local</flowable:bcc>
                    <flowable:from>noreply@example.flowable.local</flowable:from>
                    <flowable:subject>${mailSubject}</flowable:subject>
                    <flowable:text>${mailBody}</flowable:text>
                    <flowable:charset>UTF-8</flowable:charset>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review Mail Result" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail EL Deployment".to_string())
        .add_string("mailElProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail EL Instance".to_string())
                .variable("toAddress".to_string(), json!("ops@example.flowable.local"))
                .variable("ccAddress".to_string(), json!("audit@example.flowable.local"))
                .variable("mailSubject".to_string(), json!("EL Subject"))
                .variable("mailBody".to_string(), json!("EL body text")),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    // Outbox carries to/subject/body; cc/bcc/charset lived only on the cut
    // mailResult payload (no outbox columns) — drop those super-set assertions.
    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].recipient, "ops@example.flowable.local");
    assert_eq!(outbox[0].recipients, vec!["ops@example.flowable.local".to_string()]);
    assert_eq!(outbox[0].subject, "EL Subject");
    assert_eq!(outbox[0].body, "EL body text");
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    // Dropped: service/status-from-result, message.cc/bcc/charset/ccRecipients/
    // bccRecipients — resultVariable payload only (P138).
}

/// P51 S2 — ignoreException swallows mail errors and may set exceptionVariableName.
#[test]
fn mail_task_ignore_exception_swallows_missing_recipient_error() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    // to/cc/bcc all resolve empty via EL → Java would throw; ignoreException swallows.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailIgnoreProcess" name="Mail Ignore Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1"
                         name="Send Mail Ignored"
                         flowable:type="mail"
                         flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>${emptyTo}</flowable:to>
                    <flowable:subject>Ignored</flowable:subject>
                    <flowable:text>body</flowable:text>
                    <flowable:ignoreException>true</flowable:ignoreException>
                    <flowable:exceptionVariableName>mailError</flowable:exceptionVariableName>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="After Ignored Mail" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail Ignore Deployment".to_string())
        .add_string("mailIgnoreProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail Ignore Instance".to_string())
                .variable("emptyTo".to_string(), json!("")),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "ignoreException must allow the process to continue"
    );
    assert_eq!(tasks[0].name, "After Ignored Mail");

    let mail_error = runtime_service
        .get_variable(process_instance.id.clone(), "mailError".to_string())
        .unwrap()
        .expect("exceptionVariableName should capture the ignored error");
    assert!(
        mail_error
            .as_str()
            .is_some_and(|s| s.contains("no recipient")),
        "expected recipient error stored, got {mail_error}"
    );

    // P138: ignored-error payload was only written via resultVariableName super-set
    // (status=IGNORED_ERROR). Java mail has no such variable; assert absence.
    assert!(
        runtime_service
            .get_variable(process_instance.id.clone(), "mailResult".to_string())
            .unwrap()
            .is_none(),
        "ignored mail must not write mailResult (P138 cut of resultVariable super-set)"
    );

    // Failed before send → no outbox row.
    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store.list_mail_outbox_records(&mut session).is_empty(),
        "ignored send failure must not create an outbox record"
    );
}

/// P124 — headers (Java BaseMailActivityDelegate.addHeader:134-147).
/// Headers are accepted at deploy/runtime; content is not re-exported on a
/// process variable after P138 (was only on cut mailResult payload; outbox has
/// no headers column). Assert process continues + outbox body/subject.
#[test]
fn mail_task_headers_passthrough_to_result() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailHeadersProcess" name="Mail Headers Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1"
                         name="Send Mail With Headers"
                         flowable:type="mail"
                         flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>Headers mail</flowable:subject>
                    <flowable:text>Body with custom headers.</flowable:text>
                    <flowable:headers><![CDATA[X-Attribute1: value1
X-Attribute2: value2
X-Correlation-Id: corr-42]]></flowable:headers>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="After Headers Mail" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail Headers Deployment".to_string())
        .add_string("mailHeadersProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail Headers Instance".to_string()),
        )
        .unwrap();

    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .len(),
        1
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].subject, "Headers mail");
    assert_eq!(outbox[0].body, "Body with custom headers.");
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    // Dropped: message.headers.* — only lived on mailResult (P138; no outbox column).
}

/// P124 — textVar: body taken from process variable named by the field
/// (Java BaseMailActivityDelegate.createMessage:100-102, getExpression:236-239).
#[test]
fn mail_task_text_var_reads_body_from_variable() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailTextVarProcess" name="Mail TextVar Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1"
                         name="Send Mail TextVar"
                         flowable:type="mail"
                         flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>TextVar mail</flowable:subject>
                    <flowable:textVar>bodyTemplate</flowable:textVar>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="After TextVar Mail" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail TextVar Deployment".to_string())
        .add_string(
            "mailTextVarProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail TextVar Instance".to_string())
                .variable(
                    "bodyTemplate".to_string(),
                    json!("Hello from textVar body."),
                ),
        )
        .unwrap();

    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .len(),
        1
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].body, "Hello from textVar body.");
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
}

/// P124 — textVar missing variable fails (Java getExpression → createExpression(null) NPE path).
#[test]
fn mail_task_text_var_missing_variable_fails() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailTextVarMissingProcess" name="Mail TextVar Missing">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1" flowable:type="mail" flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>TextVar missing</flowable:subject>
                    <flowable:textVar>missingBodyVar</flowable:textVar>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail TextVar Missing Deployment".to_string())
        .add_string(
            "mailTextVarMissingProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let err = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail TextVar Missing Instance".to_string()),
        )
        .expect_err("missing textVar variable must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("missingBodyVar") || msg.contains("textVar"),
        "expected textVar missing error, got {msg}"
    );
}

/// P124 — htmlVar: HTML body from process variable (Java createMessage:103-105).
#[test]
fn mail_task_html_var_reads_body_from_variable() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailHtmlVarProcess" name="Mail HtmlVar Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1"
                         name="Send Mail HtmlVar"
                         flowable:type="mail"
                         flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>HtmlVar mail</flowable:subject>
                    <flowable:htmlVar>htmlTemplate</flowable:htmlVar>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="After HtmlVar Mail" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail HtmlVar Deployment".to_string())
        .add_string(
            "mailHtmlVarProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail HtmlVar Instance".to_string())
                .variable(
                    "htmlTemplate".to_string(),
                    json!("<html><body>Hello <b>Kermit</b></body></html>"),
                ),
        )
        .unwrap();

    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .len(),
        1
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(
        outbox[0].html_body.as_deref(),
        Some("<html><body>Hello <b>Kermit</b></body></html>")
    );
    // html-only is allowed (Java createMessage:112-114); text/body may be empty.
    assert_eq!(outbox[0].body, "");
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
}

/// P124 — htmlVar missing variable fails.
#[test]
fn mail_task_html_var_missing_variable_fails() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailHtmlVarMissingProcess" name="Mail HtmlVar Missing">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1" flowable:type="mail" flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>HtmlVar missing</flowable:subject>
                    <flowable:htmlVar>missingHtmlVar</flowable:htmlVar>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail HtmlVar Missing Deployment".to_string())
        .add_string(
            "mailHtmlVarMissingProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let err = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail HtmlVar Missing Instance".to_string()),
        )
        .expect_err("missing htmlVar variable must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("missingHtmlVar") || msg.contains("htmlVar"),
        "expected htmlVar missing error, got {msg}"
    );
}

/// P124 — attachments field (Java BaseMailActivityDelegate.addAttachments:149-211).
/// Deterministic outbox accepts attachments at send time; attachment names were
/// only echoed on the cut mailResult payload (outbox has no attachments column).
/// Assert send succeeds (outbox row + process continues).
#[test]
fn mail_task_attachments_passthrough_to_result() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailAttachmentsProcess" name="Mail Attachments Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1"
                         name="Send Mail With Attachments"
                         flowable:type="mail"
                         flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>Attachments mail</flowable:subject>
                    <flowable:text>See attached files.</flowable:text>
                    <flowable:attachments>${attachmentList}</flowable:attachments>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="After Attachments Mail" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail Attachments Deployment".to_string())
        .add_string(
            "mailAttachmentsProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail Attachments Instance".to_string())
                .variable(
                    "attachmentList".to_string(),
                    json!(["report.pdf", "summary.txt"]),
                ),
        )
        .unwrap();

    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .len(),
        1
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].subject, "Attachments mail");
    assert_eq!(outbox[0].body, "See attached files.");
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    // Dropped: message.attachments[*].name — only on cut mailResult (P138).
}

/// P124 — field-extension form (flowable:field) for headers/textVar is accepted at deploy + runtime.
#[test]
fn mail_task_field_extension_form_headers_and_text_var() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mailFieldExtProcess" name="Mail Field Extension Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1"
                         name="Send Mail Field Ext"
                         flowable:type="mail"
                         flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:field name="to">
                        <flowable:string>ops@example.flowable.local</flowable:string>
                    </flowable:field>
                    <flowable:field name="subject">
                        <flowable:string>Field extension mail</flowable:string>
                    </flowable:field>
                    <flowable:field name="textVar">
                        <flowable:expression>bodyTemplate</flowable:expression>
                    </flowable:field>
                    <flowable:field name="headers">
                        <flowable:string><![CDATA[X-Field-Header: from-field]]></flowable:string>
                    </flowable:field>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="After Field Ext Mail" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mail Field Ext Deployment".to_string())
        .add_string(
            "mailFieldExtProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).expect("field extension mail must deploy");
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mail Field Ext Instance".to_string())
                .variable(
                    "bodyTemplate".to_string(),
                    json!("Body via field textVar."),
                ),
        )
        .unwrap();

    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .len(),
        1
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].body, "Body via field textVar.");
    assert_eq!(outbox[0].subject, "Field extension mail");
    assert_eq!(outbox[0].status, MailOutboxStatus::Sent);
    // Dropped: message.headers["X-Field-Header"] — only on cut mailResult (P138).
}
