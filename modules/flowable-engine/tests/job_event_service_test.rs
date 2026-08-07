use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_job_and_event_subscription_services() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let job_service = process_engine.get_job_service();
    let event_sub_service = process_engine.get_event_subscription_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p1" isExecutable="true">
            <startEvent id="start" />
            <parallelGateway id="fork" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
            <sequenceFlow id="f2" sourceRef="fork" targetRef="timer" />
            <sequenceFlow id="f3" sourceRef="fork" targetRef="message" />
            <intermediateCatchEvent id="timer">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <intermediateCatchEvent id="message">
                <messageEventDefinition messageRef="msg1" />
            </intermediateCatchEvent>
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let repository_service = process_engine.get_repository_service();
    let deployment_builder = repository_service
        .create_deployment()
        .add_string("p1.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(deployment_builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance_by_id(pd_id, None)
        .unwrap();

    // 1. Check timer job
    let jobs = job_service
        .get_timer_jobs_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].activity_id, "timer");

    // 2. Check event subscription
    let subs = event_sub_service
        .get_event_subscriptions_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].activity_id.as_deref(), Some("message"));
}
