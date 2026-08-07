use crate::dialect::SqlDialect;
use crate::error::PersistenceError;
use crate::statement::{RenderedStatement, StatementCatalog, StatementId};
use crate::value::DbParams;
pub struct FlowableStatementCatalog {
    dialect: Box<dyn SqlDialect>,
}
impl FlowableStatementCatalog {
    pub fn new(dialect: Box<dyn SqlDialect>) -> Self {
        Self { dialect }
    }
}
impl StatementCatalog for FlowableStatementCatalog {
    fn render(
        &self,
        id: StatementId,
        dialect: &dyn SqlDialect,
        params: &DbParams,
    ) -> Result<RenderedStatement, PersistenceError> {
        let sql = match id {
            StatementId::InsertProperty => {
                format!(
                    "INSERT INTO ACT_GE_PROPERTY (NAME_, VALUE_, REV_) VALUES ({}, {}, {})",
                    dialect.placeholder(0),
                    dialect.placeholder(1),
                    dialect.placeholder(2)
                )
            }
            StatementId::UpdateProperty => {
                format!(
                    "UPDATE ACT_GE_PROPERTY SET VALUE_ = {}, REV_ = REV_ + 1 WHERE NAME_ = {} AND REV_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1),
                    dialect.placeholder(2)
                )
            }
            StatementId::DeleteProperty => {
                format!(
                    "DELETE FROM ACT_GE_PROPERTY WHERE NAME_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectPropertyByName => {
                format!(
                    "SELECT NAME_, VALUE_, REV_ FROM ACT_GE_PROPERTY WHERE NAME_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAllProperties => {
                "SELECT NAME_, VALUE_, REV_ FROM ACT_GE_PROPERTY".to_string()
            }
            StatementId::InsertDeployment => {
                let placeholders: Vec<String> = (0..8).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RE_DEPLOYMENT (ID_, REV_, NAME_, CATEGORY_, KEY_, TENANT_ID_, DEPLOY_TIME_, ENGINE_VERSION_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateDeployment => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "NAME_",
                    "CATEGORY_",
                    "KEY_",
                    "TENANT_ID_",
                    "DEPLOY_TIME_",
                    "ENGINE_VERSION_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RE_DEPLOYMENT SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(7),
                    dialect.placeholder(8)
                )
            }
            StatementId::DeleteDeployment => {
                format!(
                    "DELETE FROM ACT_RE_DEPLOYMENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDeploymentById => {
                format!(
                    "SELECT * FROM ACT_RE_DEPLOYMENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAllDeployments => "SELECT * FROM ACT_RE_DEPLOYMENT".to_string(),
            StatementId::SelectDeploymentsByQueryCriteria => {
                "SELECT * FROM ACT_RE_DEPLOYMENT".to_string()
            }
            StatementId::InsertDeploymentResource => {
                let placeholders: Vec<String> = (0..5).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_GE_BYTEARRAY (ID_, REV_, NAME_, DEPLOYMENT_ID_, BYTES_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteDeploymentResource => {
                format!(
                    "DELETE FROM ACT_GE_BYTEARRAY WHERE DEPLOYMENT_ID_ = {} AND NAME_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::DeleteDeploymentResourcesByDeploymentId => {
                format!(
                    "DELETE FROM ACT_GE_BYTEARRAY WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDeploymentResourceById => {
                format!(
                    "SELECT * FROM ACT_GE_BYTEARRAY WHERE DEPLOYMENT_ID_ = {} AND NAME_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::SelectDeploymentResourcesByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_GE_BYTEARRAY WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertByteArray => {
                let placeholders: Vec<String> = (0..6).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_GE_BYTEARRAY (ID_, REV_, NAME_, DEPLOYMENT_ID_, BYTES_, GENERATED_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateByteArray => {
                let set_placeholders: Vec<String> =
                    ["REV_", "NAME_", "DEPLOYMENT_ID_", "BYTES_", "GENERATED_"]
                        .into_iter()
                        .enumerate()
                        .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                        .collect();
                format!(
                    "UPDATE ACT_GE_BYTEARRAY SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(5),
                    dialect.placeholder(6)
                )
            }
            StatementId::DeleteByteArray => {
                format!(
                    "DELETE FROM ACT_GE_BYTEARRAY WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectByteArrayById => {
                format!(
                    "SELECT * FROM ACT_GE_BYTEARRAY WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertProcessDefinition => {
                let placeholders: Vec<String> = (0..16).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RE_PROCDEF (ID_, REV_, CATEGORY_, NAME_, KEY_, VERSION_, DEPLOYMENT_ID_, RESOURCE_NAME_, DGRM_RESOURCE_NAME_, DESCRIPTION_, HAS_GRAPHICAL_NOTATION_, HAS_START_FORM_KEY_, SUSPENSION_STATE_, TENANT_ID_, ENGINE_VERSION_, APP_VERSION_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateProcessDefinition => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "CATEGORY_",
                    "NAME_",
                    "KEY_",
                    "VERSION_",
                    "DEPLOYMENT_ID_",
                    "RESOURCE_NAME_",
                    "DGRM_RESOURCE_NAME_",
                    "DESCRIPTION_",
                    "HAS_GRAPHICAL_NOTATION_",
                    "HAS_START_FORM_KEY_",
                    "SUSPENSION_STATE_",
                    "TENANT_ID_",
                    "ENGINE_VERSION_",
                    "APP_VERSION_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RE_PROCDEF SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(15),
                    dialect.placeholder(16)
                )
            }
            StatementId::DeleteProcessDefinition => {
                format!(
                    "DELETE FROM ACT_RE_PROCDEF WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectProcessDefinitionById => {
                format!(
                    "SELECT * FROM ACT_RE_PROCDEF WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectProcessDefinitionByKey => {
                format!(
                    "SELECT * FROM ACT_RE_PROCDEF WHERE KEY_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectProcessDefinitionByKeyAndVersion => {
                format!(
                    "SELECT * FROM ACT_RE_PROCDEF WHERE KEY_ = {} AND VERSION_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::SelectAllProcessDefinitions => "SELECT * FROM ACT_RE_PROCDEF".to_string(),
            StatementId::SelectProcessDefinitionsByQueryCriteria => {
                "SELECT * FROM ACT_RE_PROCDEF".to_string()
            }
            StatementId::InsertExecution => {
                let placeholders: Vec<String> = (0..31).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_EXECUTION (ID_, REV_, PROC_INST_ID_, BUSINESS_KEY_, PARENT_ID_, PROC_DEF_ID_, SUPER_EXEC_, ROOT_PROC_INST_ID_, ACT_ID_, IS_ACTIVE_, IS_CONCURRENT_, IS_SCOPE_, IS_EVENT_SCOPE_, IS_MI_ROOT_, SUSPENSION_STATE_, CACHED_ENT_STATE_, TENANT_ID_, NAME_, START_ACT_ID_, START_TIME_, START_USER_ID_, LOCK_TIME_, IS_COUNT_ENABLED_, EVT_SUBSCR_COUNT_, TASK_COUNT_, JOB_COUNT_, TIMER_JOB_COUNT_, SUSP_JOB_COUNT_, DEADLETTER_JOB_COUNT_, VAR_COUNT_, ID_LINK_COUNT_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::BulkInsertExecution => {
                let placeholders: Vec<String> = (0..31).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_EXECUTION (ID_, REV_, PROC_INST_ID_, BUSINESS_KEY_, PARENT_ID_, PROC_DEF_ID_, SUPER_EXEC_, ROOT_PROC_INST_ID_, ACT_ID_, IS_ACTIVE_, IS_CONCURRENT_, IS_SCOPE_, IS_EVENT_SCOPE_, IS_MI_ROOT_, SUSPENSION_STATE_, CACHED_ENT_STATE_, TENANT_ID_, NAME_, START_ACT_ID_, START_TIME_, START_USER_ID_, LOCK_TIME_, IS_COUNT_ENABLED_, EVT_SUBSCR_COUNT_, TASK_COUNT_, JOB_COUNT_, TIMER_JOB_COUNT_, SUSP_JOB_COUNT_, DEADLETTER_JOB_COUNT_, VAR_COUNT_, ID_LINK_COUNT_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateExecution => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "PROC_INST_ID_",
                    "BUSINESS_KEY_",
                    "PARENT_ID_",
                    "PROC_DEF_ID_",
                    "SUPER_EXEC_",
                    "ROOT_PROC_INST_ID_",
                    "ACT_ID_",
                    "IS_ACTIVE_",
                    "IS_CONCURRENT_",
                    "IS_SCOPE_",
                    "IS_EVENT_SCOPE_",
                    "IS_MI_ROOT_",
                    "SUSPENSION_STATE_",
                    "CACHED_ENT_STATE_",
                    "TENANT_ID_",
                    "NAME_",
                    "START_ACT_ID_",
                    "START_TIME_",
                    "START_USER_ID_",
                    "LOCK_TIME_",
                    "IS_COUNT_ENABLED_",
                    "EVT_SUBSCR_COUNT_",
                    "TASK_COUNT_",
                    "JOB_COUNT_",
                    "TIMER_JOB_COUNT_",
                    "SUSP_JOB_COUNT_",
                    "DEADLETTER_JOB_COUNT_",
                    "VAR_COUNT_",
                    "ID_LINK_COUNT_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RU_EXECUTION SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(30),
                    dialect.placeholder(31)
                )
            }
            StatementId::DeleteExecution => {
                format!(
                    "DELETE FROM ACT_RU_EXECUTION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectExecutionById => {
                format!(
                    "SELECT * FROM ACT_RU_EXECUTION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectExecutionsByProcessInstanceId => {
                format!(
                    "SELECT * FROM ACT_RU_EXECUTION WHERE PROC_INST_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectExecutionsByParentExecutionId => {
                format!(
                    "SELECT * FROM ACT_RU_EXECUTION WHERE PARENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectExecutionsByQueryCriteria => {
                "SELECT * FROM ACT_RU_EXECUTION".to_string()
            }
            StatementId::InsertTask => {
                let placeholders: Vec<String> = (0..22).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_TASK (ID_, REV_, EXECUTION_ID_, PROC_INST_ID_, PROC_DEF_ID_, NAME_, BUSINESS_KEY_, PARENT_TASK_ID_, DESCRIPTION_, TASK_DEF_KEY_, OWNER_, ASSIGNEE_, DELEGATION_, PRIORITY_, CREATE_TIME_, DUE_DATE_, CATEGORY_, SUSPENSION_STATE_, TENANT_ID_, FORM_KEY_, CLAIM_TIME_, APP_VERSION_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateTask => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "EXECUTION_ID_",
                    "PROC_INST_ID_",
                    "PROC_DEF_ID_",
                    "NAME_",
                    "BUSINESS_KEY_",
                    "PARENT_TASK_ID_",
                    "DESCRIPTION_",
                    "TASK_DEF_KEY_",
                    "OWNER_",
                    "ASSIGNEE_",
                    "DELEGATION_",
                    "PRIORITY_",
                    "CREATE_TIME_",
                    "DUE_DATE_",
                    "CATEGORY_",
                    "SUSPENSION_STATE_",
                    "TENANT_ID_",
                    "FORM_KEY_",
                    "CLAIM_TIME_",
                    "APP_VERSION_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RU_TASK SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(21),
                    dialect.placeholder(22)
                )
            }
            StatementId::DeleteTask => {
                format!(
                    "DELETE FROM ACT_RU_TASK WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectTaskById => {
                format!(
                    "SELECT * FROM ACT_RU_TASK WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectTasksByExecutionId => {
                format!(
                    "SELECT * FROM ACT_RU_TASK WHERE EXECUTION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectTasksByProcessInstanceId => {
                format!(
                    "SELECT * FROM ACT_RU_TASK WHERE PROC_INST_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectTasksByAssignee => {
                format!(
                    "SELECT * FROM ACT_RU_TASK WHERE ASSIGNEE_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectTasksByQueryCriteria => "SELECT * FROM ACT_RU_TASK".to_string(),
            StatementId::InsertVariable => {
                let placeholders: Vec<String> = (0..16).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_VARIABLE (ID_, REV_, TYPE_, NAME_, EXECUTION_ID_, PROC_INST_ID_, TASK_ID_, SCOPE_TYPE_, SCOPE_ID_, SUB_SCOPE_ID_, BYTEARRAY_ID_, DOUBLE_, LONG_, TEXT_, TEXT2_, IS_INITIAL_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateVariable => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "TYPE_",
                    "NAME_",
                    "EXECUTION_ID_",
                    "PROC_INST_ID_",
                    "TASK_ID_",
                    "SCOPE_TYPE_",
                    "SCOPE_ID_",
                    "SUB_SCOPE_ID_",
                    "BYTEARRAY_ID_",
                    "DOUBLE_",
                    "LONG_",
                    "TEXT_",
                    "TEXT2_",
                    "IS_INITIAL_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RU_VARIABLE SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(15),
                    dialect.placeholder(16)
                )
            }
            StatementId::DeleteVariable => {
                format!(
                    "DELETE FROM ACT_RU_VARIABLE WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectVariableById => {
                format!(
                    "SELECT * FROM ACT_RU_VARIABLE WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectVariablesByExecutionId => {
                format!(
                    "SELECT * FROM ACT_RU_VARIABLE WHERE EXECUTION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectVariablesByTaskId => {
                format!(
                    "SELECT * FROM ACT_RU_VARIABLE WHERE TASK_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectVariablesByQueryCriteria => {
                "SELECT * FROM ACT_RU_VARIABLE".to_string()
            }
            StatementId::InsertTimerJob => {
                let placeholders: Vec<String> = (0..27).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_TIMER_JOB (ID_, REV_, TYPE_, PROC_DEF_ID_, PROC_INST_ID_, EXECUTION_ID_, NAME_, SCOPE_TYPE_, SCOPE_ID_, SUB_SCOPE_ID_, CREATE_TIME_, LOCK_OWNER_, LOCK_TIME_, EXCLUSIVE_, EXECUTION_, PROCESS_DEFINITION_, RETRIES_, EXCEPTION_STACK_ID_, EXCEPTION_MSG_, DUEDATE_, REPEAT_, HANDLER_TYPE_, TENANT_ID_, CUSTOM_VALUES_ID_, JOB_HANDLER_TYPE_, JOB_HANDLER_CFG_, LOCK_EXP_TIME_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateTimerJob => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "TYPE_",
                    "PROC_DEF_ID_",
                    "PROC_INST_ID_",
                    "EXECUTION_ID_",
                    "NAME_",
                    "SCOPE_TYPE_",
                    "SCOPE_ID_",
                    "SUB_SCOPE_ID_",
                    "CREATE_TIME_",
                    "LOCK_OWNER_",
                    "LOCK_TIME_",
                    "EXCLUSIVE_",
                    "EXECUTION_",
                    "PROCESS_DEFINITION_",
                    "RETRIES_",
                    "EXCEPTION_STACK_ID_",
                    "EXCEPTION_MSG_",
                    "DUEDATE_",
                    "REPEAT_",
                    "HANDLER_TYPE_",
                    "TENANT_ID_",
                    "CUSTOM_VALUES_ID_",
                    "JOB_HANDLER_TYPE_",
                    "JOB_HANDLER_CFG_",
                    "LOCK_EXP_TIME_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RU_TIMER_JOB SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(26),
                    dialect.placeholder(27)
                )
            }
            StatementId::DeleteTimerJob => {
                format!(
                    "DELETE FROM ACT_RU_TIMER_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectTimerJobById => {
                format!(
                    "SELECT * FROM ACT_RU_TIMER_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDueTimerJobs => {
                format!(
                    "SELECT * FROM ACT_RU_TIMER_JOB WHERE DUEDATE_ IS NOT NULL AND DUEDATE_ <= {} AND (LOCK_OWNER_ IS NULL OR LOCK_EXP_TIME_ IS NULL OR LOCK_EXP_TIME_ < {}) AND RETRIES_ > 0 ORDER BY DUEDATE_ ASC, ID_ ASC",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::InsertAsyncJob => {
                let placeholders: Vec<String> = (0..24).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_JOB (ID_, REV_, TYPE_, PROC_DEF_ID_, PROC_INST_ID_, EXECUTION_ID_, NAME_, SCOPE_TYPE_, SCOPE_ID_, SUB_SCOPE_ID_, CREATE_TIME_, LOCK_OWNER_, LOCK_TIME_, EXCLUSIVE_, EXECUTION_, PROCESS_DEFINITION_, RETRIES_, EXCEPTION_STACK_ID_, EXCEPTION_MSG_, DUEDATE_, REPEAT_, HISTORY_URL_, HANDLER_TYPE_, CUSTOM_VALUES_ID_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateAsyncJob => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "TYPE_",
                    "PROC_DEF_ID_",
                    "PROC_INST_ID_",
                    "EXECUTION_ID_",
                    "NAME_",
                    "SCOPE_TYPE_",
                    "SCOPE_ID_",
                    "SUB_SCOPE_ID_",
                    "CREATE_TIME_",
                    "LOCK_OWNER_",
                    "LOCK_TIME_",
                    "EXCLUSIVE_",
                    "EXECUTION_",
                    "PROCESS_DEFINITION_",
                    "RETRIES_",
                    "EXCEPTION_STACK_ID_",
                    "EXCEPTION_MSG_",
                    "DUEDATE_",
                    "REPEAT_",
                    "HISTORY_URL_",
                    "HANDLER_TYPE_",
                    "CUSTOM_VALUES_ID_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RU_JOB SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(23),
                    dialect.placeholder(24)
                )
            }
            StatementId::DeleteAsyncJob => {
                format!(
                    "DELETE FROM ACT_RU_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAsyncJobById => {
                format!(
                    "SELECT * FROM ACT_RU_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDueAsyncJobs => {
                format!(
                    "SELECT * FROM ACT_RU_JOB WHERE DUEDATE_ IS NOT NULL AND DUEDATE_ <= {} AND (LOCK_OWNER_ IS NULL OR LOCK_EXP_TIME_ IS NULL OR LOCK_EXP_TIME_ < {}) AND RETRIES_ > 0 ORDER BY DUEDATE_ ASC, ID_ ASC",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::InsertSuspendedJob => {
                let placeholders: Vec<String> = (0..27).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_SUSPENDED_JOB (ID_, REV_, TYPE_, PROC_DEF_ID_, PROC_INST_ID_, EXECUTION_ID_, NAME_, SCOPE_TYPE_, SCOPE_ID_, SUB_SCOPE_ID_, CREATE_TIME_, LOCK_OWNER_, LOCK_TIME_, EXCLUSIVE_, EXECUTION_, PROCESS_DEFINITION_, RETRIES_, EXCEPTION_STACK_ID_, EXCEPTION_MSG_, DUEDATE_, REPEAT_, HANDLER_TYPE_, TENANT_ID_, CUSTOM_VALUES_ID_, JOB_HANDLER_TYPE_, JOB_HANDLER_CFG_, LOCK_EXP_TIME_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateSuspendedJob => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "TYPE_",
                    "PROC_DEF_ID_",
                    "PROC_INST_ID_",
                    "EXECUTION_ID_",
                    "NAME_",
                    "SCOPE_TYPE_",
                    "SCOPE_ID_",
                    "SUB_SCOPE_ID_",
                    "CREATE_TIME_",
                    "LOCK_OWNER_",
                    "LOCK_TIME_",
                    "EXCLUSIVE_",
                    "EXECUTION_",
                    "PROCESS_DEFINITION_",
                    "RETRIES_",
                    "EXCEPTION_STACK_ID_",
                    "EXCEPTION_MSG_",
                    "DUEDATE_",
                    "REPEAT_",
                    "HANDLER_TYPE_",
                    "TENANT_ID_",
                    "CUSTOM_VALUES_ID_",
                    "JOB_HANDLER_TYPE_",
                    "JOB_HANDLER_CFG_",
                    "LOCK_EXP_TIME_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RU_SUSPENDED_JOB SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(26),
                    dialect.placeholder(27)
                )
            }
            StatementId::DeleteSuspendedJob => {
                format!(
                    "DELETE FROM ACT_RU_SUSPENDED_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectSuspendedJobById => {
                format!(
                    "SELECT * FROM ACT_RU_SUSPENDED_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertDeadLetterJob => {
                let placeholders: Vec<String> = (0..27).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_DEADLETTER_JOB (ID_, REV_, TYPE_, PROC_DEF_ID_, PROC_INST_ID_, EXECUTION_ID_, NAME_, SCOPE_TYPE_, SCOPE_ID_, SUB_SCOPE_ID_, CREATE_TIME_, LOCK_OWNER_, LOCK_TIME_, EXCLUSIVE_, EXECUTION_, PROCESS_DEFINITION_, RETRIES_, EXCEPTION_STACK_ID_, EXCEPTION_MSG_, DUEDATE_, REPEAT_, HANDLER_TYPE_, TENANT_ID_, CUSTOM_VALUES_ID_, JOB_HANDLER_TYPE_, JOB_HANDLER_CFG_, LOCK_EXP_TIME_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateDeadLetterJob => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "TYPE_",
                    "PROC_DEF_ID_",
                    "PROC_INST_ID_",
                    "EXECUTION_ID_",
                    "NAME_",
                    "SCOPE_TYPE_",
                    "SCOPE_ID_",
                    "SUB_SCOPE_ID_",
                    "CREATE_TIME_",
                    "LOCK_OWNER_",
                    "LOCK_TIME_",
                    "EXCLUSIVE_",
                    "EXECUTION_",
                    "PROCESS_DEFINITION_",
                    "RETRIES_",
                    "EXCEPTION_STACK_ID_",
                    "EXCEPTION_MSG_",
                    "DUEDATE_",
                    "REPEAT_",
                    "HANDLER_TYPE_",
                    "TENANT_ID_",
                    "CUSTOM_VALUES_ID_",
                    "JOB_HANDLER_TYPE_",
                    "JOB_HANDLER_CFG_",
                    "LOCK_EXP_TIME_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RU_DEADLETTER_JOB SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(26),
                    dialect.placeholder(27)
                )
            }
            StatementId::DeleteDeadLetterJob => {
                format!(
                    "DELETE FROM ACT_RU_DEADLETTER_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDeadLetterJobById => {
                format!(
                    "SELECT * FROM ACT_RU_DEADLETTER_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertEventSubscription => {
                let placeholders: Vec<String> = (0..13).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_EVENT_SUBSCR (ID_, REV_, EVENT_TYPE_, EVENT_NAME_, EXECUTION_ID_, PROC_INST_ID_, ACTIVITY_ID_, CONFIGURATION_, CREATED_, PROC_DEF_ID_, TENANT_ID_, LOCK_OWNER_, LOCK_TIME_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateEventSubscription => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "EVENT_TYPE_",
                    "EVENT_NAME_",
                    "EXECUTION_ID_",
                    "PROC_INST_ID_",
                    "ACTIVITY_ID_",
                    "CONFIGURATION_",
                    "CREATED_",
                    "PROC_DEF_ID_",
                    "TENANT_ID_",
                    "LOCK_OWNER_",
                    "LOCK_TIME_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_RU_EVENT_SUBSCR SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(12),
                    dialect.placeholder(13)
                )
            }
            StatementId::DeleteEventSubscription => {
                format!(
                    "DELETE FROM ACT_RU_EVENT_SUBSCR WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectEventSubscriptionById => {
                format!(
                    "SELECT * FROM ACT_RU_EVENT_SUBSCR WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertHistoryProcessInstance => {
                let placeholders: Vec<String> = (0..22).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_HI_PROCINST (ID_, REV_, PROC_DEF_ID_, PROC_DEF_KEY_, PROC_DEF_NAME_, PROC_DEF_VERSION_, BUSINESS_KEY_, START_TIME_, END_TIME_, DURATION_, START_USER_ID_, START_ACT_ID_, END_ACT_ID_, SUPER_PROCESS_INSTANCE_ID_, DELETE_REASON_, TENANT_ID_, NAME_, DESCRIPTION_, CALLBACK_ID_, CALLBACK_TYPE_, REFERENCE_ID_, REFERENCE_TYPE_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateHistoryProcessInstance => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "PROC_DEF_ID_",
                    "PROC_DEF_KEY_",
                    "PROC_DEF_NAME_",
                    "PROC_DEF_VERSION_",
                    "BUSINESS_KEY_",
                    "START_TIME_",
                    "END_TIME_",
                    "DURATION_",
                    "START_USER_ID_",
                    "START_ACT_ID_",
                    "END_ACT_ID_",
                    "SUPER_PROCESS_INSTANCE_ID_",
                    "DELETE_REASON_",
                    "TENANT_ID_",
                    "NAME_",
                    "DESCRIPTION_",
                    "CALLBACK_ID_",
                    "CALLBACK_TYPE_",
                    "REFERENCE_ID_",
                    "REFERENCE_TYPE_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_HI_PROCINST SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(21),
                    dialect.placeholder(22)
                )
            }
            StatementId::DeleteHistoryProcessInstance => {
                format!(
                    "DELETE FROM ACT_HI_PROCINST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectHistoryProcessInstanceById => {
                format!(
                    "SELECT * FROM ACT_HI_PROCINST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectHistoryProcessInstancesByQueryCriteria => {
                "SELECT * FROM ACT_HI_PROCINST".to_string()
            }
            StatementId::InsertHistoryTask => {
                let placeholders: Vec<String> = (0..22).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_HI_TASKINST (ID_, REV_, PROC_DEF_ID_, PROC_INST_ID_, EXECUTION_ID_, NAME_, PARENT_TASK_ID_, DESCRIPTION_, OWNER_, ASSIGNEE_, START_TIME_, CLAIM_TIME_, END_TIME_, DURATION_, DELETE_REASON_, PRIORITY_, DUE_DATE_, TASK_DEF_KEY_, CATEGORY_, FORM_KEY_, TENANT_ID_, APP_VERSION_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateHistoryTask => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "PROC_DEF_ID_",
                    "PROC_INST_ID_",
                    "EXECUTION_ID_",
                    "NAME_",
                    "PARENT_TASK_ID_",
                    "DESCRIPTION_",
                    "OWNER_",
                    "ASSIGNEE_",
                    "START_TIME_",
                    "CLAIM_TIME_",
                    "END_TIME_",
                    "DURATION_",
                    "DELETE_REASON_",
                    "PRIORITY_",
                    "DUE_DATE_",
                    "TASK_DEF_KEY_",
                    "CATEGORY_",
                    "FORM_KEY_",
                    "TENANT_ID_",
                    "APP_VERSION_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_HI_TASKINST SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(21),
                    dialect.placeholder(22)
                )
            }
            StatementId::DeleteHistoryTask => {
                format!(
                    "DELETE FROM ACT_HI_TASKINST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectHistoryTaskById => {
                format!(
                    "SELECT * FROM ACT_HI_TASKINST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectHistoryTasksByQueryCriteria => {
                "SELECT * FROM ACT_HI_TASKINST".to_string()
            }
            StatementId::InsertHistoryVariable => {
                let placeholders: Vec<String> = (0..17).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_HI_VARINST (ID_, REV_, PROC_INST_ID_, EXECUTION_ID_, TASK_ID_, CREATE_TIME_, LAST_UPDATED_TIME_, NAME_, VAR_TYPE_, SCOPE_TYPE_, SCOPE_ID_, SUB_SCOPE_ID_, BYTEARRAY_ID_, DOUBLE_, LONG_, TEXT_, TEXT2_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateHistoryVariable => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "PROC_INST_ID_",
                    "EXECUTION_ID_",
                    "TASK_ID_",
                    "CREATE_TIME_",
                    "LAST_UPDATED_TIME_",
                    "NAME_",
                    "VAR_TYPE_",
                    "SCOPE_TYPE_",
                    "SCOPE_ID_",
                    "SUB_SCOPE_ID_",
                    "BYTEARRAY_ID_",
                    "DOUBLE_",
                    "LONG_",
                    "TEXT_",
                    "TEXT2_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_HI_VARINST SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(16),
                    dialect.placeholder(17)
                )
            }
            StatementId::DeleteHistoryVariable => {
                format!(
                    "DELETE FROM ACT_HI_VARINST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectHistoryVariableById => {
                format!(
                    "SELECT * FROM ACT_HI_VARINST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectHistoryVariablesByQueryCriteria => {
                "SELECT * FROM ACT_HI_VARINST".to_string()
            }
            StatementId::InsertHistoryActivity => {
                let placeholders: Vec<String> = (0..17).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_HI_ACTINST (ID_, REV_, PROC_DEF_ID_, PROC_INST_ID_, EXECUTION_ID_, ACT_ID_, TASK_ID_, CALL_PROC_INST_ID_, ACT_NAME_, ACT_TYPE_, ASSIGNEE_, START_TIME_, END_TIME_, DURATION_, TRANSACTION_ORDER_, DELETE_REASON_, TENANT_ID_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateHistoryActivity => {
                let set_placeholders: Vec<String> = [
                    "REV_",
                    "PROC_DEF_ID_",
                    "PROC_INST_ID_",
                    "EXECUTION_ID_",
                    "ACT_ID_",
                    "TASK_ID_",
                    "CALL_PROC_INST_ID_",
                    "ACT_NAME_",
                    "ACT_TYPE_",
                    "ASSIGNEE_",
                    "START_TIME_",
                    "END_TIME_",
                    "DURATION_",
                    "TRANSACTION_ORDER_",
                    "DELETE_REASON_",
                    "TENANT_ID_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_HI_ACTINST SET {} WHERE ID_ = {} AND REV_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(16),
                    dialect.placeholder(17)
                )
            }
            StatementId::DeleteHistoryActivity => {
                format!(
                    "DELETE FROM ACT_HI_ACTINST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectHistoryActivityById => {
                format!(
                    "SELECT * FROM ACT_HI_ACTINST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertIdentityLink => {
                let placeholders: Vec<String> = (0..12).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_IDENTITYLINK (ID_, REV_, GROUP_ID_, TYPE_, USER_ID_, TASK_ID_, PROC_INST_ID_, PROC_DEF_ID_, SCOPE_ID_, SCOPE_TYPE_, SCOPE_DEFINITION_ID_, SUB_SCOPE_ID_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteIdentityLink => {
                format!(
                    "DELETE FROM ACT_RU_IDENTITYLINK WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectIdentityLinksByTaskId => {
                format!(
                    "SELECT * FROM ACT_RU_IDENTITYLINK WHERE TASK_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectIdentityLinksByExecutionId => {
                format!(
                    "SELECT * FROM ACT_RU_IDENTITYLINK WHERE EXECUTION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectIdentityLinksByProcessInstanceId => {
                format!(
                    "SELECT * FROM ACT_RU_IDENTITYLINK WHERE PROC_INST_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertEntityLink => {
                let placeholders: Vec<String> = (0..11).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_RU_ENTITYLINK (ID_, REV_, CREATE_TIME_, LINK_TYPE_, SCOPE_ID_, SCOPE_TYPE_, SCOPE_DEFINITION_ID_, REF_SCOPE_ID_, REF_SCOPE_TYPE_, REF_SCOPE_DEFINITION_ID_, HIERARCHY_TYPE_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteEntityLink => {
                format!(
                    "DELETE FROM ACT_RU_ENTITYLINK WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectEntityLinksByScopeIdAndType => {
                format!(
                    "SELECT * FROM ACT_RU_ENTITYLINK WHERE SCOPE_ID_ = {} AND SCOPE_TYPE_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::InsertDmnDeployment => {
                let placeholders: Vec<String> = (0..7).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_DMN_DEPLOYMENT (ID_, NAME_, CATEGORY_, PARENT_DEPLOYMENT_ID_, TENANT_ID_, DEPLOYED_AT_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateDmnDeployment => {
                let set_placeholders: Vec<String> = [
                    "NAME_",
                    "CATEGORY_",
                    "PARENT_DEPLOYMENT_ID_",
                    "TENANT_ID_",
                    "DEPLOYED_AT_",
                    "DATA_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_DMN_DEPLOYMENT SET {} WHERE ID_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(6)
                )
            }
            StatementId::DeleteDmnDeployment => {
                format!(
                    "DELETE FROM ACT_DMN_DEPLOYMENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnDeploymentById => {
                format!(
                    "SELECT * FROM ACT_DMN_DEPLOYMENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAllDmnDeployments => "SELECT * FROM ACT_DMN_DEPLOYMENT".to_string(),
            StatementId::InsertDmnDecisionDefinition => {
                let placeholders: Vec<String> = (0..7).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_DMN_DECISION (ID_, DECISION_KEY_, DEPLOYMENT_ID_, TENANT_ID_, VERSION_, RESOURCE_NAME_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateDmnDecisionDefinition => {
                let set_placeholders: Vec<String> = [
                    "DECISION_KEY_",
                    "DEPLOYMENT_ID_",
                    "TENANT_ID_",
                    "VERSION_",
                    "RESOURCE_NAME_",
                    "DATA_",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, column)| format!("{column} = {}", dialect.placeholder(i)))
                .collect();
                format!(
                    "UPDATE ACT_DMN_DECISION SET {} WHERE ID_ = {}",
                    set_placeholders.join(", "),
                    dialect.placeholder(6)
                )
            }
            StatementId::DeleteDmnDecisionDefinition => {
                format!(
                    "DELETE FROM ACT_DMN_DECISION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteDmnDecisionDefinitionsByDeploymentId => {
                format!(
                    "DELETE FROM ACT_DMN_DECISION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnDecisionDefinitionById => {
                format!(
                    "SELECT * FROM ACT_DMN_DECISION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnDecisionDefinitionsByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_DMN_DECISION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnDecisionDefinitionByKey => {
                format!(
                    "SELECT * FROM ACT_DMN_DECISION WHERE DECISION_KEY_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnDecisionDefinitionByKeyAndVersion => {
                format!(
                    "SELECT * FROM ACT_DMN_DECISION WHERE DECISION_KEY_ = {} AND VERSION_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::InsertDmnDeploymentResource => {
                let placeholders: Vec<String> = (0..6).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_DMN_RESOURCE (DEPLOYMENT_ID_, RESOURCE_NAME_, RESOURCE_TYPE_, CONTENT_TYPE_, BYTES_, CREATED_AT_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteDmnDeploymentResource => {
                format!(
                    "DELETE FROM ACT_DMN_RESOURCE WHERE DEPLOYMENT_ID_ = {} AND RESOURCE_NAME_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::DeleteDmnDeploymentResourcesByDeploymentId => {
                format!(
                    "DELETE FROM ACT_DMN_RESOURCE WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnDeploymentResourceById => {
                format!(
                    "SELECT * FROM ACT_DMN_RESOURCE WHERE DEPLOYMENT_ID_ = {} AND RESOURCE_NAME_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::SelectDmnDeploymentResourcesByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_DMN_RESOURCE WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertDmnDecisionRequirementsDiagram => {
                let placeholders: Vec<String> = (0..5).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_DMN_DRD (ID_, NAME_, DEPLOYMENT_ID_, RESOURCE_NAME_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteDmnDecisionRequirementsDiagram => {
                format!(
                    "DELETE FROM ACT_DMN_DRD WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteDmnDecisionRequirementsDiagramsByDeploymentId => {
                format!(
                    "DELETE FROM ACT_DMN_DRD WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnDecisionRequirementsDiagramById => {
                format!(
                    "SELECT * FROM ACT_DMN_DRD WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnDecisionRequirementsDiagramsByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_DMN_DRD WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertDmnExecutionHistory => {
                let placeholders: Vec<String> = (0..12).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_DMN_HI_EXECUTION (EXECUTION_ID_, DECISION_KEY_, DECISION_DEFINITION_ID_, DEPLOYMENT_ID_, BUSINESS_KEY_, TENANT_ID_, INSTANCE_ID_, SCOPE_EXECUTION_ID_, ACTIVITY_ID_, SCOPE_TYPE_, EXECUTED_AT_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteDmnExecutionHistory => {
                format!(
                    "DELETE FROM ACT_DMN_HI_EXECUTION WHERE EXECUTION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnExecutionHistoryById => {
                format!(
                    "SELECT * FROM ACT_DMN_HI_EXECUTION WHERE EXECUTION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnExecutionHistoriesByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_DMN_HI_EXECUTION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectDmnExecutionHistoriesByDecisionDefinitionId => {
                format!(
                    "SELECT * FROM ACT_DMN_HI_EXECUTION WHERE DECISION_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteDmnExecutionHistoriesByDecisionDefinitionId => {
                format!(
                    "DELETE FROM ACT_DMN_HI_EXECUTION WHERE DECISION_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteDmnExecutionHistoriesByDeploymentId => {
                format!(
                    "DELETE FROM ACT_DMN_HI_EXECUTION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertAppDeployment => {
                let placeholders: Vec<String> = (0..6).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_APP_DEPLOYMENT (ID_, NAME_, CATEGORY_, TENANT_ID_, DEPLOYED_AT_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteAppDeployment => {
                format!(
                    "DELETE FROM ACT_APP_DEPLOYMENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAppDeploymentById => {
                format!(
                    "SELECT * FROM ACT_APP_DEPLOYMENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAllAppDeployments => "SELECT * FROM ACT_APP_DEPLOYMENT".to_string(),
            StatementId::InsertAppDefinition => {
                let placeholders: Vec<String> = (0..7).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_APP_DEFINITION (ID_, APP_KEY_, DEPLOYMENT_ID_, TENANT_ID_, VERSION_, RESOURCE_NAME_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteAppDefinition => {
                format!(
                    "DELETE FROM ACT_APP_DEFINITION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteAppDefinitionsByDeploymentId => {
                format!(
                    "DELETE FROM ACT_APP_DEFINITION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAppDefinitionById => {
                format!(
                    "SELECT * FROM ACT_APP_DEFINITION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAppDefinitionsByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_APP_DEFINITION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAppDefinitionsByKey => {
                format!(
                    "SELECT * FROM ACT_APP_DEFINITION WHERE APP_KEY_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertAppResolvedComposition => {
                let placeholders: Vec<String> = (0..6).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_APP_RESOLVED_COMPOSITION (ID_, APP_DEFINITION_ID_, APP_KEY_, DEPLOYMENT_ID_, TENANT_ID_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteAppResolvedComposition => {
                format!(
                    "DELETE FROM ACT_APP_RESOLVED_COMPOSITION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteAppResolvedCompositionsByDeploymentId => {
                format!(
                    "DELETE FROM ACT_APP_RESOLVED_COMPOSITION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAppResolvedCompositionById => {
                format!(
                    "SELECT * FROM ACT_APP_RESOLVED_COMPOSITION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAppResolvedCompositionByAppDefinitionId => {
                format!(
                    "SELECT * FROM ACT_APP_RESOLVED_COMPOSITION WHERE APP_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAppResolvedCompositionsByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_APP_RESOLVED_COMPOSITION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertAppDeploymentResource => {
                let placeholders: Vec<String> = (0..6).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_APP_DEPLOYMENT_RESOURCE (DEPLOYMENT_ID_, RESOURCE_NAME_, RESOURCE_TYPE_, CONTENT_TYPE_, BYTES_, CREATED_AT_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteAppDeploymentResource => {
                format!(
                    "DELETE FROM ACT_APP_DEPLOYMENT_RESOURCE WHERE DEPLOYMENT_ID_ = {} AND RESOURCE_NAME_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::DeleteAppDeploymentResourcesByDeploymentId => {
                format!(
                    "DELETE FROM ACT_APP_DEPLOYMENT_RESOURCE WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAppDeploymentResourceById => {
                format!(
                    "SELECT * FROM ACT_APP_DEPLOYMENT_RESOURCE WHERE DEPLOYMENT_ID_ = {} AND RESOURCE_NAME_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::SelectAppDeploymentResourcesByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_APP_DEPLOYMENT_RESOURCE WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnDeployment => {
                let placeholders: Vec<String> = (0..8).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_CMMN_DEPLOYMENT (ID_, NAME_, CATEGORY_, KEY_, TENANT_ID_, PARENT_DEPLOYMENT_ID_, DEPLOYED_AT_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateCmmnDeploymentParentId => {
                format!(
                    "UPDATE ACT_CMMN_DEPLOYMENT SET PARENT_DEPLOYMENT_ID_ = {} WHERE ID_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::DeleteCmmnDeployment => {
                format!(
                    "DELETE FROM ACT_CMMN_DEPLOYMENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnDeploymentById => {
                format!(
                    "SELECT * FROM ACT_CMMN_DEPLOYMENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAllCmmnDeployments => {
                "SELECT * FROM ACT_CMMN_DEPLOYMENT".to_string()
            }
            StatementId::InsertCmmnCaseDefinition => {
                let placeholders: Vec<String> = (0..9).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_CMMN_CASE_DEFINITION (ID_, CASE_KEY_, DEPLOYMENT_ID_, TENANT_ID_, CATEGORY_, VERSION_, RESOURCE_NAME_, DIAGRAM_RESOURCE_NAME_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::UpdateCmmnCaseDefinitionCategory => {
                format!(
                    "UPDATE ACT_CMMN_CASE_DEFINITION SET CATEGORY_ = {} WHERE ID_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::DeleteCmmnCaseDefinition => {
                format!(
                    "DELETE FROM ACT_CMMN_CASE_DEFINITION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnCaseDefinitionsByDeploymentId => {
                format!(
                    "DELETE FROM ACT_CMMN_CASE_DEFINITION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnCaseDefinitionById => {
                format!(
                    "SELECT * FROM ACT_CMMN_CASE_DEFINITION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAllCmmnCaseDefinitions => {
                "SELECT * FROM ACT_CMMN_CASE_DEFINITION".to_string()
            }
            StatementId::SelectCmmnCaseDefinitionsByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_CMMN_CASE_DEFINITION WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnCaseDefinitionByKey => {
                format!(
                    "SELECT * FROM ACT_CMMN_CASE_DEFINITION WHERE CASE_KEY_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnCaseDefinitionByKeyAndVersion => {
                format!(
                    "SELECT * FROM ACT_CMMN_CASE_DEFINITION WHERE CASE_KEY_ = {} AND VERSION_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::InsertCmmnDeploymentResource => {
                let placeholders: Vec<String> = (0..6).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_CMMN_DEPLOYMENT_RESOURCE (DEPLOYMENT_ID_, RESOURCE_NAME_, RESOURCE_TYPE_, CONTENT_TYPE_, BYTES_, CREATED_AT_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteCmmnDeploymentResource => {
                format!(
                    "DELETE FROM ACT_CMMN_DEPLOYMENT_RESOURCE WHERE DEPLOYMENT_ID_ = {} AND RESOURCE_NAME_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::DeleteCmmnDeploymentResourcesByDeploymentId => {
                format!(
                    "DELETE FROM ACT_CMMN_DEPLOYMENT_RESOURCE WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnDeploymentResourceById => {
                format!(
                    "SELECT * FROM ACT_CMMN_DEPLOYMENT_RESOURCE WHERE DEPLOYMENT_ID_ = {} AND RESOURCE_NAME_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::SelectCmmnDeploymentResourcesByDeploymentId => {
                format!(
                    "SELECT * FROM ACT_CMMN_DEPLOYMENT_RESOURCE WHERE DEPLOYMENT_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnCaseInstance => {
                let placeholders: Vec<String> = (0..8).map(|i| dialect.placeholder(i)).collect();
                let columns = [
                    "CASE_DEFINITION_ID_",
                    "CASE_KEY_",
                    "TENANT_ID_",
                    "BUSINESS_KEY_",
                    "STATE_",
                    "STARTED_AT_",
                    "DATA_",
                ];
                let mut sql = format!(
                    "{} ACT_CMMN_CASE_INSTANCE (ID_, CASE_DEFINITION_ID_, CASE_KEY_, TENANT_ID_, BUSINESS_KEY_, STATE_, STARTED_AT_, DATA_) VALUES ({})",
                    dialect.insert_or_replace_into(),
                    placeholders.join(", ")
                );
                if dialect.supports_on_conflict_update() {
                    sql += &dialect.on_conflict_do_update_suffix("ID_", &columns);
                }
                sql
            }
            StatementId::DeleteCmmnCaseInstance => {
                format!(
                    "DELETE FROM ACT_CMMN_CASE_INSTANCE WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnCaseInstancesByCaseDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_CASE_INSTANCE WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnCaseInstanceById => {
                format!(
                    "SELECT * FROM ACT_CMMN_CASE_INSTANCE WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnCaseInstancesByCaseDefinitionId => {
                format!(
                    "SELECT * FROM ACT_CMMN_CASE_INSTANCE WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnCaseInstanceIdsByCaseDefinitionId => {
                format!(
                    "SELECT ID_ FROM ACT_CMMN_CASE_INSTANCE WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectHistoricCmmnCaseInstanceIdsByCaseDefinitionId => {
                format!(
                    "SELECT CASE_INSTANCE_ID_ FROM ACT_CMMN_CASE_HISTORY WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnStageInstance => {
                let placeholders: Vec<String> = (0..7).map(|i| dialect.placeholder(i)).collect();
                let columns = [
                    "CASE_INSTANCE_ID_",
                    "PARENT_STAGE_INSTANCE_ID_",
                    "STAGE_DEFINITION_ID_",
                    "STATE_",
                    "ACTIVATED_AT_",
                    "DATA_",
                ];
                let mut sql = format!(
                    "{} ACT_CMMN_STAGE_INSTANCE (ID_, CASE_INSTANCE_ID_, PARENT_STAGE_INSTANCE_ID_, STAGE_DEFINITION_ID_, STATE_, ACTIVATED_AT_, DATA_) VALUES ({})",
                    dialect.insert_or_replace_into(),
                    placeholders.join(", ")
                );
                if dialect.supports_on_conflict_update() {
                    sql += &dialect.on_conflict_do_update_suffix("ID_", &columns);
                }
                sql
            }
            StatementId::DeleteCmmnStageInstance => {
                format!(
                    "DELETE FROM ACT_CMMN_STAGE_INSTANCE WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnStageInstancesByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnStageInstanceById => {
                format!(
                    "SELECT * FROM ACT_CMMN_STAGE_INSTANCE WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnStageInstancesByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnStageHistory => {
                let placeholders: Vec<String> = (0..9).map(|i| dialect.placeholder(i)).collect();
                let columns = [
                    "CASE_INSTANCE_ID_",
                    "CASE_DEFINITION_ID_",
                    "PARENT_STAGE_INSTANCE_ID_",
                    "STAGE_DEFINITION_ID_",
                    "STATE_",
                    "ACTIVATED_AT_",
                    "ENDED_AT_",
                    "DATA_",
                ];
                let mut sql = format!(
                    "{} ACT_CMMN_STAGE_HISTORY (ID_, CASE_INSTANCE_ID_, CASE_DEFINITION_ID_, PARENT_STAGE_INSTANCE_ID_, STAGE_DEFINITION_ID_, STATE_, ACTIVATED_AT_, ENDED_AT_, DATA_) VALUES ({})",
                    dialect.insert_or_replace_into(),
                    placeholders.join(", ")
                );
                if dialect.supports_on_conflict_update() {
                    sql += &dialect.on_conflict_do_update_suffix("ID_", &columns);
                }
                sql
            }
            StatementId::DeleteCmmnStageHistory => {
                format!(
                    "DELETE FROM ACT_CMMN_STAGE_HISTORY WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnStageHistoryByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_STAGE_HISTORY WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnStageHistoryByCaseDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_STAGE_HISTORY WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnStageHistoryById => {
                format!(
                    "SELECT * FROM ACT_CMMN_STAGE_HISTORY WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnStageHistoryByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_STAGE_HISTORY WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnHumanTask => {
                let placeholders: Vec<String> = (0..8).map(|i| dialect.placeholder(i)).collect();
                let columns = [
                    "CASE_INSTANCE_ID_",
                    "CASE_DEFINITION_ID_",
                    "CASE_KEY_",
                    "STAGE_INSTANCE_ID_",
                    "STATE_",
                    "ACTIVATED_AT_",
                    "DATA_",
                ];
                let mut sql = format!(
                    "{} ACT_CMMN_HUMAN_TASK (ID_, CASE_INSTANCE_ID_, CASE_DEFINITION_ID_, CASE_KEY_, STAGE_INSTANCE_ID_, STATE_, ACTIVATED_AT_, DATA_) VALUES ({})",
                    dialect.insert_or_replace_into(),
                    placeholders.join(", ")
                );
                if dialect.supports_on_conflict_update() {
                    sql += &dialect.on_conflict_do_update_suffix("ID_", &columns);
                }
                sql
            }
            StatementId::DeleteCmmnHumanTask => {
                format!(
                    "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnHumanTasksByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnHumanTasksByCaseDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnHumanTaskById => {
                format!(
                    "SELECT * FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnHumanTasksByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnCaseHistory => {
                let placeholders: Vec<String> = (0..9).map(|i| dialect.placeholder(i)).collect();
                let columns = [
                    "CASE_DEFINITION_ID_",
                    "CASE_KEY_",
                    "TENANT_ID_",
                    "BUSINESS_KEY_",
                    "STATE_",
                    "STARTED_AT_",
                    "COMPLETED_AT_",
                    "DATA_",
                ];
                let mut sql = format!(
                    "{} ACT_CMMN_CASE_HISTORY (CASE_INSTANCE_ID_, CASE_DEFINITION_ID_, CASE_KEY_, TENANT_ID_, BUSINESS_KEY_, STATE_, STARTED_AT_, COMPLETED_AT_, DATA_) VALUES ({})",
                    dialect.insert_or_replace_into(),
                    placeholders.join(", ")
                );
                if dialect.supports_on_conflict_update() {
                    sql += &dialect.on_conflict_do_update_suffix("CASE_INSTANCE_ID_", &columns);
                }
                sql
            }
            StatementId::DeleteCmmnCaseHistory => {
                format!(
                    "DELETE FROM ACT_CMMN_CASE_HISTORY WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnCaseHistoryByCaseDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_CASE_HISTORY WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnCaseHistoryById => {
                format!(
                    "SELECT * FROM ACT_CMMN_CASE_HISTORY WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnHumanTaskHistory => {
                let placeholders: Vec<String> = (0..9).map(|i| dialect.placeholder(i)).collect();
                let columns = [
                    "CASE_INSTANCE_ID_",
                    "CASE_DEFINITION_ID_",
                    "CASE_KEY_",
                    "STAGE_INSTANCE_ID_",
                    "STATE_",
                    "ACTIVATED_AT_",
                    "COMPLETED_AT_",
                    "DATA_",
                ];
                let mut sql = format!(
                    "{} ACT_CMMN_HUMAN_TASK_HISTORY (TASK_ID_, CASE_INSTANCE_ID_, CASE_DEFINITION_ID_, CASE_KEY_, STAGE_INSTANCE_ID_, STATE_, ACTIVATED_AT_, COMPLETED_AT_, DATA_) VALUES ({})",
                    dialect.insert_or_replace_into(),
                    placeholders.join(", ")
                );
                if dialect.supports_on_conflict_update() {
                    sql += &dialect.on_conflict_do_update_suffix("TASK_ID_", &columns);
                }
                sql
            }
            StatementId::DeleteCmmnHumanTaskHistory => {
                format!(
                    "DELETE FROM ACT_CMMN_HUMAN_TASK_HISTORY WHERE TASK_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnHumanTaskHistoryByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_HUMAN_TASK_HISTORY WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnHumanTaskHistoryByCaseDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_HUMAN_TASK_HISTORY WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnHumanTaskHistoryById => {
                format!(
                    "SELECT * FROM ACT_CMMN_HUMAN_TASK_HISTORY WHERE TASK_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnHumanTaskHistoryByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_HUMAN_TASK_HISTORY WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnMilestoneHistory => {
                let placeholders: Vec<String> = (0..7).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_CMMN_MILESTONE_HISTORY (ID_, CASE_INSTANCE_ID_, CASE_DEFINITION_ID_, CASE_KEY_, MILESTONE_ID_, TIME_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteCmmnMilestoneHistory => {
                format!(
                    "DELETE FROM ACT_CMMN_MILESTONE_HISTORY WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnMilestoneHistoryByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_MILESTONE_HISTORY WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnMilestoneHistoryByCaseDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_MILESTONE_HISTORY WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnMilestoneHistoryById => {
                format!(
                    "SELECT * FROM ACT_CMMN_MILESTONE_HISTORY WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnMilestoneHistoryByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_MILESTONE_HISTORY WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnIdentityLink => {
                let placeholders: Vec<String> = (0..7).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_CMMN_IDENTITY_LINK (ID_, SCOPE_TYPE_, SCOPE_ID_, LINK_TYPE_, USER_ID_, GROUP_ID_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteCmmnIdentityLink => {
                format!(
                    "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnIdentityLinksByScopeDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_ID_ = {} AND SCOPE_TYPE_ = 'definition'",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnIdentityLinksByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_ID_ = {} AND SCOPE_TYPE_ = 'caseInstance'",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnIdentityLinksByTaskId => {
                format!(
                    "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_ID_ = {} AND SCOPE_TYPE_ = 'humanTask'",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnIdentityLinkById => {
                format!(
                    "SELECT * FROM ACT_CMMN_IDENTITY_LINK WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnIdentityLinksByScope => {
                format!(
                    "SELECT * FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_TYPE_ = {} AND SCOPE_ID_ = {}",
                    dialect.placeholder(0),
                    dialect.placeholder(1)
                )
            }
            StatementId::InsertCmmnJob => {
                let placeholders: Vec<String> = (0..15).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_CMMN_JOB (ID_, FAMILY_, STATE_, SCOPE_ID_, SUB_SCOPE_ID_, SCOPE_DEFINITION_ID_, ELEMENT_ID_, TENANT_ID_, DUE_DATE_, LOCK_OWNER_, RETRIES_, EXCEPTION_MESSAGE_, EXCEPTION_STACKTRACE_, CREATED_AT_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteCmmnJob => {
                format!(
                    "DELETE FROM ACT_CMMN_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnJobsByScopeId => {
                format!(
                    "DELETE FROM ACT_CMMN_JOB WHERE SCOPE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnJobsBySubScopeId => {
                format!(
                    "DELETE FROM ACT_CMMN_JOB WHERE SUB_SCOPE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnJobsByScopeDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_JOB WHERE SCOPE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnJobById => {
                format!(
                    "SELECT * FROM ACT_CMMN_JOB WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnJobsByScopeId => {
                format!(
                    "SELECT * FROM ACT_CMMN_JOB WHERE SCOPE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnEventSubscription => {
                let placeholders: Vec<String> = (0..11).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_CMMN_EVENT_SUBSCRIPTION (ID_, EVENT_TYPE_, EVENT_NAME_, ACTIVITY_ID_, CASE_INSTANCE_ID_, CASE_DEFINITION_ID_, PLAN_ITEM_INSTANCE_ID_, TENANT_ID_, CONFIGURATION_, CREATED_AT_, DATA_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteCmmnEventSubscription => {
                format!(
                    "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnEventSubscriptionsByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnEventSubscriptionsByCaseDefinitionId => {
                format!(
                    "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_DEFINITION_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnEventSubscriptionById => {
                format!(
                    "SELECT * FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnEventSubscriptionsByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnTaskInstanceAssociation => {
                let placeholders: Vec<String> = (0..15).map(|i| dialect.placeholder(i)).collect();
                let columns = [
                    "KIND_",
                    "STATE_",
                    "CASE_INSTANCE_ID_",
                    "CASE_DEFINITION_ID_",
                    "CASE_KEY_",
                    "STAGE_INSTANCE_ID_",
                    "PLAN_ITEM_ID_",
                    "TASK_DEFINITION_ID_",
                    "CHILD_DEFINITION_KEY_",
                    "CHILD_INSTANCE_ID_",
                    "CREATED_AT_",
                    "COMPLETED_AT_",
                    "FAILURE_MESSAGE_",
                    "DATA_",
                ];
                let mut sql = format!(
                    "{} ACT_CMMN_TASK_INSTANCE_ASSOCIATION (ID_, KIND_, STATE_, CASE_INSTANCE_ID_, CASE_DEFINITION_ID_, CASE_KEY_, STAGE_INSTANCE_ID_, PLAN_ITEM_ID_, TASK_DEFINITION_ID_, CHILD_DEFINITION_KEY_, CHILD_INSTANCE_ID_, CREATED_AT_, COMPLETED_AT_, FAILURE_MESSAGE_, DATA_) VALUES ({})",
                    dialect.insert_or_replace_into(),
                    placeholders.join(", ")
                );
                if dialect.supports_on_conflict_update() {
                    sql += &dialect.on_conflict_do_update_suffix("ID_", &columns);
                }
                sql
            }
            StatementId::DeleteCmmnTaskInstanceAssociation => {
                format!(
                    "DELETE FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnTaskInstanceAssociationsByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnTaskInstanceAssociationById => {
                format!(
                    "SELECT * FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnTaskInstanceAssociationsByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnPlanItemEvent => {
                let placeholders: Vec<String> = (0..5).map(|i| dialect.placeholder(i)).collect();
                format!(
                    "INSERT INTO ACT_CMMN_PLAN_ITEM_EVENT (ID_, CASE_INSTANCE_ID_, PLAN_ITEM_ID_, STANDARD_EVENT_, OCCURRED_AT_) VALUES ({})",
                    placeholders.join(", ")
                )
            }
            StatementId::DeleteCmmnPlanItemEvent => {
                format!(
                    "DELETE FROM ACT_CMMN_PLAN_ITEM_EVENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnPlanItemEventsByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_PLAN_ITEM_EVENT WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnPlanItemEventById => {
                format!(
                    "SELECT * FROM ACT_CMMN_PLAN_ITEM_EVENT WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnPlanItemEventsByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_PLAN_ITEM_EVENT WHERE CASE_INSTANCE_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::InsertCmmnPlanItemInstance => {
                let placeholders: Vec<String> = (0..15).map(|i| dialect.placeholder(i)).collect();
                let columns = [
                    "CASE_DEF_ID_",
                    "CASE_INST_ID_",
                    "STAGE_INST_ID_",
                    "ELEMENT_ID_",
                    "ITEM_DEFINITION_ID_",
                    "ITEM_DEFINITION_TYPE_",
                    "NAME_",
                    "STATE_",
                    "CREATE_TIME_",
                    "ENDED_TIME_",
                    "OCCURRED_TIME_",
                    "ASSIGNEE_",
                    "TENANT_ID_",
                    "DATA_",
                ];
                let mut sql = format!(
                    "{} ACT_CMMN_RU_PLAN_ITEM_INST (ID_, CASE_DEF_ID_, CASE_INST_ID_, STAGE_INST_ID_, ELEMENT_ID_, ITEM_DEFINITION_ID_, ITEM_DEFINITION_TYPE_, NAME_, STATE_, CREATE_TIME_, ENDED_TIME_, OCCURRED_TIME_, ASSIGNEE_, TENANT_ID_, DATA_) VALUES ({})",
                    dialect.insert_or_replace_into(),
                    placeholders.join(", ")
                );
                if dialect.supports_on_conflict_update() {
                    sql += &dialect.on_conflict_do_update_suffix("ID_", &columns);
                }
                sql
            }
            StatementId::DeleteCmmnPlanItemInstance => {
                format!(
                    "DELETE FROM ACT_CMMN_RU_PLAN_ITEM_INST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::DeleteCmmnPlanItemInstancesByCaseInstanceId => {
                format!(
                    "DELETE FROM ACT_CMMN_RU_PLAN_ITEM_INST WHERE CASE_INST_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnPlanItemInstanceById => {
                format!(
                    "SELECT * FROM ACT_CMMN_RU_PLAN_ITEM_INST WHERE ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectCmmnPlanItemInstancesByCaseInstanceId => {
                format!(
                    "SELECT * FROM ACT_CMMN_RU_PLAN_ITEM_INST WHERE CASE_INST_ID_ = {}",
                    dialect.placeholder(0)
                )
            }
            StatementId::SelectAllCmmnPlanItemInstances => {
                format!("SELECT * FROM ACT_CMMN_RU_PLAN_ITEM_INST")
            }
        };
        Ok(RenderedStatement::new(sql, params.clone()))
    }
    fn dialect(&self) -> &dyn SqlDialect {
        self.dialect.as_ref()
    }
}
pub type PropertyStatementCatalog = FlowableStatementCatalog;
