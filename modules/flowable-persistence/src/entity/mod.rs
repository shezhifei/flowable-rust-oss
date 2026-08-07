pub mod app_definition;
pub mod app_deployment;
pub mod app_deployment_resource;
pub mod app_resolved_composition;
pub mod byte_array;
pub mod cmmn_case_definition;
pub mod cmmn_case_history;
pub mod cmmn_case_instance;
pub mod cmmn_deployment;
pub mod cmmn_deployment_resource;
pub mod cmmn_event_subscription;
pub mod cmmn_human_task;
pub mod cmmn_human_task_history;
pub mod cmmn_identity_link;
pub mod cmmn_job;
pub mod cmmn_milestone_history;
pub mod cmmn_plan_item_event;
pub mod cmmn_plan_item_instance;
pub mod cmmn_stage_history;
pub mod cmmn_stage_instance;
pub mod cmmn_task_instance_association;
pub mod dead_letter_job;
pub mod deployment;
pub mod deployment_resource;
pub mod dmn_decision_definition;
pub mod dmn_decision_requirements_diagram;
pub mod dmn_deployment;
pub mod dmn_deployment_resource;
pub mod dmn_execution_history;
pub mod entity_link;
pub mod event_subscription;
pub mod execution;
pub mod history;
pub mod history_activity;
pub mod identity_link;
pub mod job;
pub mod process_definition;
pub mod property;
pub mod suspended_job;
pub mod task;
pub mod timer_job;
pub mod variable;

use std::any::Any;
use std::fmt::Debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityType {
    Property,
    Deployment,
    DeploymentResource,
    ByteArray,
    ProcessDefinition,
    Execution,
    Task,
    Variable,
    Job,
    TimerJob,
    SuspendedJob,
    DeadLetterJob,
    EventSubscription,
    IdentityLink,
    EntityLink,
    HistoryProcessInstance,
    HistoryActivity,
    HistoryTask,
    HistoryVariable,
    // DMN entities
    DmnDeployment,
    DmnDecisionDefinition,
    DmnDeploymentResource,
    DmnDecisionRequirementsDiagram,
    DmnExecutionHistory,
    // App entities
    AppDeployment,
    AppDefinition,
    AppResolvedComposition,
    AppDeploymentResource,
    // CMMN entities
    CmmnDeployment,
    CmmnCaseDefinition,
    CmmnDeploymentResource,
    CmmnCaseInstance,
    CmmnStageInstance,
    CmmnStageHistory,
    CmmnHumanTask,
    CmmnCaseHistory,
    CmmnHumanTaskHistory,
    CmmnMilestoneHistory,
    CmmnIdentityLink,
    CmmnJob,
    CmmnEventSubscription,
    CmmnTaskInstanceAssociation,
    CmmnPlanItemEvent,
    CmmnPlanItemInstance,
}

impl EntityType {
    pub fn table_name(&self) -> &'static str {
        match self {
            EntityType::Property => "ACT_GE_PROPERTY",
            EntityType::Deployment => "ACT_RE_DEPLOYMENT",
            EntityType::DeploymentResource => "ACT_GE_BYTEARRAY",
            EntityType::ByteArray => "ACT_GE_BYTEARRAY",
            EntityType::ProcessDefinition => "ACT_RE_PROCDEF",
            EntityType::Execution => "ACT_RU_EXECUTION",
            EntityType::Task => "ACT_RU_TASK",
            EntityType::Variable => "ACT_RU_VARIABLE",
            EntityType::Job => "ACT_RU_JOB",
            EntityType::TimerJob => "ACT_RU_TIMER_JOB",
            EntityType::SuspendedJob => "ACT_RU_SUSPENDED_JOB",
            EntityType::DeadLetterJob => "ACT_RU_DEADLETTER_JOB",
            EntityType::EventSubscription => "ACT_RU_EVENT_SUBSCR",
            EntityType::IdentityLink => "ACT_RU_IDENTITYLINK",
            EntityType::EntityLink => "ACT_RU_ENTITYLINK",
            EntityType::HistoryProcessInstance => "ACT_HI_PROCINST",
            EntityType::HistoryActivity => "ACT_HI_ACTINST",
            EntityType::HistoryTask => "ACT_HI_TASKINST",
            EntityType::HistoryVariable => "ACT_HI_VARINST",
            // DMN entities
            EntityType::DmnDeployment => "ACT_DMN_DEPLOYMENT",
            EntityType::DmnDecisionDefinition => "ACT_DMN_DECISION",
            EntityType::DmnDeploymentResource => "ACT_DMN_RESOURCE",
            EntityType::DmnDecisionRequirementsDiagram => "ACT_DMN_DRD",
            EntityType::DmnExecutionHistory => "ACT_DMN_HI_EXECUTION",
            // App entities
            EntityType::AppDeployment => "ACT_APP_DEPLOYMENT",
            EntityType::AppDefinition => "ACT_APP_DEFINITION",
            EntityType::AppResolvedComposition => "ACT_APP_RESOLVED_COMPOSITION",
            EntityType::AppDeploymentResource => "ACT_APP_DEPLOYMENT_RESOURCE",
            // CMMN entities
            EntityType::CmmnDeployment => "ACT_CMMN_DEPLOYMENT",
            EntityType::CmmnCaseDefinition => "ACT_CMMN_CASE_DEFINITION",
            EntityType::CmmnDeploymentResource => "ACT_CMMN_DEPLOYMENT_RESOURCE",
            EntityType::CmmnCaseInstance => "ACT_CMMN_CASE_INSTANCE",
            EntityType::CmmnStageInstance => "ACT_CMMN_STAGE_INSTANCE",
            EntityType::CmmnStageHistory => "ACT_CMMN_STAGE_HISTORY",
            EntityType::CmmnHumanTask => "ACT_CMMN_HUMAN_TASK",
            EntityType::CmmnCaseHistory => "ACT_CMMN_CASE_HISTORY",
            EntityType::CmmnHumanTaskHistory => "ACT_CMMN_HUMAN_TASK_HISTORY",
            EntityType::CmmnMilestoneHistory => "ACT_CMMN_MILESTONE_HISTORY",
            EntityType::CmmnIdentityLink => "ACT_CMMN_IDENTITY_LINK",
            EntityType::CmmnJob => "ACT_CMMN_JOB",
            EntityType::CmmnEventSubscription => "ACT_CMMN_EVENT_SUBSCRIPTION",
            EntityType::CmmnTaskInstanceAssociation => "ACT_CMMN_TASK_INSTANCE_ASSOCIATION",
            EntityType::CmmnPlanItemEvent => "ACT_CMMN_PLAN_ITEM_EVENT",
            EntityType::CmmnPlanItemInstance => "ACT_CMMN_RU_PLAN_ITEM_INST",
        }
    }
}

pub trait Entity: Send + Sync + Debug + 'static {
    fn id(&self) -> &str;
    fn set_id(&mut self, id: String);
    fn entity_type(&self) -> EntityType;
    fn as_any(&self) -> &dyn Any;
    fn clone_box(&self) -> Box<dyn Entity>;
}

impl Clone for Box<dyn Entity> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

pub trait RevisionedEntity: Entity {
    fn revision(&self) -> i32;
    fn set_revision(&mut self, revision: i32);
    fn revision_next(&self) -> i32 {
        self.revision() + 1
    }
}
