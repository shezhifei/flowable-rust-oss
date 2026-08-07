use crate::dialect::SqlDialect;
use crate::error::PersistenceError;
use crate::value::DbParams;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementId {
    // Property statements
    InsertProperty,
    UpdateProperty,
    DeleteProperty,
    SelectPropertyByName,
    SelectAllProperties,

    // Deployment statements
    InsertDeployment,
    UpdateDeployment,
    DeleteDeployment,
    SelectDeploymentById,
    SelectAllDeployments,
    SelectDeploymentsByQueryCriteria,

    // Deployment resource (ByteArray) statements
    InsertDeploymentResource,
    DeleteDeploymentResource,
    DeleteDeploymentResourcesByDeploymentId,
    SelectDeploymentResourceById,
    SelectDeploymentResourcesByDeploymentId,

    // Process definition statements
    InsertProcessDefinition,
    UpdateProcessDefinition,
    DeleteProcessDefinition,
    SelectProcessDefinitionById,
    SelectProcessDefinitionByKey,
    SelectProcessDefinitionByKeyAndVersion,
    SelectAllProcessDefinitions,
    SelectProcessDefinitionsByQueryCriteria,

    // Execution statements
    InsertExecution,
    BulkInsertExecution,
    UpdateExecution,
    DeleteExecution,
    SelectExecutionById,
    SelectExecutionsByProcessInstanceId,
    SelectExecutionsByParentExecutionId,
    SelectExecutionsByQueryCriteria,

    // Task statements
    InsertTask,
    UpdateTask,
    DeleteTask,
    SelectTaskById,
    SelectTasksByExecutionId,
    SelectTasksByProcessInstanceId,
    SelectTasksByAssignee,
    SelectTasksByQueryCriteria,

    // Variable statements
    InsertVariable,
    UpdateVariable,
    DeleteVariable,
    SelectVariableById,
    SelectVariablesByExecutionId,
    SelectVariablesByTaskId,
    SelectVariablesByQueryCriteria,

    // Job statements
    InsertTimerJob,
    UpdateTimerJob,
    DeleteTimerJob,
    SelectTimerJobById,
    SelectDueTimerJobs,
    // AcquireDueTimerJobs removed (C4): never called; engine acquires via
    // optimistic CAS / global lock on runtime JSON state. See dialect.rs.

    InsertAsyncJob,
    UpdateAsyncJob,
    DeleteAsyncJob,
    SelectAsyncJobById,
    SelectDueAsyncJobs,
    // AcquireDueAsyncJobs removed (C4): same rationale as timer acquire.

    InsertSuspendedJob,
    UpdateSuspendedJob,
    DeleteSuspendedJob,
    SelectSuspendedJobById,

    InsertDeadLetterJob,
    UpdateDeadLetterJob,
    DeleteDeadLetterJob,
    SelectDeadLetterJobById,

    // Event subscription statements
    InsertEventSubscription,
    UpdateEventSubscription,
    DeleteEventSubscription,
    SelectEventSubscriptionById,

    // ByteArray statements
    InsertByteArray,
    UpdateByteArray,
    DeleteByteArray,
    SelectByteArrayById,

    // History statements
    InsertHistoryProcessInstance,
    UpdateHistoryProcessInstance,
    DeleteHistoryProcessInstance,
    SelectHistoryProcessInstanceById,
    SelectHistoryProcessInstancesByQueryCriteria,

    InsertHistoryTask,
    UpdateHistoryTask,
    DeleteHistoryTask,
    SelectHistoryTaskById,
    SelectHistoryTasksByQueryCriteria,

    InsertHistoryVariable,
    UpdateHistoryVariable,
    DeleteHistoryVariable,
    SelectHistoryVariableById,
    SelectHistoryVariablesByQueryCriteria,

    InsertHistoryActivity,
    UpdateHistoryActivity,
    DeleteHistoryActivity,
    SelectHistoryActivityById,

    // Identity link statements
    InsertIdentityLink,
    DeleteIdentityLink,
    SelectIdentityLinksByTaskId,
    SelectIdentityLinksByExecutionId,
    SelectIdentityLinksByProcessInstanceId,

    // Entity link statements
    InsertEntityLink,
    DeleteEntityLink,
    SelectEntityLinksByScopeIdAndType,

    // DMN Deployment statements
    InsertDmnDeployment,
    UpdateDmnDeployment,
    DeleteDmnDeployment,
    SelectDmnDeploymentById,
    SelectAllDmnDeployments,

    // DMN Decision Definition statements
    InsertDmnDecisionDefinition,
    UpdateDmnDecisionDefinition,
    DeleteDmnDecisionDefinition,
    DeleteDmnDecisionDefinitionsByDeploymentId,
    SelectDmnDecisionDefinitionById,
    SelectDmnDecisionDefinitionsByDeploymentId,
    SelectDmnDecisionDefinitionByKey,
    SelectDmnDecisionDefinitionByKeyAndVersion,

    // DMN Deployment Resource statements
    InsertDmnDeploymentResource,
    DeleteDmnDeploymentResource,
    DeleteDmnDeploymentResourcesByDeploymentId,
    SelectDmnDeploymentResourceById,
    SelectDmnDeploymentResourcesByDeploymentId,

    // DMN Decision Requirements Diagram statements
    InsertDmnDecisionRequirementsDiagram,
    DeleteDmnDecisionRequirementsDiagram,
    DeleteDmnDecisionRequirementsDiagramsByDeploymentId,
    SelectDmnDecisionRequirementsDiagramById,
    SelectDmnDecisionRequirementsDiagramsByDeploymentId,

    // DMN Execution History statements
    InsertDmnExecutionHistory,
    DeleteDmnExecutionHistory,
    SelectDmnExecutionHistoryById,
    SelectDmnExecutionHistoriesByDeploymentId,
    SelectDmnExecutionHistoriesByDecisionDefinitionId,
    DeleteDmnExecutionHistoriesByDecisionDefinitionId,
    DeleteDmnExecutionHistoriesByDeploymentId,

    // App Deployment statements
    InsertAppDeployment,
    DeleteAppDeployment,
    SelectAppDeploymentById,
    SelectAllAppDeployments,

    // App Definition statements
    InsertAppDefinition,
    DeleteAppDefinition,
    DeleteAppDefinitionsByDeploymentId,
    SelectAppDefinitionById,
    SelectAppDefinitionsByDeploymentId,
    SelectAppDefinitionsByKey,

    // App Resolved Composition statements
    InsertAppResolvedComposition,
    DeleteAppResolvedComposition,
    DeleteAppResolvedCompositionsByDeploymentId,
    SelectAppResolvedCompositionById,
    SelectAppResolvedCompositionByAppDefinitionId,
    SelectAppResolvedCompositionsByDeploymentId,

    // App Deployment Resource statements
    InsertAppDeploymentResource,
    DeleteAppDeploymentResource,
    DeleteAppDeploymentResourcesByDeploymentId,
    SelectAppDeploymentResourceById,
    SelectAppDeploymentResourcesByDeploymentId,

    // CMMN Deployment statements
    InsertCmmnDeployment,
    UpdateCmmnDeploymentParentId,
    DeleteCmmnDeployment,
    SelectCmmnDeploymentById,
    SelectAllCmmnDeployments,

    // CMMN Case Definition statements
    InsertCmmnCaseDefinition,
    UpdateCmmnCaseDefinitionCategory,
    DeleteCmmnCaseDefinition,
    DeleteCmmnCaseDefinitionsByDeploymentId,
    SelectCmmnCaseDefinitionById,
    SelectAllCmmnCaseDefinitions,
    SelectCmmnCaseDefinitionsByDeploymentId,
    SelectCmmnCaseDefinitionByKey,
    SelectCmmnCaseDefinitionByKeyAndVersion,

    // CMMN Deployment Resource statements
    InsertCmmnDeploymentResource,
    DeleteCmmnDeploymentResource,
    DeleteCmmnDeploymentResourcesByDeploymentId,
    SelectCmmnDeploymentResourceById,
    SelectCmmnDeploymentResourcesByDeploymentId,

    // CMMN Case Instance statements
    InsertCmmnCaseInstance,
    DeleteCmmnCaseInstance,
    DeleteCmmnCaseInstancesByCaseDefinitionId,
    SelectCmmnCaseInstanceById,
    SelectCmmnCaseInstancesByCaseDefinitionId,
    SelectCmmnCaseInstanceIdsByCaseDefinitionId,
    SelectHistoricCmmnCaseInstanceIdsByCaseDefinitionId,

    // CMMN Stage Instance statements
    InsertCmmnStageInstance,
    DeleteCmmnStageInstance,
    DeleteCmmnStageInstancesByCaseInstanceId,
    SelectCmmnStageInstanceById,
    SelectCmmnStageInstancesByCaseInstanceId,

    // CMMN Plan Item Instance statements
    InsertCmmnPlanItemInstance,
    DeleteCmmnPlanItemInstance,
    DeleteCmmnPlanItemInstancesByCaseInstanceId,
    SelectCmmnPlanItemInstanceById,
    SelectCmmnPlanItemInstancesByCaseInstanceId,
    SelectAllCmmnPlanItemInstances,

    // CMMN Stage History statements
    InsertCmmnStageHistory,
    DeleteCmmnStageHistory,
    DeleteCmmnStageHistoryByCaseInstanceId,
    DeleteCmmnStageHistoryByCaseDefinitionId,
    SelectCmmnStageHistoryById,
    SelectCmmnStageHistoryByCaseInstanceId,

    // CMMN Human Task statements
    InsertCmmnHumanTask,
    DeleteCmmnHumanTask,
    DeleteCmmnHumanTasksByCaseInstanceId,
    DeleteCmmnHumanTasksByCaseDefinitionId,
    SelectCmmnHumanTaskById,
    SelectCmmnHumanTasksByCaseInstanceId,

    // CMMN Case History statements
    InsertCmmnCaseHistory,
    DeleteCmmnCaseHistory,
    DeleteCmmnCaseHistoryByCaseDefinitionId,
    SelectCmmnCaseHistoryById,

    // CMMN Human Task History statements
    InsertCmmnHumanTaskHistory,
    DeleteCmmnHumanTaskHistory,
    DeleteCmmnHumanTaskHistoryByCaseInstanceId,
    DeleteCmmnHumanTaskHistoryByCaseDefinitionId,
    SelectCmmnHumanTaskHistoryById,
    SelectCmmnHumanTaskHistoryByCaseInstanceId,

    // CMMN Milestone History statements
    InsertCmmnMilestoneHistory,
    DeleteCmmnMilestoneHistory,
    DeleteCmmnMilestoneHistoryByCaseInstanceId,
    DeleteCmmnMilestoneHistoryByCaseDefinitionId,
    SelectCmmnMilestoneHistoryById,
    SelectCmmnMilestoneHistoryByCaseInstanceId,

    // CMMN Identity Link statements
    InsertCmmnIdentityLink,
    DeleteCmmnIdentityLink,
    DeleteCmmnIdentityLinksByScopeDefinitionId,
    DeleteCmmnIdentityLinksByCaseInstanceId,
    DeleteCmmnIdentityLinksByTaskId,
    SelectCmmnIdentityLinkById,
    SelectCmmnIdentityLinksByScope,

    // CMMN Job statements
    InsertCmmnJob,
    DeleteCmmnJob,
    DeleteCmmnJobsByScopeId,
    DeleteCmmnJobsBySubScopeId,
    DeleteCmmnJobsByScopeDefinitionId,
    SelectCmmnJobById,
    SelectCmmnJobsByScopeId,

    // CMMN Event Subscription statements
    InsertCmmnEventSubscription,
    DeleteCmmnEventSubscription,
    DeleteCmmnEventSubscriptionsByCaseInstanceId,
    DeleteCmmnEventSubscriptionsByCaseDefinitionId,
    SelectCmmnEventSubscriptionById,
    SelectCmmnEventSubscriptionsByCaseInstanceId,

    // CMMN Task Instance Association statements
    InsertCmmnTaskInstanceAssociation,
    DeleteCmmnTaskInstanceAssociation,
    DeleteCmmnTaskInstanceAssociationsByCaseInstanceId,
    SelectCmmnTaskInstanceAssociationById,
    SelectCmmnTaskInstanceAssociationsByCaseInstanceId,

    // CMMN Plan Item Event statements
    InsertCmmnPlanItemEvent,
    DeleteCmmnPlanItemEvent,
    DeleteCmmnPlanItemEventsByCaseInstanceId,
    SelectCmmnPlanItemEventById,
    SelectCmmnPlanItemEventsByCaseInstanceId,
}

#[derive(Debug, Clone)]
pub struct RenderedStatement {
    pub sql: String,
    pub params: DbParams,
}

impl RenderedStatement {
    pub fn new(sql: String, params: DbParams) -> Self {
        Self { sql, params }
    }
}

pub trait StatementCatalog: Send + Sync {
    fn render(
        &self,
        id: StatementId,
        dialect: &dyn SqlDialect,
        params: &DbParams,
    ) -> Result<RenderedStatement, PersistenceError>;

    fn dialect(&self) -> &dyn SqlDialect;
}
