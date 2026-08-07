use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnCaseTask, CmmnDeploymentRequest,
    CmmnEngine, CmmnError, CmmnHumanTask, CmmnHumanTaskState, CmmnIdentityLink, CmmnJob,
    CmmnJobFamily, CmmnModel, CmmnPlanItem, CmmnProcessTask, CmmnProcessTaskRunner,
    CmmnProcessTaskStartRequest, CmmnProcessTaskStartResult, ProcessInstanceCleanup,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn simple_case_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));

    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "Review case",
        plan_model,
    )])
}

fn case_definition_key_for(deployment_index: usize) -> String {
    format!("cascadeDeleteCaseKey-{deployment_index}")
}

fn deploy_case(engine: &CmmnEngine, deployment_index: usize) -> String {
    let key = case_definition_key_for(deployment_index);
    let deployment = engine
        .deploy(
            CmmnDeploymentRequest::new(format!("cascade-delete-deployment-{deployment_index}"))
                .with_resource(format!("{key}.cmmn"), simple_case_model(&key)),
        )
        .expect("deployment should succeed");
    deployment.id
}

fn latest_case_definition_key(engine: &CmmnEngine, key: &str) -> String {
    engine
        .repository_service()
        .create_case_definition_query()
        .key(key)
        .single_result()
        .expect("definition query should succeed")
        .expect("definition should exist")
        .id
}

