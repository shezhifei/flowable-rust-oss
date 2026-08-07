//! P57 contract tests for TaskQuery `or()` composite queries.
//!
//! Java evidence:
//! - orActive/orQueryObjects/currentOrQueryObject fields: TaskQueryImpl.java:172-174
//! - setter routing while orActive (write into currentOrQueryObject):
//!   TaskQueryImpl.java:215-219 (same pattern for every filter setter)
//! - or() misuse: "the query is already in an or statement"
//!   TaskQueryImpl.java:2048-2062
//! - endOr() misuse: "endOr() can only be called after calling or()"
//!   TaskQueryImpl.java:2065-2073
//! - Task.xml renders each orQueryObject as one parenthesised OR group AND'ed
//!   with the main query; candidate clauses keep the `RES.ASSIGNEE_ is null`
//!   gate inside the block unless ignoreAssigneeValue (Task.xml:867-870).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::error::FlowableError;
use flowable_engine::identity::entities::Group;
use flowable_engine::task::Task;

fn standalone_task(engine: &ProcessEngine, task_id: &str, name: &str) {
    standalone_task_with_key(engine, task_id, name, "standalone");
}

fn standalone_task_with_key(engine: &ProcessEngine, task_id: &str, name: &str, def_key: &str) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let task = Task::new(
        task_id.to_string(),
        String::new(),
        String::new(),
        def_key.to_string(),
        name.to_string(),
    );
    store.insert_task(&task, &mut session);
    session.flush_and_commit().unwrap();
}

/// Conditions inside one or() block are OR'ed:
/// or().taskAssignee(A).taskCandidateGroup(G).endOr() matches tasks assigned
/// to A plus open tasks with candidate group G (TaskQueryImpl.java:172-174).
#[test]
fn or_block_ors_conditions_inside_block() {
    let engine = ProcessEngine::new("p57-or-basic".to_string());
    let task_service = engine.get_task_service();

    standalone_task(&engine, "t-assigned", "Assigned Task");
    task_service
        .claim_task_by_id("t-assigned".to_string(), "kermit".to_string())
        .unwrap();

    standalone_task(&engine, "t-cand", "Candidate Task");
    task_service
        .add_candidate_group("t-cand".to_string(), "sales".to_string())
        .unwrap();

    standalone_task(&engine, "t-none", "Unrelated Task");

    let matched = task_service
        .create_task_query()
        .or()
        .task_assignee("kermit".to_string())
        .task_candidate_group("sales".to_string())
        .end_or()
        .list()
        .unwrap();
    let ids: Vec<&str> = matched.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"t-assigned"), "assignee term must match: {ids:?}");
    assert!(ids.contains(&"t-cand"), "candidate term must match: {ids:?}");
    assert!(!ids.contains(&"t-none"), "unrelated task must not match: {ids:?}");
}

/// The or() block ANDs with criteria set outside the block
/// (Java Task.xml: orQueryObjects render as additional AND groups).
#[test]
fn or_block_ands_with_main_criteria() {
    let engine = ProcessEngine::new("p57-or-and-main".to_string());
    let task_service = engine.get_task_service();

    standalone_task(&engine, "t-match", "Target");
    task_service
        .claim_task_by_id("t-match".to_string(), "kermit".to_string())
        .unwrap();

    // Fails the main criteria (name), passes the or block.
    standalone_task(&engine, "t-wrong-name", "Other");
    task_service
        .claim_task_by_id("t-wrong-name".to_string(), "kermit".to_string())
        .unwrap();

    // Passes the main criteria, fails the or block.
    standalone_task(&engine, "t-wrong-block", "Target");

    let matched = task_service
        .create_task_query()
        .task_name("Target".to_string())
        .or()
        .task_assignee("kermit".to_string())
        .task_candidate_group("sales".to_string())
        .end_or()
        .list()
        .unwrap();
    let ids: Vec<&str> = matched.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["t-match"], "main criteria AND or-block: {ids:?}");
}

/// Multiple or() blocks all AND together (TaskQueryImpl.java:173 list).
#[test]
fn multiple_or_blocks_and_together() {
    let engine = ProcessEngine::new("p57-or-multi-block".to_string());
    let task_service = engine.get_task_service();

    standalone_task(&engine, "t-both", "Alpha");
    task_service
        .claim_task_by_id("t-both".to_string(), "kermit".to_string())
        .unwrap();

    // Passes block 1 (name Alpha), fails block 2 (no assignee).
    standalone_task(&engine, "t-block1-only", "Alpha");

    // Fails block 1 (name Gamma), passes block 2.
    standalone_task(&engine, "t-block2-only", "Gamma");
    task_service
        .claim_task_by_id("t-block2-only".to_string(), "kermit".to_string())
        .unwrap();

    let matched = task_service
        .create_task_query()
        .or()
        .task_name("Alpha".to_string())
        .task_name("Beta".to_string()) // overwrite inside same block keeps last value, like Java field assignment
        .end_or()
        .or()
        .task_assignee("kermit".to_string())
        .end_or()
        .list()
        .unwrap();
    let ids: Vec<&str> = matched.iter().map(|t| t.id.as_str()).collect();
    // Second task_name overwrote the first inside the block (Java sets the
    // same field on currentOrQueryObject), so the block is name == "Beta".
    assert!(
        ids.is_empty(),
        "block field overwrite must behave like Java field assignment: {ids:?}"
    );

    let matched = task_service
        .create_task_query()
        .or()
        .task_name("Alpha".to_string())
        .end_or()
        .or()
        .task_assignee("kermit".to_string())
        .end_or()
        .list()
        .unwrap();
    let ids: Vec<&str> = matched.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["t-both"], "blocks must AND together: {ids:?}");
}

