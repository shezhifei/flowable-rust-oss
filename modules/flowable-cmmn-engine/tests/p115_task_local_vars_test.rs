// P115: CMMN human task task-local variables — the task's own variable scope,
// keyed by task id, shadowing the case (parent) scope on non-local reads and
// deleted with the task on completion.
//
// Java references:
// - TaskService#setVariableLocal / getVariableLocal / getVariablesLocal /
//   hasVariableLocal / removeVariableLocal (TaskServiceImpl.java:400-461) →
//   SetTaskVariablesCmd.java:42-47 / GetTaskVariableCmd.java:62-63 /
//   HasTaskVariableCmd.java:61-62 / RemoveTaskVariablesCmd.java:38-42
// - TaskEntity variable semantics (VariableScopeImpl.java):
//   getVariableLocal :338-384 (local only), getVariablesLocal :455-470,
//   hasVariableLocal :425-431, removeVariableLocal :814-820,
//   setVariableLocal :743-785, getVariable :268-323 (local first, then parent),
//   collectVariables :203-225 (parent first, then local — local shadows case)
// - CMMN parent scope = the case instance
//   (DefaultCmmnTaskVariableScopeResolver.java:34-43)
// - lifecycle: local variables are deleted with the task entity on completion
//   (HumanTaskActivityBehavior.java:482 completeTask → CMMN
//   TaskHelper.internalDeleteTask.java:109-128)

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest,
    CmmnEngine, CmmnHumanTask, CmmnModel, CmmnPlanItem,
};
use serde_json::json;

fn simple_case_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-a", "Task A"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "task-a"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P115 task-local variables case",
        plan_model,
    )])
}

fn two_task_case_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-a", "Task A"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "task-a"))
        .with_human_task(CmmnHumanTask::new("task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-b", "task-b"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P115 task-local variables case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, case_key: &str) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), simple_case_model(case_key)),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn deploy_and_start_two_tasks(engine: &CmmnEngine, case_key: &str) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), two_task_case_model(case_key)),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn active_task_ids(engine: &CmmnEngine, case_id: &str) -> Vec<String> {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_id)
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.id)
        .collect()
}

fn single_active_task_id(engine: &CmmnEngine, case_id: &str) -> String {
    let ids = active_task_ids(engine, case_id);
    assert_eq!(ids.len(), 1, "expected exactly one active task");
    ids.into_iter().next().expect("one task")
}

#[test]
fn task_local_variables_set_get_has_remove() {
    // Java: setVariableLocal / getVariableLocal / getVariablesLocal /
    // hasVariableLocal / removeVariableLocal (VariableScopeImpl.java:338-470,
    // 743-785, 814-820).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p115CrudCase");
    let task_id = single_active_task_id(&engine, &case_id);
    let runtime = engine.runtime_service();

    runtime
        .set_task_variable_local(&task_id, "alpha", json!(1))
        .expect("set local alpha");
    runtime
        .set_task_variable_local(&task_id, "beta", json!("two"))
        .expect("set local beta");

    // getVariableLocal
    assert_eq!(
        runtime
            .get_task_variable_local(&task_id, "alpha")
            .expect("get local alpha"),
        Some(json!(1))
    );
    // missing name → None, not an error
    assert_eq!(
        runtime
            .get_task_variable_local(&task_id, "ghost")
            .expect("get local ghost"),
        None
    );

    // getVariablesLocal returns only the task's own variables
    let locals = runtime
        .get_task_variables_local(&task_id)
        .expect("get locals");
    assert_eq!(locals.len(), 2);
    assert_eq!(locals.get("alpha"), Some(&json!(1)));
    assert_eq!(locals.get("beta"), Some(&json!("two")));

    // hasVariableLocal
    assert!(runtime
        .has_task_variable_local(&task_id, "alpha")
        .expect("has local alpha"));
    assert!(!runtime
        .has_task_variable_local(&task_id, "ghost")
        .expect("has local ghost"));

    // removeVariableLocal
    runtime
        .remove_task_variable_local(&task_id, "alpha")
        .expect("remove local alpha");
    assert_eq!(
        runtime
            .get_task_variable_local(&task_id, "alpha")
            .expect("get local alpha after remove"),
        None
    );
    let locals = runtime
        .get_task_variables_local(&task_id)
        .expect("get locals after remove");
    assert_eq!(locals.len(), 1);
    assert_eq!(locals.get("beta"), Some(&json!("two")));

    // case variables are untouched
    let case = runtime.get_case_instance(&case_id).expect("case");
    assert!(case.variables.is_empty(), "local writes must not touch the case");
}

