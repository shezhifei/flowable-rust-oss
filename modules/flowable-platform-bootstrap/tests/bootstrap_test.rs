use flowable_app_engine::{
    AppDefinition, AppDeploymentRequest, AppModel, AppPage, AppReference, DefinitionType,
};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnHumanTask, CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
    CmmnPlanItemOnPart, CmmnProcessTask, CmmnSentry, CmmnTaskAssociationState,
};
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use flowable_event_registry_service::{
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
};
use flowable_platform_bootstrap::{FlowablePlatform, PlatformConfiguration};

fn isolated_platform_config(engine_name: &str) -> PlatformConfiguration {
    let mut configuration = PlatformConfiguration::default();
    configuration.process.engine_name = engine_name.to_string();
    configuration.process.database_path = ":memory:".to_string();
    configuration.dmn.database_path = Some(":memory:".to_string());
    configuration.cmmn.database_path = Some(":memory:".to_string());
    configuration.app.database_path = Some(":memory:".to_string());
    configuration
}

#[test]
fn bootstrap_uses_isolated_memory_databases_for_memory_module_paths() {
    let first = FlowablePlatform::bootstrap(isolated_platform_config("memory-isolation-first"))
        .expect("first platform");
    let second = FlowablePlatform::bootstrap(isolated_platform_config("memory-isolation-second"))
        .expect("second platform");

    first
        .cmmn_engine()
        .deploy(
            CmmnDeploymentRequest::new("first cmmn deployment").with_resource(
                "first.cmmn",
                CmmnModel::new(vec![CmmnCase::new(
                    "first-case",
                    "firstCase",
                    "First case",
                    CmmnCasePlanModel::new("first-plan", "First plan"),
                )]),
            ),
        )
        .expect("first cmmn deployment");

    assert_eq!(
        second
            .cmmn_engine()
            .repository_service()
            .create_case_definition_query()
            .list()
            .expect("second cmmn definitions")
            .len(),
        0
    );
}

