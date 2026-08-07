//! P17: timer EL evaluation + lifecycle (retries, suspended start skip,
//! redeploy cancel / undeploy restore).
//!
//! Java evidence:
//! - TimerUtil.java:128-221 (EL + hard fail on unparseable)
//! - BoundaryTimerEventTest.testExpressionOnTimer / testInfiniteRepeatingTimer
//! - StartTimerEventTest.testExpressionStartTimerEvent
//! - TimerStartEventJobHandler.java:54,77-79 (suspended skip)
//! - TimerManager.removeObsoleteTimers / DeploymentProcessDefinitionDeletionManagerImpl restore

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use serde_json::json;
use std::sync::Arc;

// ── Task 1: EL evaluation ──────────────────────────────────────────────────

#[test]
fn boundary_timer_time_duration_expression_evaluates() {
    // Java BoundaryTimerEventTest.testExpressionOnTimer
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap(),
    ));
    let engine =
        ProcessEngine::with_time_source("p17-boundary-el-duration".to_string(), time_source);
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="testExpressionOnTimer" isExecutable="true">
            <startEvent id="theStart" />
            <sequenceFlow id="flow1" sourceRef="theStart" targetRef="task" />
            <userTask id="task" name="Task with timer" />
            <boundaryEvent id="boundaryTimer" cancelActivity="true" attachedToRef="task">
                <timerEventDefinition>
                    <timeDuration>${duration}</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="task" targetRef="theEnd" />
            <sequenceFlow id="flow3" sourceRef="boundaryTimer" targetRef="theEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("el-duration".to_string())
                .add_string("el.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("duration".to_string(), json!("PT10M")),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let jobs = store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(jobs.len(), 1, "EL duration must create a timer job");
    assert_eq!(jobs[0].time_duration.as_deref(), Some("PT10M"));
    assert!(
        jobs[0].due_time.is_some(),
        "due_time must be resolved from evaluated duration"
    );
    // Default asyncExecutorNumberOfRetries = 3
    assert_eq!(
        jobs[0].retries,
        Some(3),
        "timer retries must come from asyncExecutorNumberOfRetries (default 3)"
    );
}

#[test]
fn boundary_timer_missing_expression_variable_fails_hard() {
    // Java TimerUtil: evaluation failure rolls back the command (no silent no-fire).
    let engine = ProcessEngine::new("p17-boundary-el-fail".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="elFailProcess" isExecutable="true">
            <startEvent id="theStart" />
            <sequenceFlow id="flow1" sourceRef="theStart" targetRef="task" />
            <userTask id="task" />
            <boundaryEvent id="boundaryTimer" cancelActivity="true" attachedToRef="task">
                <timerEventDefinition>
                    <timeDuration>${missingDuration}</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="task" targetRef="theEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("el-fail".to_string())
                .add_string("el_fail.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let err = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .expect_err("missing timer EL variable must hard-fail start");
    let msg = err.to_string();
    assert!(
        msg.contains("could not be evaluated")
            || msg.contains("null")
            || msg.contains("Due date")
            || msg.contains("Timer"),
        "unexpected error text: {msg}"
    );
}

#[test]
fn intermediate_timer_time_cycle_expression_evaluates() {
    // Java BoundaryTimerEventTest.testInfiniteRepeatingTimer uses ${timerString}
    // on a cycle; intermediate catch is the same TimerUtil path.
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap(),
    ));
    let engine =
        ProcessEngine::with_time_source("p17-intermediate-el-cycle".to_string(), time_source);
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="cycleElProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catch" />
            <intermediateCatchEvent id="catch">
                <timerEventDefinition>
                    <timeCycle>${timerString}</timeCycle>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("cycle-el".to_string())
                .add_string("cycle_el.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("timerString".to_string(), json!("R3/PT1H")),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let jobs = store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(jobs.len(), 1);
    assert!(
        jobs[0]
            .time_cycle
            .as_deref()
            .is_some_and(|c| c.contains("PT1H")),
        "evaluated cycle must be prepared: {:?}",
        jobs[0].time_cycle
    );
    assert!(jobs[0].due_time.is_some());
    assert_eq!(jobs[0].retries, Some(3));
}

#[test]
fn boundary_timer_end_date_expression_evaluates() {
    // Java BoundaryTimerEventRepeatWithEndTest: endDate="${EndDateForBoundary}"
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::with_time_source("p17-enddate-el".to_string(), time_source);
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="endDateElProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task" />
            <userTask id="task" />
            <boundaryEvent id="boundaryTimer" cancelActivity="false" attachedToRef="task">
                <timerEventDefinition>
                    <timeCycle flowable:endDate="${EndDateForBoundary}">R5/PT1H</timeCycle>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("enddate-el".to_string())
                .add_string("enddate.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable(
                    "EndDateForBoundary".to_string(),
                    json!("2030-01-01T00:00:00Z"),
                ),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let jobs = store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].end_date.as_deref(),
        Some("2030-01-01T00:00:00Z"),
        "endDate EL must be evaluated at create time"
    );
}

