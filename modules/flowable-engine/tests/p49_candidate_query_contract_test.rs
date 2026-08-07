//! P49 contract tests for task candidate / involvement query parity.
//!
//! Java evidence:
//! - T1 group expansion: TaskQueryImpl.getCandidateGroups / getGroupsForCandidateUser
//!   (TaskQueryImpl.java:1997-2031) + Task.xml candidate EXISTS (LINK.USER_ID_ OR
//!   LINK.GROUP_ID_ IN memberships)
//! - T3 taskInvolvedGroups: TaskQueryImpl.java:604-617, Task.xml:904-919
//! - T4 ignoreAssignee / default exclude assigned: TaskQueryImpl.java:680-687,
//!   Task.xml:867-870 (`RES.ASSIGNEE_ is null` unless ignoreAssigneeValue)
//! - T5 historic candidate filter effectiveness: HistoricTaskInstanceQueryCmd
//!   history_service.rs candidate filter path (direct link match)
//! - T6 historic candidateUser group expansion: HistoricTaskInstanceQueryImpl.java:2221-2246
//! - P75a historic candidate default ASSIGNEE_ is null / ignoreAssigneeValue:
//!   HistoricTaskInstance.xml:1485-1487, HistoricTaskInstanceQueryImpl.java:1972-1978

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::Group;
use flowable_engine::task::Task;

fn standalone_task(engine: &ProcessEngine, task_id: &str, name: &str) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let task = Task::new(
        task_id.to_string(),
        String::new(),
        String::new(),
        "standalone".to_string(),
        name.to_string(),
    );
    store.insert_task(&task, &mut session);
    session.flush_and_commit().unwrap();
}

/// T1: candidateUser expands the user's group memberships.
/// User A belongs to G1; task only has candidate group G1 → must match candidateUser=A.
#[test]
fn t1_candidate_user_expands_group_memberships() {
    let engine = ProcessEngine::new("p49-t1-group-expand".to_string());
    let identity = engine.get_identity_service();
    let task_service = engine.get_task_service();

    identity.save_group(Group {
        id: "G1".to_string(),
        name: "Group One".to_string(),
        group_type: None,
    });
    identity.create_membership("userA".to_string(), "G1".to_string());

    standalone_task(&engine, "task-group-only", "Group Only Task");
    task_service
        .add_candidate_group("task-group-only".to_string(), "G1".to_string())
        .unwrap();

    // Direct user candidate link should still match (control).
    standalone_task(&engine, "task-direct-user", "Direct User Task");
    task_service
        .add_candidate_user("task-direct-user".to_string(), "userA".to_string())
        .unwrap();

    // Unrelated group — must not match.
    standalone_task(&engine, "task-other-group", "Other Group Task");
    task_service
        .add_candidate_group("task-other-group".to_string(), "G2".to_string())
        .unwrap();

    let matched = task_service
        .create_task_query()
        .task_candidate_user("userA".to_string())
        .list()
        .unwrap();
    let ids: Vec<&str> = matched.iter().map(|t| t.id.as_str()).collect();
    assert!(
        ids.contains(&"task-group-only"),
        "T1: group-only candidate link must match via membership expansion; got {ids:?}"
    );
    assert!(
        ids.contains(&"task-direct-user"),
        "direct user candidate link must still match; got {ids:?}"
    );
    assert!(
        !ids.contains(&"task-other-group"),
        "unrelated group must not match; got {ids:?}"
    );
}

/// T3: taskInvolvedGroups matches any identity-link group id (not only candidate type).
#[test]
fn t3_task_involved_groups_matches_identity_link_groups() {
    let engine = ProcessEngine::new("p49-t3-involved-groups".to_string());
    let task_service = engine.get_task_service();

    standalone_task(&engine, "task-cand-g", "Candidate Group Task");
    task_service
        .add_candidate_group("task-cand-g".to_string(), "sales".to_string())
        .unwrap();

    standalone_task(&engine, "task-participant-g", "Participant Group Task");
    task_service
        .add_identity_link(
            "task-participant-g".to_string(),
            None,
            Some("sales".to_string()),
            "participant".to_string(),
        )
        .unwrap();

    standalone_task(&engine, "task-other", "Other Task");
    task_service
        .add_candidate_group("task-other".to_string(), "hr".to_string())
        .unwrap();

    let matched = task_service
        .create_task_query()
        .task_involved_groups(vec!["sales".to_string()])
        .list()
        .unwrap();
    let ids: Vec<&str> = matched.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"task-cand-g"), "candidate group link: {ids:?}");
    assert!(
        ids.contains(&"task-participant-g"),
        "non-candidate group link must match involvedGroups: {ids:?}"
    );
    assert!(!ids.contains(&"task-other"), "other group must not match: {ids:?}");
}