#[test]
fn bootstraps_owned_engine_graph_and_default_admin() {
    let mut configuration = isolated_platform_config("bootstrap-test");
    // Explicit opt-in: create_default_admin is false by default (security deviation from Java).
    configuration.bootstrap.create_default_admin = true;
    configuration.bootstrap.admin_password = "bootstrap-secret".to_string();

    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");
    let process_engine = platform.process_engine();

    assert_eq!(process_engine.get_name(), "bootstrap-test");
    assert!(
        process_engine
            .get_identity_service()
            .check_password("admin", "bootstrap-secret")
    );
    assert!(process_engine.get_config().dmn_engine.is_some());
    assert_eq!(
        platform
            .dmn_engine()
            .repository_service()
            .create_decision_query()
            .list()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        platform
            .cmmn_engine()
            .repository_service()
            .create_case_definition_query()
            .list()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        platform
            .app_engine()
            .repository_service()
            .create_app_definition_query()
            .list()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn default_configuration_does_not_create_admin() {
    let configuration = isolated_platform_config("no-default-admin");
    assert!(
        !configuration.bootstrap.create_default_admin,
        "create_default_admin must default to false"
    );
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");
    assert!(
        platform
            .process_engine()
            .get_identity_service()
            .find_user_by_id("admin")
            .is_none(),
        "default config must not seed admin user"
    );
}

#[test]
fn create_default_admin_with_default_password_is_rejected() {
    let mut configuration = isolated_platform_config("reject-default-password");
    configuration.bootstrap.create_default_admin = true;
    configuration.bootstrap.admin_password = "admin".to_string();
    let err = match FlowablePlatform::bootstrap(configuration) {
        Ok(_) => panic!("must refuse admin/admin"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("admin") && msg.contains("password"),
        "expected refuse-default-password message, got: {msg}"
    );
}

#[test]
#[test]
fn platform_app_engine_rehydrates_cached_composition_via_deployment_manager() {
    use flowable_app_engine::{
        AppDefinition, AppDeploymentRequest, AppModel, AppPage, AppReference,
    };

    let configuration = isolated_platform_config("app-cache-platform-test");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    // Seed a BPMN definition so the platform catalog can resolve app references.
    let process_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="employeeOnboarding" name="Employee Onboarding">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    let repository_service = platform.process_engine().get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("onboarding".to_string())
                .add_string(
                    "onboarding.bpmn20.xml".to_string(),
                    process_xml.to_string(),
                ),
        )
        .expect("process deployment");

    let app_engine = platform.app_engine();
    app_engine
        .deploy(
            AppDeploymentRequest::new("employee-apps").with_resource(
                "employee-app.json",
                AppModel::new().with_app_definition(
                    AppDefinition::new("app-employee", "employee-portal", "Employee Portal")
                        .with_page(
                            AppPage::new("page-process", "Process Dashboard").with_reference(
                                AppReference::process("start-onboarding")
                                    .with_definition_key("employeeOnboarding"),
                            ),
                        ),
                ),
            ),
        )
        .expect("app deploy");

    let definition_id = app_engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .single_result()
        .unwrap()
        .expect("definition")
        .id;

    let warm = app_engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert_eq!(warm.composition.references.len(), 1);
    let resolved_id = warm.composition.references[0]
        .resolved_definition_id
        .clone();

    app_engine
        .deployment_manager()
        .evict_app_definition(&definition_id);
    assert!(
        !app_engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );

    let cold = app_engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert_eq!(
        cold.composition.references[0].resolved_definition_id,
        resolved_id
    );
    assert_eq!(cold.definition.model.key, "employee-portal");
}

#[test]
fn platform_app_resolves_event_registry_references_after_deploy_and_cache_change() {
    let configuration = isolated_platform_config("platform-app-event-registry");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    let event_registry =
        FlowableEventRegistryService::new(std::sync::Arc::clone(&platform.process_engine()));
    event_registry
        .deploy(EventRegistryDeploymentRequest {
            name: "employee events v1".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "employee-updated.event".to_string(),
                    resource: r#"{"key":"employeeUpdated","name":"Employee Updated","eventType":"employee.updated","channelKey":"employeeInbound","resourceName":"employee-updated.event","payload":[]}"#.to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "employee-inbound.channel".to_string(),
                    resource: r#"{"key":"employeeInbound","name":"Employee inbound","channelType":"inbound","resourceName":"employee-inbound.channel","type":"in-memory"}"#.to_string(),
                },
            ],
        })
        .expect("event registry deploy v1");

    platform
        .app_engine()
        .deploy(
            AppDeploymentRequest::new("employee-apps").with_resource(
                "employee-app.json",
                AppModel::new().with_app_definition(
                    AppDefinition::new("app-employee", "employee-portal", "Employee Portal")
                        .with_page(
                            AppPage::new("page-events", "Events").with_reference(
                                AppReference::event("employee-event-page")
                                    .with_definition_key("employeeUpdated"),
                            ),
                        ),
                ),
            ),
        )
        .expect("app deploy");

    let composition = platform
        .app_engine()
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal", None)
        .expect("resolve composition");
    assert_eq!(composition.references.len(), 1);
    assert_eq!(
        composition.references[0].definition_type,
        DefinitionType::EventRegistry
    );
    assert_eq!(
        composition.references[0].resolved_definition_key,
        "employeeUpdated"
    );
    assert_eq!(composition.references[0].resolved_definition_version, 1);
    let v1_id = composition.references[0].resolved_definition_id.clone();

    // Deploy a new version and ensure catalog/cache reconciliation surfaces it for a new app deploy.
    event_registry
        .deploy(EventRegistryDeploymentRequest {
            name: "employee events v2".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "employee-updated.event".to_string(),
                    resource: r#"{"key":"employeeUpdated","name":"Employee Updated v2","eventType":"employee.updated","channelKey":"employeeInbound","resourceName":"employee-updated.event","payload":[]}"#.to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "employee-inbound.channel".to_string(),
                    resource: r#"{"key":"employeeInbound","name":"Employee inbound","channelType":"inbound","resourceName":"employee-inbound.channel","type":"in-memory"}"#.to_string(),
                },
            ],
        })
        .expect("event registry deploy v2");

    platform
        .app_engine()
        .deploy(
            AppDeploymentRequest::new("employee-apps-v2").with_resource(
                "employee-app-v2.json",
                AppModel::new().with_app_definition(
                    AppDefinition::new("app-employee-v2", "employee-portal-v2", "Employee Portal V2")
                        .with_page(
                            AppPage::new("page-events", "Events").with_reference(
                                AppReference::event("employee-event-page")
                                    .with_definition_key("employeeUpdated"),
                            ),
                        ),
                ),
            ),
        )
        .expect("app deploy v2");

    let composition_v2 = platform
        .app_engine()
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal-v2", None)
        .expect("resolve composition v2");
    assert_eq!(composition_v2.references[0].resolved_definition_version, 2);
    assert_ne!(composition_v2.references[0].resolved_definition_id, v1_id);
}


