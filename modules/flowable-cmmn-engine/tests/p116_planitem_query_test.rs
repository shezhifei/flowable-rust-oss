// P116: unified CMMN plan item instance query surface.
//
// Java references:
// - PlanItemInstanceEntityManagerImpl.java:83-152 (entity creation; elementId = plan item
//   id at :92, planItemDefinitionType = lowercased definition class at :94-99)
// - PlanItemInstanceQueryImpl.java:118-834 (query filter parameters)
// - PlanItemInstanceBaseResource.java:59-139 (REST query builders)
// - Occurs: OccurPlanItemInstanceOperation.java:34-61 (milestone / event listener -> COMPLETED)
// - Human task: HumanTaskActivityBehavior.java:82-178 (task entity created from the ACTIVE
//   plan item instance)
//
// Scope: stage / milestone / event listener plan item instances are mirrored into the
// unified ACT_CMMN_RU_PLAN_ITEM_INST table and queried via
// `CmmnRuntimeService::create_plan_item_instance_query`. Human-task plan items stay backed
// by ACT_CMMN_HUMAN_TASK (`CmmnHumanTaskQuery`) — the REST layer merges both sources.
// `timerEventListener` is not modeled by the Rust converter (its dispatch chain is a
// separate package), so the type filter matches nothing.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest,
    CmmnEngine, CmmnEventListener, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnMilestone, CmmnModel, CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry,
    CmmnStage,
};
use serde_json::json;