#[test]
fn task_local_variable_shadows_case_variable_on_read() {
    // Java: getVariable / getVariables read the local scope first and only fall
    // back to the parent (case) scope — local shadows case
    // (VariableScopeImpl.java:268-323, collectVariables :203-225).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p115ShadowCase");
    let task_id = single_active_task_id(&engine, &case_id);
    let runtime = engine.runtime_service();

    runtime
        .set_case_instance_variables(&case_id, vec![("shared".to_string(), json!("case"))])
        .expect("set case shared");

    // Non-local read without a local shadow → the case value.
    assert_eq!(
        runtime
            .get_task_variable(&task_id, "shared")
            .expect("get case-backed shared"),
        Some(json!("case"))
    );

    // Write a task-local variable with the same name → shadows the case value.
    runtime
        .set_task_variable_local(&task_id, "shared", json!("local"))
        .expect("set local shared");

    assert_eq!(
        runtime
            .get_task_variable(&task_id, "shared")
            .expect("get shadowed shared"),
        Some(json!("local"))
    );
    assert!(runtime
        .has_task_variable(&task_id, "shared")
        .expect("has shadowed shared"));

    // getVariables merges case + local with local winning on conflicts.
    let merged = runtime.get_task_variables(&task_id).expect("merged variables");
    assert_eq!(merged.get("shared"), Some(&json!("local")));

    // The case variable itself is not overwritten by the local write
    // (VariableScopeImpl.setVariableLocal only touches the local scope).
    let case = runtime.get_case_instance(&case_id).expect("case");
    assert_eq!(case.variables.get("shared"), Some(&json!("case")));
}

#[test]
fn case_variable_does_not_leak_into_local_scope() {
    // Java: getVariablesLocal / hasVariableLocal never walk the parent scope
    // (VariableScopeImpl.java:425-431, :455-470).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p115NoLeakCase");
    let task_id = single_active_task_id(&engine, &case_id);
    let runtime = engine.runtime_service();

    runtime
        .set_case_instance_variables(&case_id, vec![("caseOnly".to_string(), json!(42))])
        .expect("set case-only variable");

    assert!(!runtime
        .has_task_variable_local(&task_id, "caseOnly")
        .expect("has caseOnly local"));
    let locals = runtime
        .get_task_variables_local(&task_id)
        .expect("get locals");
    assert!(locals.is_empty(), "case variables must not appear local");
    assert_eq!(
        runtime
            .get_task_variable_local(&task_id, "caseOnly")
            .expect("get caseOnly local"),
        None
    );

    // The non-local read still sees it through the parent scope.
    assert_eq!(
        runtime
            .get_task_variable(&task_id, "caseOnly")
            .expect("get caseOnly"),
        Some(json!(42))
    );
}

#[test]
fn task_completion_clears_local_variables() {
    // Java: completing the human task deletes the TaskEntity and its task-local
    // variables (HumanTaskActivityBehavior.java:482 → CMMN
    // TaskHelper.internalDeleteTask.java:109-128).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p115LifecycleCase");
    let task_id = single_active_task_id(&engine, &case_id);
    let runtime = engine.runtime_service();

    runtime
        .set_task_variable_local(&task_id, "scratch", json!("value"))
        .expect("set local scratch");
    runtime
        .set_case_instance_variables(&case_id, vec![("persist".to_string(), json!("case"))])
        .expect("set case persist");

    engine
        .runtime_service()
        .complete_human_task(&task_id, Default::default())
        .expect("complete");

    let locals = runtime
        .get_task_variables_local(&task_id)
        .expect("get locals after complete");
    assert!(
        locals.is_empty(),
        "task-local variables must be cleared on completion"
    );
    assert!(!runtime
        .has_task_variable_local(&task_id, "scratch")
        .expect("has scratch after complete"));

    // Non-local reads fall through to the case scope, which survives.
    assert_eq!(
        runtime
            .get_task_variable(&task_id, "persist")
            .expect("get persist after complete"),
        Some(json!("case"))
    );
}

#[test]
fn task_local_variables_are_isolated_between_tasks() {
    // Java: task-local variables are keyed by the task (ACT_RU_VARIABLE.TASK_ID_)
    // — one task's local scope never leaks into another's.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start_two_tasks(&engine, "p115IsolationCase");
    let mut task_ids = active_task_ids(&engine, &case_id);
    assert_eq!(task_ids.len(), 2);
    task_ids.sort();
    let (task_a, task_b) = (&task_ids[0], &task_ids[1]);
    let runtime = engine.runtime_service();

    runtime
        .set_task_variable_local(task_a, "shared", json!("from-a"))
        .expect("set local on task a");
    runtime
        .set_task_variable_local(task_b, "shared", json!("from-b"))
        .expect("set local on task b");

    assert_eq!(
        runtime
            .get_task_variable_local(task_a, "shared")
            .expect("get a shared"),
        Some(json!("from-a"))
    );
    assert_eq!(
        runtime
            .get_task_variable_local(task_b, "shared")
            .expect("get b shared"),
        Some(json!("from-b"))
    );

    // Removing from one task leaves the other untouched.
    runtime
        .remove_task_variable_local(task_a, "shared")
        .expect("remove a shared");
    assert_eq!(
        runtime
            .get_task_variable_local(task_a, "shared")
            .expect("get a shared after remove"),
        None
    );
    assert_eq!(
        runtime
            .get_task_variable_local(task_b, "shared")
            .expect("get b shared after remove"),
        Some(json!("from-b"))
    );

    // Neither task-local write leaked into the case.
    let case = runtime.get_case_instance(&case_id).expect("case");
    assert!(case.variables.is_empty(), "no case variables expected");
}