#[test]
fn platform_wires_cmmn_process_task_to_real_bpmn_engine_and_completion_callback() {
    let configuration = isolated_platform_config("cmmn-process-task-platform-test");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    let process_engine = platform.process_engine();
    let repository_service = process_engine.get_repository_service();
    let task_service = process_engine.get_task_service();

    let process_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="approvalProcess" name="Approval Process">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask" />
            <userTask id="approveTask" name="Approve" />
            <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("approval process".to_string())
                .add_string(
                    "approvalProcess.bpmn20.xml".to_string(),
                    process_xml.to_string(),
                ),
        )
        .expect("process deployment");

    let cmmn_engine = platform.cmmn_engine();
    let sentry = CmmnSentry::new(
        "after-process",
        CmmnPlanItemOnPart::new("on-process-complete", "plan-item-process", "complete"),
    );
    let parent_case = CmmnCase::new(
        "case-process-parent",
        "platformProcessTaskParent",
        "Platform process task parent",
        CmmnCasePlanModel::new("parent-plan", "Parent plan")
            .with_process_task(
                CmmnProcessTask::new("process-task-approval", "Approval process")
                    .with_process_ref("approvalProcess"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-approval",
            ))
            .with_human_task(CmmnHumanTask::new("human-task-archive", "Archive"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-archive", "human-task-archive")
                    .with_entry_criterion("after-process"),
            )
            .with_sentry(sentry),
    );
    cmmn_engine
        .deploy(
            CmmnDeploymentRequest::new("platform process task")
                .with_resource("process-task.cmmn", CmmnModel::new(vec![parent_case])),
        )
        .expect("cmmn deployment");

    let parent_instance = cmmn_engine
        .start_case_instance_by_key(
            "platformProcessTaskParent",
            CmmnCaseInstanceStartRequest::new().with_business_key("BK-PLATFORM-PROCESS"),
        )
        .expect("parent case");

    let association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("association query")
        .expect("association");
    assert_eq!(association.state, CmmnTaskAssociationState::Active);
    let child_process_instance_id = association
        .child_instance_id
        .clone()
        .expect("child process instance id");

    let process_tasks = task_service
        .get_tasks_by_process_instance_id(child_process_instance_id.clone())
        .expect("process tasks");
    assert_eq!(process_tasks.len(), 1);
    assert_eq!(process_tasks[0].task_definition_key, "approveTask");

    task_service
        .complete_task_by_id(process_tasks[0].id.clone())
        .expect("complete process task");

    let updated_association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .id(&association.id)
        .single_result()
        .expect("association query")
        .expect("association");
    assert_eq!(
        updated_association.state,
        CmmnTaskAssociationState::Completed
    );

    let archive_task = cmmn_engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&parent_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("archive query")
        .expect("archive task");
    assert_eq!(archive_task.name, "Archive");
}