/// Case plan model with: one stage (holding one human task), one milestone (occurred by
/// completing the trigger task), one variable event listener (occurred by writing
/// `watchedVar`), plus trigger/keepalive human tasks keeping the case open.
fn model_with_stage_milestone_listener(case_key: &str) -> CmmnModel {
    let stage = CmmnStage::new("stage-work", "Work stage")
        .with_human_task(CmmnHumanTask::new("task-inner", "Inner task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-inner", "task-inner"));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_stage(stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-stage", "stage-work"))
        .with_milestone(CmmnMilestone::new("milestone-shipped", "Shipped"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-milestone", "milestone-shipped")
                .with_entry_criterion("sentry-after-trigger"),
        )
        .with_event_listener(
            CmmnEventListener::new("listener-watched", "variable")
                .with_name("Watched variable")
                .with_event_name("watchedVar"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-listener", "listener-watched"))
        .with_human_task(CmmnHumanTask::new("task-trigger", "Trigger"))
        .with_plan_item(CmmnPlanItem::new("plan-item-trigger", "task-trigger"))
        .with_human_task(CmmnHumanTask::new("task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new("plan-item-keepalive", "task-keepalive"))
        .with_sentry(CmmnSentry::new(
            "sentry-after-trigger",
            CmmnPlanItemOnPart::new("on-trigger-complete", "plan-item-trigger", "complete"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-p116",
        case_key,
        "P116 plan item query case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, case_key: &str) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), model_with_stage_milestone_listener(case_key)),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn plan_item_query(engine: &CmmnEngine) -> flowable_cmmn_engine::CmmnPlanItemInstanceQuery {
    engine.runtime_service().create_plan_item_instance_query()
}

fn task_query(engine: &CmmnEngine) -> flowable_cmmn_engine::CmmnHumanTaskQuery {
    engine.runtime_service().create_human_task_query()
}

fn complete_task_named(engine: &CmmnEngine, case_id: &str, name: &str) {
    let task = task_query(engine)
        .case_instance_id(case_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|task| task.name == name)
        .unwrap_or_else(|| panic!("task '{name}' not found"));
    engine
        .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("task completion");
}

#[test]
fn stage_milestone_and_event_listener_are_queryable_by_case() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p116Queryable");

    // Java `CmmnOperation.java:117-210` materializes every plan item while activating
    // its container, so the sentry-waiting milestone is queryable as AVAILABLE too.
    let mut mirrors = plan_item_query(&engine)
        .case_instance_id(&case_id)
        .list()
        .expect("mirror query");
    mirrors.sort_by(|a, b| a.plan_item_definition_type.cmp(&b.plan_item_definition_type));
    assert_eq!(mirrors.len(), 3, "stage + event listener + available milestone");
    assert_eq!(mirrors[0].plan_item_definition_type, "eventlistener");
    assert_eq!(mirrors[0].state, "AVAILABLE");
    assert_eq!(mirrors[0].name, "Watched variable");
    assert_eq!(mirrors[1].plan_item_definition_type, "milestone");
    assert_eq!(mirrors[1].state, "AVAILABLE");
    assert!(mirrors[1].ended_at.is_none());
    assert!(mirrors[1].occurred_at.is_none());
    let milestone_instance_id = mirrors[1].id.clone();
    assert_eq!(mirrors[2].plan_item_definition_type, "stage");
    assert_eq!(mirrors[2].state, "ACTIVE");
    assert_eq!(mirrors[2].name, "Work stage");
    assert_eq!(mirrors[2].plan_item_id, "plan-item-stage");
    assert_eq!(mirrors[2].plan_item_definition_id, "stage-work");

    // Human-task plan items come from ACT_CMMN_HUMAN_TASK: three active tasks.
    let tasks = task_query(&engine).case_instance_id(&case_id).list().expect("tasks");
    assert_eq!(tasks.len(), 3);

    // Occur the milestone and the event listener.
    complete_task_named(&engine, &case_id, "Trigger");
    engine
        .runtime_service()
        .set_case_instance_variables(&case_id, vec![("watchedVar".to_string(), json!("go"))])
        .expect("variable write");

    // Java runtime queries no longer expose terminal plan-item rows. The stage is
    // still ACTIVE; the occurred milestone and listener move to historic-only mirrors.
    let mut mirrors = plan_item_query(&engine)
        .case_instance_id(&case_id)
        .list()
        .expect("mirror query");
    mirrors.sort_by(|a, b| a.plan_item_definition_type.cmp(&b.plan_item_definition_type));
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].plan_item_definition_type, "stage");
    assert_eq!(mirrors[0].state, "ACTIVE");

    let milestone = plan_item_query(&engine)
        .include_ended()
        .id(milestone_instance_id.clone())
        .single_result()
        .expect("retained milestone query")
        .expect("retained milestone");
    assert_eq!(milestone.state, "COMPLETED");
    assert_eq!(milestone.id, milestone_instance_id, "occur updates the same row");
    assert!(milestone.ended_at.is_some());
    assert!(milestone.occurred_at.is_some());

    // `timerEventListener` is not modeled by the Rust converter → matches nothing.
    assert_eq!(
        plan_item_query(&engine)
            .plan_item_definition_type("timereventlistener")
            .list()
            .expect("timer type")
            .len(),
        0
    );
}

#[test]
fn filters_by_type_state_name_element_and_definition_id() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p116Filters");
    complete_task_named(&engine, &case_id, "Trigger");

    // planItemDefinitionType — case-insensitive, Java stores the lowercased type.
    assert_eq!(
        plan_item_query(&engine).plan_item_definition_type("stage").list().expect("q").len(),
        1
    );
    assert_eq!(
        plan_item_query(&engine).plan_item_definition_type("Stage").list().expect("q").len(),
        1
    );
    assert_eq!(
        plan_item_query(&engine)
            .plan_item_definition_type("milestone")
            .list()
            .expect("q")
            .len(),
        0
    );
    assert_eq!(
        plan_item_query(&engine)
            .plan_item_definition_type("eventlistener")
            .list()
            .expect("q")
            .len(),
        1
    );
    assert_eq!(
        plan_item_query(&engine)
            .plan_item_definition_types(vec!["stage".to_string(), "milestone".to_string()])
            .list()
            .expect("q")
            .len(),
        1
    );

    // state — UPPERCASE Rust convention (PlanItemInstanceState values are lowercase in Java;
    // the Rust engine stores the same strings uppercase).
    assert_eq!(
        plan_item_query(&engine).state("ACTIVE").list().expect("q").len(),
        1
    );
    assert_eq!(
        plan_item_query(&engine).state("COMPLETED").list().expect("q").len(),
        0
    );
    assert_eq!(
        plan_item_query(&engine).state("AVAILABLE").list().expect("q").len(),
        1
    );

    // name / nameLike / nameLikeIgnoreCase.
    assert_eq!(plan_item_query(&engine).name("Shipped").list().expect("q").len(), 0);
    assert_eq!(plan_item_query(&engine).name_like("Shipped%").list().expect("q").len(), 0);
    assert_eq!(
        plan_item_query(&engine)
            .name_like_ignore_case("shipped%")
            .list()
            .expect("q")
            .len(),
        0
    );

    // elementId — the plan item id (Java planItemInstanceElementId).
    assert_eq!(
        plan_item_query(&engine)
            .element_id("plan-item-milestone")
            .list()
            .expect("q")
            .len(),
        0
    );
    assert_eq!(
        plan_item_query(&engine).element_id("plan-item-stage").list().expect("q").len(),
        1
    );

    // planItemDefinitionId — the definition id (Java filters ITEM_DEFINITION_ID_).
    assert_eq!(
        plan_item_query(&engine)
            .plan_item_definition_id("milestone-shipped")
            .list()
            .expect("q")
            .len(),
        0
    );
    assert_eq!(
        plan_item_query(&engine)
            .plan_item_definition_id("stage-work")
            .list()
            .expect("q")
            .len(),
        1
    );

    // caseInstanceId filter narrows to this case.
    assert_eq!(
        plan_item_query(&engine).case_instance_id(&case_id).list().expect("q").len(),
        2
    );
    assert_eq!(
        plan_item_query(&engine).case_instance_id("no-such-case").list().expect("q").len(),
        0
    );
}

#[test]
fn milestone_occur_transition_is_reflected_in_query_state() {
    // OccurPlanItemInstanceOperation.java:34-61 — a milestone reaches COMPLETED on occur.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p116MilestoneTransition");

    let available = plan_item_query(&engine)
        .plan_item_definition_type("milestone")
        .single_result()
        .expect("available milestone query")
        .expect("pre-materialized milestone row");
    assert_eq!(available.state, "AVAILABLE");
    let available_id = available.id;

    complete_task_named(&engine, &case_id, "Trigger");

    assert!(
        plan_item_query(&engine)
            .plan_item_definition_type("milestone")
            .single_result()
            .expect("runtime milestone query")
            .is_none(),
        "terminal milestones are hidden from runtime queries"
    );
    let milestone = plan_item_query(&engine)
        .include_ended()
        .plan_item_definition_type("milestone")
        .single_result()
        .expect("milestone query")
        .expect("milestone row");
    assert_eq!(milestone.id, available_id, "occur must not insert a duplicate row");
    assert_eq!(milestone.state, "COMPLETED");
    assert!(milestone.ended_at.is_some());
    assert!(milestone.occurred_at.is_some());
}

#[test]
fn stage_instance_id_filter_applies_to_child_human_tasks() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p116StageInst");

    // The stage mirror's own id is the stage plan item instance id.
    let stage = plan_item_query(&engine)
        .plan_item_definition_type("stage")
        .single_result()
        .expect("stage query")
        .expect("stage row");
    assert_eq!(stage.stage_instance_id, None, "the stage itself has no parent stage");

    // The inner human task carries the stage instance id (HumanTaskActivityBehavior
    // task scope = plan item instance; Rust stores stage_instance_id on the task).
    let inner = task_query(&engine)
        .case_instance_id(&case_id)
        .stage_instance_id(&stage.id)
        .single_result()
        .expect("inner query")
        .expect("inner task");
    assert_eq!(inner.name, "Inner task");

    // Mirror rows do not thread a parent stage id except for milestones reached inside a
    // stage, so a stage-instance filter matches nothing on the mirror here (documented).
    assert_eq!(
        plan_item_query(&engine)
            .stage_instance_id(&stage.id)
            .list()
            .expect("q")
            .len(),
        0
    );
}

#[test]
fn plan_item_mirror_does_not_affect_stage_completion() {
    // Regression guard: the P116 mirror is additive — C8 stage completion (counts over
    // ACT_CMMN_HUMAN_TASK / ACT_CMMN_STAGE_INSTANCE / ACT_CMMN_EVENT_SUBSCRIPTION) is
    // unchanged, so completing the stage's only task completes the stage.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p116C8");

    complete_task_named(&engine, &case_id, "Inner task");

    assert!(
        plan_item_query(&engine)
            .plan_item_definition_type("stage")
            .single_result()
            .expect("runtime stage query")
            .is_none(),
        "completed stages are hidden from runtime queries"
    );
    let stage = plan_item_query(&engine)
        .include_ended()
        .plan_item_definition_type("stage")
        .single_result()
        .expect("stage query")
        .expect("stage row");
    assert_eq!(
        stage.state, "COMPLETED",
        "stage completes once its only child task completes"
    );
}