fn start_case_instance(engine: &CmmnEngine, case_definition_key: &str) -> String {
    let instance = engine
        .start_case_instance_by_key(case_definition_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance should start");
    instance.id
}

fn count_case_instances(engine: &CmmnEngine, case_definition_id: &str) -> usize {
    engine
        .runtime_service()
        .create_case_instance_query()
        .case_definition_id(case_definition_id)
        .list()
        .expect("case instance query should succeed")
        .len()
}

fn count_human_tasks(engine: &CmmnEngine, case_instance_id: &str) -> usize {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("human task query should succeed")
        .len()
}

fn count_human_tasks_by_definition(engine: &CmmnEngine, case_definition_id: &str) -> usize {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_definition_id(case_definition_id)
        .list()
        .expect("human task query should succeed")
        .len()
}

fn count_historic_case_instances(engine: &CmmnEngine, case_definition_id: &str) -> usize {
    engine
        .history_service()
        .create_historic_case_instance_query()
        .case_definition_id(case_definition_id)
        .list()
        .expect("historic case query should succeed")
        .len()
}

fn count_historic_human_tasks(engine: &CmmnEngine, case_definition_id: &str) -> usize {
    engine
        .history_service()
        .create_historic_human_task_query()
        .case_definition_id(case_definition_id)
        .list()
        .expect("historic human task query should succeed")
        .len()
}

fn count_event_subscriptions_by_definition(engine: &CmmnEngine, case_definition_id: &str) -> usize {
    engine
        .runtime_service()
        .create_event_subscription_query()
        .case_definition_id(case_definition_id)
        .list()
        .expect("event subscription query should succeed")
        .len()
}

fn count_identity_links_by_case(engine: &CmmnEngine, case_instance_id: &str) -> usize {
    engine
        .identity_link_service()
        .list_identity_links("caseInstance", case_instance_id)
        .expect("identity link query should succeed")
        .len()
}

fn insert_job_for_scope(
    engine: &CmmnEngine,
    scope_id: Option<&str>,
    sub_scope_id: Option<&str>,
    scope_definition_id: Option<&str>,
) -> String {
    let mut job = CmmnJob::new(
        format!("test-job-{}", Uuid::new_v4()),
        CmmnJobFamily::Executable,
    );
    job.scope_id = scope_id.map(str::to_string);
    job.sub_scope_id = sub_scope_id.map(str::to_string);
    job.scope_definition_id = scope_definition_id.map(str::to_string);
    job.state = "executable".to_string();
    engine
        .management_service()
        .insert_job(job)
        .expect("insert job should succeed")
        .id
}

fn jobs_count(engine: &CmmnEngine) -> usize {
    engine
        .management_service()
        .create_job_query()
        .list()
        .expect("job query should succeed")
        .len()
}

fn add_identity_link_for_case(
    engine: &CmmnEngine,
    case_instance_id: &str,
    user_id: &str,
) -> String {
    let link_id = format!("id-link:{}", Uuid::new_v4());
    let link = CmmnIdentityLink {
        id: link_id.clone(),
        scope_type: "caseInstance".to_string(),
        scope_id: case_instance_id.to_string(),
        link_type: "participant".to_string(),
        user_id: Some(user_id.to_string()),
        group_id: None,
    };
    engine
        .identity_link_service()
        .add_identity_link(link)
        .expect("add identity link should succeed");
    link_id
}

struct ProcessChildRunner;

impl CmmnProcessTaskRunner for ProcessChildRunner {
    fn start_process(
        &self,
        _request: CmmnProcessTaskStartRequest,
    ) -> Result<CmmnProcessTaskStartResult, CmmnError> {
        Ok(CmmnProcessTaskStartResult {
            process_instance_id: "external-bpmn-child-instance".to_string(),
            completed: false,
        })
    }
}

#[derive(Default)]
struct RecordingProcessCleanup {
    deleted: Mutex<Vec<String>>,
}

impl ProcessInstanceCleanup for RecordingProcessCleanup {
    fn delete_process_instance_cascade(&self, process_instance_id: &str) -> Result<(), CmmnError> {
        self.deleted
            .lock()
            .expect("cleanup mutex")
            .push(process_instance_id.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct FailingProcessCleanup;

impl ProcessInstanceCleanup for FailingProcessCleanup {
    fn delete_process_instance_cascade(&self, process_instance_id: &str) -> Result<(), CmmnError> {
        Err(CmmnError::execution(format!(
            "simulated BPMN cleanup failure for '{process_instance_id}'"
        )))
    }
}

#[test]
fn deployment_non_cascade_rejects_active_case_instances() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment_id = deploy_case(&engine, 0);
    let key = case_definition_key_for(0);
    let definition_id = latest_case_definition_key(&engine, &key);

    let instance_id = start_case_instance(&engine, &key);

    assert_eq!(count_case_instances(&engine, &definition_id), 1);
    assert!(count_human_tasks(&engine, &instance_id) >= 1);

    let err = engine
        .repository_service()
        .delete_deployment(&deployment_id, false)
        .expect_err("non-cascade delete must fail when active case instances exist");

    let message = err.to_string();
    assert!(
        message.contains("active case instances") || message.contains("cannot be deleted"),
        "unexpected error message: {message}"
    );

    assert_eq!(count_case_instances(&engine, &definition_id), 1);
    assert!(count_human_tasks(&engine, &instance_id) >= 1);
}

#[test]
fn deployment_cascade_delete_removes_all_owned_rows_without_touching_other_deployments() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let target_deployment_id = deploy_case(&engine, 1);
    let _other_deployment_id = deploy_case(&engine, 2);

    let target_key = case_definition_key_for(1);
    let other_key = case_definition_key_for(2);

    let target_definition_id = latest_case_definition_key(&engine, &target_key);
    let other_definition_id = latest_case_definition_key(&engine, &other_key);

    let target_instance_id = start_case_instance(&engine, &target_key);
    let second_target_instance_id = start_case_instance(&engine, &target_key);
    let other_instance_id = start_case_instance(&engine, &other_key);

    let target_human_tasks = count_human_tasks(&engine, &target_instance_id);
    assert!(
        target_human_tasks >= 1,
        "target deployment should have human tasks"
    );

    add_identity_link_for_case(&engine, &target_instance_id, "target-user");
    add_identity_link_for_case(&engine, &second_target_instance_id, "second-target-user");
    // Add definition-level candidate starter links (Task 14 cascade integration)
    engine
        .repository_service()
        .add_candidate_starter_user(&target_definition_id, "starter-user-target")
        .expect("add starter user");
    engine
        .repository_service()
        .add_candidate_starter_group(&target_definition_id, "starter-group-target")
        .expect("add starter group");
    engine
        .repository_service()
        .add_candidate_starter_user(&other_definition_id, "starter-user-other")
        .expect("add starter user for other");
    let target_job_id = insert_job_for_scope(
        &engine,
        Some(&target_instance_id),
        Some(&target_instance_id),
        Some(&target_definition_id),
    );

    let other_tasks_before = count_human_tasks(&engine, &other_instance_id);
    assert!(other_tasks_before >= 1);
    let other_job_id = insert_job_for_scope(
        &engine,
        Some(&other_instance_id),
        Some(&other_instance_id),
        Some(&other_definition_id),
    );
    let other_jobs_before = 1usize;

    let err = engine
        .repository_service()
        .delete_deployment(&target_deployment_id, false)
        .expect_err("non-cascade delete must fail while active case instances exist");
    assert!(err.to_string().contains("active case instances"));

    engine
        .repository_service()
        .delete_deployment(&target_deployment_id, true)
        .expect("cascade delete should succeed");

    assert!(
        count_case_instances(&engine, &target_definition_id) == 0,
        "target case instances must be removed by cascade delete"
    );
    assert!(
        count_human_tasks(&engine, &second_target_instance_id) == 0,
        "second target instance tasks must be removed by cascade delete"
    );
    assert!(
        count_human_tasks_by_definition(&engine, &target_definition_id) == 0,
        "target human tasks must be removed by cascade delete"
    );
    assert!(
        count_historic_case_instances(&engine, &target_definition_id) == 0,
        "target historic case instances must be removed by cascade delete"
    );
    assert!(
        count_historic_human_tasks(&engine, &target_definition_id) == 0,
        "target historic human tasks must be removed by cascade delete"
    );
    assert!(
        count_event_subscriptions_by_definition(&engine, &target_definition_id) == 0,
        "target definition event subscriptions must be removed by cascade delete"
    );
    assert!(
        count_identity_links_by_case(&engine, &target_instance_id) == 0,
        "target case identity links must be removed by cascade delete"
    );
    assert!(
        count_identity_links_by_case(&engine, &second_target_instance_id) == 0,
        "second target case identity links must be removed by cascade delete"
    );
    assert!(
        engine.management_service().get_job(&target_job_id).is_err(),
        "target deployment job must be deleted"
    );
    assert!(
        engine.management_service().get_job(&other_job_id).is_ok(),
        "unrelated deployment job must remain"
    );
    assert_eq!(
        jobs_count(&engine),
        other_jobs_before,
        "only the unrelated job remains"
    );

    assert_eq!(
        count_case_instances(&engine, &other_definition_id),
        1,
        "other deployment case instances must be preserved"
    );
    assert_eq!(
        count_human_tasks(&engine, &other_instance_id),
        other_tasks_before,
        "other deployment human tasks must be preserved"
    );
    assert!(
        count_historic_case_instances(&engine, &other_definition_id) >= 1,
        "other deployment historic case instances must be preserved"
    );

    // Definition-level candidate starter links must be removed for the target
    // definition but preserved for the other definition.
    assert!(
        engine
            .repository_service()
            .get_identity_links_for_case_definition(&target_definition_id)
            .expect("query target starter links")
            .is_empty(),
        "target definition candidate starter links must be removed by cascade delete"
    );
    assert_eq!(
        engine
            .repository_service()
            .get_identity_links_for_case_definition(&other_definition_id)
            .expect("query other starter links")
            .len(),
        1,
        "other definition candidate starter links must survive cascade delete"
    );

    let _ = other_jobs_before;
}

#[test]
fn deployment_cascade_delete_recursively_removes_child_case_from_another_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let child_model = CmmnModel::new(vec![CmmnCase::new(
        "child-case-model",
        "cascade-child-case",
        "Child case",
        CmmnCasePlanModel::new("child-plan-model", "Child plan")
            .with_human_task(CmmnHumanTask::new("child-task", "Child work"))
            .with_plan_item(CmmnPlanItem::new("child-plan-item", "child-task")),
    )]);
    let child_deployment = engine
        .deploy(
            CmmnDeploymentRequest::new("cascade-child-deployment")
                .with_resource("child.cmmn", child_model),
        )
        .expect("child deployment should succeed");
    let child_definition_id = latest_case_definition_key(&engine, "cascade-child-case");

    let parent_model = CmmnModel::new(vec![CmmnCase::new(
        "parent-case-model",
        "cascade-parent-case",
        "Parent case",
        CmmnCasePlanModel::new("parent-plan-model", "Parent plan")
            .with_case_task(
                CmmnCaseTask::new("parent-case-task", "Child case task")
                    .with_case_ref("cascade-child-case"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "parent-case-plan-item",
                "parent-case-task",
            )),
    )]);
    let parent_deployment = engine
        .deploy(
            CmmnDeploymentRequest::new("cascade-parent-deployment")
                .with_resource("parent.cmmn", parent_model),
        )
        .expect("parent deployment should succeed");

    let parent_instance = engine
        .start_case_instance_by_key("cascade-parent-case", CmmnCaseInstanceStartRequest::new())
        .expect("parent case should start");
    let association = engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("association query should succeed")
        .expect("parent case should create an association");
    let child_instance_id = association
        .child_instance_id
        .expect("case task should start its child case");

    engine
        .repository_service()
        .delete_deployment(&parent_deployment.id, true)
        .expect("cascade delete should succeed");

    assert_eq!(
        count_case_instances(&engine, &child_definition_id),
        0,
        "a child case started by the deleted parent must be purged even when its definition belongs to another deployment"
    );
    assert!(
        engine
            .runtime_service()
            .get_case_instance(&child_instance_id)
            .is_err(),
        "child runtime instance must no longer be addressable"
    );
    assert!(
        engine
            .repository_service()
            .get_deployment(&child_deployment.id)
            .is_ok(),
        "child deployment metadata must remain because only the child instance is owned by the parent"
    );
}

#[test]
fn deployment_cascade_delete_rejects_unsupported_process_child_without_partial_cleanup() {
    let engine = CmmnEngine::new_in_memory_with_process_task_runner(Arc::new(ProcessChildRunner))
        .expect("engine");
    let model = CmmnModel::new(vec![CmmnCase::new(
        "process-child-parent-case",
        "process-child-parent",
        "Process child parent",
        CmmnCasePlanModel::new("process-child-plan", "Process child plan")
            .with_process_task(
                CmmnProcessTask::new("external-process-task", "External process")
                    .with_process_ref("external-process"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "external-process-plan-item",
                "external-process-task",
            )),
    )]);
    let deployment = engine
        .deploy(
            CmmnDeploymentRequest::new("process-child-deployment")
                .with_resource("case.cmmn", model),
        )
        .expect("deployment should succeed");
    let definition_id = latest_case_definition_key(&engine, "process-child-parent");
    let instance_id = start_case_instance(&engine, "process-child-parent");
    let association_count = engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&instance_id)
        .list()
        .expect("association query should succeed")
        .len();
    assert_eq!(association_count, 1);
    add_identity_link_for_case(&engine, &instance_id, "process-child-owner");
    let job_id = insert_job_for_scope(
        &engine,
        Some(&instance_id),
        Some(&instance_id),
        Some(&definition_id),
    );

    let error = engine
        .repository_service()
        .delete_deployment(&deployment.id, true)
        .expect_err("unsupported process child cleanup must abort the transaction");
    assert!(error.to_string().contains("process"));

    assert_eq!(count_case_instances(&engine, &definition_id), 1);
    assert_eq!(
        count_identity_links_by_case(&engine, &instance_id),
        1,
        "the case identity link must survive rollback"
    );
    assert!(
        engine.management_service().get_job(&job_id).is_ok(),
        "the case job must survive rollback"
    );
    assert_eq!(
        engine
            .runtime_service()
            .create_task_association_query()
            .case_instance_id(&instance_id)
            .list()
            .expect("association query should succeed")
            .len(),
        1,
        "the association must survive rollback"
    );
    assert!(
        engine
            .repository_service()
            .get_deployment(&deployment.id)
            .is_ok()
    );
}