/// T4 secondary confirmation + fix:
/// Java candidate queries default to `ASSIGNEE_ is null`; ignoreAssigneeValue turns that off.
#[test]
fn t4_candidate_query_excludes_assigned_unless_ignore_assignee() {
    let engine = ProcessEngine::new("p49-t4-ignore-assignee".to_string());
    let task_service = engine.get_task_service();

    standalone_task(&engine, "task-open", "Open Candidate");
    task_service
        .add_candidate_group("task-open".to_string(), "sales".to_string())
        .unwrap();

    standalone_task(&engine, "task-claimed", "Claimed Candidate");
    task_service
        .add_candidate_group("task-claimed".to_string(), "sales".to_string())
        .unwrap();
    task_service
        .claim_task_by_id("task-claimed".to_string(), "johnDoe".to_string())
        .unwrap();

    let default_hits = task_service
        .create_task_query()
        .task_candidate_group("sales".to_string())
        .list()
        .unwrap();
    let default_ids: Vec<&str> = default_hits.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        default_ids,
        vec!["task-open"],
        "Java default: candidate query excludes assigned tasks; got {default_ids:?}"
    );

    let with_ignore = task_service
        .create_task_query()
        .task_candidate_group("sales".to_string())
        .ignore_assignee_value()
        .list()
        .unwrap();
    let ignore_ids: Vec<&str> = with_ignore.iter().map(|t| t.id.as_str()).collect();
    assert!(
        ignore_ids.contains(&"task-open") && ignore_ids.contains(&"task-claimed"),
        "ignoreAssignee must keep assigned candidate tasks; got {ignore_ids:?}"
    );
}

/// T4 also applies to candidateUser (including after group expansion).
#[test]
fn t4_candidate_user_excludes_assigned_after_group_expand() {
    let engine = ProcessEngine::new("p49-t4-cand-user-assigned".to_string());
    let identity = engine.get_identity_service();
    let task_service = engine.get_task_service();

    identity.save_group(Group {
        id: "sales".to_string(),
        name: "Sales".to_string(),
        group_type: None,
    });
    identity.create_membership("aSalesUser".to_string(), "sales".to_string());

    standalone_task(&engine, "task-open", "Open");
    task_service
        .add_candidate_group("task-open".to_string(), "sales".to_string())
        .unwrap();

    standalone_task(&engine, "task-claimed", "Claimed");
    task_service
        .add_candidate_group("task-claimed".to_string(), "sales".to_string())
        .unwrap();
    task_service
        .claim_task_by_id("task-claimed".to_string(), "someone".to_string())
        .unwrap();

    let default_hits = task_service
        .create_task_query()
        .task_candidate_user("aSalesUser".to_string())
        .list()
        .unwrap();
    assert_eq!(default_hits.len(), 1);
    assert_eq!(default_hits[0].id, "task-open");

    let with_ignore = task_service
        .create_task_query()
        .task_candidate_user("aSalesUser".to_string())
        .ignore_assignee_value()
        .list()
        .unwrap();
    assert_eq!(with_ignore.len(), 2);
}

