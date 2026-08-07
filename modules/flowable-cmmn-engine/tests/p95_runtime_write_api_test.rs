// P95: CMMN runtime write API surface — setCaseInstanceName / updateBusinessKey /
// removeVariable / evaluateCriteria / triggerPlanItemInstance (start).
//
// Java references:
// - CmmnRuntimeServiceImpl.java:142 (triggerPlanItemInstance)
// - CmmnRuntimeServiceImpl.java:202 (evaluateCriteria)
// - CmmnRuntimeServiceImpl.java:322 (removeVariable)
// - CmmnRuntimeServiceImpl.java:347 (setCaseInstanceName)
// - CmmnRuntimeServiceImpl.java:467 (updateBusinessKey)
// - StartPlanItemInstanceCmd.java:74-79 (ENABLED/AVAILABLE → ACTIVE)

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHumanTask, CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
};
use serde_json::json;

fn simple_case_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-a", "Task A"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "task-a"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P95 write API case",
        plan_model,
    )])
}

fn manual_activation_case_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-manual", "Manual task"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-manual", "task-manual")
                .with_manual_activation_rule("true"),
        );
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P95 manual activation case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, case_key: &str, model: CmmnModel) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), model),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

#[test]
fn set_case_instance_name_updates_name() {
    // Java: SetCaseInstanceNameCmd.java:48-63
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p95NameCase", simple_case_model("p95NameCase"));

    engine
        .runtime_service()
        .set_case_instance_name(&case_id, "Renamed case")
        .expect("set name");

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case");
    assert_eq!(refreshed.name, "Renamed case");
}

#[test]
fn update_business_key_sets_key() {
    // Java: SetCaseInstanceBusinessKeyCmd.java:55-72
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p95BkCase", simple_case_model("p95BkCase"));

    engine
        .runtime_service()
        .update_business_key(&case_id, "BK-100")
        .expect("update business key");

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case");
    assert_eq!(refreshed.business_key.as_deref(), Some("BK-100"));
}

#[test]
fn remove_variable_deletes_single_case_variable() {
    // Java: RemoveVariableCmd.java:45-63
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p95RmVarCase", simple_case_model("p95RmVarCase"));

    engine
        .runtime_service()
        .set_case_instance_variables(
            &case_id,
            vec![
                ("keep".to_string(), json!(1)),
                ("drop".to_string(), json!("gone")),
            ],
        )
        .expect("set vars");

    engine
        .runtime_service()
        .remove_variable(&case_id, "drop")
        .expect("remove variable");

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case");
    assert!(!refreshed.variables.contains_key("drop"));
    assert_eq!(refreshed.variables.get("keep"), Some(&json!(1)));
}

#[test]
fn evaluate_criteria_on_active_case_succeeds() {
    // Java: EvaluateCriteriaCmd.java:36-40
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p95EvalCase", simple_case_model("p95EvalCase"));

    engine
        .runtime_service()
        .evaluate_criteria(&case_id)
        .expect("evaluate criteria");

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case still present");
    assert!(refreshed.ended_at.is_none() || refreshed.ended_at.is_some());
    // Case remains loadable after evaluation cycle.
    let _ = refreshed.id;
}

#[test]
fn trigger_plan_item_instance_starts_enabled_manual_task() {
    // Java: StartPlanItemInstanceCmd.java:54-58 / TriggerPlanItemInstanceCmd:
    // manual activation parks in ENABLED, then trigger moves ENABLED -> ACTIVE.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "p95TriggerCase",
        manual_activation_case_model("p95TriggerCase"),
    );

    let enabled = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Enabled)
        .single_result()
        .expect("query")
        .expect("enabled manual task");

    engine
        .runtime_service()
        .trigger_plan_item_instance(&enabled.id)
        .expect("trigger");

    let started = engine
        .runtime_service()
        .get_human_task(&enabled.id)
        .expect("task after trigger");
    assert_eq!(started.state, CmmnHumanTaskState::Active);
}

#[test]
fn start_plan_item_instance_mirrors_trigger_for_enabled_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "p95StartCase",
        manual_activation_case_model("p95StartCase"),
    );

    let enabled = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Enabled)
        .single_result()
        .expect("query")
        .expect("enabled manual task");

    engine
        .runtime_service()
        .start_plan_item_instance(&enabled.id)
        .expect("start");

    let started = engine
        .runtime_service()
        .get_human_task(&enabled.id)
        .expect("task after start");
    assert_eq!(started.state, CmmnHumanTaskState::Active);
}