#[test]
fn deployment_cascade_delete_removes_bpmn_child_when_process_cleanup_injected() {
    let cleanup = Arc::new(RecordingProcessCleanup::default());
    let engine = CmmnEngine::new_in_memory_with_process_integrations(
        Some(Arc::new(ProcessChildRunner)),
        Some(cleanup.clone()),
    )
    .expect("engine");
    let model = CmmnModel::new(vec![CmmnCase::new(
        "process-child-parent-case",
        "process-child-parent",
        "Process child parent",
        CmmnCasePlanModel::new("process-child-plan", "Process child plan")
            .with_process_task(
                CmmnProcessTask::new("external-process-task", "External process")
                    .with_process_ref("external-process"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "external-process-plan-item",
                "external-process-task",
            )),
    )]);
    let deployment = engine
        .deploy(
            CmmnDeploymentRequest::new("process-child-deployment-with-cleanup")
                .with_resource("case.cmmn", model),
        )
        .expect("deployment should succeed");
    let definition_id = latest_case_definition_key(&engine, "process-child-parent");
    let instance_id = start_case_instance(&engine, "process-child-parent");
    assert_eq!(
        engine
            .runtime_service()
            .create_task_association_query()
            .case_instance_id(&instance_id)
            .list()
            .expect("association query")
            .len(),
        1
    );
    add_identity_link_for_case(&engine, &instance_id, "process-child-owner");
    let _job_id = insert_job_for_scope(
        &engine,
        Some(&instance_id),
        Some(&instance_id),
        Some(&definition_id),
    );

    engine
        .repository_service()
        .delete_deployment(&deployment.id, true)
        .expect("cascade delete should succeed when process cleanup is injected");

    let deleted = cleanup.deleted.lock().expect("cleanup mutex").clone();
    assert_eq!(
        deleted,
        vec!["external-bpmn-child-instance".to_string()],
        "injected cleanup must be invoked for the BPMN child process instance"
    );
    assert_eq!(
        count_case_instances(&engine, &definition_id),
        0,
        "parent case runtime must be purged after BPMN child cleanup"
    );
    assert_eq!(
        count_historic_case_instances(&engine, &definition_id),
        0,
        "parent historic case rows must be purged"
    );
    assert_eq!(
        count_identity_links_by_case(&engine, &instance_id),
        0,
        "case identity links must be purged"
    );
    assert!(
        engine
            .repository_service()
            .get_deployment(&deployment.id)
            .is_err(),
        "deployment metadata must be removed"
    );
    assert_eq!(
        engine
            .runtime_service()
            .create_task_association_query()
            .case_instance_id(&instance_id)
            .list()
            .expect("association query")
            .len(),
        0,
        "process-task associations must be removed"
    );
}