#[test]
fn platform_fails_cmmn_bpmn_child_when_child_ends_with_uncaught_error_end_event() {
    let configuration = isolated_platform_config("cmmn-process-task-uncaught-error-test");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    // BPMN child: complete the user task to reach an uncaught error end event.
    let process_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="approvalProcess" name="Approval Process">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask" />
            <userTask id="approveTask" name="Approve" />
            <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="throwError" />
            <endEvent id="throwError">
                <errorEventDefinition errorCode="UNCAUGHT_FLOW" />
            </endEvent>
        </process>
    </definitions>"#;
    deploy_process_task_parent_with_terminate_sentry(
        &platform,
        "platformUncaughtErrorProcessTaskParent",
        process_xml,
    );

    let cmmn_engine = platform.cmmn_engine();
    let parent_instance = cmmn_engine
        .start_case_instance_by_key(
            "platformUncaughtErrorProcessTaskParent",
            CmmnCaseInstanceStartRequest::new().with_business_key("BK-UNCAUGHT-ERR"),
        )
        .expect("parent case");

    let association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("association query")
        .expect("association");
    assert_eq!(association.state, CmmnTaskAssociationState::Active);
    let child_process_instance_id = association
        .child_instance_id
        .clone()
        .expect("child process instance id");

    let process_engine = platform.process_engine();
    let task_service = process_engine.get_task_service();

    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child_process_instance_id.clone())
        .expect("child tasks");
    assert_eq!(child_tasks.len(), 1);
    assert_eq!(child_tasks[0].task_definition_key, "approveTask");

    // Completing the child user task should cause the uncaught error end event
    // to fire, which should end the process instance and propagate as a
    // CMMN processTask failure.
    task_service
        .complete_task_by_id(child_tasks[0].id.clone())
        .expect("complete child task");

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    let process_instance = store
        .find_process_instance(&child_process_instance_id, &mut session)
        .expect("child process instance still locatable");
    session.rollback().expect("rollback");
    assert!(
        process_instance.is_ended,
        "uncaught error end event must end the BPMN child process instance"
    );

    let updated_association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .id(&association.id)
        .single_result()
        .expect("association query")
        .expect("association");
    assert_eq!(updated_association.state, CmmnTaskAssociationState::Failed);
    assert!(updated_association.completed_at.is_some());
    let failure_message = updated_association
        .failure_message
        .clone()
        .expect("failure message");
    assert!(
        failure_message.contains("UNCAUGHT_FLOW"),
        "failure message should carry the uncaught error code: {failure_message}"
    );

    let recovery_task = cmmn_engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&parent_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("recovery query")
        .expect("recovery task");
    assert_eq!(recovery_task.name, "Recovery");
}

#[test]
fn platform_fails_cmmn_bpmn_child_when_child_ends_with_uncaught_error_end_event_without_error_code()
{
    let configuration = isolated_platform_config("cmmn-process-task-uncaught-no-code-test");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    // BPMN child: error end event without an explicit errorCode should still
    // be treated as an uncaught failure that ends the process instance.
    let process_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="approvalProcess" name="Approval Process">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask" />
            <userTask id="approveTask" name="Approve" />
            <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="throwError" />
            <endEvent id="throwError">
                <errorEventDefinition />
            </endEvent>
        </process>
    </definitions>"#;
    deploy_process_task_parent_with_terminate_sentry(
        &platform,
        "platformUncaughtNoCodeProcessTaskParent",
        process_xml,
    );

    let cmmn_engine = platform.cmmn_engine();
    let parent_instance = cmmn_engine
        .start_case_instance_by_key(
            "platformUncaughtNoCodeProcessTaskParent",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("parent case");

    let association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("association query")
        .expect("association");
    let child_process_instance_id = association
        .child_instance_id
        .clone()
        .expect("child process instance id");

    let process_engine = platform.process_engine();
    let task_service = process_engine.get_task_service();

    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child_process_instance_id.clone())
        .expect("child tasks");
    assert_eq!(child_tasks.len(), 1);
    task_service
        .complete_task_by_id(child_tasks[0].id.clone())
        .expect("complete child task");

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    let process_instance = store
        .find_process_instance(&child_process_instance_id, &mut session)
        .expect("child process instance still locatable");
    session.rollback().expect("rollback");
    assert!(process_instance.is_ended);

    let updated_association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .id(&association.id)
        .single_result()
        .expect("association query")
        .expect("association");
    assert_eq!(updated_association.state, CmmnTaskAssociationState::Failed);
    assert!(updated_association.failure_message.is_some());

    let recovery_task = cmmn_engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&parent_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("recovery query")
        .expect("recovery task");
    assert_eq!(recovery_task.name, "Recovery");
}

