// C4 parity tests: case/process task in/out parameter mapping and business key handling.
// Java references:
// - CaseTaskActivityBehavior.java:97-98 (in-parameters build the child variable map from scratch),
//   :123 (business key via getBusinessKey), :177/:244-253 (out-parameters on child completion)
// - ProcessTaskActivityBehavior.java:87-88/:115/:156 (same shape for process tasks)
// - ChildTaskActivityBehavior.java:89-104 (explicit business key wins over inheritance)
// - IOParameterUtil.java:56-92 (copy algorithm; missing source resolves to null)
// Kept deviation (baseline compatibility, see cmmn_runtime_test.rs:164-165): without declared
// in-parameters the full parent variable map and the parent business key keep being passed.
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnCaseTask,
    CmmnDeploymentRequest, CmmnEngine, CmmnError, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnIOParameter, CmmnModel, CmmnPlanItem, CmmnProcessTask,
    CmmnProcessTaskRunner, CmmnProcessTaskStartRequest, CmmnProcessTaskStartResult,
};
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingProcessTaskRunner {
    requests: Mutex<Vec<CmmnProcessTaskStartRequest>>,
}

impl CmmnProcessTaskRunner for RecordingProcessTaskRunner {
    fn start_process(
        &self,
        request: CmmnProcessTaskStartRequest,
    ) -> Result<CmmnProcessTaskStartResult, CmmnError> {
        self.requests.lock().expect("requests").push(request);
        Ok(CmmnProcessTaskStartResult {
            process_instance_id: "process-instance-1".to_string(),
            completed: false,
        })
    }
}

fn child_case() -> CmmnCase {
    CmmnCase::new(
        "case-child",
        "childCase",
        "Child case",
        CmmnCasePlanModel::new("child-plan-model", "Child plan")
            .with_human_task(CmmnHumanTask::new("human-task-child", "Child work"))
            .with_plan_item(CmmnPlanItem::new("plan-item-child", "human-task-child")),
    )
}

// Parent keeps an always-active human task so the parent case stays open after the child ends.
fn deploy_case_task_parent(engine: &CmmnEngine, deployment_key: &str, case_task: CmmnCaseTask) {
    let parent_case = CmmnCase::new(
        "case-parent",
        "parameterParentCase",
        "Parameter parent case",
        CmmnCasePlanModel::new("parent-plan-model", "Parent plan")
            .with_case_task(case_task)
            .with_plan_item(CmmnPlanItem::new("plan-item-case", "case-task-child"))
            .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-keepalive",
                "human-task-keepalive",
            )),
    );
    engine
        .deploy(
            CmmnDeploymentRequest::new(deployment_key)
                .with_resource("cases.cmmn", CmmnModel::new(vec![child_case(), parent_case])),
        )
        .expect("deployment");
}

fn child_case_instance_id(engine: &CmmnEngine, parent_instance_id: &str) -> String {
    engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(parent_instance_id)
        .single_result()
        .expect("association query")
        .expect("association")
        .child_instance_id
        .expect("child case instance id")
}

#[test]
fn case_task_in_parameters_pass_only_mapped_variables() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case_task_parent(
        &engine,
        "case-task-in-parameters",
        CmmnCaseTask::new("case-task-child", "Child case task")
            .with_case_ref("childCase")
            // IOParameterUtil.java:56-92 — only declared parameters reach the child; the missing
            // source still writes its target with null (IOParameterUtil.java:64-66).
            .with_in_parameter(CmmnIOParameter::new("amount", "childAmount"))
            .with_in_parameter(CmmnIOParameter::new("missing", "copiedMissing")),
    );

    let parent_instance = engine
        .start_case_instance_by_key(
            "parameterParentCase",
            CmmnCaseInstanceStartRequest::new()
                .with_variables(json!({ "amount": 42, "secret": "hidden" })),
        )
        .expect("parent case");

    let child_id = child_case_instance_id(&engine, &parent_instance.id);
    let child_instance = engine
        .runtime_service()
        .get_case_instance(&child_id)
        .expect("child case instance");
    assert_eq!(child_instance.variables["childAmount"], json!(42));
    assert_eq!(child_instance.variables["copiedMissing"], Value::Null);
    // CaseTaskActivityBehavior.java:97-98 — the child map starts empty, undeclared parent
    // variables never leak into the child.
    assert!(!child_instance.variables.contains_key("amount"));
    assert!(!child_instance.variables.contains_key("secret"));
}

#[test]
fn case_task_without_parameters_keeps_full_variables_and_inherited_business_key() {
    // Baseline-compatibility guard (deviation from Java): no declared in-parameters means the
    // full parent variable map and the parent business key keep flowing to the child.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case_task_parent(
        &engine,
        "case-task-no-parameters",
        CmmnCaseTask::new("case-task-child", "Child case task").with_case_ref("childCase"),
    );

    let parent_instance = engine
        .start_case_instance_by_key(
            "parameterParentCase",
            CmmnCaseInstanceStartRequest::new()
                .with_business_key("BK-PARENT")
                .with_variables(json!({ "amount": 42 })),
        )
        .expect("parent case");

    let child_id = child_case_instance_id(&engine, &parent_instance.id);
    let child_instance = engine
        .runtime_service()
        .get_case_instance(&child_id)
        .expect("child case instance");
    assert_eq!(child_instance.variables["amount"], json!(42));
    assert_eq!(child_instance.business_key.as_deref(), Some("BK-PARENT"));
}