/// P49 candidate semantics inside an or() block must not regress:
/// candidateUser expands group memberships, and the assignee-null gate
/// (Task.xml:867-870) applies unless ignoreAssigneeValue is set in the block.
#[test]
fn or_block_candidate_user_keeps_p49_semantics() {
    let engine = ProcessEngine::new("p57-or-candidate-p49".to_string());
    let identity = engine.get_identity_service();
    let task_service = engine.get_task_service();

    identity.save_group(Group {
        id: "sales".to_string(),
        name: "Sales".to_string(),
        group_type: None,
    });
    identity.create_membership("aSalesUser".to_string(), "sales".to_string());

    standalone_task(&engine, "t-open", "Open");
    task_service
        .add_candidate_group("t-open".to_string(), "sales".to_string())
        .unwrap();

    standalone_task(&engine, "t-claimed", "Claimed");
    task_service
        .add_candidate_group("t-claimed".to_string(), "sales".to_string())
        .unwrap();
    task_service
        .claim_task_by_id("t-claimed".to_string(), "someone".to_string())
        .unwrap();

    // Group expansion works inside the block; assigned task is gated out.
    let default_hits = task_service
        .create_task_query()
        .or()
        .task_candidate_user("aSalesUser".to_string())
        .end_or()
        .list()
        .unwrap();
    let ids: Vec<&str> = default_hits.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["t-open"],
        "or-block candidateUser must expand groups and exclude assigned: {ids:?}"
    );

    // ignoreAssigneeValue set inside the block lifts the gate for the block.
    let with_ignore = task_service
        .create_task_query()
        .or()
        .task_candidate_user("aSalesUser".to_string())
        .ignore_assignee_value()
        .end_or()
        .list()
        .unwrap();
    let ids: Vec<&str> = with_ignore.iter().map(|t| t.id.as_str()).collect();
    assert!(
        ids.contains(&"t-open") && ids.contains(&"t-claimed"),
        "ignoreAssigneeValue in block must keep assigned candidate tasks: {ids:?}"
    );
}

/// Nested or() reports the Java message (TaskQueryImpl.java:2049-2050).
#[test]
fn nested_or_reports_java_error() {
    let engine = ProcessEngine::new("p57-or-nested-error".to_string());
    let task_service = engine.get_task_service();

    let result = task_service
        .create_task_query()
        .or()
        .or()
        .task_assignee("kermit".to_string())
        .end_or()
        .list();
    match result {
        Err(FlowableError::Generic(msg)) => {
            assert_eq!(msg, "the query is already in an or statement")
        }
        other => panic!("nested or() must fail with the Java message; got {other:?}"),
    }
}

/// endOr() without or() reports the Java message (TaskQueryImpl.java:2066-2067).
#[test]
fn end_or_without_or_reports_java_error() {
    let engine = ProcessEngine::new("p57-endor-error".to_string());
    let task_service = engine.get_task_service();

    let result = task_service.create_task_query().end_or().list();
    match result {
        Err(FlowableError::Generic(msg)) => {
            assert_eq!(msg, "endOr() can only be called after calling or()")
        }
        other => panic!("stray endOr() must fail with the Java message; got {other:?}"),
    }
}

/// An or() block with no conditions constrains nothing (Java renders no
/// clauses for an empty orQueryObject).
#[test]
fn empty_or_block_matches_everything() {
    let engine = ProcessEngine::new("p57-or-empty-block".to_string());
    let task_service = engine.get_task_service();

    standalone_task(&engine, "t-a", "A");
    standalone_task(&engine, "t-b", "B");

    let matched = task_service
        .create_task_query()
        .or()
        .end_or()
        .list()
        .unwrap();
    assert_eq!(matched.len(), 2, "empty or block must not filter anything");
}

/// Priority / due-date / definition-key-LIKE conditions participate as OR
/// terms inside a block.
#[test]
fn or_block_priority_due_and_like_terms() {
    let engine = ProcessEngine::new("p57-or-scalar-terms".to_string());
    let task_service = engine.get_task_service();

    standalone_task(&engine, "t-prio", "Priority Task");
    task_service
        .set_task_priority("t-prio".to_string(), 50)
        .unwrap();

    standalone_task(&engine, "t-due", "Due Task");
    task_service
        .set_task_due_date(
            "t-due".to_string(),
            chrono::DateTime::from_timestamp_millis(1_000_000),
        )
        .unwrap();

    standalone_task_with_key(&engine, "t-key", "Keyed Task", "orderReview");

    standalone_task(&engine, "t-miss", "Missing Task");
    task_service
        .set_task_priority("t-miss".to_string(), 10)
        .unwrap();
    task_service
        .set_task_due_date(
            "t-miss".to_string(),
            chrono::DateTime::from_timestamp_millis(9_000_000),
        )
        .unwrap();

    let matched = task_service
        .create_task_query()
        .or()
        .task_priority(50)
        .task_due_before_millis(2_000_000)
        .task_definition_key_like("order%".to_string())
        .end_or()
        .list()
        .unwrap();
    let ids: Vec<&str> = matched.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"t-prio"), "priority term: {ids:?}");
    assert!(ids.contains(&"t-due"), "dueBefore term: {ids:?}");
    assert!(ids.contains(&"t-key"), "definitionKey LIKE term: {ids:?}");
    assert!(!ids.contains(&"t-miss"), "no term matches t-miss: {ids:?}");
}