fn deploy_process_task_parent_with_terminate_sentry(
    platform: &FlowablePlatform,
    parent_case_key: &str,
    process_xml: &str,
) {
    let process_engine = platform.process_engine();
    let repository_service = process_engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("approval process".to_string())
                .add_string(
                    "approvalProcess.bpmn20.xml".to_string(),
                    process_xml.to_string(),
                ),
        )
        .expect("process deployment");

    let cmmn_engine = platform.cmmn_engine();
    let sentry = CmmnSentry::new(
        "after-process-failure",
        CmmnPlanItemOnPart::new("on-process-failure", "plan-item-process", "terminate"),
    );
    let parent_case = CmmnCase::new(
        "case-process-parent",
        parent_case_key,
        "Platform process task parent",
        CmmnCasePlanModel::new("parent-plan", "Parent plan")
            .with_process_task(
                CmmnProcessTask::new("process-task-approval", "Approval process")
                    .with_process_ref("approvalProcess"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-approval",
            ))
            .with_human_task(CmmnHumanTask::new("human-task-recovery", "Recovery"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-recovery", "human-task-recovery")
                    .with_entry_criterion("after-process-failure"),
            )
            .with_sentry(sentry),
    );
    cmmn_engine
        .deploy(
            CmmnDeploymentRequest::new("platform process task failure")
                .with_resource("process-task.cmmn", CmmnModel::new(vec![parent_case])),
        )
        .expect("cmmn deployment");
}

