use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::service::config::ProcessEngineConfiguration;

#[test]
fn test_job_query_deterministic_ordering() {
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();
    let management_service = process_engine.get_management_service();

    // Distinct process keys: P17 redeploy cancels obsolete timer-start jobs for
    // the same key (Java TimerManager.removeObsoleteTimers), so re-deploying one
    // definition no longer yields multiple concurrent start-timer subscriptions.
    let xml_a = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="jobProcessA">
            <startEvent id="start">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    let xml_b = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="jobProcessB">
            <startEvent id="start">
                <timerEventDefinition>
                    <timeDuration>PT2H</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("job_a.bpmn20.xml".to_string(), xml_a.to_string()),
        )
        .unwrap();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("job_b.bpmn20.xml".to_string(), xml_b.to_string()),
        )
        .unwrap();

    let jobs = management_service.create_timer_job_query().list().unwrap();

    assert!(jobs.len() >= 2);

    let sorted_jobs = management_service
        .create_timer_job_query()
        .order_by_job_id()
        .asc()
        .list()
        .unwrap();

    for i in 0..sorted_jobs.len() - 1 {
        assert!(sorted_jobs[i].timer_job_id <= sorted_jobs[i + 1].timer_job_id);
    }
}