/// T5: historic task candidateUser / candidateGroup filters are effective
/// (direct identity-link match — already wired in HistoricTaskInstanceQueryCmd).
#[test]
fn t5_historic_task_candidate_filters_effective() {
    let engine = ProcessEngine::new("p49-t5-historic-candidate".to_string());
    let repository = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history = engine.get_history_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p49HistCand" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Hist Cand"
                flowable:candidateUsers="gonzo"
                flowable:candidateGroups="management" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository
        .deploy(
            repository
                .create_deployment()
                .add_string("p49-hist.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let pi = runtime
        .start_process_instance_by_key("p49HistCand")
        .unwrap();
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    let task_id = tasks[0].id.clone();
    task_service.complete_task_by_id(task_id.clone()).unwrap();

    let by_user = history
        .create_historic_task_instance_query()
        .task_candidate_user("gonzo".to_string())
        .list()
        .unwrap();
    assert!(
        by_user.iter().any(|t| t.id == task_id),
        "T5: historic candidateUser must filter; got {:?}",
        by_user.iter().map(|t| &t.id).collect::<Vec<_>>()
    );

    let by_group = history
        .create_historic_task_instance_query()
        .task_candidate_group("management".to_string())
        .list()
        .unwrap();
    assert!(
        by_group.iter().any(|t| t.id == task_id),
        "T5: historic candidateGroup must filter"
    );

    let wrong = history
        .create_historic_task_instance_query()
        .task_candidate_user("fozzie".to_string())
        .list()
        .unwrap();
    assert!(
        !wrong.iter().any(|t| t.id == task_id),
        "T5: wrong candidateUser must not match"
    );
}

/// T6 / P66: historic candidateUser expands the user's group memberships.
/// Task only has candidate group link (no direct user link) → still matches via membership.
/// Java: HistoricTaskInstanceQueryImpl.getCandidateGroups / getGroupsForCandidateUser
/// (HistoricTaskInstanceQueryImpl.java:2221-2246) + HistoricTaskInstance.xml:1484-1510.
#[test]
fn t6_historic_candidate_user_expands_group_memberships() {
    let engine = ProcessEngine::new("p66-t6-historic-group-expand".to_string());
    let identity = engine.get_identity_service();
    let repository = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history = engine.get_history_service();

    identity.save_group(Group {
        id: "sales".to_string(),
        name: "Sales".to_string(),
        group_type: None,
    });
    identity.create_membership("aSalesUser".to_string(), "sales".to_string());

    // Group-only candidate link (no candidateUsers attribute).
    let xml_group = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p66HistGroupOnly" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Group Only"
                flowable:candidateGroups="sales" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository
        .deploy(
            repository
                .create_deployment()
                .add_string("p66-hist-group.bpmn20.xml".to_string(), xml_group.to_string()),
        )
        .unwrap();
    let pi_group = runtime
        .start_process_instance_by_key("p66HistGroupOnly")
        .unwrap();
    let tasks_group = task_service
        .get_tasks_by_process_instance_id(pi_group.id.clone())
        .unwrap();
    assert_eq!(tasks_group.len(), 1);
    let group_task_id = tasks_group[0].id.clone();
    task_service
        .complete_task_by_id(group_task_id.clone())
        .unwrap();

    // Direct user candidate link control.
    let xml_user = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p66HistUserOnly" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="User Only"
                flowable:candidateUsers="aSalesUser" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository
        .deploy(
            repository
                .create_deployment()
                .add_string("p66-hist-user.bpmn20.xml".to_string(), xml_user.to_string()),
        )
        .unwrap();
    let pi_user = runtime
        .start_process_instance_by_key("p66HistUserOnly")
        .unwrap();
    let tasks_user = task_service
        .get_tasks_by_process_instance_id(pi_user.id.clone())
        .unwrap();
    assert_eq!(tasks_user.len(), 1);
    let user_task_id = tasks_user[0].id.clone();
    task_service
        .complete_task_by_id(user_task_id.clone())
        .unwrap();

    // Unrelated group must not match.
    let xml_other = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p66HistOtherGroup" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Other Group"
                flowable:candidateGroups="hr" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository
        .deploy(
            repository
                .create_deployment()
                .add_string("p66-hist-other.bpmn20.xml".to_string(), xml_other.to_string()),
        )
        .unwrap();
    let pi_other = runtime
        .start_process_instance_by_key("p66HistOtherGroup")
        .unwrap();
    let tasks_other = task_service
        .get_tasks_by_process_instance_id(pi_other.id.clone())
        .unwrap();
    assert_eq!(tasks_other.len(), 1);
    let other_task_id = tasks_other[0].id.clone();
    task_service
        .complete_task_by_id(other_task_id.clone())
        .unwrap();

    let matched = history
        .create_historic_task_instance_query()
        .task_candidate_user("aSalesUser".to_string())
        .list()
        .unwrap();
    let ids: Vec<&str> = matched.iter().map(|t| t.id.as_str()).collect();
    assert!(
        ids.contains(&group_task_id.as_str()),
        "T6: historic group-only candidate link must match via membership expansion; got {ids:?}"
    );
    assert!(
        ids.contains(&user_task_id.as_str()),
        "T6: historic direct user candidate link must still match; got {ids:?}"
    );
    assert!(
        !ids.contains(&other_task_id.as_str()),
        "T6: historic unrelated group must not match; got {ids:?}"
    );
}

/// P75a: historic candidate queries default to `ASSIGNEE_ is null`
/// (HistoricTaskInstance.xml:1485-1487). Claimed-then-completed tasks keep
/// their assignee on the historic row and are excluded unless
/// `ignoreAssigneeValue` is set (HistoricTaskInstanceQueryImpl.java:1972-1978).
#[test]
fn p75a_historic_candidate_excludes_assigned_unless_ignore_assignee() {
    let engine = ProcessEngine::new("p75a-hist-ignore-assignee".to_string());
    let repository = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history = engine.get_history_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p75aHistCand" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Hist Cand Gate"
                flowable:candidateGroups="sales" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository
        .deploy(
            repository
                .create_deployment()
                .add_string("p75a-hist.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    // Unassigned path: complete without claim → historic assignee remains null.
    let pi_open = runtime
        .start_process_instance_by_key("p75aHistCand")
        .unwrap();
    let open_tasks = task_service
        .get_tasks_by_process_instance_id(pi_open.id.clone())
        .unwrap();
    assert_eq!(open_tasks.len(), 1);
    let open_task_id = open_tasks[0].id.clone();
    task_service
        .complete_task_by_id(open_task_id.clone())
        .unwrap();

    // Assigned path: claim then complete → historic assignee is set.
    let pi_claimed = runtime
        .start_process_instance_by_key("p75aHistCand")
        .unwrap();
    let claimed_tasks = task_service
        .get_tasks_by_process_instance_id(pi_claimed.id.clone())
        .unwrap();
    assert_eq!(claimed_tasks.len(), 1);
    let claimed_task_id = claimed_tasks[0].id.clone();
    task_service
        .claim_task_by_id(claimed_task_id.clone(), "johnDoe".to_string())
        .unwrap();
    task_service
        .complete_task_by_id(claimed_task_id.clone())
        .unwrap();

    let default_hits = history
        .create_historic_task_instance_query()
        .task_candidate_group("sales".to_string())
        .list()
        .unwrap();
    let default_ids: Vec<&str> = default_hits.iter().map(|t| t.id.as_str()).collect();
    assert!(
        default_ids.contains(&open_task_id.as_str()),
        "P75a: unassigned historic candidate must match; got {default_ids:?}"
    );
    assert!(
        !default_ids.contains(&claimed_task_id.as_str()),
        "P75a: assigned historic candidate must be excluded by default; got {default_ids:?}"
    );

    let with_ignore = history
        .create_historic_task_instance_query()
        .task_candidate_group("sales".to_string())
        .ignore_assignee_value()
        .list()
        .unwrap();
    let ignore_ids: Vec<&str> = with_ignore.iter().map(|t| t.id.as_str()).collect();
    assert!(
        ignore_ids.contains(&open_task_id.as_str())
            && ignore_ids.contains(&claimed_task_id.as_str()),
        "P75a: ignoreAssigneeValue must keep assigned historic candidates; got {ignore_ids:?}"
    );
}

/// P75a: same assignee-null gate applies after candidateUser group expansion
/// (parity with runtime T4 after group expand).
#[test]
fn p75a_historic_candidate_user_excludes_assigned_after_group_expand() {
    let engine = ProcessEngine::new("p75a-hist-cand-user-assigned".to_string());
    let identity = engine.get_identity_service();
    let repository = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history = engine.get_history_service();

    identity.save_group(Group {
        id: "sales".to_string(),
        name: "Sales".to_string(),
        group_type: None,
    });
    identity.create_membership("aSalesUser".to_string(), "sales".to_string());

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p75aHistUserCand" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Group Expand Gate"
                flowable:candidateGroups="sales" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository
        .deploy(
            repository
                .create_deployment()
                .add_string("p75a-hist-user.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pi_open = runtime
        .start_process_instance_by_key("p75aHistUserCand")
        .unwrap();
    let open_id = task_service
        .get_tasks_by_process_instance_id(pi_open.id.clone())
        .unwrap()[0]
        .id
        .clone();
    task_service.complete_task_by_id(open_id.clone()).unwrap();

    let pi_claimed = runtime
        .start_process_instance_by_key("p75aHistUserCand")
        .unwrap();
    let claimed_id = task_service
        .get_tasks_by_process_instance_id(pi_claimed.id.clone())
        .unwrap()[0]
        .id
        .clone();
    task_service
        .claim_task_by_id(claimed_id.clone(), "someone".to_string())
        .unwrap();
    task_service
        .complete_task_by_id(claimed_id.clone())
        .unwrap();

    let default_hits = history
        .create_historic_task_instance_query()
        .task_candidate_user("aSalesUser".to_string())
        .list()
        .unwrap();
    let default_ids: Vec<&str> = default_hits.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        default_ids.iter().filter(|id| **id == open_id).count(),
        1,
        "P75a: expanded candidateUser must hit unassigned historic task; got {default_ids:?}"
    );
    assert!(
        !default_ids.contains(&claimed_id.as_str()),
        "P75a: expanded candidateUser must exclude assigned historic task; got {default_ids:?}"
    );

    let with_ignore = history
        .create_historic_task_instance_query()
        .task_candidate_user("aSalesUser".to_string())
        .ignore_assignee_value()
        .list()
        .unwrap();
    let ignore_ids: Vec<&str> = with_ignore.iter().map(|t| t.id.as_str()).collect();
    assert!(
        ignore_ids.contains(&open_id.as_str()) && ignore_ids.contains(&claimed_id.as_str()),
        "P75a: ignoreAssigneeValue after group expand must keep assigned; got {ignore_ids:?}"
    );
}