fn assert_process_task_association_failed_and_recovery_activated(
    cmmn_engine: &flowable_cmmn_engine::CmmnEngine,
    parent_instance_id: &str,
    association_id: &str,
) {
    let updated_association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .id(association_id)
        .single_result()
        .expect("association query")
        .expect("association");
    assert_eq!(updated_association.state, CmmnTaskAssociationState::Failed);
    assert!(updated_association.completed_at.is_some());
    assert!(updated_association.failure_message.is_some());

    let recovery_task = cmmn_engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(parent_instance_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("recovery query")
        .expect("recovery task");
    assert_eq!(recovery_task.name, "Recovery");

    // The case instance must remain active while the recovery task is pending.
    let case_instance = cmmn_engine
        .runtime_service()
        .get_case_instance(parent_instance_id)
        .expect("case instance");
    assert_eq!(case_instance.state, CmmnCaseInstanceState::Active);
    let historic_case = cmmn_engine
        .history_service()
        .get_historic_case_instance(parent_instance_id)
        .expect("historic case instance");
    assert_eq!(historic_case.state, CmmnCaseInstanceState::Active);
    assert!(historic_case.completed_at.is_none());
}

#[test]
fn platform_deletes_cmmn_bpmn_child_and_triggers_failure_sentry() {
    let configuration = isolated_platform_config("cmmn-process-task-delete-test");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    let process_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="approvalProcess" name="Approval Process">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask" />
            <userTask id="approveTask" name="Approve" />
            <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    deploy_process_task_parent_with_terminate_sentry(
        &platform,
        "platformDeleteProcessTaskParent",
        process_xml,
    );

    let cmmn_engine = platform.cmmn_engine();
    let parent_instance = cmmn_engine
        .start_case_instance_by_key(
            "platformDeleteProcessTaskParent",
            CmmnCaseInstanceStartRequest::new().with_business_key("BK-DELETE-CHILD"),
        )
        .expect("parent case");

    let association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("association query")
        .expect("association");
    assert_eq!(association.state, CmmnTaskAssociationState::Active);
    let child_process_instance_id = association
        .child_instance_id
        .clone()
        .expect("child process instance id");

    let process_engine = platform.process_engine();
    let runtime_service = process_engine.get_runtime_service();
    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    let process_instance = store
        .find_process_instance(&child_process_instance_id, &mut session)
        .expect("child process instance");
    session.rollback().expect("rollback");
    assert_eq!(
        process_instance.callback_type.as_deref(),
        Some(flowable_cmmn_engine::CMMN_PROCESS_TASK_CALLBACK_TYPE)
    );

    runtime_service
        .delete_process_instance(
            child_process_instance_id.clone(),
            Some("forced delete".into()),
        )
        .expect("delete process instance");

    assert_process_task_association_failed_and_recovery_activated(
        &cmmn_engine,
        &parent_instance.id,
        &association.id,
    );
}

#[test]
fn platform_terminates_cmmn_bpmn_child_via_change_state_and_triggers_failure_sentry() {
    let configuration = isolated_platform_config("cmmn-process-task-change-state-test");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    let process_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="approvalProcess" name="Approval Process">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask" />
            <userTask id="approveTask" name="Approve" />
            <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    deploy_process_task_parent_with_terminate_sentry(
        &platform,
        "platformTerminateProcessTaskParent",
        process_xml,
    );

    let cmmn_engine = platform.cmmn_engine();
    let parent_instance = cmmn_engine
        .start_case_instance_by_key(
            "platformTerminateProcessTaskParent",
            CmmnCaseInstanceStartRequest::new().with_business_key("BK-TERMINATE-CHILD"),
        )
        .expect("parent case");

    let association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("association query")
        .expect("association");
    let child_process_instance_id = association
        .child_instance_id
        .clone()
        .expect("child process instance id");

    let process_engine = platform.process_engine();
    let runtime_service = process_engine.get_runtime_service();

    runtime_service
        .change_process_instance_activity_state(
            child_process_instance_id.clone(),
            vec!["approveTask".to_string()],
            vec![],
        )
        .expect("change state terminates child process");

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    let ended_child = store
        .find_process_instance(&child_process_instance_id, &mut session)
        .expect("child process instance still present after change-state");
    session.rollback().expect("rollback");
    assert!(ended_child.is_ended);

    assert_process_task_association_failed_and_recovery_activated(
        &cmmn_engine,
        &parent_instance.id,
        &association.id,
    );
}

#[test]
fn platform_fails_cmmn_bpmn_child_when_callback_metadata_is_cleared_via_update() {
    let configuration = isolated_platform_config("cmmn-process-task-callback-update-test");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    let process_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="approvalProcess" name="Approval Process">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask" />
            <userTask id="approveTask" name="Approve" />
            <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    deploy_process_task_parent_with_terminate_sentry(
        &platform,
        "platformClearCallbackParent",
        process_xml,
    );

    let cmmn_engine = platform.cmmn_engine();
    let parent_instance = cmmn_engine
        .start_case_instance_by_key(
            "platformClearCallbackParent",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("parent case");

    let association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("association query")
        .expect("association");
    let child_process_instance_id = association
        .child_instance_id
        .clone()
        .expect("child process instance id");

    let process_engine = platform.process_engine();
    let runtime_service = process_engine.get_runtime_service();

    runtime_service
        .update_process_instance(
            child_process_instance_id.clone(),
            ProcessInstanceUpdate {
                callback_type: Some(None),
                callback_id: Some(None),
                reference_id: Some(None),
                reference_type: Some(None),
                ..Default::default()
            },
        )
        .expect("clear callback metadata");

    runtime_service
        .delete_process_instance(
            child_process_instance_id.clone(),
            Some("delete without cmmn callback".to_string()),
        )
        .expect("delete process instance");

    let still_active_association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .id(&association.id)
        .single_result()
        .expect("association query")
        .expect("association");
    assert_eq!(
        still_active_association.state,
        CmmnTaskAssociationState::Active
    );
    assert!(
        cmmn_engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&parent_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("task query")
            .is_empty()
    );
}

#[test]
fn platform_cascade_delete_cmmn_deployment_removes_bpmn_child_process_instance() {
    let configuration = isolated_platform_config("cmmn-cascade-bpmn-child-cleanup-test");
    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");

    let process_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="cascadeChildProcess" name="Cascade Child Process">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask" />
            <userTask id="approveTask" name="Approve" />
            <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    platform
        .process_engine()
        .get_repository_service()
        .deploy(
            platform
                .process_engine()
                .get_repository_service()
                .create_deployment()
                .name("cascade child process".to_string())
                .add_string(
                    "cascadeChildProcess.bpmn20.xml".to_string(),
                    process_xml.to_string(),
                ),
        )
        .expect("process deployment");

    let cmmn_engine = platform.cmmn_engine();
    let parent_case = CmmnCase::new(
        "case-cascade-process-parent",
        "platformCascadeProcessParent",
        "Platform cascade process parent",
        CmmnCasePlanModel::new("parent-plan", "Parent plan")
            .with_process_task(
                CmmnProcessTask::new("process-task-child", "Child process")
                    .with_process_ref("cascadeChildProcess"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-child",
            )),
    );
    let deployment = cmmn_engine
        .deploy(
            CmmnDeploymentRequest::new("platform cascade process task")
                .with_resource("process-task.cmmn", CmmnModel::new(vec![parent_case])),
        )
        .expect("cmmn deployment");

    let parent_instance = cmmn_engine
        .start_case_instance_by_key(
            "platformCascadeProcessParent",
            CmmnCaseInstanceStartRequest::new().with_business_key("BK-CASCADE-CHILD"),
        )
        .expect("parent case");
    let association = cmmn_engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("association query")
        .expect("association");
    let child_process_instance_id = association
        .child_instance_id
        .clone()
        .expect("child process instance id");

    let process_engine = platform.process_engine();
    let store = process_engine.get_runtime_store();
    {
        let mut session = store.create_session().expect("session");
        assert!(
            store
                .find_process_instance(&child_process_instance_id, &mut session)
                .is_some(),
            "BPMN child must exist before cascade delete"
        );
        session.rollback().expect("rollback");
    }

    cmmn_engine
        .repository_service()
        .delete_deployment(&deployment.id, true)
        .expect("cascade delete should purge BPMN children via injected cleanup");

    {
        let mut session = store.create_session().expect("session");
        assert!(
            store
                .find_process_instance(&child_process_instance_id, &mut session)
                .is_none(),
            "BPMN child process instance must be removed by cascade cleanup"
        );
        session.rollback().expect("rollback");
    }
    assert!(
        cmmn_engine
            .runtime_service()
            .get_case_instance(&parent_instance.id)
            .is_err(),
        "parent case instance must be purged"
    );
    assert!(
        cmmn_engine
            .repository_service()
            .get_deployment(&deployment.id)
            .is_err(),
        "CMMN deployment must be removed"
    );
}

// ---------------------------------------------------------------------------
// P91④ — [dmn] strict_mode passthrough (Java `DmnEngineConfiguration.java:202`
// default true; false tolerates UNIQUE/ANY/PRIORITY/OUTPUT_ORDER violations
// with a validationMessage instead of an error).
// ---------------------------------------------------------------------------

fn p91_unique_model() -> flowable_dmn_engine::DmnModel {
    use flowable_dmn_engine::{
        DmnDecision, DmnHitPolicy, DmnInputClause, DmnModel, DmnOutputClause, DmnRule,
        DmnRuleInputEntry, DmnRuleOutputEntry, DmnUnaryTest,
    };
    use serde_json::json;

    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing decision",
        DmnHitPolicy::Unique,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![DmnOutputClause::new("output-1", "route")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("fallback"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                vec![DmnRuleOutputEntry::new(json!("email-queue"))],
            ),
        ],
    )])
}

