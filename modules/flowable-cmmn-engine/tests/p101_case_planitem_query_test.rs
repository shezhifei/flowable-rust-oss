// P101: CMMN case-instance and plan-item query filter surface.
//
// Java references:
// - CaseInstanceCollectionResource.java:114-297 (GET param parsing)
// - BaseCaseInstanceResource.java:68-263 (CaseInstanceQuery builders)
// - PlanItemInstanceCollectionResource.java:71-159 (GET param parsing)
// - PlanItemInstanceBaseResource.java:59-139 (PlanItemInstanceQuery builders)
//
// Intentional deviations (P101 acceptance): caseDefinitionCategory /
// activePlanItemDefinitionId(s) / involvedUser are cut; plan-item queries only
// cover human-task plan items (a non-`humantask` type matches nothing); the
// tenantId family on the plan-item side is cut.

use chrono::{DateTime, Utc};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest,
    CmmnEngine, CmmnHumanTask, CmmnModel, CmmnPlanItem,
};

fn model_with_tasks(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-alpha", "Alpha review"))
        .with_human_task(CmmnHumanTask::new("task-beta", "Beta review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-alpha", "task-alpha"))
        .with_plan_item(CmmnPlanItem::new("plan-item-beta", "task-beta"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        format!("{case_key} definition name"),
        plan_model,
    )])
}

/// Deploy the model once per case key — re-deploying a key leaves two definitions
/// and `start_case_instance_by_key` fails with NonUniqueResult.
fn deploy(engine: &CmmnEngine, case_key: &str) {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), model_with_tasks(case_key)),
        )
        .expect("deployment");
}

fn start_case(
    engine: &CmmnEngine,
    case_key: &str,
    request: CmmnCaseInstanceStartRequest,
) -> String {
    engine
        .start_case_instance_by_key(case_key, request)
        .expect("case instance")
        .id
}

fn case_query(engine: &CmmnEngine) -> flowable_cmmn_engine::CmmnCaseInstanceQuery {
    engine.runtime_service().create_case_instance_query()
}

fn task_query(engine: &CmmnEngine) -> flowable_cmmn_engine::CmmnHumanTaskQuery {
    engine.runtime_service().create_human_task_query()
}

#[test]
fn case_query_filters_by_ids_and_case_definition_key_family() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p101CaseKey");
    let id_a = start_case(&engine, "p101CaseKey", CmmnCaseInstanceStartRequest::new());
    let id_b = start_case(&engine, "p101CaseKey", CmmnCaseInstanceStartRequest::new());

    assert_eq!(
        case_query(&engine)
            .ids(vec![id_a.clone(), id_b.clone()])
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .ids(vec![id_a.clone()])
            .list()
            .expect("query")
            .into_iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>(),
        vec![id_a]
    );

    // Deploying the same key twice keeps both definitions, so key filters match.
    assert_eq!(
        case_query(&engine)
            .case_definition_key("p101CaseKey")
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .case_definition_key_like("p101Case%")
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .case_definition_key_like_ignore_case("p101case%")
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .case_definition_keys(vec!["p101CaseKey".to_string()])
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .exclude_case_definition_keys(vec!["p101CaseKey".to_string()])
            .list()
            .expect("query")
            .len(),
        0
    );
    // case_definition_name is carried on the instance.
    assert_eq!(
        case_query(&engine)
            .case_definition_name("p101CaseKey definition name")
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .case_definition_name_like("p101%")
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .case_definition_name_like_ignore_case("P101CASE%")
            .list()
            .expect("query")
            .len(),
        2
    );
}

#[test]
fn case_query_filters_by_name_and_business_key_like() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p101Name");
    start_case(
        &engine,
        "p101Name",
        CmmnCaseInstanceStartRequest::new().with_name("Order A"),
    );
    start_case(
        &engine,
        "p101Name",
        CmmnCaseInstanceStartRequest::new()
            .with_name("Order B")
            .with_business_key("bk-1001"),
    );

    assert_eq!(
        case_query(&engine)
            .name("Order A")
            .list()
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        case_query(&engine)
            .name_like("Order%")
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .name_like_ignore_case("order%")
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .business_key_like("bk-1%")
            .list()
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        case_query(&engine)
            .business_key_like_ignore_case("BK-1001")
            .list()
            .expect("query")
            .len(),
        1
    );
}

#[test]
fn case_query_filters_by_business_status_and_started_by() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p101Status");
    let id_a = start_case(
        &engine,
        "p101Status",
        CmmnCaseInstanceStartRequest::new()
            .with_name("A")
            .with_started_by("alice"),
    );
    let id_b = start_case(
        &engine,
        "p101Status",
        CmmnCaseInstanceStartRequest::new()
            .with_name("B")
            .with_started_by("bob"),
    );
    engine
        .runtime_service()
        .update_business_status(&id_a, "in-progress")
        .expect("business status");

    assert_eq!(
        case_query(&engine)
            .business_status("in-progress")
            .list()
            .expect("query")
            .into_iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>(),
        vec![id_a.clone()]
    );
    assert_eq!(
        case_query(&engine)
            .business_status_like("in-%")
            .list()
            .expect("query")
            .into_iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>(),
        vec![id_a.clone()]
    );
    assert_eq!(
        case_query(&engine)
            .business_status_like_ignore_case("IN-PROGRESS")
            .list()
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        case_query(&engine)
            .started_by("alice")
            .list()
            .expect("query")
            .into_iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>(),
        vec![id_a]
    );
    assert_eq!(
        case_query(&engine)
            .started_by("bob")
            .list()
            .expect("query")
            .into_iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>(),
        vec![id_b]
    );
}