#[test]
fn deployment_cascade_delete_aborts_when_process_cleanup_fails() {
    let engine = CmmnEngine::new_in_memory_with_process_integrations(
        Some(Arc::new(ProcessChildRunner)),
        Some(Arc::new(FailingProcessCleanup)),
    )
    .expect("engine");
    let model = CmmnModel::new(vec![CmmnCase::new(
        "process-child-fail-case",
        "process-child-fail",
        "Process child fail",
        CmmnCasePlanModel::new("process-child-fail-plan", "Process child fail plan")
            .with_process_task(
                CmmnProcessTask::new("external-process-task", "External process")
                    .with_process_ref("external-process"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "external-process-plan-item",
                "external-process-task",
            )),
    )]);
    let deployment = engine
        .deploy(
            CmmnDeploymentRequest::new("process-child-fail-deployment")
                .with_resource("case.cmmn", model),
        )
        .expect("deployment should succeed");
    let definition_id = latest_case_definition_key(&engine, "process-child-fail");
    let instance_id = start_case_instance(&engine, "process-child-fail");

    let error = engine
        .repository_service()
        .delete_deployment(&deployment.id, true)
        .expect_err("cleanup failure must abort cascade");
    assert!(
        error.to_string().contains("simulated BPMN cleanup failure"),
        "cleanup failure should surface: {error}"
    );

    assert_eq!(
        count_case_instances(&engine, &definition_id),
        1,
        "CMMN runtime must roll back when BPMN cleanup fails"
    );
    assert!(
        engine
            .repository_service()
            .get_deployment(&deployment.id)
            .is_ok(),
        "deployment must survive when cascade aborts"
    );
    assert_eq!(
        engine
            .runtime_service()
            .create_task_association_query()
            .case_instance_id(&instance_id)
            .list()
            .expect("association query")
            .len(),
        1,
        "association must survive rollback"
    );
}