#[test]
fn start_timer_string_literal_expression_resolves() {
    // Java StartTimerEventTest.testExpressionStartTimerEvent:
    // <timeDate>${'2036-11-14T11:12:22'}</timeDate>
    let engine = ProcessEngine::new("p17-start-el-literal".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="startTimerEventExample" isExecutable="true">
            <startEvent id="theStart">
                <timerEventDefinition>
                    <timeDate>${'2036-11-14T11:12:22Z'}</timeDate>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="theStart" targetRef="task" />
            <userTask id="task" />
            <endEvent id="end" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("start-el".to_string())
                .add_string("start_el.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let subs = engine.get_timer_start_subscriptions();
    assert_eq!(subs.len(), 1);
    assert_eq!(
        subs[0].time_date.as_deref(),
        Some("2036-11-14T11:12:22Z")
    );
    assert!(subs[0].due_time.is_some());
}

// ── Task 2: lifecycle ──────────────────────────────────────────────────────

#[test]
fn suspended_definition_timer_start_skips_without_panic_and_reschedules_cycle() {
    // Java TimerStartEventJobHandler: ignore suspended definition; cycle still reschedules.
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap(),
    ));
    let engine =
        ProcessEngine::with_time_source("p17-suspended-start".to_string(), time_source.clone());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="suspendedCycleStart" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition>
                    <timeCycle>R/PT5S</timeCycle>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="task" />
            <userTask id="task" />
            <endEvent id="end" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("suspended-cycle".to_string())
                .add_string("suspended.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    // Suspend the definition.
    engine
        .get_repository_service()
        .set_process_definition_suspended(&def_id, true)
        .unwrap();

    let due_before = engine
        .get_timer_start_subscriptions()
        .into_iter()
        .find(|s| s.process_definition_id == def_id)
        .and_then(|s| s.due_time)
        .expect("subscription due_time");

    time_source.advance_time(5_000);
    // Must not panic (previous path: StartProcessInstanceCmd.unwrap on suspended).
    let triggered = engine.run_due_timers();
    assert!(
        triggered
            .iter()
            .any(|id| id.contains("timer_start_skipped_suspended") || id.contains("timer_start:")),
        "suspended fire should complete without panic: {triggered:?}"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let instances = store.snapshot_process_instances(&mut session);
    assert!(
        instances.is_empty(),
        "suspended definition must not start a process instance"
    );

    // Cycle must still reschedule to a future due.
    let due_after = engine
        .get_timer_start_subscriptions()
        .into_iter()
        .find(|s| s.process_definition_id == def_id)
        .and_then(|s| s.due_time);
    assert!(
        due_after.is_some_and(|d| d > due_before),
        "cycle subscription must reschedule after suspended skip (before={due_before}, after={due_after:?})"
    );
}

#[test]
fn redeploy_cancels_old_version_timer_start_subscriptions() {
    // Java StartTimerEventTest.testVersionUpgradeShouldCancelJobs /
    // testOldJobsDeletedOnRedeploy
    let engine = ProcessEngine::new("p17-redeploy-cancel".to_string());
    let xml_v1 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="versionUpgradeTimer" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="task" />
            <userTask id="task" />
            <endEvent id="end" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
        </process>
    </definitions>"#;
    let xml_v2 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="versionUpgradeTimer" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition>
                    <timeDuration>PT2H</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="task" />
            <userTask id="task" />
            <endEvent id="end" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("v1".to_string())
                .add_string("v1.bpmn20.xml".to_string(), xml_v1.to_string()),
        )
        .unwrap();
    let subs_v1 = engine.get_timer_start_subscriptions();
    assert_eq!(subs_v1.len(), 1);
    assert_eq!(subs_v1[0].time_duration.as_deref(), Some("PT1H"));
    let v1_def = subs_v1[0].process_definition_id.clone();

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("v2".to_string())
                .add_string("v2.bpmn20.xml".to_string(), xml_v2.to_string()),
        )
        .unwrap();

    let subs = engine.get_timer_start_subscriptions();
    assert_eq!(
        subs.len(),
        1,
        "redeploy must cancel old timer start jobs; only latest remains"
    );
    assert_ne!(
        subs[0].process_definition_id, v1_def,
        "remaining subscription must belong to the new version"
    );
    assert_eq!(subs[0].time_duration.as_deref(), Some("PT2H"));
}