#[test]
fn dmn_strict_mode_defaults_to_true() {
    let configuration = PlatformConfiguration::default();
    assert!(
        configuration.dmn.strict_mode,
        "strictMode defaults to true (DmnEngineConfiguration.java:202)"
    );

    let platform =
        FlowablePlatform::bootstrap(isolated_platform_config("p91-strict-default")).expect("platform");
    let dmn = platform.dmn_engine();
    dmn.deploy(
        flowable_dmn_engine::DmnDeploymentRequest::new("p91-strict")
            .with_resource("routing.dmn", p91_unique_model()),
    )
    .expect("deployment");

    let result = dmn.execute_by_key(
        "routingDecision",
        flowable_dmn_engine::DmnExecutionRequest::new(serde_json::json!({ "channel": "email" })),
    );
    assert!(
        result.is_err(),
        "default strict mode must reject a UNIQUE multi-match"
    );
}

#[test]
fn dmn_strict_mode_false_tolerates_unique_violation() {
    let mut configuration = isolated_platform_config("p91-strict-false");
    configuration.dmn.strict_mode = false;

    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");
    let dmn = platform.dmn_engine();
    dmn.deploy(
        flowable_dmn_engine::DmnDeploymentRequest::new("p91-lenient")
            .with_resource("routing.dmn", p91_unique_model()),
    )
    .expect("deployment");

    let result = dmn
        .execute_by_key(
            "routingDecision",
            flowable_dmn_engine::DmnExecutionRequest::new(serde_json::json!({ "channel": "email" })),
        )
        .expect("non-strict UNIQUE tolerates multi-match");
    assert!(
        result.validation_message.is_some(),
        "soft violation must surface as validationMessage (HitPolicyUnique.java:73)"
    );
}