#[test]
fn case_query_filters_by_started_time_and_tenant() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p101Time");
    // A tenant-scoped definition so a tenant-id start resolves (the tenant-id is
    // part of the definition lookup — runtime.rs start_case_instance_by_key:207-216).
    engine
        .deploy(
            CmmnDeploymentRequest::new("p101TimeTenant-deployment")
                .with_tenant_id("tenant-x")
                .with_resource("p101TimeTenant.cmmn", model_with_tasks("p101TimeTenant")),
        )
        .expect("deployment");
    let id_a = start_case(
        &engine,
        "p101Time",
        CmmnCaseInstanceStartRequest::new().with_name("A"),
    );
    let id_b = start_case(
        &engine,
        "p101TimeTenant",
        CmmnCaseInstanceStartRequest::new()
            .with_name("B")
            .with_tenant_id("tenant-x"),
    );

    let started_at = engine
        .runtime_service()
        .get_case_instance(&id_a)
        .expect("case")
        .started_at;

    assert_eq!(
        case_query(&engine)
            .started_before(started_at + chrono::Duration::seconds(1))
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        case_query(&engine)
            .started_after(started_at - chrono::Duration::seconds(1))
            .list()
            .expect("query")
            .len(),
        2
    );

    assert_eq!(
        case_query(&engine)
            .tenant_id("tenant-x")
            .list()
            .expect("query")
            .into_iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>(),
        vec![id_b.clone()]
    );
    assert_eq!(
        case_query(&engine)
            .tenant_id_like("tenant-%")
            .list()
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        case_query(&engine)
            .tenant_id_like_ignore_case("TENANT-X")
            .list()
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        case_query(&engine)
            .without_tenant_id()
            .list()
            .expect("query")
            .into_iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>(),
        vec![id_a]
    );
}

#[test]
fn case_query_filters_by_callback() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p101Callback");
    start_case(
        &engine,
        "p101Callback",
        CmmnCaseInstanceStartRequest::new()
            .with_name("A")
            .with_callback("exec-1", "bpmn-2.0-to-cmmn-1.1-child-case"),
    );
    start_case(&engine, "p101Callback", CmmnCaseInstanceStartRequest::new().with_name("B"));

    assert_eq!(
        case_query(&engine)
            .callback_id("exec-1")
            .list()
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        case_query(&engine)
            .callback_ids(vec!["exec-1".to_string()])
            .list()
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        case_query(&engine)
            .callback_type("bpmn-2.0-to-cmmn-1.1-child-case")
            .list()
            .expect("query")
            .len(),
        1
    );
}

#[test]
fn plan_item_query_filters_by_case_instance_ids_element_id_and_type() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p101PlanItem");
    let case_id = start_case(&engine, "p101PlanItem", CmmnCaseInstanceStartRequest::new());

    assert_eq!(
        task_query(&engine)
            .case_instance_ids(vec![case_id.clone()])
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        task_query(&engine)
            .element_id("plan-item-alpha")
            .list()
            .expect("query")
            .into_iter()
            .map(|task| task.name)
            .collect::<Vec<_>>(),
        vec!["Alpha review"]
    );
    // Java planItemInstanceElementId matches the plan item id.
    assert_eq!(
        task_query(&engine)
            .element_id("plan-item-beta")
            .list()
            .expect("query")
            .len(),
        1
    );

    // Only human-task plan items exist → type filters match all/only `humantask`.
    assert_eq!(
        task_query(&engine)
            .plan_item_definition_type("humantask")
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        task_query(&engine)
            .plan_item_definition_type("stage")
            .list()
            .expect("query")
            .len(),
        0
    );
    assert_eq!(
        task_query(&engine)
            .plan_item_definition_types(vec!["stage".to_string(), "humantask".to_string()])
            .list()
            .expect("query")
            .len(),
        2
    );
    assert_eq!(
        task_query(&engine)
            .plan_item_definition_types(vec!["stage".to_string(), "milestone".to_string()])
            .list()
            .expect("query")
            .len(),
        0
    );
}

#[test]
fn case_query_started_before_after_use_absolute_instants() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p101Absolute");
    let id = start_case(&engine, "p101Absolute", CmmnCaseInstanceStartRequest::new());
    let started_at: DateTime<Utc> = engine
        .runtime_service()
        .get_case_instance(&id)
        .expect("case")
        .started_at;

    let before = case_query(&engine)
        .started_before(started_at + chrono::Duration::milliseconds(1))
        .list()
        .expect("query")
        .len();
    assert_eq!(before, 1);
    let after = case_query(&engine)
        .started_after(started_at - chrono::Duration::milliseconds(1))
        .list()
        .expect("query")
        .len();
    assert_eq!(after, 1);
}

#[test]
fn case_query_supports_combined_filters() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p101Combined");
    let id_a = start_case(
        &engine,
        "p101Combined",
        CmmnCaseInstanceStartRequest::new()
            .with_name("Target")
            .with_business_key("bk-9")
            .with_started_by("alice"),
    );
    start_case(
        &engine,
        "p101Combined",
        CmmnCaseInstanceStartRequest::new()
            .with_name("Other")
            .with_business_key("bk-8")
            .with_started_by("bob"),
    );

    let result = case_query(&engine)
        .name("Target")
        .business_key_like("bk-%")
        .started_by("alice")
        .list()
        .expect("query")
        .into_iter()
        .map(|instance| instance.id)
        .collect::<Vec<_>>();
    assert_eq!(result, vec![id_a]);
}
