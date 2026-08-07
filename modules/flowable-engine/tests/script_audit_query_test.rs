use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::service::config::ProcessEngineConfiguration;

#[test]
fn script_task_execution_is_visible_through_history_audit_query() {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };

    let process_engine = ProcessEngine::new_with_config("default".to_string(), config);
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = process_engine.get_history_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="scriptAuditProcess">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="scriptTask" />
            <scriptTask id="scriptTask" name="Audit Script" scriptFormat="javascript">
                <script>
                    var total = 40 + 2;
                </script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("script_audit.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let process_instance = runtime_service
        .start_process_instance_by_key("scriptAuditProcess")
        .unwrap();

    let audit_logs = history_service
        .create_historic_audit_log_query()
        .process_instance_id(process_instance.id.clone())
        .event_type("script-task-executed".to_string())
        .list()
        .unwrap();

    assert_eq!(audit_logs.len(), 1);
    assert_eq!(
        audit_logs[0].process_instance_id.as_deref(),
        Some(process_instance.id.as_str())
    );
    assert_eq!(audit_logs[0].event_type, "script-task-executed");
    let details = audit_logs[0].details.as_deref().unwrap_or("");
    assert!(
        details.contains("scriptTask") && details.contains("javascript"),
        "script audit details should include the activity id and language"
    );
}
