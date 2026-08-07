use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::service::config::ProcessEngineConfiguration;

#[test]
fn test_event_subscription_query_filtering() {
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="eventProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catch" />
            <intermediateCatchEvent id="catch">
                <messageEventDefinition messageRef="msg1" />
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catch" targetRef="end" />
            <endEvent id="end" />
        </process>
        <message id="msg1" name="My Message" />
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("event.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    runtime_service
        .start_process_instance_by_key("eventProcess")
        .unwrap();

    let event_subscriptions = runtime_service
        .create_event_subscription_query()
        .event_name("My Message".to_string())
        .list()
        .unwrap();

    assert_eq!(event_subscriptions.len(), 1);
    assert_eq!(event_subscriptions[0].event_name(), "My Message");
}