#[test]
fn undeploy_old_version_keeps_latest_timer_start() {
    // Java StartTimerEventTest.testTimerShouldNotBeRemovedWhenUndeployingOldVersion
    let engine = ProcessEngine::new("p17-undeploy-old-keep".to_string());
    let xml_v1 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="keepLatestTimer" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition>
                    <timeDuration>PT30M</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="task" />
            <userTask id="task" />
            <endEvent id="end" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
        </process>
    </definitions>"#;
    let xml_v2 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="keepLatestTimer" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition>
                    <timeDuration>PT45M</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="task" />
            <userTask id="task" />
            <endEvent id="end" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
        </process>
    </definitions>"#;

    let dep1 = engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("keep-v1".to_string())
                .add_string("v1.bpmn20.xml".to_string(), xml_v1.to_string()),
        )
        .unwrap();
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("keep-v2".to_string())
                .add_string("v2.bpmn20.xml".to_string(), xml_v2.to_string()),
        )
        .unwrap();

    engine
        .get_repository_service()
        .delete_deployment(&dep1.id)
        .unwrap();
    let after_old = engine.get_timer_start_subscriptions();
    assert_eq!(
        after_old.len(),
        1,
        "undeploying old version must keep latest timer start"
    );
    assert_eq!(after_old[0].time_duration.as_deref(), Some("PT45M"));
}

#[test]
fn undeploy_latest_restores_previous_version_timer_start() {
    // Java DeploymentProcessDefinitionDeletionManagerImpl.restorePreviousStartEventsIfNeeded
    let engine = ProcessEngine::new("p17-undeploy-restore".to_string());
    let xml_v1 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="restoreTimer" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition>
                    <timeDuration>PT30M</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="task" />
            <userTask id="task" />
            <endEvent id="end" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
        </process>
    </definitions>"#;
    let xml_v2 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="restoreTimer" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition>
                    <timeDuration>PT45M</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="task" />
            <userTask id="task" />
            <endEvent id="end" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("restore-v1".to_string())
                .add_string("v1.bpmn20.xml".to_string(), xml_v1.to_string()),
        )
        .unwrap();
    let dep2 = engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("restore-v2".to_string())
                .add_string("v2.bpmn20.xml".to_string(), xml_v2.to_string()),
        )
        .unwrap();

    // After v2, only PT45M remains (v1's subscription was cancelled on redeploy).
    assert_eq!(engine.get_timer_start_subscriptions().len(), 1);
    assert_eq!(
        engine.get_timer_start_subscriptions()[0]
            .time_duration
            .as_deref(),
        Some("PT45M")
    );

    // Undeploying latest while previous version still exists restores v1 timer.
    engine
        .get_repository_service()
        .delete_deployment(&dep2.id)
        .unwrap();
    let after_latest = engine.get_timer_start_subscriptions();
    assert_eq!(
        after_latest.len(),
        1,
        "undeploying latest must restore previous version timer start"
    );
    assert_eq!(
        after_latest[0].time_duration.as_deref(),
        Some("PT30M"),
        "restored subscription must match previous version definition"
    );
}