#[test]
fn case_task_business_key_literal_overrides_parent_inheritance() {
    // ChildTaskActivityBehavior.java:89-104 — an explicit business key on the child task wins
    // over inheriting the parent case business key.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case_task_parent(
        &engine,
        "case-task-business-key",
        CmmnCaseTask::new("case-task-child", "Child case task")
            .with_case_ref("childCase")
            .with_business_key("BK-CHILD"),
    );

    let parent_instance = engine
        .start_case_instance_by_key(
            "parameterParentCase",
            CmmnCaseInstanceStartRequest::new().with_business_key("BK-PARENT"),
        )
        .expect("parent case");

    let child_id = child_case_instance_id(&engine, &parent_instance.id);
    let child_instance = engine
        .runtime_service()
        .get_case_instance(&child_id)
        .expect("child case instance");
    assert_eq!(child_instance.business_key.as_deref(), Some("BK-CHILD"));
}

#[test]
fn case_task_out_parameters_map_child_variables_to_parent_on_completion() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case_task_parent(
        &engine,
        "case-task-out-parameters",
        CmmnCaseTask::new("case-task-child", "Child case task")
            .with_case_ref("childCase")
            // CaseTaskActivityBehavior.java:177/:244-253 — on child completion the child case is
            // the source and the parent case the target of the out-parameter copy.
            .with_out_parameter(CmmnIOParameter::new("childResult", "parentResult")),
    );

    let parent_instance = engine
        .start_case_instance_by_key("parameterParentCase", CmmnCaseInstanceStartRequest::new())
        .expect("parent case");
    let child_id = child_case_instance_id(&engine, &parent_instance.id);

    engine
        .runtime_service()
        .set_case_instance_variables(&child_id, vec![("childResult".to_string(), json!("ok"))])
        .expect("child variables");
    let child_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&child_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("child task query")
        .expect("child task");
    engine
        .complete_human_task(&child_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("child completion");

    let refreshed_parent = engine
        .runtime_service()
        .get_case_instance(&parent_instance.id)
        .expect("parent case instance");
    assert_eq!(refreshed_parent.variables["parentResult"], json!("ok"));
}

#[test]
fn process_task_in_parameters_and_business_key_override_shape_start_request() {
    let runner = Arc::new(RecordingProcessTaskRunner::default());
    let engine =
        CmmnEngine::new_in_memory_with_process_task_runner(runner.clone()).expect("engine");

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-process-parent",
        "processParameterCase",
        "Process parameter case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_process_task(
                CmmnProcessTask::new("process-task-approval", "Approval process")
                    .with_process_ref("approvalProcess")
                    // ProcessTaskActivityBehavior.java:87-88/:115 — mapped variables only, and
                    // the explicit business key replaces the inherited parent key.
                    .with_business_key("BK-PROC")
                    .with_in_parameter(CmmnIOParameter::new("amount", "procAmount")),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-approval",
            )),
    )]);
    engine
        .deploy(CmmnDeploymentRequest::new("process-task-in").with_resource("case.cmmn", model))
        .expect("deployment");

    engine
        .start_case_instance_by_key(
            "processParameterCase",
            CmmnCaseInstanceStartRequest::new()
                .with_business_key("BK-PARENT")
                .with_variables(json!({ "amount": 42, "secret": "hidden" })),
        )
        .expect("case instance");

    let requests = runner.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].business_key.as_deref(), Some("BK-PROC"));
    assert_eq!(requests[0].variables["procAmount"], json!(42));
    assert!(!requests[0].variables.contains_key("amount"));
    assert!(!requests[0].variables.contains_key("secret"));
}

fn deploy_process_task_parent_with_out_parameter(engine: &CmmnEngine, deployment_key: &str) {
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-process-parent",
        "processOutParameterCase",
        "Process out parameter case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_process_task(
                CmmnProcessTask::new("process-task-approval", "Approval process")
                    .with_process_ref("approvalProcess")
                    .with_out_parameter(CmmnIOParameter::new("result", "processResult")),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-approval",
            ))
            .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-keepalive",
                "human-task-keepalive",
            )),
    )]);
    engine
        .deploy(CmmnDeploymentRequest::new(deployment_key).with_resource("case.cmmn", model))
        .expect("deployment");
}

#[test]
fn process_task_out_parameters_apply_completion_payload_to_parent() {
    let runner = Arc::new(RecordingProcessTaskRunner::default());
    let engine =
        CmmnEngine::new_in_memory_with_process_task_runner(runner.clone()).expect("engine");
    deploy_process_task_parent_with_out_parameter(&engine, "process-task-out");

    let case_instance = engine
        .start_case_instance_by_key("processOutParameterCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    // ProcessTaskActivityBehavior.java:156 — the child process variables feed the out-parameter
    // mapping on trigger; the Rust runner is one-way, so the payload is handed in explicitly.
    let mut child_variables = Map::new();
    child_variables.insert("result".to_string(), json!("done"));
    engine
        .runtime_service()
        .complete_process_task_child_instance_with_variables(
            "process-instance-1",
            child_variables,
        )
        .expect("process completion");

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_instance.id)
        .expect("case instance");
    assert_eq!(refreshed.variables["processResult"], json!("done"));
}

#[test]
fn process_task_out_parameters_resolve_missing_payload_to_null() {
    let runner = Arc::new(RecordingProcessTaskRunner::default());
    let engine =
        CmmnEngine::new_in_memory_with_process_task_runner(runner.clone()).expect("engine");
    deploy_process_task_parent_with_out_parameter(&engine, "process-task-out-null");

    let case_instance = engine
        .start_case_instance_by_key("processOutParameterCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    // IOParameterUtil.java:64-66 — a source that cannot be resolved still writes the declared
    // target with a null value; the payload-less completion API behaves the same way.
    engine
        .runtime_service()
        .complete_process_task_child_instance("process-instance-1")
        .expect("process completion");

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_instance.id)
        .expect("case instance");
    assert_eq!(refreshed.variables["processResult"], Value::Null);
    assert!(refreshed.variables.contains_key("processResult"));
}
