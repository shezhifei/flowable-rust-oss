use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use crate::routes::{dmn::DecisionTableRecord, forms::FormDefinitionRecord};
use flowable_cmmn_engine::{
    CMMN_SCOPE_TYPE, CmmnHumanTaskUpdate, QueryVariableCondition, QueryVariableOperation,
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::Path,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

pub type DynCmmnRepository = Arc<dyn CmmnRepositoryApi>;
pub type DynCmmnRuntime = Arc<dyn CmmnRuntimeApi>;
pub type DynCmmnHistory = Arc<dyn CmmnHistoryApi>;
pub type DynCmmnManagement = Arc<dyn CmmnManagementApi>;

pub trait CmmnRepositoryApi: Send + Sync {
    fn deploy_case_definitions(
        &self,
        command: CmmnDeploymentCommand,
    ) -> Result<CmmnDeploymentRecord, ApiError>;
    fn get_engine_info(&self) -> Result<CmmnEngineInfoRecord, ApiError> {
        Ok(CmmnEngineInfoRecord {
            name: "cmmn-engine".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            resource_url: None,
            exception: None,
        })
    }
    fn get_deployment(&self, deployment_id: &str) -> Result<CmmnDeploymentRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN deployment '{deployment_id}' was not found"
        )))
    }
    fn list_deployments(
        &self,
        query: CmmnDeploymentQuery,
    ) -> Result<PagedResponse<CmmnDeploymentRecord>, ApiError> {
        let deployment_id = query.id.clone();
        let mut deployments = match deployment_id {
            Some(deployment_id) => vec![self.get_deployment(&deployment_id)?],
            None => Vec::new(),
        };
        deployments.retain(|deployment| deployment_matches_query(deployment, &query));
        Ok(query.paging.paginate(deployments))
    }
    fn delete_deployment(&self, deployment_id: &str, cascade: bool) -> Result<(), ApiError> {
        let _ = cascade;
        Err(ApiError::NotFound(format!(
            "CMMN deployment '{deployment_id}' was not found"
        )))
    }
    fn get_deployment_resource_data(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<CmmnResourceDataRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN deployment resource '{resource_name}' was not found in deployment '{deployment_id}'"
        )))
    }
    fn list_case_definitions(
        &self,
        query: CaseDefinitionQuery,
    ) -> Result<PagedResponse<CaseDefinitionRecord>, ApiError>;
    fn get_case_definition(
        &self,
        case_definition_id: &str,
    ) -> Result<CaseDefinitionRecord, ApiError>;
    fn get_case_definition_resource_data(
        &self,
        case_definition_id: &str,
    ) -> Result<CmmnResourceDataRecord, ApiError> {
        let definition = self.get_case_definition(case_definition_id)?;
        self.get_deployment_resource_data(&definition.deployment_id, &definition.resource_name)
    }
    fn get_case_definition_model(&self, case_definition_id: &str) -> Result<Value, ApiError> {
        serde_json::to_value(self.get_case_definition(case_definition_id)?)
            .map_err(|err| ApiError::InternalServerError(err.to_string()))
    }
    fn list_case_definition_decision_tables(
        &self,
        case_definition_id: &str,
        paging: PagingQuery,
    ) -> Result<PagedResponse<DecisionTableRecord>, ApiError> {
        self.get_case_definition(case_definition_id)?;
        Ok(paging.paginate(Vec::new()))
    }
    fn list_case_definition_decisions(
        &self,
        case_definition_id: &str,
        paging: PagingQuery,
    ) -> Result<PagedResponse<DecisionTableRecord>, ApiError> {
        self.list_case_definition_decision_tables(case_definition_id, paging)
    }
    fn list_case_definition_form_definitions(
        &self,
        case_definition_id: &str,
        paging: PagingQuery,
    ) -> Result<PagedResponse<FormDefinitionRecord>, ApiError> {
        self.get_case_definition(case_definition_id)?;
        Ok(paging.paginate(Vec::new()))
    }
    fn get_case_definition_start_form(&self, case_definition_id: &str) -> Result<Value, ApiError> {
        self.get_case_definition(case_definition_id)?;
        Err(ApiError::NotFound(format!(
            "CMMN case definition '{case_definition_id}' start form was not found"
        )))
    }
    fn list_case_definition_identity_links(
        &self,
        case_definition_id: &str,
    ) -> Result<Vec<CmmnIdentityLinkRecord>, ApiError> {
        let _ = case_definition_id;
        Ok(Vec::new())
    }
    fn create_case_definition_identity_link(
        &self,
        case_definition_id: &str,
        command: CmmnIdentityLinkCreateCommand,
    ) -> Result<CmmnIdentityLinkRecord, ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case definition '{case_definition_id}' was not found"
        )))
    }
    fn delete_case_definition_identity_links(
        &self,
        case_definition_id: &str,
        family: &str,
        identity_id: &str,
    ) -> Result<(), ApiError> {
        let _ = (family, identity_id);
        Err(ApiError::NotFound(format!(
            "CMMN case definition '{case_definition_id}' identity link was not found"
        )))
    }
    /// Java `CaseDefinitionResource.executeCaseDefinitionAction` delegates to
    /// `CmmnRepositoryService.setCaseDefinitionCategory`
    /// (CaseDefinitionResource.java:100).
    fn set_case_definition_category(
        &self,
        case_definition_id: &str,
        category: &str,
    ) -> Result<CaseDefinitionRecord, ApiError> {
        let _ = category;
        Err(ApiError::NotFound(format!(
            "CMMN case definition '{case_definition_id}' was not found"
        )))
    }
    fn migrate_case_definition_instances(
        &self,
        case_definition_id: &str,
        command: CmmnMigrationCommand,
    ) -> Result<(), ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case definition '{case_definition_id}' was not found"
        )))
    }
    fn batch_migrate_case_definition_instances(
        &self,
        case_definition_id: &str,
        command: CmmnMigrationCommand,
    ) -> Result<(), ApiError> {
        self.migrate_case_definition_instances(case_definition_id, command)
    }
    fn migrate_historic_case_definition_instances(
        &self,
        case_definition_id: &str,
        command: CmmnMigrationCommand,
    ) -> Result<(), ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case definition '{case_definition_id}' was not found"
        )))
    }
    fn batch_migrate_historic_case_definition_instances(
        &self,
        case_definition_id: &str,
        command: CmmnMigrationCommand,
    ) -> Result<(), ApiError> {
        self.migrate_historic_case_definition_instances(case_definition_id, command)
    }
}

pub trait CmmnRuntimeApi: Send + Sync {
    fn start_case_instance(
        &self,
        command: StartCaseInstanceCommand,
    ) -> Result<CaseInstanceRecord, ApiError>;
    fn list_case_instances(
        &self,
        query: CaseInstanceQuery,
    ) -> Result<PagedResponse<CaseInstanceRecord>, ApiError>;
    fn terminate_case_instance(&self, case_instance_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    fn delete_case_instance(&self, case_instance_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    fn bulk_delete_case_instances(&self, case_instance_ids: Vec<String>) -> Result<(), ApiError> {
        for case_instance_id in case_instance_ids {
            self.delete_case_instance(&case_instance_id)?;
        }
        Ok(())
    }
    fn bulk_terminate_case_instances(
        &self,
        case_instance_ids: Vec<String>,
    ) -> Result<(), ApiError> {
        for case_instance_id in case_instance_ids {
            self.terminate_case_instance(&case_instance_id)?;
        }
        Ok(())
    }
    fn list_plan_item_instances(
        &self,
        query: PlanItemInstanceQuery,
    ) -> Result<PagedResponse<PlanItemInstanceRecord>, ApiError>;
    fn complete_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), ApiError>;
    fn reactivate_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), ApiError> {
        let _ = plan_item_instance_id;
        Err(ApiError::NotFound(format!(
            "CMMN plan item instance '{plan_item_instance_id}' was not found"
        )))
    }
    fn disable_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), ApiError> {
        let _ = plan_item_instance_id;
        Err(ApiError::NotFound(format!(
            "CMMN plan item instance '{plan_item_instance_id}' was not found"
        )))
    }
    fn enable_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), ApiError> {
        let _ = plan_item_instance_id;
        Err(ApiError::NotFound(format!(
            "CMMN plan item instance '{plan_item_instance_id}' was not found"
        )))
    }
    fn get_stage_overview(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<StageOverviewRecord>, ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN stage overview for case instance '{case_instance_id}' was not found"
        )))
    }
    fn list_variable_instances(
        &self,
        query: VariableInstanceQuery,
    ) -> Result<PagedResponse<VariableInstanceRecord>, ApiError> {
        let _ = query;
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }
    fn get_variable_instance(
        &self,
        variable_instance_id: &str,
    ) -> Result<VariableInstanceRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN variable instance '{variable_instance_id}' was not found"
        )))
    }
    fn set_case_instance_variables(
        &self,
        case_instance_id: &str,
        variables: Vec<CmmnVariableUpdate>,
    ) -> Result<(), ApiError> {
        let _ = variables;
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    // Java: CaseInstanceResource.java:88-130 — PUT case-instance name/businessKey/evaluateCriteria.
    // None means the case ended as a result of the action (HTTP 204).
    fn update_case_instance(
        &self,
        case_instance_id: &str,
        command: CmmnCaseInstanceUpdateCommand,
    ) -> Result<Option<CaseInstanceRecord>, ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    // Java: PlanItemInstanceResource.java:59-84 — PUT plan-item action surface
    fn perform_plan_item_instance_action(
        &self,
        plan_item_instance_id: &str,
        action: &str,
    ) -> Result<Option<PlanItemInstanceRecord>, ApiError> {
        let _ = action;
        Err(ApiError::NotFound(format!(
            "CMMN plan item instance '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: CaseInstanceVariableResource.java:176 — DELETE single variable
    fn remove_case_instance_variable(
        &self,
        case_instance_id: &str,
        variable_name: &str,
    ) -> Result<(), ApiError> {
        let _ = variable_name;
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    // Java: CaseInstanceVariableCollectionResource.java:180 — DELETE all variables
    fn remove_case_instance_variables(&self, case_instance_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    fn list_case_instance_identity_links(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnIdentityLinkRecord>, ApiError> {
        let _ = case_instance_id;
        Ok(Vec::new())
    }
    fn create_case_instance_identity_link(
        &self,
        case_instance_id: &str,
        command: CmmnIdentityLinkCreateCommand,
    ) -> Result<CmmnIdentityLinkRecord, ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    fn delete_case_instance_identity_link(
        &self,
        case_instance_id: &str,
        identity_id: &str,
        link_type: &str,
    ) -> Result<(), ApiError> {
        let _ = (identity_id, link_type);
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' identity link was not found"
        )))
    }
    fn get_task_form(&self, plan_item_instance_id: &str) -> Result<Value, ApiError> {
        let _ = plan_item_instance_id;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' form was not found"
        )))
    }
    fn list_task_identity_links(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<Vec<CmmnIdentityLinkRecord>, ApiError> {
        let _ = plan_item_instance_id;
        Ok(Vec::new())
    }
    fn create_task_identity_link(
        &self,
        plan_item_instance_id: &str,
        command: CmmnIdentityLinkCreateCommand,
    ) -> Result<CmmnIdentityLinkRecord, ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    fn delete_task_identity_link(
        &self,
        plan_item_instance_id: &str,
        family: &str,
        identity_id: &str,
        link_type: &str,
    ) -> Result<(), ApiError> {
        let _ = (family, identity_id, link_type);
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' identity link was not found"
        )))
    }
    // Java: TaskResource.java:76-99 — PUT update task fields (null clears).
    fn update_task(
        &self,
        plan_item_instance_id: &str,
        update: CmmnHumanTaskUpdate,
    ) -> Result<PlanItemInstanceRecord, ApiError> {
        let _ = update;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: TaskResource.java:109-137 — POST task action (complete/claim/delegate/resolve).
    fn execute_task_action(
        &self,
        plan_item_instance_id: &str,
        action_request: CmmnTaskActionRequest,
    ) -> Result<(), ApiError> {
        let _ = action_request;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: TaskResource.java:149-174 — DELETE task; CMMN tasks are always forbidden.
    fn delete_task(&self, plan_item_instance_id: &str) -> Result<(), ApiError> {
        let _ = plan_item_instance_id;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: TaskVariableCollectionResource.java:122-212 — POST batch create (GLOBAL scope).
    fn create_task_variables(
        &self,
        plan_item_instance_id: &str,
        variables: Vec<CmmnVariableUpdate>,
    ) -> Result<(), ApiError> {
        let _ = variables;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: TaskVariableCollectionResource.java:219-228 — DELETE all local variables.
    fn delete_task_variables(&self, plan_item_instance_id: &str) -> Result<(), ApiError> {
        let _ = plan_item_instance_id;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: TaskVariableResource.java:94-130 — PUT single variable (GLOBAL scope).
    fn update_task_variable(
        &self,
        plan_item_instance_id: &str,
        variable_name: &str,
        variable: CmmnVariableUpdate,
    ) -> Result<(), ApiError> {
        let _ = (variable_name, variable);
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: TaskVariableResource.java:138-167 — DELETE single variable (GLOBAL scope).
    fn delete_task_variable(
        &self,
        plan_item_instance_id: &str,
        variable_name: &str,
    ) -> Result<(), ApiError> {
        let _ = variable_name;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // P115: task-local variables — the task's own scope
    // (TaskService.getVariablesLocal, VariableScopeImpl.java:455-470).
    fn list_task_variables_local(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<Vec<VariableInstanceRecord>, ApiError> {
        let _ = plan_item_instance_id;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: TaskService.setVariablesLocal (TaskServiceImpl.java:445-447) →
    // SetTaskVariablesCmd.java:42-47.
    fn set_task_variables_local(
        &self,
        plan_item_instance_id: &str,
        variables: Vec<CmmnVariableUpdate>,
    ) -> Result<(), ApiError> {
        let _ = variables;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    // Java: TaskService.removeVariableLocal (TaskServiceImpl.java:457-461).
    fn remove_task_variable_local(
        &self,
        plan_item_instance_id: &str,
        variable_name: &str,
    ) -> Result<(), ApiError> {
        let _ = variable_name;
        Err(ApiError::NotFound(format!(
            "CMMN task '{plan_item_instance_id}' was not found"
        )))
    }
    fn validate_case_instance_migration(
        &self,
        case_instance_id: &str,
        command: CmmnMigrationCommand,
    ) -> Result<CmmnMigrationValidationRecord, ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    fn migrate_case_instance(
        &self,
        case_instance_id: &str,
        command: CmmnMigrationCommand,
    ) -> Result<(), ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    fn change_plan_item_state(
        &self,
        case_instance_id: &str,
        command: CmmnChangePlanItemStateCommand,
    ) -> Result<(), ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    fn list_event_subscriptions(
        &self,
        query: CmmnEventSubscriptionQuery,
    ) -> Result<PagedResponse<CmmnEventSubscriptionRecord>, ApiError> {
        let _ = query;
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }
    fn get_event_subscription(
        &self,
        event_subscription_id: &str,
    ) -> Result<CmmnEventSubscriptionRecord, ApiError> {
        self.list_event_subscriptions(CmmnEventSubscriptionQuery {
            id: Some(event_subscription_id.to_string()),
            paging: PagingQuery {
                start: 0,
                size: Some(1),
            },
            ..CmmnEventSubscriptionQuery::default()
        })?
        .data
        .into_iter()
        .next()
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "CMMN event subscription '{event_subscription_id}' was not found"
            ))
        })
    }
    fn trigger_case_event(
        &self,
        case_instance_id: &str,
        command: CmmnTriggerEventCommand,
    ) -> Result<(), ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "CMMN case instance '{case_instance_id}' was not found"
        )))
    }
}

pub trait CmmnHistoryApi: Send + Sync {
    fn list_historic_case_instances(
        &self,
        query: HistoricCaseInstanceQuery,
    ) -> Result<PagedResponse<HistoricCaseInstanceRecord>, ApiError>;
    fn delete_historic_case_instance(&self, case_instance_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "Historic CMMN case instance '{case_instance_id}' was not found"
        )))
    }
    fn bulk_delete_historic_case_instances(
        &self,
        case_instance_ids: Vec<String>,
    ) -> Result<(), ApiError> {
        for case_instance_id in case_instance_ids {
            self.delete_historic_case_instance(&case_instance_id)?;
        }
        Ok(())
    }
    fn list_historic_plan_item_instances(
        &self,
        query: HistoricPlanItemInstanceQuery,
    ) -> Result<PagedResponse<HistoricPlanItemInstanceRecord>, ApiError>;
    fn list_historic_milestone_instances(
        &self,
        query: HistoricMilestoneInstanceQuery,
    ) -> Result<PagedResponse<HistoricMilestoneInstanceRecord>, ApiError> {
        let _ = query;
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }
    fn get_historic_stage_overview(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<StageOverviewRecord>, ApiError> {
        Err(ApiError::NotFound(format!(
            "Historic CMMN stage overview for case instance '{case_instance_id}' was not found"
        )))
    }
    fn list_historic_variable_instances(
        &self,
        query: HistoricVariableInstanceQuery,
    ) -> Result<PagedResponse<HistoricVariableInstanceRecord>, ApiError> {
        let _ = query;
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }
    fn get_historic_variable_instance(
        &self,
        variable_instance_id: &str,
    ) -> Result<HistoricVariableInstanceRecord, ApiError> {
        self.list_historic_variable_instances(HistoricVariableInstanceQuery {
            id: Some(variable_instance_id.to_string()),
            paging: PagingQuery {
                start: 0,
                size: Some(1),
            },
            ..HistoricVariableInstanceQuery::default()
        })?
        .data
        .into_iter()
        .next()
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic CMMN variable instance '{variable_instance_id}' was not found"
            ))
        })
    }
    fn list_historic_case_instance_identity_links(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnIdentityLinkRecord>, ApiError> {
        let _ = case_instance_id;
        Ok(Vec::new())
    }
    fn get_historic_task_form(&self, plan_item_instance_id: &str) -> Result<Value, ApiError> {
        let _ = plan_item_instance_id;
        Err(ApiError::NotFound(format!(
            "Historic CMMN task '{plan_item_instance_id}' form was not found"
        )))
    }
    fn list_historic_task_identity_links(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<Vec<CmmnIdentityLinkRecord>, ApiError> {
        let _ = plan_item_instance_id;
        Ok(Vec::new())
    }
    fn migrate_historic_case_instance(
        &self,
        case_instance_id: &str,
        command: CmmnMigrationCommand,
    ) -> Result<(), ApiError> {
        let _ = command;
        Err(ApiError::NotFound(format!(
            "Historic CMMN case instance '{case_instance_id}' was not found"
        )))
    }
}

pub trait CmmnManagementApi: Send + Sync {
    fn list_jobs(
        &self,
        query: CmmnManagementJobQuery,
    ) -> Result<PagedResponse<CmmnManagementJobRecord>, ApiError>;
    fn get_job(
        &self,
        family: CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<CmmnManagementJobRecord, ApiError>;
    fn get_job_exception_stacktrace(
        &self,
        family: CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<String, ApiError>;
    /// Family-typed delete. REST maps Ok → 204, NotFound → 404.
    ///
    /// The default keeps existing third-party/mock implementations source
    /// compatible when this additive endpoint is not implemented by them.
    fn delete_job(&self, family: CmmnManagementJobFamily, job_id: &str) -> Result<(), ApiError> {
        let _ = family;
        Err(ApiError::NotFound(format!(
            "CMMN job '{}' was not found",
            job_id
        )))
    }

    /// Java `JobResource.executeJobAction` → `CmmnManagementService.executeJob`
    /// (JobResource.java:216-231). REST maps Ok → 204, NotFound → 404 and any
    /// handler failure → 500, matching Java's documented status set
    /// (JobResource.java:210-215).
    fn execute_job(&self, job_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN job '{}' was not found",
            job_id
        )))
    }

    /// Java `JobResource.executeTimerJobAction` 'move' action →
    /// `CmmnManagementService.moveTimerToExecutableJob` (JobResource.java:248-254).
    fn move_timer_job_to_executable(&self, job_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN timer job '{}' was not found",
            job_id
        )))
    }

    /// Java `JobResource.executeTimerJobAction` reschedule branch validates dueDate and
    /// delegates to `rescheduleTimeDateValueJob` (JobResource.java:255-264).
    fn reschedule_timer_job(&self, job_id: &str, due_date: &str) -> Result<(), ApiError> {
        let _ = due_date;
        Err(ApiError::NotFound(format!(
            "CMMN timer job '{}' was not found",
            job_id
        )))
    }

    /// Java `JobResource.executeDeadLetterJobAction` ordinary `move` selects the
    /// destination from jobType (JobResource.java:299-328).
    fn move_deadletter_job(&self, job_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN deadletter job '{}' was not found",
            job_id
        )))
    }

    /// Java `moveToHistoryJob` forces the history destination
    /// (JobResource.java:330-339).
    fn move_deadletter_job_to_history(&self, job_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN deadletter job '{}' was not found",
            job_id
        )))
    }

    /// Java `JobResource.executeHistoryJob` validates `execute`, resolves a history
    /// job and delegates to management execution (JobResource.java:274-289).
    fn execute_history_job(&self, job_id: &str) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "CMMN history job '{}' was not found",
            job_id
        )))
    }
}

#[derive(Debug, Clone)]
pub struct CmmnDeploymentCommand {
    pub name: String,
    pub tenant_id: Option<String>,
    pub resources: Vec<CmmnDeploymentResourcePayload>,
}

#[derive(Debug, Clone)]
pub struct CmmnDeploymentResourcePayload {
    pub resource_name: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnEngineInfoRecord {
    pub name: String,
    pub version: String,
    pub resource_url: Option<String>,
    pub exception: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnDeploymentRecord {
    pub id: String,
    pub name: String,
    pub deployed_at: i64,
    pub resource_names: Vec<String>,
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CmmnDeploymentQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub tenant_id: Option<String>,
    /// P133: Java DeploymentCollectionResource tenantIdLike
    pub tenant_id_like: Option<String>,
    pub without_tenant_id: bool,
    pub resource_name: Option<String>,
    /// P133: Java DeploymentCollectionResource category / categoryNotEquals
    pub category: Option<String>,
    pub category_not_equals: Option<String>,
    /// P133: Java DeploymentCollectionResource parentDeploymentId(+Like)
    pub parent_deployment_id: Option<String>,
    pub parent_deployment_id_like: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CmmnResourceDataRecord {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CmmnIdentityLinkCreateCommand {
    pub user: Option<String>,
    pub group: Option<String>,
    pub link_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CmmnIdentityLinkRecord {
    pub user: Option<String>,
    pub group: Option<String>,
    #[serde(rename = "type")]
    pub link_type: String,
}

#[derive(Debug, Clone)]
pub struct CmmnMigrationCommand {
    pub target_case_definition_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnMigrationValidationRecord {
    pub valid: bool,
    pub validation_messages: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CmmnChangePlanItemStateCommand {
    pub activate_plan_item_definition_ids: Vec<String>,
    pub move_to_available_plan_item_definition_ids: Vec<String>,
    pub terminate_plan_item_definition_ids: Vec<String>,
    pub add_waiting_for_repetition_plan_item_definition_ids: Vec<String>,
    pub remove_waiting_for_repetition_plan_item_definition_ids: Vec<String>,
    pub change_plan_item_ids: BTreeMap<String, String>,
    pub change_plan_item_ids_with_definition_id: BTreeMap<String, String>,
    pub change_plan_item_definitions_with_new_target_ids:
        Vec<PlanItemDefinitionWithTargetIdsCommand>,
}

#[derive(Debug, Clone)]
pub struct PlanItemDefinitionWithTargetIdsCommand {
    pub existing_plan_item_definition_id: String,
    pub new_plan_item_id: String,
    pub new_plan_item_definition_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct CmmnEventSubscriptionQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub event_type: Option<String>,
    pub event_name: Option<String>,
    pub activity_id: Option<String>,
    pub case_instance_id: Option<String>,
    pub case_definition_id: Option<String>,
    pub plan_item_instance_id: Option<String>,
    pub tenant_id: Option<String>,
    pub configuration: Option<String>,
    pub without_scope_id: bool,
    pub without_scope_definition_id: bool,
    pub without_tenant_id: bool,
    pub without_configuration: bool,
    /// P133: filter on CmmnEventSubscription.created_at (models.rs:3182)
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnEventSubscriptionRecord {
    pub id: String,
    pub event_type: String,
    pub event_name: Option<String>,
    pub activity_id: Option<String>,
    pub case_instance_id: Option<String>,
    pub case_definition_id: Option<String>,
    pub plan_item_instance_id: Option<String>,
    pub tenant_id: Option<String>,
    pub configuration: Option<String>,
    pub created: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnTriggerEventCommand {
    pub event_name: Option<String>,
    pub event_type: Option<String>,
    #[serde(default)]
    pub variables: Vec<CmmnVariableUpdate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmmnManagementJobFamily {
    Executable,
    Timer,
    Deadletter,
    History,
    Suspended,
}

#[derive(Debug, Clone, Default)]
pub struct CmmnManagementJobQuery {
    pub paging: PagingQuery,
    pub family: Option<CmmnManagementJobFamily>,
    pub id: Option<String>,
    /// Java maps `caseInstanceId` onto `scopeId` and forces `scopeType` to CMMN
    /// (JobCollectionResource.java:115-118); the same holds for
    /// `planItemInstanceId` → `subScopeId` (:122-125) and `scopeDefinitionId`
    /// (:129-132).
    pub scope_id: Option<String>,
    pub sub_scope_id: Option<String>,
    pub scope_definition_id: Option<String>,
    pub scope_type: Option<String>,
    pub element_id: Option<String>,
    pub without_scope_id: bool,
    pub timers_only: bool,
    pub messages_only: bool,
    pub with_exception: bool,
    pub exception_message: Option<String>,
    pub due_before: Option<DateTime<Utc>>,
    pub due_after: Option<DateTime<Utc>>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub without_tenant_id: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnManagementJobRecord {
    pub id: String,
    pub job_type: String,
    pub scope_id: Option<String>,
    pub sub_scope_id: Option<String>,
    pub scope_type: String,
    pub scope_definition_id: Option<String>,
    pub element_id: Option<String>,
    pub tenant_id: Option<String>,
    pub create_time: String,
    pub due_date: Option<String>,
    pub lock_owner: Option<String>,
    pub retries: i32,
    pub exception_message: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CaseDefinitionQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub key: Option<String>,
    /// P133: CaseDefinitionCollectionResource.java:115
    pub key_like: Option<String>,
    pub name: Option<String>,
    /// P133: CaseDefinitionCollectionResource.java:121 / 124
    pub name_like: Option<String>,
    pub name_like_ignore_case: Option<String>,
    pub deployment_id: Option<String>,
    pub version: Option<i32>,
    /// P133: CaseDefinitionCollectionResource.java:103-109
    pub category: Option<String>,
    pub category_like: Option<String>,
    pub category_not_equals: Option<String>,
    /// P133: CaseDefinitionCollectionResource.java:127-130
    pub resource_name: Option<String>,
    pub resource_name_like: Option<String>,
    /// P133: CaseDefinitionCollectionResource.java:136 latestVersion()
    pub latest: bool,
    /// P133: CaseDefinitionCollectionResource.java:150-153
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseDefinitionRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    pub deployment_id: String,
    pub resource_name: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StartCaseInstanceCommand {
    pub case_definition_id: Option<String>,
    pub case_definition_key: Option<String>,
    pub business_key: Option<String>,
    pub name: Option<String>,
    pub tenant_id: Option<String>,
    pub variables: BTreeMap<String, Value>,
    pub transient_variables: BTreeMap<String, Value>,
    pub outcome: Option<String>,
    pub override_definition_tenant_id: Option<String>,
    pub return_variables: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CaseInstanceQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub ids: Vec<String>,
    pub case_definition_id: Option<String>,
    pub case_definition_key: Option<String>,
    pub case_definition_key_like: Option<String>,
    pub case_definition_key_like_ignore_case: Option<String>,
    pub case_definition_keys: Vec<String>,
    pub exclude_case_definition_keys: Vec<String>,
    pub case_definition_name: Option<String>,
    pub case_definition_name_like: Option<String>,
    pub case_definition_name_like_ignore_case: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub name_like_ignore_case: Option<String>,
    pub business_key: Option<String>,
    pub business_key_like: Option<String>,
    pub business_key_like_ignore_case: Option<String>,
    pub business_status: Option<String>,
    pub business_status_like: Option<String>,
    pub business_status_like_ignore_case: Option<String>,
    pub started_by: Option<String>,
    /// Java runtime GET uses `referenceId/referenceType`, while POST uses
    /// `caseInstanceReferenceId/Type` (`CaseInstanceCollectionResource.java:246-252`;
    /// `CaseInstanceQueryRequest.java:68-69`).
    pub reference_id: Option<String>,
    pub reference_type: Option<String>,
    pub started_before: Option<DateTime<Utc>>,
    pub started_after: Option<DateTime<Utc>>,
    pub callback_id: Option<String>,
    pub callback_ids: Vec<String>,
    pub callback_type: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub tenant_id_like_ignore_case: Option<String>,
    pub without_tenant_id: bool,
    pub state: Option<String>,
    pub include_case_variables: bool,
    pub include_case_variables_names: Vec<String>,
    /// Java CaseInstanceQueryRequest.variables (CaseInstanceQueryRequest.java:75).
    pub variable_conditions: Vec<QueryVariableCondition>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseInstanceRecord {
    pub id: String,
    pub case_definition_id: String,
    pub case_definition_key: String,
    pub business_key: Option<String>,
    pub name: Option<String>,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_type: Option<String>,
    /// Java `CaseInstanceResponse.referenceId/referenceType`
    /// (`CaseInstanceResponse.java:51-52`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<String>,
    /// Java CaseInstanceResponse.caseDefinitionName — resolved from the deployed
    /// definition by BaseCaseInstanceResource.java:246-260.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_definition_name: Option<String>,
    /// Java CaseInstanceResponse.variables — populated only when
    /// `includeCaseVariables`/`includeCaseVariablesNames` is requested.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Value>,
    pub tenant_id: Option<String>,
    pub started_at: String,
}

// Java: CaseInstanceUpdateRequest (CaseInstanceResource.java:88 body)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnCaseInstanceUpdateCommand {
    pub action: Option<String>,
    pub name: Option<String>,
    pub business_key: Option<String>,
}

// Java: RestActionRequest (PlanItemInstanceResource.java:61 body)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanItemActionRequest {
    action: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BulkCaseInstanceActionRequest {
    action: String,
    instance_ids: Option<Vec<String>>,
}

impl BulkCaseInstanceActionRequest {
    fn require_instance_ids(self) -> Result<(String, Vec<String>), ApiError> {
        let instance_ids = self
            .instance_ids
            .ok_or_else(|| ApiError::bad_request("instanceIds is required"))?;
        Ok((self.action, instance_ids))
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlanItemInstanceQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub case_instance_id: Option<String>,
    pub case_instance_ids: Vec<String>,
    pub case_definition_id: Option<String>,
    pub stage_instance_id: Option<String>,
    pub plan_item_definition_id: Option<String>,
    pub plan_item_definition_type: Option<String>,
    pub plan_item_definition_types: Vec<String>,
    /// Java `elementId` — the plan item id (`task.plan_item_id`).
    pub element_id: Option<String>,
    /// Java `includeEnded` — the Rust task query always includes ended plan items
    /// (documented as effectively always true).
    pub include_ended: bool,
    /// Java `includeLocalVariables` — task-local variables are always empty.
    pub include_local_variables: bool,
    pub state: Option<String>,
    // P100 task query surface (Java TaskCollectionResource / TaskBaseResource).
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub name_like_ignore_case: Option<String>,
    pub assignee: Option<String>,
    pub assignee_like: Option<String>,
    pub owner: Option<String>,
    pub owner_like: Option<String>,
    pub unassigned: Option<bool>,
    /// Validated to `pending`/`resolved` at the REST layer.
    pub delegation_state: Option<String>,
    pub category: Option<String>,
    pub category_in: Vec<String>,
    pub category_not_in: Vec<String>,
    pub without_category: bool,
    pub task_definition_key: Option<String>,
    pub task_definition_key_like: Option<String>,
    pub priority: Option<i64>,
    pub min_priority: Option<i64>,
    pub max_priority: Option<i64>,
    pub created_on: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub created_after: Option<DateTime<Utc>>,
    pub due_date: Option<DateTime<Utc>>,
    pub due_before: Option<DateTime<Utc>>,
    pub due_after: Option<DateTime<Utc>>,
    pub without_due_date: bool,
    pub case_definition_key: Option<String>,
    pub case_definition_key_like: Option<String>,
    pub case_definition_key_like_ignore_case: Option<String>,
    /// Java `active` (true → not suspended, false → suspended).
    pub active: Option<bool>,
    // P114 candidate filters (Java TaskBaseResource.java:191-205, 328-330).
    pub candidate_user: Option<String>,
    pub candidate_group: Option<String>,
    pub candidate_group_in: Vec<String>,
    pub candidate_or_assigned: Option<String>,
    pub ignore_assignee: Option<bool>,
    pub scope_id: Option<String>,
    pub include_task_local_variables: bool,
    pub include_process_variables: bool,
    /// Java PlanItemInstanceQueryRequest.caseInstanceVariables
    /// (PlanItemInstanceQueryRequest.java:50) — join case-instance variables.
    pub case_instance_variable_conditions: Vec<QueryVariableCondition>,
    /// Java PlanItemInstanceQueryRequest.variables (local, :49) and/or
    /// TaskQueryRequest.taskVariables (:87). Rust has no task/plan-item local
    /// variable store, so any non-empty local condition set yields an empty
    /// result (documented empty-local convention).
    pub local_variable_conditions: Vec<QueryVariableCondition>,
    pub sort: Option<String>,
    pub order: Option<String>,
    /// P116: when set (the `/cmmn-runtime/tasks` endpoints), only human-task plan
    /// items are returned — the stage/milestone/event-listener mirror sources are
    /// skipped. `/cmmn-runtime/plan-item-instances` leaves this false.
    pub task_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItemInstanceRecord {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub plan_item_definition_id: String,
    /// Java `planItemDefinitionType` (PlanItemInstanceResponse.java:42) — the
    /// lowercased definition type (`stage` / `humantask` / `milestone` /
    /// `eventlistener`).
    pub plan_item_definition_type: String,
    /// Java `elementId` — the plan item id (PlanItemInstanceResponse.java:45).
    pub element_id: String,
    /// Java `stageInstanceId` — parent stage plan item instance id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_instance_id: Option<String>,
    /// Java `stage` (PlanItemInstanceResponse.java:41) — whether this plan item
    /// instance is a stage.
    pub stage: bool,
    pub name: String,
    pub state: String,
    /// Java `occurredTime` (PlanItemInstanceResponse.java:62) — set for occurred
    /// milestones / event listeners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_time: Option<String>,
    pub assignee: Option<String>,
    /// Java TaskResponse.owner / priority / dueDate / category — surfaced so the
    /// task update/action responses reflect the mutated fields (TaskResource.java:76-99).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Java TaskResponse.delegationState (TaskResponse.java:39,78) — only present
    /// for tasks in a delegation state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_state: Option<String>,
    /// Java TaskResponse.variables (TaskResponse.java:72) — populated only when
    /// `includeProcessVariables`/`includeTaskLocalVariables` is requested; task
    /// local variables are always empty by the documented convention.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Value>,
    pub tenant_id: Option<String>,
    pub created_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct VariableInstanceQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub case_instance_id: Option<String>,
    pub scope_id: Option<String>,
    pub variable_name: Option<String>,
    /// P133: VariableInstanceCollectionResource.java:80-81
    pub variable_name_like: Option<String>,
    /// P133: VariableInstanceCollectionResource.java:60-61 — exclude task-scoped
    pub exclude_task_variables: bool,
    /// P133: VariableInstanceCollectionResource.java:84-85 — exclude local scope
    /// when engine exposes a local flag; currently case-scoped only.
    pub exclude_local_variables: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableInstanceRecord {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub variable_type: String,
    pub value: Value,
    pub case_instance_id: String,
    pub scope_id: String,
    pub scope_type: String,
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnVariableUpdate {
    pub name: String,
    pub value: Value,
}

/// P120: engine-facing shape of the historic case instance query. Dates arrive
/// pre-parsed so the route layer owns the 400 on a malformed date, matching
/// Java's `RequestUtil.getDate` behaviour.
#[derive(Debug, Clone, Default)]
pub struct HistoricCaseInstanceQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub ids: Option<Vec<String>>,
    pub case_definition_id: Option<String>,
    pub case_definition_key: Option<String>,
    pub case_definition_key_like: Option<String>,
    pub case_definition_key_like_ignore_case: Option<String>,
    pub case_definition_category: Option<String>,
    pub case_definition_category_like: Option<String>,
    pub case_definition_category_like_ignore_case: Option<String>,
    pub case_definition_name: Option<String>,
    pub case_definition_name_like: Option<String>,
    pub case_definition_name_like_ignore_case: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub name_like_ignore_case: Option<String>,
    pub business_key: Option<String>,
    pub business_key_like: Option<String>,
    pub business_key_like_ignore_case: Option<String>,
    pub business_status: Option<String>,
    pub business_status_like: Option<String>,
    pub business_status_like_ignore_case: Option<String>,
    pub started_by: Option<String>,
    /// Java historic GET/POST reference predicates
    /// (`HistoricCaseInstanceCollectionResource.java:224-230`;
    /// `HistoricCaseInstanceQueryRequest.java:64-65`).
    pub reference_id: Option<String>,
    pub reference_type: Option<String>,
    pub started_before: Option<DateTime<Utc>>,
    pub started_after: Option<DateTime<Utc>>,
    pub finished: Option<bool>,
    pub finished_before: Option<DateTime<Utc>>,
    pub finished_after: Option<DateTime<Utc>>,
    /// Java POST `finishedBy` request field (`HistoricCaseInstanceQueryRequest.java:73`),
    /// forwarded by `HistoricCaseInstanceBaseResource.java:188-189`.
    pub finished_by: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub tenant_id_like_ignore_case: Option<String>,
    pub without_tenant_id: bool,
    pub callback_id: Option<String>,
    pub callback_ids: Option<Vec<String>>,
    pub callback_type: Option<String>,
    pub without_callback_id: bool,
    pub involved_user: Option<String>,
    pub active_plan_item_definition_id: Option<String>,
    pub include_case_variables: bool,
    pub include_case_variables_names: Vec<String>,
    pub state: Option<String>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricCaseInstanceRecord {
    pub id: String,
    pub case_definition_id: String,
    pub case_definition_key: String,
    pub business_key: Option<String>,
    pub name: Option<String>,
    pub state: String,
    pub tenant_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Java `HistoricCaseInstanceResponse.referenceId/referenceType`
    /// (`HistoricCaseInstanceResponse.java:53-54`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<String>,
    /// Java historic response exposes the finishing actor as `endUserId`
    /// (`HistoricCaseInstanceResponse.java:46`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_user_id: Option<String>,
    /// Java `HistoricCaseInstanceResponse.variables`, populated only when the
    /// historic query requests all variables or selected names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct HistoricPlanItemInstanceQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub case_instance_id: Option<String>,
    pub case_definition_id: Option<String>,
    pub plan_item_definition_id: Option<String>,
    pub state: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub name_like_ignore_case: Option<String>,
    pub task_definition_key: Option<String>,
    pub task_definition_key_like: Option<String>,
    pub assignee: Option<String>,
    pub assignee_like: Option<String>,
    pub owner: Option<String>,
    pub owner_like: Option<String>,
    pub category: Option<String>,
    pub delete_reason: Option<String>,
    pub created_before: Option<DateTime<Utc>>,
    pub created_after: Option<DateTime<Utc>>,
    pub completed_before: Option<DateTime<Utc>>,
    pub completed_after: Option<DateTime<Utc>>,
    pub finished: Option<bool>,
    pub candidate_group: Option<String>,
    pub involved_user: Option<String>,
    pub ignore_assignee: bool,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricPlanItemInstanceRecord {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub plan_item_definition_id: String,
    /// Java `planItemDefinitionType` (HistoricPlanItemInstanceResponse.java:43) —
    /// the lowercased definition type (`humantask` / `stage` / `milestone` /
    /// `eventlistener`). Human-task historic rows are always `"humantask"`.
    pub plan_item_definition_type: String,
    /// Java `elementId` (HistoricPlanItemInstanceResponse.java:41) — the plan
    /// item XML id. After P131, `planItemDefinitionId` holds the definitionRef
    /// target; clients that still need the old plan-item XML id read it here.
    pub element_id: String,
    /// Java `stageInstanceId` (HistoricPlanItemInstanceResponse.java:39) —
    /// parent stage plan item instance id when nested in a stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_instance_id: Option<String>,
    pub name: String,
    pub state: String,
    pub assignee: Option<String>,
    pub tenant_id: Option<String>,
    pub created_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HistoricMilestoneInstanceQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub case_instance_id: Option<String>,
    pub case_definition_id: Option<String>,
    pub case_definition_key: Option<String>,
    pub milestone_id: Option<String>,
    pub milestone_name: Option<String>,
    pub reached_before: Option<String>,
    pub reached_after: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricMilestoneInstanceRecord {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_definition_key: String,
    pub milestone_id: String,
    pub name: String,
    pub tenant_id: Option<String>,
    pub time: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageOverviewRecord {
    pub id: String,
    pub name: String,
    pub current: bool,
    pub ended: bool,
    pub end_time: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HistoricVariableInstanceQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub case_instance_id: Option<String>,
    pub scope_id: Option<String>,
    pub variable_name: Option<String>,
    /// P133: historic variable name like / exclude flags (same semantics as runtime)
    pub variable_name_like: Option<String>,
    pub exclude_task_variables: bool,
    pub exclude_local_variables: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricVariableInstanceRecord {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub variable_type: String,
    pub value: Value,
    pub case_instance_id: String,
    pub scope_id: String,
    pub scope_type: String,
    pub tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CmmnDeploymentRequest {
    name: String,
    #[serde(default)]
    resources: Vec<CmmnDeploymentResourceRequest>,
    tenant_id: Option<String>,
    resource_name: Option<String>,
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CmmnDeploymentResourceRequest {
    resource_name: String,
    resource: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct CmmnDeploymentQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    tenant_id: Option<String>,
    /// P133: Java DeploymentCollectionResource
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    resource_name: Option<String>,
    category: Option<String>,
    category_not_equals: Option<String>,
    parent_deployment_id: Option<String>,
    parent_deployment_id_like: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct DeleteDeploymentQueryParams {
    cascade: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct CaseDefinitionQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    key: Option<String>,
    /// P133: CaseDefinitionCollectionResource.java:115
    key_like: Option<String>,
    name: Option<String>,
    /// P133: CaseDefinitionCollectionResource.java:121 / 124
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    deployment_id: Option<String>,
    version: Option<i32>,
    /// P133: CaseDefinitionCollectionResource.java:103-109
    category: Option<String>,
    category_like: Option<String>,
    category_not_equals: Option<String>,
    /// P133: CaseDefinitionCollectionResource.java:127-130
    resource_name: Option<String>,
    resource_name_like: Option<String>,
    /// P133: CaseDefinitionCollectionResource.java:136
    latest: Option<bool>,
    /// P133: CaseDefinitionCollectionResource.java:150-153
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct LinkedDefinitionQueryParams {
    start: usize,
    size: Option<usize>,
}

impl LinkedDefinitionQueryParams {
    fn paging(self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

/// Java CaseInstanceCreateRequest (CaseInstanceCreateRequest.java:37-48): the
/// POST /cmmn-runtime/case-instances body. `startFormVariables` is cut (no form
/// engine); `fallbackToDefaultTenant` does not exist on the CMMN side.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartCaseInstanceRequest {
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    business_key: Option<String>,
    name: Option<String>,
    tenant_id: Option<String>,
    #[serde(default)]
    variables: BTreeMap<String, Value>,
    /// Java CaseInstanceCreateRequest.transientVariables
    /// (CaseInstanceCollectionResource.java:357-365).
    #[serde(default)]
    transient_variables: BTreeMap<String, Value>,
    /// Java CaseInstanceCreateRequest.outcome (CaseInstanceCollectionResource.java:399-401).
    outcome: Option<String>,
    /// Java CaseInstanceCreateRequest.overrideDefinitionTenantId
    /// (CaseInstanceCollectionResource.java:387-389).
    override_definition_tenant_id: Option<String>,
    /// Java CaseInstanceCreateRequest.returnVariables
    /// (CaseInstanceCollectionResource.java:410-416).
    #[serde(default)]
    return_variables: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CmmnMigrationRequest {
    target_case_definition_id: Option<String>,
    to_case_definition_id: Option<String>,
    case_definition_id: Option<String>,
}

/// Body of `PUT /cmmn-repository/case-definitions/{id}`. Java models this as an
/// *action* request carrying an optional category
/// (CaseDefinitionActionRequest.java:21-32); `category` is the only field the
/// CMMN handler acts on (CaseDefinitionResource.java:98-108). `action` is
/// accepted but only used to build the 400 message, matching Java.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct CaseDefinitionActionRequest {
    action: Option<String>,
    category: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct CmmnChangePlanItemStateRequest {
    activate_plan_item_definition_ids: Vec<String>,
    move_to_available_plan_item_definition_ids: Vec<String>,
    terminate_plan_item_definition_ids: Vec<String>,
    add_waiting_for_repetition_plan_item_definition_ids: Vec<String>,
    remove_waiting_for_repetition_plan_item_definition_ids: Vec<String>,
    change_plan_item_ids: BTreeMap<String, String>,
    change_plan_item_ids_with_definition_id: BTreeMap<String, String>,
    change_plan_item_definitions_with_new_target_ids: Vec<PlanItemDefinitionWithTargetIdsRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanItemDefinitionWithTargetIdsRequest {
    existing_plan_item_definition_id: String,
    new_plan_item_id: String,
    new_plan_item_definition_id: String,
}

/// GET /cmmn-runtime/case-instances query params, shared with the POST
/// /cmmn-query/case-instances body. Mirrors Java `CaseInstanceCollectionResource
/// getCaseInstances` (CaseInstanceCollectionResource.java:114-297) and
/// `BaseCaseInstanceResource.getQueryResponse` (BaseCaseInstanceResource.java:68-263).
///
/// Intentional cuts (P101 acceptance): caseDefinitionCategory (needs definition
/// join), activePlanItemDefinitionId(s) (needs plan-item join), involvedUser
/// (identity links) are rejected via `deny_unknown_fields`.
///
/// P103: POST body may carry `variables` (QueryVariable list); GET never sends
/// them so the field stays empty via `#[serde(default)]`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct CaseInstanceQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    ids: Option<Vec<String>>,
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    case_definition_key_like: Option<String>,
    case_definition_key_like_ignore_case: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    case_definition_keys: Option<Vec<String>>,
    #[serde(deserialize_with = "deserialize_string_list")]
    exclude_case_definition_keys: Option<Vec<String>>,
    case_definition_name: Option<String>,
    case_definition_name_like: Option<String>,
    case_definition_name_like_ignore_case: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    business_key: Option<String>,
    business_key_like: Option<String>,
    business_key_like_ignore_case: Option<String>,
    business_status: Option<String>,
    business_status_like: Option<String>,
    business_status_like_ignore_case: Option<String>,
    started_by: Option<String>,
    /// GET names come from `CaseInstanceCollectionResource.java:95-96,246-252`;
    /// aliases accept Java POST `CaseInstanceQueryRequest.java:68-69` names.
    #[serde(alias = "caseInstanceReferenceId")]
    reference_id: Option<String>,
    #[serde(alias = "caseInstanceReferenceType")]
    reference_type: Option<String>,
    started_before: Option<String>,
    started_after: Option<String>,
    callback_id: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    callback_ids: Option<Vec<String>>,
    callback_type: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    tenant_id_like_ignore_case: Option<String>,
    without_tenant_id: bool,
    state: Option<String>,
    include_case_variables: bool,
    #[serde(deserialize_with = "deserialize_string_list")]
    include_case_variables_names: Option<Vec<String>>,
    /// Java CaseInstanceQueryRequest.variables (CaseInstanceQueryRequest.java:75;
    /// BaseCaseInstanceResource.java:204-206).
    variables: Option<Vec<RestQueryVariable>>,
    sort: Option<String>,
    order: Option<String>,
}

/// GET /cmmn-runtime/tasks (and /cmmn-runtime/plan-item-instances) query params,
/// shared with the POST /cmmn-query/tasks body. The task-side surface mirrors
/// Java `TaskCollectionResource.getTasks` (TaskCollectionResource.java:125-349)
/// and `TaskQueryResource` (TaskQueryResource.java:50-52); P101 adds the
/// plan-item-specific params.
///
/// P114: candidateUser/candidateGroup/candidateGroups/candidateOrAssigned/
/// ignoreAssignee are supported (TaskCollectionResource.java:185-203,321-323).
/// Intentional cuts (P100 acceptance): involvedUser/involvedGroups and the
/// tenantId family (no tenant field on the human-task entity) are not supported
/// and are rejected by `deny_unknown_fields`.
///
/// P103: POST body may carry `variables` / `caseInstanceVariables` (plan-item) or
/// `taskVariables` (task). CMMN TaskQueryRequest has no processVariables filter
/// field (TaskQueryRequest.java:32-90; TaskBaseResource.java:308-310 only calls
/// addTaskvariables).
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct PlanItemInstanceQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    plan_item_instance_id: Option<String>,
    case_instance_id: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    case_instance_ids: Option<Vec<String>>,
    case_definition_id: Option<String>,
    stage_instance_id: Option<String>,
    plan_item_definition_id: Option<String>,
    plan_item_definition_type: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    plan_item_definition_types: Option<Vec<String>>,
    element_id: Option<String>,
    include_ended: bool,
    include_local_variables: bool,
    state: Option<String>,
    // Java TaskCollectionResource.java:129-139 — task name.
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    // Java TaskCollectionResource.java:161-175 — assignee / owner.
    assignee: Option<String>,
    assignee_like: Option<String>,
    owner: Option<String>,
    owner_like: Option<String>,
    // Java TaskCollectionResource.java:177-179 — present means the filter applies
    // (Java ignores the boolean value; TaskBaseResource.java:182-184).
    unassigned: Option<bool>,
    // Java TaskCollectionResource.java:181-183 — pending/resolved (validated in
    // `plan_item_query_from_params`, TaskBaseResource.java:74-86).
    delegation_state: Option<String>,
    // Java TaskCollectionResource.java:325-339 — category filters.
    category: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    category_in: Option<Vec<String>>,
    #[serde(deserialize_with = "deserialize_string_list")]
    category_not_in: Option<Vec<String>>,
    without_category: bool,
    // Java TaskCollectionResource.java:273-279 — task definition key.
    task_definition_key: Option<String>,
    task_definition_key_like: Option<String>,
    // Java TaskCollectionResource.java:149-159 — numeric priority (400 on
    // non-integer, Java `Integer.valueOf`).
    priority: Option<String>,
    minimum_priority: Option<String>,
    maximum_priority: Option<String>,
    // Java TaskCollectionResource.java:257-267 — created date-time.
    created_on: Option<String>,
    created_before: Option<String>,
    created_after: Option<String>,
    // Java TaskCollectionResource.java:281-291 + TaskBaseResource.java:264-266.
    due_date: Option<String>,
    due_before: Option<String>,
    due_after: Option<String>,
    without_due_date: bool,
    // Java TaskCollectionResource.java:205-219 — case definition / instance.
    case_definition_key: Option<String>,
    case_definition_key_like: Option<String>,
    case_definition_key_like_ignore_case: Option<String>,
    // Java TaskCollectionResource.java:293-295 → TaskBaseResource.java:268-274.
    active: Option<bool>,
    // P114 candidate filters. Java GET accepts `candidateGroups` (plural, CSV →
    // candidateGroupIn, TaskCollectionResource.java:197-199) and POST accepts
    // `candidateGroupIn` (array, TaskQueryRequest.java:49); the shared struct
    // accepts both. `ignoreAssignee` only applies when true (Java parses the
    // boolean, TaskCollectionResource.java:201-203).
    candidate_user: Option<String>,
    candidate_group: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    candidate_groups: Option<Vec<String>>,
    #[serde(deserialize_with = "deserialize_string_list")]
    candidate_group_in: Option<Vec<String>>,
    candidate_or_assigned: Option<String>,
    ignore_assignee: Option<bool>,
    // Java TaskCollectionResource.java:237-239 — scopeId maps to case instance id.
    scope_id: Option<String>,
    // Java TaskCollectionResource.java:297-303 — response assembly.
    include_task_local_variables: bool,
    include_process_variables: bool,
    /// Java PlanItemInstanceQueryRequest.variables (local plan-item vars; :49).
    variables: Option<Vec<RestQueryVariable>>,
    /// Java PlanItemInstanceQueryRequest.caseInstanceVariables (:50).
    case_instance_variables: Option<Vec<RestQueryVariable>>,
    /// Java TaskQueryRequest.taskVariables (TaskQueryRequest.java:87).
    task_variables: Option<Vec<RestQueryVariable>>,
    // Java TaskBaseResource.java:46-60 / :357 — in-memory sort/order.
    sort: Option<String>,
    order: Option<String>,
}

/// Java `QueryVariable` (cmmn-rest QueryVariable.java:26-72) — shared POST body
/// element for case `variables`, plan-item `variables`/`caseInstanceVariables`,
/// and task `taskVariables`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestQueryVariable {
    name: Option<String>,
    operation: Option<String>,
    value: Option<Value>,
    /// Accepted for parity with Java JSON; conversion is driven by the JSON
    /// value shape (QueryVariable.java:66-71).
    #[serde(rename = "type")]
    #[allow(dead_code)]
    variable_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct VariableInstanceQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    case_instance_id: Option<String>,
    scope_id: Option<String>,
    task_id: Option<String>,
    plan_item_instance_id: Option<String>,
    variable_name: Option<String>,
    name: Option<String>,
    /// P133: VariableInstanceCollectionResource.java:80-81
    variable_name_like: Option<String>,
    /// P133: VariableInstanceCollectionResource.java:60-61
    exclude_task_variables: Option<bool>,
    /// P133: VariableInstanceCollectionResource.java:84-85
    exclude_local_variables: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct VariableScopeQueryParams {
    scope: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct CmmnEventSubscriptionQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    event_type: Option<String>,
    event_name: Option<String>,
    activity_id: Option<String>,
    case_instance_id: Option<String>,
    case_definition_id: Option<String>,
    plan_item_instance_id: Option<String>,
    tenant_id: Option<String>,
    configuration: Option<String>,
    without_scope_id: Option<bool>,
    without_scope_definition_id: Option<bool>,
    without_process_instance_id: Option<bool>,
    without_process_definition_id: Option<bool>,
    without_tenant_id: Option<bool>,
    without_configuration: Option<bool>,
    /// P133: created_at filters (CmmnEventSubscription.created_at)
    created_after: Option<String>,
    created_before: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VariableRequest {
    name: Option<String>,
    #[serde(rename = "type")]
    variable_type: Option<String>,
    value: Value,
}

/// Java `TaskActionRequest` (TaskResource.java:109-137) — the subset relevant
/// to Rust: action, assignee, outcome and completion variables. `transientVariables`
/// and `formDefinitionId` are intentionally omitted (no transient/form engine
/// path for CMMN tasks in this batch).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnTaskActionRequest {
    pub action: Option<String>,
    pub assignee: Option<String>,
    pub outcome: Option<String>,
    #[serde(default)]
    pub variables: Vec<CmmnTaskVariableRequest>,
}

/// Java `RestVariable` inside a TaskActionRequest / variables collection —
/// adds the per-variable `scope` (Java defaults to LOCAL, TaskVariableCollectionResource.java:164-166).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnTaskVariableRequest {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub variable_type: Option<String>,
    pub value: Value,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityLinkRequest {
    user: Option<String>,
    group: Option<String>,
    #[serde(rename = "type")]
    link_type: Option<String>,
}

/// GET /cmmn-history/historic-case-instances query params, shared with the POST
/// /cmmn-query/historic-case-instances body. Mirrors Java
/// `HistoricCaseInstanceCollectionResource.getHistoricCaseInstances`
/// (HistoricCaseInstanceCollectionResource.java:108-300).
///
/// P120 raises the surface from 6 params to the high-frequency subset; P128 adds
/// the tail backed by reliable Rust data: callbacks, case identity links, active
/// plan items, and historic variable response enrichment.
///
/// The remaining Java parameters stay rejected by `deny_unknown_fields` because
/// Rust records no value that could implement their predicate faithfully:
/// - rootScopeId/parentScopeId: no historic entity-link hierarchy;
/// - parentCaseInstanceId/withoutCaseInstanceParentId: no historic parent/case-task link;
/// - lastReactivated*: no case-level reactivation audit fields;
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct HistoricCaseInstanceQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    /// Java uses `caseInstanceId`; `id` stays for the pre-P120 Rust surface.
    case_instance_id: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    case_instance_ids: Option<Vec<String>>,
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    case_definition_key_like: Option<String>,
    case_definition_key_like_ignore_case: Option<String>,
    case_definition_category: Option<String>,
    case_definition_category_like: Option<String>,
    case_definition_category_like_ignore_case: Option<String>,
    case_definition_name: Option<String>,
    case_definition_name_like: Option<String>,
    case_definition_name_like_ignore_case: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    business_key: Option<String>,
    business_key_like: Option<String>,
    business_key_like_ignore_case: Option<String>,
    business_status: Option<String>,
    business_status_like: Option<String>,
    business_status_like_ignore_case: Option<String>,
    started_by: Option<String>,
    /// GET names come from `HistoricCaseInstanceCollectionResource.java:85-86,224-230`;
    /// aliases accept Java POST `HistoricCaseInstanceQueryRequest.java:64-65` names.
    #[serde(alias = "caseInstanceReferenceId")]
    reference_id: Option<String>,
    #[serde(alias = "caseInstanceReferenceType")]
    reference_type: Option<String>,
    started_before: Option<String>,
    started_after: Option<String>,
    finished: Option<bool>,
    finished_before: Option<String>,
    finished_after: Option<String>,
    /// Java POST field (`HistoricCaseInstanceQueryRequest.java:73`) and query wiring
    /// (`HistoricCaseInstanceBaseResource.java:188-189`). This shared Rust parser
    /// also accepts it on GET.
    finished_by: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    tenant_id_like_ignore_case: Option<String>,
    without_tenant_id: Option<bool>,
    callback_id: Option<String>,
    #[serde(deserialize_with = "deserialize_string_list")]
    callback_ids: Option<Vec<String>>,
    callback_type: Option<String>,
    without_case_instance_callback_id: Option<bool>,
    involved_user: Option<String>,
    active_plan_item_definition_id: Option<String>,
    include_case_variables: Option<bool>,
    #[serde(deserialize_with = "deserialize_string_list")]
    include_case_variables_names: Option<Vec<String>>,
    state: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

/// GET /cmmn-history/historic-task-instances (and the historic plan-item
/// aliases) query params, shared with the POST /cmmn-query/historic-task-instances
/// body. Mirrors Java `HistoricTaskInstanceCollectionResource`
/// (HistoricTaskInstanceCollectionResource.java:97-306) and the query wiring in
/// `HistoricTaskInstanceBaseResource` (:105-280).
///
/// P120 raises the surface from 7 params to the high-frequency subset plus the
/// candidate/involved filters Java's CMMN REST actually exposes:
/// `taskCandidateGroup`, `taskInvolvedUser`, `ignoreTaskAssignee`. The engine
/// query also carries candidateUser/candidateGroupIn/involvedGroups for reuse,
/// but the CMMN REST layer never sets them, so they stay rejected here.
///
/// Params Java accepts but the Rust historic human task cannot express are still
/// rejected by `deny_unknown_fields`, and are listed in the P120 report:
/// taskDescription(Like), taskCategoryIn/NotIn/taskWithoutCategory,
/// taskDeleteReasonLike, taskPriority/Min/Max, parentTaskId, dueDate family,
/// taskCreatedOn/taskCompletedOn, includeTaskLocalVariables/includeProcessVariables,
/// tenantId family, scopeId(s)/rootScopeId/parentScopeId/withoutScopeId,
/// caseInstanceIdWithChildren, propagatedStageInstanceId, processFinished,
/// withoutProcessInstanceId.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct HistoricPlanItemInstanceQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    plan_item_instance_id: Option<String>,
    case_instance_id: Option<String>,
    plan_item_definition_id: Option<String>,
    state: Option<String>,
    /// Java `taskId` (HistoricTaskInstanceCollectionResource.java:101-103); an
    /// alias of the Rust `id` on this shared handler.
    task_id: Option<String>,
    case_definition_id: Option<String>,
    task_name: Option<String>,
    task_name_like: Option<String>,
    task_name_like_ignore_case: Option<String>,
    task_definition_key: Option<String>,
    task_definition_key_like: Option<String>,
    task_assignee: Option<String>,
    task_assignee_like: Option<String>,
    task_owner: Option<String>,
    task_owner_like: Option<String>,
    task_category: Option<String>,
    task_delete_reason: Option<String>,
    task_created_before: Option<String>,
    task_created_after: Option<String>,
    task_completed_before: Option<String>,
    task_completed_after: Option<String>,
    finished: Option<bool>,
    task_candidate_group: Option<String>,
    task_involved_user: Option<String>,
    ignore_task_assignee: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct HistoricMilestoneInstanceQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    case_instance_id: Option<String>,
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    milestone_id: Option<String>,
    milestone_name: Option<String>,
    reached_before: Option<String>,
    reached_after: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct HistoricVariableInstanceQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    case_instance_id: Option<String>,
    scope_id: Option<String>,
    task_id: Option<String>,
    plan_item_instance_id: Option<String>,
    variable_name: Option<String>,
    name: Option<String>,
    /// P133: historic variable name like / exclude flags
    variable_name_like: Option<String>,
    exclude_task_variables: Option<bool>,
    exclude_local_variables: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct CmmnManagementJobQueryParams {
    start: usize,
    size: Option<usize>,
    /// Java `JobCollectionResource.getJobs` filter set
    /// (JobCollectionResource.java:112-182). Params Java accepts but whose
    /// backing column the Rust `CmmnJob` model does not carry (`elementName`,
    /// `locked`/`unlocked`, `withoutProcessInstanceId`) are deliberately absent
    /// rather than silently ignored, so `deny_unknown_fields` rejects them.
    id: Option<String>,
    case_instance_id: Option<String>,
    plan_item_instance_id: Option<String>,
    case_definition_id: Option<String>,
    scope_definition_id: Option<String>,
    scope_type: Option<String>,
    element_id: Option<String>,
    without_scope_id: bool,
    timers_only: bool,
    messages_only: bool,
    with_exception: bool,
    exception_message: Option<String>,
    due_before: Option<String>,
    due_after: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
}

impl CmmnDeploymentRequest {
    fn into_command(self) -> Result<CmmnDeploymentCommand, ApiError> {
        let resources = if self.resources.is_empty() {
            match (self.resource_name, self.resource) {
                (Some(resource_name), Some(resource)) => {
                    vec![CmmnDeploymentResourcePayload {
                        resource_name,
                        resource,
                    }]
                }
                _ => {
                    return Err(ApiError::bad_request(
                        "CMMN deployment requires at least one resource",
                    ));
                }
            }
        } else {
            self.resources
                .into_iter()
                .map(|resource| CmmnDeploymentResourcePayload {
                    resource_name: resource.resource_name,
                    resource: resource.resource,
                })
                .collect()
        };

        Ok(CmmnDeploymentCommand {
            name: self.name,
            tenant_id: self.tenant_id,
            resources,
        })
    }
}

impl From<CmmnDeploymentQueryParams> for CmmnDeploymentQuery {
    fn from(value: CmmnDeploymentQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            name: value.name,
            name_like: value.name_like,
            tenant_id: value.tenant_id,
            tenant_id_like: value.tenant_id_like,
            without_tenant_id: value.without_tenant_id,
            resource_name: value.resource_name,
            category: value.category,
            category_not_equals: value.category_not_equals,
            parent_deployment_id: value.parent_deployment_id,
            parent_deployment_id_like: value.parent_deployment_id_like,
        }
    }
}

impl StartCaseInstanceRequest {
    fn into_command(self) -> Result<StartCaseInstanceCommand, ApiError> {
        if self.case_definition_id.is_none() && self.case_definition_key.is_none() {
            return Err(ApiError::bad_request(
                "Either caseDefinitionId or caseDefinitionKey is required",
            ));
        }
        // Java CaseInstanceCollectionResource.java:320-324 — only one of
        // caseDefinitionId or caseDefinitionKey.
        if self.case_definition_id.is_some() && self.case_definition_key.is_some() {
            return Err(ApiError::bad_request(
                "Only one of caseDefinitionId or caseDefinitionKey should be set",
            ));
        }
        // Java CaseInstanceCollectionResource.java:326-331 — tenantId only with key.
        if self.tenant_id.is_some() && self.case_definition_id.is_some() {
            return Err(ApiError::bad_request(
                "TenantId can only be used with either caseDefinitionKey",
            ));
        }

        Ok(StartCaseInstanceCommand {
            case_definition_id: self.case_definition_id,
            case_definition_key: self.case_definition_key,
            business_key: self.business_key,
            name: self.name,
            tenant_id: self.tenant_id,
            variables: self.variables,
            transient_variables: self.transient_variables,
            outcome: self.outcome,
            override_definition_tenant_id: self.override_definition_tenant_id,
            return_variables: self.return_variables,
        })
    }
}

impl CmmnMigrationRequest {
    fn into_command(self) -> Result<CmmnMigrationCommand, ApiError> {
        let target_case_definition_id = self
            .target_case_definition_id
            .or(self.to_case_definition_id)
            .or(self.case_definition_id)
            .ok_or_else(|| ApiError::bad_request("targetCaseDefinitionId is required"))?;
        Ok(CmmnMigrationCommand {
            target_case_definition_id,
        })
    }
}

impl CmmnChangePlanItemStateRequest {
    fn into_command(self) -> CmmnChangePlanItemStateCommand {
        CmmnChangePlanItemStateCommand {
            activate_plan_item_definition_ids: self.activate_plan_item_definition_ids,
            move_to_available_plan_item_definition_ids: self
                .move_to_available_plan_item_definition_ids,
            terminate_plan_item_definition_ids: self.terminate_plan_item_definition_ids,
            add_waiting_for_repetition_plan_item_definition_ids: self
                .add_waiting_for_repetition_plan_item_definition_ids,
            remove_waiting_for_repetition_plan_item_definition_ids: self
                .remove_waiting_for_repetition_plan_item_definition_ids,
            change_plan_item_ids: self.change_plan_item_ids,
            change_plan_item_ids_with_definition_id: self.change_plan_item_ids_with_definition_id,
            change_plan_item_definitions_with_new_target_ids: self
                .change_plan_item_definitions_with_new_target_ids
                .into_iter()
                .map(|item| PlanItemDefinitionWithTargetIdsCommand {
                    existing_plan_item_definition_id: item.existing_plan_item_definition_id,
                    new_plan_item_id: item.new_plan_item_id,
                    new_plan_item_definition_id: item.new_plan_item_definition_id,
                })
                .collect(),
        }
    }
}

impl From<CaseDefinitionQueryParams> for CaseDefinitionQuery {
    fn from(value: CaseDefinitionQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            key: value.key,
            key_like: value.key_like,
            name: value.name,
            name_like: value.name_like,
            name_like_ignore_case: value.name_like_ignore_case,
            deployment_id: value.deployment_id,
            version: value.version,
            category: value.category,
            category_like: value.category_like,
            category_not_equals: value.category_not_equals,
            resource_name: value.resource_name,
            resource_name_like: value.resource_name_like,
            latest: value.latest.unwrap_or(false),
            tenant_id: value.tenant_id,
            tenant_id_like: value.tenant_id_like,
        }
    }
}

impl CaseInstanceQueryParams {
    /// Build the domain query, parsing date params (Java `RequestUtil.getDate`,
    /// RequestUtil.java:76-86) and defaulting the list-style params to empty.
    fn into_query(self) -> Result<CaseInstanceQuery, ApiError> {
        let started_before =
            parse_optional_flowable_date("startedBefore", self.started_before.as_deref())?;
        let started_after =
            parse_optional_flowable_date("startedAfter", self.started_after.as_deref())?;
        Ok(CaseInstanceQuery {
            paging: PagingQuery {
                start: self.start,
                size: self.size,
            },
            id: self.id,
            ids: self.ids.unwrap_or_default(),
            case_definition_id: self.case_definition_id,
            case_definition_key: self.case_definition_key,
            case_definition_key_like: self.case_definition_key_like,
            case_definition_key_like_ignore_case: self.case_definition_key_like_ignore_case,
            case_definition_keys: self.case_definition_keys.unwrap_or_default(),
            exclude_case_definition_keys: self.exclude_case_definition_keys.unwrap_or_default(),
            case_definition_name: self.case_definition_name,
            case_definition_name_like: self.case_definition_name_like,
            case_definition_name_like_ignore_case: self.case_definition_name_like_ignore_case,
            name: self.name,
            name_like: self.name_like,
            name_like_ignore_case: self.name_like_ignore_case,
            business_key: self.business_key,
            business_key_like: self.business_key_like,
            business_key_like_ignore_case: self.business_key_like_ignore_case,
            business_status: self.business_status,
            business_status_like: self.business_status_like,
            business_status_like_ignore_case: self.business_status_like_ignore_case,
            started_by: self.started_by,
            reference_id: self.reference_id,
            reference_type: self.reference_type,
            started_before,
            started_after,
            callback_id: self.callback_id,
            callback_ids: self.callback_ids.unwrap_or_default(),
            callback_type: self.callback_type,
            tenant_id: self.tenant_id,
            tenant_id_like: self.tenant_id_like,
            tenant_id_like_ignore_case: self.tenant_id_like_ignore_case,
            without_tenant_id: self.without_tenant_id,
            state: self.state,
            include_case_variables: self.include_case_variables,
            include_case_variables_names: self.include_case_variables_names.unwrap_or_default(),
            // Java BaseCaseInstanceResource.java:204-206 + addVariables (:292-376).
            variable_conditions: parse_query_variables(self.variables.as_deref().unwrap_or(&[]))?,
            sort: self.sort,
            order: self.order,
        })
    }
}

impl From<VariableInstanceQueryParams> for VariableInstanceQuery {
    fn from(value: VariableInstanceQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            case_instance_id: value.case_instance_id,
            scope_id: value.scope_id,
            variable_name: value.variable_name.or(value.name),
            variable_name_like: value.variable_name_like,
            exclude_task_variables: value.exclude_task_variables.unwrap_or(false),
            exclude_local_variables: value.exclude_local_variables.unwrap_or(false),
        }
    }
}

impl CmmnEventSubscriptionQueryParams {
    /// P133: parse createdAfter/createdBefore (Java RequestUtil.getDate).
    fn into_query(self) -> Result<CmmnEventSubscriptionQuery, ApiError> {
        Ok(CmmnEventSubscriptionQuery {
            paging: PagingQuery {
                start: self.start,
                size: self.size,
            },
            id: self.id,
            event_type: self.event_type,
            event_name: self.event_name,
            activity_id: self.activity_id,
            case_instance_id: self.case_instance_id,
            case_definition_id: self.case_definition_id,
            plan_item_instance_id: self.plan_item_instance_id,
            tenant_id: self.tenant_id,
            configuration: self.configuration,
            without_scope_id: self.without_scope_id.unwrap_or(false)
                || self.without_process_instance_id.unwrap_or(false),
            without_scope_definition_id: self.without_scope_definition_id.unwrap_or(false)
                || self.without_process_definition_id.unwrap_or(false),
            without_tenant_id: self.without_tenant_id.unwrap_or(false),
            without_configuration: self.without_configuration.unwrap_or(false),
            created_after: parse_optional_flowable_date(
                "createdAfter",
                self.created_after.as_deref(),
            )?,
            created_before: parse_optional_flowable_date(
                "createdBefore",
                self.created_before.as_deref(),
            )?,
        })
    }
}



/// Java `HistoricCaseInstanceCollectionResource.getHistoricCaseInstances`
/// (HistoricCaseInstanceCollectionResource.java:108-300). Dates go through
/// `RequestUtil.getDate`, which throws `FlowableIllegalArgumentException` → 400
/// on a malformed value.
fn historic_case_instance_query_from_params(
    query: HistoricCaseInstanceQueryParams,
) -> Result<HistoricCaseInstanceQuery, ApiError> {
    let id = single_alias_value(query.id, "id", query.case_instance_id, "caseInstanceId")?;
    Ok(HistoricCaseInstanceQuery {
        paging: PagingQuery {
            start: query.start,
            size: query.size,
        },
        id,
        ids: query.case_instance_ids,
        case_definition_id: query.case_definition_id,
        case_definition_key: query.case_definition_key,
        case_definition_key_like: query.case_definition_key_like,
        case_definition_key_like_ignore_case: query.case_definition_key_like_ignore_case,
        case_definition_category: query.case_definition_category,
        case_definition_category_like: query.case_definition_category_like,
        case_definition_category_like_ignore_case: query.case_definition_category_like_ignore_case,
        case_definition_name: query.case_definition_name,
        case_definition_name_like: query.case_definition_name_like,
        case_definition_name_like_ignore_case: query.case_definition_name_like_ignore_case,
        name: query.name,
        name_like: query.name_like,
        name_like_ignore_case: query.name_like_ignore_case,
        business_key: query.business_key,
        business_key_like: query.business_key_like,
        business_key_like_ignore_case: query.business_key_like_ignore_case,
        business_status: query.business_status,
        business_status_like: query.business_status_like,
        business_status_like_ignore_case: query.business_status_like_ignore_case,
        started_by: query.started_by,
        reference_id: query.reference_id,
        reference_type: query.reference_type,
        started_before: parse_optional_flowable_date(
            "startedBefore",
            query.started_before.as_deref(),
        )?,
        started_after: parse_optional_flowable_date("startedAfter", query.started_after.as_deref())?,
        finished: query.finished,
        finished_before: parse_optional_flowable_date(
            "finishedBefore",
            query.finished_before.as_deref(),
        )?,
        finished_after: parse_optional_flowable_date(
            "finishedAfter",
            query.finished_after.as_deref(),
        )?,
        finished_by: query.finished_by,
        tenant_id: query.tenant_id,
        tenant_id_like: query.tenant_id_like,
        tenant_id_like_ignore_case: query.tenant_id_like_ignore_case,
        without_tenant_id: query.without_tenant_id.unwrap_or(false),
        callback_id: query.callback_id,
        callback_ids: query.callback_ids,
        callback_type: query.callback_type,
        without_callback_id: query
            .without_case_instance_callback_id
            .unwrap_or(false),
        involved_user: query.involved_user,
        active_plan_item_definition_id: query.active_plan_item_definition_id,
        include_case_variables: query.include_case_variables.unwrap_or(false),
        include_case_variables_names: query.include_case_variables_names.unwrap_or_default(),
        state: query.state,
        sort: query.sort,
        order: query.order,
    })
}

impl From<HistoricMilestoneInstanceQueryParams> for HistoricMilestoneInstanceQuery {
    fn from(value: HistoricMilestoneInstanceQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            case_instance_id: value.case_instance_id,
            case_definition_id: value.case_definition_id,
            case_definition_key: value.case_definition_key,
            milestone_id: value.milestone_id,
            milestone_name: value.milestone_name,
            reached_before: value.reached_before,
            reached_after: value.reached_after,
        }
    }
}

impl From<HistoricVariableInstanceQueryParams> for HistoricVariableInstanceQuery {
    fn from(value: HistoricVariableInstanceQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            case_instance_id: value.case_instance_id,
            scope_id: value.scope_id,
            variable_name: value.variable_name.or(value.name),
            variable_name_like: value.variable_name_like,
            exclude_task_variables: value.exclude_task_variables.unwrap_or(false),
            exclude_local_variables: value.exclude_local_variables.unwrap_or(false),
        }
    }
}

impl CmmnManagementJobQueryParams {
    fn into_query(
        self,
        family: CmmnManagementJobFamily,
    ) -> Result<CmmnManagementJobQuery, ApiError> {
        // Java rejects the combination up front, before any filter is applied
        // (JobCollectionResource.java:139-146).
        if self.timers_only && self.messages_only {
            return Err(ApiError::bad_request(
                "Only one of 'timersOnly' or 'messagesOnly' can be provided.".to_string(),
            ));
        }

        // caseInstanceId/planItemInstanceId/scopeDefinitionId each force scopeType
        // to CMMN in Java; an explicit scopeType param still wins because Java
        // applies it last (JobCollectionResource.java:177-179).
        let forces_cmmn_scope = self.case_instance_id.is_some()
            || self.plan_item_instance_id.is_some()
            || self.scope_definition_id.is_some();
        let scope_type = self.scope_type.or_else(|| {
            forces_cmmn_scope.then(|| CMMN_SCOPE_TYPE.to_string())
        });

        Ok(CmmnManagementJobQuery {
            paging: PagingQuery {
                start: self.start,
                size: self.size,
            },
            family: Some(family),
            id: self.id,
            scope_id: self.case_instance_id,
            sub_scope_id: self.plan_item_instance_id,
            // Java exposes this as both caseDefinitionId and scopeDefinitionId
            // (JobCollectionResource.java:126-132); both land on the same column.
            scope_definition_id: self.scope_definition_id.or(self.case_definition_id),
            scope_type,
            element_id: self.element_id,
            without_scope_id: self.without_scope_id,
            timers_only: self.timers_only,
            messages_only: self.messages_only,
            with_exception: self.with_exception,
            exception_message: self.exception_message,
            due_before: parse_optional_flowable_date("dueBefore", self.due_before.as_deref())?,
            due_after: parse_optional_flowable_date("dueAfter", self.due_after.as_deref())?,
            tenant_id: self.tenant_id,
            tenant_id_like: self.tenant_id_like,
            without_tenant_id: self.without_tenant_id,
        })
    }
}

struct EmptyCmmnManagementApi;

impl CmmnManagementApi for EmptyCmmnManagementApi {
    fn list_jobs(
        &self,
        query: CmmnManagementJobQuery,
    ) -> Result<PagedResponse<CmmnManagementJobRecord>, ApiError> {
        Ok(query.paging.paginate(Vec::new()))
    }

    fn get_job(
        &self,
        family: CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<CmmnManagementJobRecord, ApiError> {
        let _ = family;
        Err(ApiError::NotFound(format!(
            "CMMN job '{job_id}' was not found"
        )))
    }

    fn get_job_exception_stacktrace(
        &self,
        family: CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<String, ApiError> {
        let _ = family;
        Err(ApiError::NotFound(format!(
            "CMMN job '{job_id}' exception stacktrace was not found"
        )))
    }

    fn delete_job(&self, family: CmmnManagementJobFamily, job_id: &str) -> Result<(), ApiError> {
        let _ = family;
        Err(ApiError::NotFound(format!(
            "CMMN job '{job_id}' was not found"
        )))
    }
}

pub fn router(
    repository: DynCmmnRepository,
    runtime: DynCmmnRuntime,
    history: DynCmmnHistory,
) -> Router {
    router_with_management(
        repository,
        runtime,
        history,
        Arc::new(EmptyCmmnManagementApi),
    )
}

pub fn router_with_management(
    repository: DynCmmnRepository,
    runtime: DynCmmnRuntime,
    history: DynCmmnHistory,
    management: DynCmmnManagement,
) -> Router {
    Router::new()
        .route("/cmmn-management/engine", get(get_engine_info))
        .route("/cmmn-management/jobs", get(list_jobs))
        .route(
            "/cmmn-management/jobs/:job_id",
            get(get_job).post(execute_job_action).delete(delete_job),
        )
        .route(
            "/cmmn-management/jobs/:job_id/exception-stacktrace",
            get(get_job_exception_stacktrace),
        )
        .route("/cmmn-management/timer-jobs", get(list_timer_jobs))
        .route(
            "/cmmn-management/timer-jobs/:job_id",
            get(get_timer_job)
                .post(execute_timer_job_action)
                .delete(delete_timer_job),
        )
        .route(
            "/cmmn-management/timer-jobs/:job_id/exception-stacktrace",
            get(get_timer_job_exception_stacktrace),
        )
        .route(
            "/cmmn-management/deadletter-jobs",
            get(list_deadletter_jobs),
        )
        .route(
            "/cmmn-management/deadletter-jobs/:job_id",
            get(get_deadletter_job)
                .post(execute_deadletter_job_action)
                .delete(delete_deadletter_job),
        )
        .route(
            "/cmmn-management/deadletter-jobs/:job_id/exception-stacktrace",
            get(get_deadletter_job_exception_stacktrace),
        )
        .route("/cmmn-management/history-jobs", get(list_history_jobs))
        .route(
            "/cmmn-management/history-jobs/:job_id",
            get(get_history_job).post(execute_history_job_action),
        )
        .route(
            "/cmmn-management/suspended-jobs",
            get(list_suspended_jobs),
        )
        .route(
            "/cmmn-management/suspended-jobs/:job_id",
            get(get_suspended_job).delete(delete_suspended_job),
        )
        .route(
            "/cmmn-management/suspended-jobs/:job_id/exception-stacktrace",
            get(get_suspended_job_exception_stacktrace),
        )
        .route(
            "/cmmn-repository/deployments",
            get(list_deployments).post(deploy),
        )
        .route(
            "/cmmn-repository/deployments/:deployment_id",
            get(get_deployment).delete(delete_deployment),
        )
        .route(
            "/cmmn-repository/deployments/:deployment_id/resources",
            get(list_deployment_resources),
        )
        .route(
            "/cmmn-repository/deployments/:deployment_id/resourcedata/*resource_name",
            get(get_deployment_resource_data),
        )
        .route(
            "/cmmn-repository/deployments/:deployment_id/resources/*resource_name",
            get(get_deployment_resource),
        )
        .route(
            "/cmmn-repository/case-definitions",
            get(list_case_definitions),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/resourcedata",
            get(get_case_definition_resource_data),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/model",
            get(get_case_definition_model),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/decision-tables",
            get(list_case_definition_decision_tables),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/decisions",
            get(list_case_definition_decisions),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/form-definitions",
            get(list_case_definition_form_definitions),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/start-form",
            get(get_case_definition_start_form),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/identitylinks",
            get(list_case_definition_identity_links).post(create_case_definition_identity_link),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/identitylinks/:family/:identity_id",
            delete(delete_case_definition_identity_links),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/migrate",
            post(migrate_case_definition_instances),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/batch-migrate",
            post(batch_migrate_case_definition_instances),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/migrate-historic-instances",
            post(migrate_historic_case_definition_instances),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id/batch-migrate-historic-instances",
            post(batch_migrate_historic_case_definition_instances),
        )
        .route(
            "/cmmn-repository/case-definitions/:case_definition_id",
            get(get_case_definition).put(execute_case_definition_action),
        )
        .route(
            "/cmmn-runtime/case-instances",
            post(start_case_instance).get(list_case_instances),
        )
        .route(
            "/cmmn-runtime/case-instances/delete",
            post(bulk_delete_case_instances),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id",
            // Java: CaseInstanceResource.java:88 PUT updateCaseInstance
            get(get_case_instance)
                .put(update_case_instance)
                .delete(terminate_case_instance),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/delete",
            delete(delete_case_instance),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/stage-overview",
            get(get_stage_overview),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/validate-migration",
            post(validate_case_instance_migration),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/migrate",
            post(migrate_case_instance),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/change-state",
            post(change_plan_item_state),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/identitylinks",
            get(list_case_instance_identity_links).post(create_case_instance_identity_link),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/identitylinks/users/:identity_id/:link_type",
            get(get_case_instance_identity_link).delete(delete_case_instance_identity_link),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/variables",
            // Java: CaseInstanceVariableCollectionResource.java:53/83/141/180
            get(list_case_instance_variables)
                .post(create_case_instance_variables)
                .put(update_case_instance_variables)
                .delete(delete_case_instance_variables),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/variables/:variable_name",
            // Java: CaseInstanceVariableResource.java:88/176
            get(get_case_instance_variable)
                .put(update_case_instance_variable)
                .delete(delete_case_instance_variable),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/variables/:variable_name/data",
            get(get_case_instance_variable_data).put(update_case_instance_variable_data),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/variables-async",
            post(create_case_instance_variables_async).put(update_case_instance_variables_async),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/variables-async/:variable_name",
            put(update_case_instance_variable_async),
        )
        .route(
            "/cmmn-runtime/case-instances/:case_instance_id/events",
            post(trigger_case_event),
        )
        .route("/cmmn-runtime/tasks", get(list_tasks))
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id",
            // Java: TaskResource.java:76/109/149 — GET/PUT update/POST action/DELETE (403)
            get(get_plan_item_instance)
                .put(update_task)
                .post(execute_task_action)
                .delete(delete_task),
        )
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id/subtasks",
            get(list_task_subtasks),
        )
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id/form",
            get(get_task_form),
        )
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id/identitylinks",
            get(list_task_identity_links).post(create_task_identity_link),
        )
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id/identitylinks/:family",
            get(list_task_identity_links_by_family),
        )
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id/identitylinks/:family/:identity_id/:link_type",
            get(get_task_identity_link).delete(delete_task_identity_link),
        )
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id/variables",
            // Java: TaskVariableCollectionResource.java:69/122/219
            get(list_task_variables)
                .post(create_task_variables)
                .delete(delete_task_variables),
        )
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id/variables/:variable_name",
            // Java: TaskVariableResource.java:66/94/138
            get(get_task_variable)
                .put(update_task_variable)
                .delete(delete_task_variable),
        )
        .route(
            "/cmmn-runtime/tasks/:plan_item_instance_id/variables/:variable_name/data",
            get(get_task_variable_data).put(update_task_variable_data),
        )
        .route("/cmmn-query/tasks", post(query_tasks))
        .route("/cmmn-query/case-instances", post(query_case_instances))
        .route(
            "/cmmn-runtime/plan-item-instances",
            get(list_plan_item_instances),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id",
            // Java: PlanItemInstanceResource.java:59 PUT performPlanItemInstanceAction
            get(get_plan_item_instance).put(perform_plan_item_instance_action),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables",
            get(list_plan_item_variables),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables/:variable_name",
            get(get_plan_item_variable),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables/:variable_name/data",
            get(get_plan_item_variable_data).put(update_plan_item_variable_data),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables-async",
            post(create_plan_item_variables_async).put(update_plan_item_variables_async),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables-async/:variable_name",
            put(update_plan_item_variable_async),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/complete",
            post(complete_plan_item_instance),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/reactivate",
            post(reactivate_plan_item_instance),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/disable",
            post(disable_plan_item_instance),
        )
        .route(
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/enable",
            post(enable_plan_item_instance),
        )
        .route(
            "/cmmn-runtime/variable-instances",
            get(list_variable_instances),
        )
        .route(
            "/cmmn-runtime/variable-instances/:variable_instance_id",
            get(get_variable_instance),
        )
        .route(
            "/cmmn-runtime/variable-instances/:variable_instance_id/data",
            get(get_variable_instance_data),
        )
        .route(
            "/cmmn-runtime/event-subscriptions",
            get(list_event_subscriptions),
        )
        .route(
            "/cmmn-runtime/event-subscriptions/:event_subscription_id",
            get(get_event_subscription),
        )
        .route(
            "/cmmn-query/plan-item-instances",
            post(query_plan_item_instances),
        )
        .route(
            "/cmmn-query/variable-instances",
            post(query_variable_instances),
        )
        .route(
            "/cmmn-history/historic-case-instances",
            get(list_historic_case_instances),
        )
        .route(
            "/cmmn-history/historic-case-instances/delete",
            post(bulk_delete_historic_case_instances),
        )
        .route(
            "/cmmn-history/historic-case-instances/:case_instance_id",
            get(get_historic_case_instance).delete(delete_historic_case_instance),
        )
        .route(
            "/cmmn-history/historic-case-instances/:case_instance_id/stage-overview",
            get(get_historic_stage_overview),
        )
        .route(
            "/cmmn-history/historic-case-instances/:case_instance_id/migrate",
            post(migrate_historic_case_instance),
        )
        .route(
            "/cmmn-history/historic-case-instances/:case_instance_id/identitylinks",
            get(list_historic_case_instance_identity_links),
        )
        .route(
            "/cmmn-history/historic-case-instances/:case_instance_id/variables/:variable_name/data",
            get(get_historic_case_instance_variable_data),
        )
        .route(
            "/cmmn-query/historic-case-instances",
            post(query_historic_case_instances),
        )
        .route(
            "/cmmn-history/historic-task-instances",
            get(list_historic_plan_item_instances),
        )
        .route(
            "/cmmn-history/historic-task-instances/:plan_item_instance_id",
            get(get_historic_plan_item_instance),
        )
        .route(
            "/cmmn-history/historic-task-instances/:plan_item_instance_id/form",
            get(get_historic_task_form),
        )
        .route(
            "/cmmn-history/historic-task-instances/:plan_item_instance_id/identitylinks",
            get(list_historic_task_identity_links),
        )
        .route(
            "/cmmn-history/historic-task-instances/:plan_item_instance_id/variables/:variable_name/data",
            get(get_historic_task_variable_data),
        )
        .route(
            "/cmmn-query/historic-task-instances",
            post(query_historic_plan_item_instances),
        )
        .route(
            "/cmmn-history/historic-milestone-instances",
            get(list_historic_milestone_instances),
        )
        .route(
            "/cmmn-history/historic-milestone-instances/:milestone_instance_id",
            get(get_historic_milestone_instance),
        )
        .route(
            "/cmmn-query/historic-milestone-instances",
            post(query_historic_milestone_instances),
        )
        .route(
            "/cmmn-history/historic-variable-instances",
            get(list_historic_variable_instances),
        )
        .route(
            "/cmmn-history/historic-variable-instances/:variable_instance_id/data",
            get(get_historic_variable_instance_data),
        )
        .route(
            "/cmmn-query/historic-variable-instances",
            post(query_historic_variable_instances),
        )
        .route(
            "/cmmn-history/historic-plan-item-instances",
            get(list_historic_plan_item_instances),
        )
        .route(
            "/cmmn-history/historic-plan-item-instances/:plan_item_instance_id",
            get(get_historic_plan_item_instance),
        )
        .route(
            "/cmmn-history/historic-planitem-instances",
            get(list_historic_plan_item_instances),
        )
        .route(
            "/cmmn-history/historic-planitem-instances/:plan_item_instance_id",
            get(get_historic_plan_item_instance),
        )
        .route(
            "/cmmn-query/historic-planitem-instances",
            post(query_historic_plan_item_instances),
        )
        .layer(Extension(repository))
        .layer(Extension(runtime))
        .layer(Extension(history))
        .layer(Extension(management))
}

pub async fn get_engine_info(
    Extension(repository): Extension<DynCmmnRepository>,
) -> Result<Json<CmmnEngineInfoRecord>, ApiError> {
    Ok(Json(repository.get_engine_info()?))
}

pub async fn list_jobs(
    Extension(management): Extension<DynCmmnManagement>,
    uri: Uri,
) -> Result<Json<PagedResponse<CmmnManagementJobRecord>>, ApiError> {
    Ok(Json(list_cmmn_jobs(
        management.as_ref(),
        CmmnManagementJobFamily::Executable,
        uri,
    )?))
}

pub async fn get_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Json<CmmnManagementJobRecord>, ApiError> {
    Ok(Json(
        management.get_job(CmmnManagementJobFamily::Executable, &job_id)?,
    ))
}

pub async fn get_job_exception_stacktrace(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    stacktrace_response(
        management.as_ref(),
        CmmnManagementJobFamily::Executable,
        &job_id,
    )
}

pub async fn list_timer_jobs(
    Extension(management): Extension<DynCmmnManagement>,
    uri: Uri,
) -> Result<Json<PagedResponse<CmmnManagementJobRecord>>, ApiError> {
    Ok(Json(list_cmmn_jobs(
        management.as_ref(),
        CmmnManagementJobFamily::Timer,
        uri,
    )?))
}

pub async fn get_timer_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Json<CmmnManagementJobRecord>, ApiError> {
    Ok(Json(
        management.get_job(CmmnManagementJobFamily::Timer, &job_id)?,
    ))
}

pub async fn get_timer_job_exception_stacktrace(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    stacktrace_response(management.as_ref(), CmmnManagementJobFamily::Timer, &job_id)
}

pub async fn list_deadletter_jobs(
    Extension(management): Extension<DynCmmnManagement>,
    uri: Uri,
) -> Result<Json<PagedResponse<CmmnManagementJobRecord>>, ApiError> {
    Ok(Json(list_cmmn_jobs(
        management.as_ref(),
        CmmnManagementJobFamily::Deadletter,
        uri,
    )?))
}

pub async fn get_deadletter_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Json<CmmnManagementJobRecord>, ApiError> {
    Ok(Json(
        management.get_job(CmmnManagementJobFamily::Deadletter, &job_id)?,
    ))
}

pub async fn get_deadletter_job_exception_stacktrace(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    stacktrace_response(
        management.as_ref(),
        CmmnManagementJobFamily::Deadletter,
        &job_id,
    )
}

pub async fn list_history_jobs(
    Extension(management): Extension<DynCmmnManagement>,
    uri: Uri,
) -> Result<Json<PagedResponse<CmmnManagementJobRecord>>, ApiError> {
    Ok(Json(list_cmmn_jobs(
        management.as_ref(),
        CmmnManagementJobFamily::History,
        uri,
    )?))
}

pub async fn get_history_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Json<CmmnManagementJobRecord>, ApiError> {
    Ok(Json(
        management.get_job(CmmnManagementJobFamily::History, &job_id)?,
    ))
}

/// Java `JobResource.executeHistoryJob` (JobResource.java:274-289): validate the
/// `execute` action before resolving the history row, execute it, return 204.
pub async fn execute_history_job_action(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: CmmnJobActionRequest = parse_optional_action_body(&body)?;
    if request.action.as_deref() != Some("execute") {
        return Err(ApiError::bad_request(
            "Invalid action, only 'execute' is supported.".to_string(),
        ));
    }
    management.execute_history_job(&job_id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_suspended_jobs(
    Extension(management): Extension<DynCmmnManagement>,
    uri: Uri,
) -> Result<Json<PagedResponse<CmmnManagementJobRecord>>, ApiError> {
    Ok(Json(list_cmmn_jobs(
        management.as_ref(),
        CmmnManagementJobFamily::Suspended,
        uri,
    )?))
}

pub async fn get_suspended_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Json<CmmnManagementJobRecord>, ApiError> {
    Ok(Json(
        management.get_job(CmmnManagementJobFamily::Suspended, &job_id)?,
    ))
}

pub async fn delete_suspended_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    management.delete_job(CmmnManagementJobFamily::Suspended, &job_id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `JobResource.deleteJob` (JobResource.java:126-136): resolve through
/// `getJobById` (404 when absent, JobBaseResource.java:34-72), delete, 204.
pub async fn delete_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    management.delete_job(CmmnManagementJobFamily::Executable, &job_id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `JobResource.deleteTimerJob` (JobResource.java:143-153).
pub async fn delete_timer_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    management.delete_job(CmmnManagementJobFamily::Timer, &job_id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `JobResource.deleteDeadLetterJob` (JobResource.java:198-208).
pub async fn delete_deadletter_job(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    management.delete_job(CmmnManagementJobFamily::Deadletter, &job_id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `JobResource.executeDeadLetterJobAction` (JobResource.java:291-343): only
/// `move` and `moveToHistoryJob` are legal; a valid action resolves/moves the row and
/// returns 204, while an unknown id remains a family-typed 404.
pub async fn execute_deadletter_job_action(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: CmmnJobActionRequest = parse_optional_action_body(&body)?;
    match request.action.as_deref() {
        Some("move") => management.move_deadletter_job(&job_id)?,
        Some("moveToHistoryJob") => management.move_deadletter_job_to_history(&job_id)?,
        _ => {
            return Err(ApiError::bad_request(
                "Invalid action, only 'move' or 'moveToHistoryJob' is supported.".to_string(),
            ));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Java `RestActionRequest` (RestActionRequest.java:21-37) — a bare `action` string.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct CmmnJobActionRequest {
    pub action: Option<String>,
}

/// Java `TimerJobActionRequest` (TimerJobActionRequest.java:17-27) extends
/// `RestActionRequest` with the reschedule due date.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct CmmnTimerJobActionRequest {
    pub action: Option<String>,
    pub due_date: Option<String>,
}

/// Java `JobResource.executeJobAction` (JobResource.java:216-231): a null body or any
/// action other than `execute` is a 400 with this exact message; the job is resolved
/// afterwards, so an unknown id on a valid action is a 404.
pub async fn execute_job_action(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: CmmnJobActionRequest = parse_optional_action_body(&body)?;
    if request.action.as_deref() != Some("execute") {
        return Err(ApiError::bad_request(
            "Invalid action, only 'execute' is supported.".to_string(),
        ));
    }
    management.execute_job(&job_id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Java `JobResource.executeTimerJobAction` (JobResource.java:239-266) accepts `move` and
/// `reschedule`, validates reschedule's dueDate before resolving the job, and returns 204.
pub async fn execute_timer_job_action(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: CmmnTimerJobActionRequest = parse_optional_action_body(&body)?;
    match request.action.as_deref() {
        Some("move") => {
            management.move_timer_job_to_executable(&job_id)?;
            Ok(StatusCode::NO_CONTENT)
        }
        Some("reschedule") => {
            // Java JobResource.java:255-260 validates the due date before touching the job,
            // then passes the original date-value string to the management service.
            let Some(due_date) = request.due_date.as_deref() else {
                return Err(ApiError::bad_request(
                    "Invalid reschedule timer action. Reschedule timer actions must have a valid due date"
                        .to_string(),
                ));
            };
            management.reschedule_timer_job(&job_id, due_date)?;
            Ok(StatusCode::NO_CONTENT)
        }
        _ => Err(ApiError::bad_request(
            "Invalid action, only 'move' or 'reschedule' are supported.".to_string(),
        )),
    }
}

/// Java binds `@RequestBody` action requests such that an absent/empty body arrives as
/// `null` and is rejected by the action check rather than by deserialization
/// (JobResource.java:225-227).
fn parse_optional_action_body<T: Default + serde::de::DeserializeOwned>(
    body: &str,
) -> Result<T, ApiError> {
    if body.trim().is_empty() {
        return Ok(T::default());
    }
    serde_json::from_str(body).map_err(|err| ApiError::bad_request(err.to_string()))
}

pub async fn get_suspended_job_exception_stacktrace(
    Extension(management): Extension<DynCmmnManagement>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    stacktrace_response(
        management.as_ref(),
        CmmnManagementJobFamily::Suspended,
        &job_id,
    )
}

fn list_cmmn_jobs(
    management: &dyn CmmnManagementApi,
    family: CmmnManagementJobFamily,
    uri: Uri,
) -> Result<PagedResponse<CmmnManagementJobRecord>, ApiError> {
    let query: CmmnManagementJobQueryParams = parse_query(&uri)?;
    management.list_jobs(query.into_query(family)?)
}

fn stacktrace_response(
    management: &dyn CmmnManagementApi,
    family: CmmnManagementJobFamily,
    job_id: &str,
) -> Result<Response, ApiError> {
    let stacktrace = management.get_job_exception_stacktrace(family, job_id)?;
    Ok(([(header::CONTENT_TYPE, "text/plain")], stacktrace).into_response())
}

pub async fn deploy(
    Extension(repository): Extension<DynCmmnRepository>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let payload: CmmnDeploymentRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let deployment = repository.deploy_case_definitions(payload.into_command()?)?;
    Ok((StatusCode::CREATED, Json(deployment)))
}

pub async fn list_deployments(
    Extension(repository): Extension<DynCmmnRepository>,
    uri: Uri,
) -> Result<Json<PagedResponse<CmmnDeploymentRecord>>, ApiError> {
    let query: CmmnDeploymentQueryParams = parse_query(&uri)?;
    Ok(Json(repository.list_deployments(query.into())?))
}

pub async fn get_deployment(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(deployment_id): Path<String>,
) -> Result<Json<CmmnDeploymentRecord>, ApiError> {
    Ok(Json(repository.get_deployment(&deployment_id)?))
}

pub async fn delete_deployment(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(deployment_id): Path<String>,
    uri: Uri,
) -> Result<StatusCode, ApiError> {
    let query: DeleteDeploymentQueryParams = parse_query(&uri)?;
    repository.delete_deployment(&deployment_id, query.cascade)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_deployment_resources(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(deployment_id): Path<String>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let deployment = repository.get_deployment(&deployment_id)?;
    let resources = deployment
        .resource_names
        .iter()
        .map(|resource_name| resource_response(&deployment_id, resource_name))
        .collect();
    Ok(Json(resources))
}

pub async fn get_deployment_resource(
    Extension(repository): Extension<DynCmmnRepository>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    repository.get_deployment_resource_data(&deployment_id, &resource_name)?;
    Ok(Json(resource_response(&deployment_id, &resource_name)))
}

pub async fn get_deployment_resource_data(
    Extension(repository): Extension<DynCmmnRepository>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(resource_data_response(
        repository.get_deployment_resource_data(&deployment_id, &resource_name)?,
    ))
}

pub async fn list_case_definitions(
    Extension(repository): Extension<DynCmmnRepository>,
    uri: Uri,
) -> Result<Json<PagedResponse<CaseDefinitionRecord>>, ApiError> {
    let query: CaseDefinitionQueryParams = parse_query(&uri)?;
    Ok(Json(repository.list_case_definitions(query.into())?))
}

pub async fn get_case_definition(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
) -> Result<Json<CaseDefinitionRecord>, ApiError> {
    Ok(Json(repository.get_case_definition(&case_definition_id)?))
}

/// PUT /cmmn-repository/case-definitions/{id} — Java
/// `CaseDefinitionResource.executeCaseDefinitionAction`
/// (CaseDefinitionResource.java:87-109).
///
/// Ordering is load-then-dispatch: the definition is fetched first, so a missing
/// definition is 404 even when the body carries no category
/// (CaseDefinitionResource.java:96). A present category updates and returns the
/// definition; anything else is 400 naming the action
/// (CaseDefinitionResource.java:108).
pub async fn execute_case_definition_action(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    body: String,
) -> Result<Json<CaseDefinitionRecord>, ApiError> {
    let request: CaseDefinitionActionRequest = if body.trim().is_empty() {
        // Java rejects a null body up front ("No action found in request body.",
        // CaseDefinitionResource.java:92-94) before the definition is loaded.
        return Err(ApiError::bad_request(
            "No action found in request body.".to_string(),
        ));
    } else {
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?
    };

    repository.get_case_definition(&case_definition_id)?;

    match request.category.as_deref() {
        Some(category) => Ok(Json(
            repository.set_case_definition_category(&case_definition_id, category)?,
        )),
        None => Err(ApiError::bad_request(format!(
            "Invalid action: '{}'.",
            request.action.as_deref().unwrap_or("null")
        ))),
    }
}

pub async fn migrate_case_definition_instances(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    body: String,
) -> Result<Json<Value>, ApiError> {
    let request: CmmnMigrationRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    repository.migrate_case_definition_instances(&case_definition_id, request.into_command()?)?;
    Ok(Json(json!({ "status": "migrated" })))
}

pub async fn batch_migrate_case_definition_instances(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    body: String,
) -> Result<Json<Value>, ApiError> {
    let request: CmmnMigrationRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    repository
        .batch_migrate_case_definition_instances(&case_definition_id, request.into_command()?)?;
    Ok(Json(json!({ "status": "batchMigrationStarted" })))
}

pub async fn migrate_historic_case_definition_instances(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    body: String,
) -> Result<Json<Value>, ApiError> {
    let request: CmmnMigrationRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    repository
        .migrate_historic_case_definition_instances(&case_definition_id, request.into_command()?)?;
    Ok(Json(json!({ "status": "migrated" })))
}

pub async fn batch_migrate_historic_case_definition_instances(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    body: String,
) -> Result<Json<Value>, ApiError> {
    let request: CmmnMigrationRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    repository.batch_migrate_historic_case_definition_instances(
        &case_definition_id,
        request.into_command()?,
    )?;
    Ok(Json(json!({ "status": "batchMigrationStarted" })))
}

pub async fn get_case_definition_resource_data(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(resource_data_response(
        repository.get_case_definition_resource_data(&case_definition_id)?,
    ))
}

pub async fn get_case_definition_model(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        repository.get_case_definition_model(&case_definition_id)?,
    ))
}

pub async fn list_case_definition_decision_tables(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    uri: Uri,
) -> Result<Json<PagedResponse<DecisionTableRecord>>, ApiError> {
    let query: LinkedDefinitionQueryParams = parse_query(&uri)?;
    Ok(Json(repository.list_case_definition_decision_tables(
        &case_definition_id,
        query.paging(),
    )?))
}

pub async fn list_case_definition_decisions(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    uri: Uri,
) -> Result<Json<PagedResponse<DecisionTableRecord>>, ApiError> {
    let query: LinkedDefinitionQueryParams = parse_query(&uri)?;
    Ok(Json(repository.list_case_definition_decisions(
        &case_definition_id,
        query.paging(),
    )?))
}

pub async fn list_case_definition_form_definitions(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    uri: Uri,
) -> Result<Json<PagedResponse<FormDefinitionRecord>>, ApiError> {
    let query: LinkedDefinitionQueryParams = parse_query(&uri)?;
    Ok(Json(repository.list_case_definition_form_definitions(
        &case_definition_id,
        query.paging(),
    )?))
}

pub async fn get_case_definition_start_form(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        repository.get_case_definition_start_form(&case_definition_id)?,
    ))
}

pub async fn list_case_definition_identity_links(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
) -> Result<Json<Vec<CmmnIdentityLinkRecord>>, ApiError> {
    Ok(Json(repository.list_case_definition_identity_links(
        &case_definition_id,
    )?))
}

pub async fn create_case_definition_identity_link(
    Extension(repository): Extension<DynCmmnRepository>,
    Path(case_definition_id): Path<String>,
    Json(request): Json<IdentityLinkRequest>,
) -> Result<(StatusCode, Json<CmmnIdentityLinkRecord>), ApiError> {
    let link = repository
        .create_case_definition_identity_link(&case_definition_id, request.into_command()?)?;
    Ok((StatusCode::CREATED, Json(link)))
}

pub async fn delete_case_definition_identity_links(
    Extension(repository): Extension<DynCmmnRepository>,
    Path((case_definition_id, family, identity_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let family = normalize_identity_link_family(&family)?;
    repository.delete_case_definition_identity_links(&case_definition_id, family, &identity_id)?;
    Ok(StatusCode::NO_CONTENT)
}

fn resource_response(deployment_id: &str, resource_name: &str) -> Value {
    json!({
        "id": resource_name,
        "url": format!(
            "/cmmn-repository/deployments/{deployment_id}/resourcedata/{resource_name}"
        ),
        "contentUrl": format!(
            "/cmmn-repository/deployments/{deployment_id}/resourcedata/{resource_name}"
        ),
        "mediaType": "application/xml",
        "type": "cmmn",
    })
}

fn resource_data_response(resource: CmmnResourceDataRecord) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, resource.mime_type)], resource.bytes)
}

fn deployment_matches_query(
    deployment: &CmmnDeploymentRecord,
    query: &CmmnDeploymentQuery,
) -> bool {
    query.id.as_ref().is_none_or(|id| deployment.id == *id)
        && query
            .name
            .as_ref()
            .is_none_or(|name| deployment.name == *name)
        && query
            .name_like
            .as_ref()
            .is_none_or(|name_like| deployment.name.contains(name_like))
        && query
            .tenant_id
            .as_ref()
            .is_none_or(|tenant_id| deployment.tenant_id.as_deref() == Some(tenant_id.as_str()))
        && (!query.without_tenant_id || deployment.tenant_id.is_none())
        && query.resource_name.as_ref().is_none_or(|resource_name| {
            deployment
                .resource_names
                .iter()
                .any(|candidate| candidate == resource_name)
        })
}

pub async fn start_case_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    body: String,
) -> Result<(StatusCode, Json<CaseInstanceRecord>), ApiError> {
    let payload: StartCaseInstanceRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(runtime.start_case_instance(payload.into_command()?)?),
    ))
}

pub async fn list_case_instances(
    Extension(runtime): Extension<DynCmmnRuntime>,
    uri: Uri,
) -> Result<Json<PagedResponse<CaseInstanceRecord>>, ApiError> {
    let query: CaseInstanceQueryParams = parse_query(&uri)?;
    Ok(Json(runtime.list_case_instances(query.into_query()?)?))
}

pub async fn query_case_instances(
    Extension(runtime): Extension<DynCmmnRuntime>,
    body: String,
) -> Result<Json<PagedResponse<CaseInstanceRecord>>, ApiError> {
    let query: CaseInstanceQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(runtime.list_case_instances(query.into_query()?)?))
}

pub async fn get_case_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
) -> Result<Json<CaseInstanceRecord>, ApiError> {
    Ok(Json(load_case_instance(
        runtime.as_ref(),
        &case_instance_id,
    )?))
}

// Java: CaseInstanceResource.java:88-130 — update name/businessKey or evaluateCriteria
pub async fn update_case_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let command: CmmnCaseInstanceUpdateCommand =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    match runtime.update_case_instance(&case_instance_id, command)? {
        Some(record) => Ok(Json(record).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

pub async fn terminate_case_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    runtime.terminate_case_instance(&case_instance_id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_case_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    runtime.delete_case_instance(&case_instance_id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn bulk_delete_case_instances(
    Extension(runtime): Extension<DynCmmnRuntime>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: BulkCaseInstanceActionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let (action, instance_ids) = request.require_instance_ids()?;

    match action.as_str() {
        "delete" => runtime.bulk_delete_case_instances(instance_ids)?,
        "terminate" => runtime.bulk_terminate_case_instances(instance_ids)?,
        other => return Err(ApiError::bad_request(format!("Illegal action: '{other}'."))),
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_stage_overview(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
) -> Result<Json<Vec<StageOverviewRecord>>, ApiError> {
    Ok(Json(runtime.get_stage_overview(&case_instance_id)?))
}

pub async fn validate_case_instance_migration(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<Json<CmmnMigrationValidationRecord>, ApiError> {
    let request: CmmnMigrationRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(runtime.validate_case_instance_migration(
        &case_instance_id,
        request.into_command()?,
    )?))
}

pub async fn migrate_case_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<Json<Value>, ApiError> {
    let request: CmmnMigrationRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    runtime.migrate_case_instance(&case_instance_id, request.into_command()?)?;
    Ok(Json(json!({ "status": "migrated" })))
}

pub async fn change_plan_item_state(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<Json<Value>, ApiError> {
    let request: CmmnChangePlanItemStateRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    runtime.change_plan_item_state(&case_instance_id, request.into_command())?;
    Ok(Json(json!({ "status": "changed" })))
}

pub async fn list_event_subscriptions(
    Extension(runtime): Extension<DynCmmnRuntime>,
    uri: Uri,
) -> Result<Json<PagedResponse<CmmnEventSubscriptionRecord>>, ApiError> {
    let query: CmmnEventSubscriptionQueryParams = parse_query(&uri)?;
    Ok(Json(runtime.list_event_subscriptions(query.into_query()?)?))
}

pub async fn get_event_subscription(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(event_subscription_id): Path<String>,
) -> Result<Json<CmmnEventSubscriptionRecord>, ApiError> {
    Ok(Json(
        runtime.get_event_subscription(&event_subscription_id)?,
    ))
}

pub async fn trigger_case_event(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    Json(command): Json<CmmnTriggerEventCommand>,
) -> Result<StatusCode, ApiError> {
    runtime.trigger_case_event(&case_instance_id, command)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_case_instance_identity_links(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
) -> Result<Json<Vec<CmmnIdentityLinkRecord>>, ApiError> {
    Ok(Json(
        runtime.list_case_instance_identity_links(&case_instance_id)?,
    ))
}

pub async fn create_case_instance_identity_link(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    Json(request): Json<IdentityLinkRequest>,
) -> Result<(StatusCode, Json<CmmnIdentityLinkRecord>), ApiError> {
    let link =
        runtime.create_case_instance_identity_link(&case_instance_id, request.into_command()?)?;
    Ok((StatusCode::CREATED, Json(link)))
}

pub async fn get_case_instance_identity_link(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((case_instance_id, identity_id, link_type)): Path<(String, String, String)>,
) -> Result<Json<CmmnIdentityLinkRecord>, ApiError> {
    let link = runtime
        .list_case_instance_identity_links(&case_instance_id)?
        .into_iter()
        .find(|link| {
            link.user.as_deref() == Some(identity_id.as_str())
                && link.group.is_none()
                && link.link_type == link_type
        })
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "CMMN case instance '{case_instance_id}' identity link was not found"
            ))
        })?;
    Ok(Json(link))
}

pub async fn delete_case_instance_identity_link(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((case_instance_id, identity_id, link_type)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    runtime.delete_case_instance_identity_link(&case_instance_id, &identity_id, &link_type)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_case_instance_variables(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
) -> Result<Json<Vec<VariableInstanceRecord>>, ApiError> {
    Ok(Json(
        list_variables_for_case_instance(runtime.as_ref(), &case_instance_id)?
            .into_iter()
            .map(normalize_cmmn_variable_record)
            .collect(),
    ))
}

// Java: CaseInstanceVariableCollectionResource.java:141 — POST create variables
pub async fn create_case_instance_variables(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<(StatusCode, Json<Vec<VariableInstanceRecord>>), ApiError> {
    let updates = parse_variable_updates(&body)?;
    runtime.set_case_instance_variables(&case_instance_id, updates.clone())?;
    let records = variables_after_set(runtime.as_ref(), &case_instance_id, &updates)?;
    Ok((StatusCode::CREATED, Json(records)))
}

// Java: CaseInstanceVariableCollectionResource.java:83 — PUT bulk create/update
pub async fn update_case_instance_variables(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<(StatusCode, Json<Vec<VariableInstanceRecord>>), ApiError> {
    let updates = parse_variable_updates(&body)?;
    runtime.set_case_instance_variables(&case_instance_id, updates.clone())?;
    let records = variables_after_set(runtime.as_ref(), &case_instance_id, &updates)?;
    Ok((StatusCode::CREATED, Json(records)))
}

// Java: CaseInstanceVariableCollectionResource.java:180 — DELETE all variables
pub async fn delete_case_instance_variables(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    runtime.remove_case_instance_variables(&case_instance_id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_case_instance_variable(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
) -> Result<Json<VariableInstanceRecord>, ApiError> {
    Ok(Json(normalize_cmmn_variable_record(
        load_case_instance_variable(runtime.as_ref(), &case_instance_id, &variable_name)?,
    )))
}

// Java: CaseInstanceVariableResource.java:88 — PUT single variable
pub async fn update_case_instance_variable(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
    body: String,
) -> Result<(StatusCode, Json<VariableInstanceRecord>), ApiError> {
    let update = parse_single_variable_update(&body, &variable_name)?;
    runtime.set_case_instance_variables(&case_instance_id, vec![update])?;
    Ok((
        StatusCode::CREATED,
        Json(normalize_cmmn_variable_record(load_case_instance_variable(
            runtime.as_ref(),
            &case_instance_id,
            &variable_name,
        )?)),
    ))
}

// Java: CaseInstanceVariableResource.java:176 — DELETE single variable
pub async fn delete_case_instance_variable(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    // Java: hasVariable check before remove — 404 when missing
    load_case_instance_variable(runtime.as_ref(), &case_instance_id, &variable_name)?;
    runtime.remove_case_instance_variable(&case_instance_id, &variable_name)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_case_instance_variable_data(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    super::process_instances::variable_data_response(
        load_case_instance_variable(runtime.as_ref(), &case_instance_id, &variable_name)?.value,
    )
}

pub async fn update_case_instance_variable_data(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    set_case_variable_data(
        runtime.as_ref(),
        &case_instance_id,
        &variable_name,
        body.as_ref(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_case_instance_variables_async(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    set_case_variables_async(
        runtime.as_ref(),
        &case_instance_id,
        parse_variable_updates(&body)?,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_case_instance_variables_async(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    set_case_variables_async(
        runtime.as_ref(),
        &case_instance_id,
        parse_variable_updates(&body)?,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_case_instance_variable_async(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
    body: String,
) -> Result<StatusCode, ApiError> {
    set_case_variables_async(
        runtime.as_ref(),
        &case_instance_id,
        vec![parse_single_variable_update(&body, &variable_name)?],
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_plan_item_instances(
    Extension(runtime): Extension<DynCmmnRuntime>,
    uri: Uri,
) -> Result<Json<PagedResponse<PlanItemInstanceRecord>>, ApiError> {
    let query: PlanItemInstanceQueryParams = parse_query(&uri)?;
    Ok(Json(runtime.list_plan_item_instances(
        plan_item_query_from_params(query)?,
    )?))
}

pub async fn query_plan_item_instances(
    Extension(runtime): Extension<DynCmmnRuntime>,
    body: String,
) -> Result<Json<PagedResponse<PlanItemInstanceRecord>>, ApiError> {
    let query: PlanItemInstanceQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(runtime.list_plan_item_instances(
        plan_item_query_from_params(query)?,
    )?))
}

// P116: the `/cmmn-runtime/tasks` and `/cmmn-query/tasks` endpoints must return only
// human-task plan items — they share the PlanItemInstanceQuery surface with the
// plan-item-instances endpoints but set `task_only` so the stage/milestone/event-listener
// mirror sources are skipped (Java TaskCollectionResource vs PlanItemInstanceCollectionResource).
pub async fn list_tasks(
    Extension(runtime): Extension<DynCmmnRuntime>,
    uri: Uri,
) -> Result<Json<PagedResponse<PlanItemInstanceRecord>>, ApiError> {
    let query: PlanItemInstanceQueryParams = parse_query(&uri)?;
    let mut query = plan_item_query_from_params(query)?;
    query.task_only = true;
    Ok(Json(runtime.list_plan_item_instances(query)?))
}

pub async fn query_tasks(
    Extension(runtime): Extension<DynCmmnRuntime>,
    body: String,
) -> Result<Json<PagedResponse<PlanItemInstanceRecord>>, ApiError> {
    let query: PlanItemInstanceQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let mut query = plan_item_query_from_params(query)?;
    query.task_only = true;
    Ok(Json(runtime.list_plan_item_instances(query)?))
}

pub async fn get_plan_item_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<PlanItemInstanceRecord>, ApiError> {
    Ok(Json(load_plan_item_instance(
        runtime.as_ref(),
        &plan_item_instance_id,
    )?))
}

// Java: PlanItemInstanceResource.java:59-95 — PUT action trigger|enable|disable|start
pub async fn perform_plan_item_instance_action(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let request: PlanItemActionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let action = request
        .action
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("action is required".to_string()))?;
    match runtime.perform_plan_item_instance_action(&plan_item_instance_id, action)? {
        Some(record) => Ok(Json(record).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

/// Java `TaskSubTaskCollectionResource.getSubTasks`
/// (TaskSubTaskCollectionResource.java:42-46) resolves the task (404 when absent) and
/// returns `taskService.getSubTasks(taskId)`, i.e. the tasks whose `parentTaskId` is this
/// task.
///
/// P123 verified that this list is necessarily empty for case-produced tasks: the whole of
/// `flowable-cmmn-engine/src/main/java` contains zero `setParentTaskId` calls, so no CMMN
/// plan-item execution path ever parents one task to another. Java's own test builds the
/// relation by hand through `taskService.newTask()` + `setParentTaskId`
/// (TaskSubTaskCollectionResourceTest.java:44-56), which reaches REST only via
/// `POST /cmmn-runtime/tasks` (TaskCollectionResource.java:357, TaskRequest.java:113-118).
/// Rust exposes no such standalone-task creation (`/cmmn-runtime/tasks` is GET-only) and
/// carries no parent field on `CmmnHumanTaskInstance` or `ACT_CMMN_HUMAN_TASK`, so an empty
/// list is the correct answer rather than a stub. Wiring a parent column would be dead
/// storage until standalone task creation exists — tracked as out of scope for P123.
pub async fn list_task_subtasks(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Vec<PlanItemInstanceRecord>>, ApiError> {
    load_plan_item_instance(runtime.as_ref(), &plan_item_instance_id)?;
    Ok(Json(Vec::new()))
}

pub async fn get_task_form(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(runtime.get_task_form(&plan_item_instance_id)?))
}

pub async fn list_task_identity_links(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Vec<CmmnIdentityLinkRecord>>, ApiError> {
    Ok(Json(
        runtime.list_task_identity_links(&plan_item_instance_id)?,
    ))
}

pub async fn create_task_identity_link(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    Json(request): Json<IdentityLinkRequest>,
) -> Result<(StatusCode, Json<CmmnIdentityLinkRecord>), ApiError> {
    let link =
        runtime.create_task_identity_link(&plan_item_instance_id, request.into_command()?)?;
    Ok((StatusCode::CREATED, Json(link)))
}

pub async fn list_task_identity_links_by_family(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, family)): Path<(String, String)>,
) -> Result<Json<Vec<CmmnIdentityLinkRecord>>, ApiError> {
    let family = normalize_identity_link_family(&family)?;
    let mut links = runtime.list_task_identity_links(&plan_item_instance_id)?;
    retain_identity_link_family(&mut links, family);
    Ok(Json(links))
}

pub async fn get_task_identity_link(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, family, identity_id, link_type)): Path<(
        String,
        String,
        String,
        String,
    )>,
) -> Result<Json<CmmnIdentityLinkRecord>, ApiError> {
    let family = normalize_identity_link_family(&family)?;
    let link = runtime
        .list_task_identity_links(&plan_item_instance_id)?
        .into_iter()
        .find(|link| {
            identity_link_matches_family(link, family, &identity_id) && link.link_type == link_type
        })
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "CMMN task '{plan_item_instance_id}' identity link was not found"
            ))
        })?;
    Ok(Json(link))
}

pub async fn delete_task_identity_link(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, family, identity_id, link_type)): Path<(
        String,
        String,
        String,
        String,
    )>,
) -> Result<StatusCode, ApiError> {
    let family = normalize_identity_link_family(&family)?;
    runtime.delete_task_identity_link(&plan_item_instance_id, family, &identity_id, &link_type)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_task_variables(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    uri: Uri,
) -> Result<Json<Vec<VariableInstanceRecord>>, ApiError> {
    Ok(Json(
        list_variables_for_plan_item(
            runtime.as_ref(),
            &plan_item_instance_id,
            requested_variable_scope(&uri)?,
        )?
        .into_iter()
        .map(normalize_cmmn_variable_record)
        .collect(),
    ))
}

pub async fn get_task_variable(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    uri: Uri,
) -> Result<Json<VariableInstanceRecord>, ApiError> {
    Ok(Json(normalize_cmmn_variable_record(
        load_plan_item_variable(
            runtime.as_ref(),
            &plan_item_instance_id,
            &variable_name,
            requested_variable_scope(&uri)?,
        )?,
    )))
}

pub async fn get_task_variable_data(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    uri: Uri,
) -> Result<Response, ApiError> {
    super::process_instances::variable_data_response(
        load_plan_item_variable(
            runtime.as_ref(),
            &plan_item_instance_id,
            &variable_name,
            requested_variable_scope(&uri)?,
        )?
        .value,
    )
}

pub async fn update_task_variable_data(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    uri: Uri,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    set_plan_item_variable_data(
        runtime.as_ref(),
        &plan_item_instance_id,
        &variable_name,
        requested_variable_scope(&uri)?,
        body.as_ref(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

// Java: TaskResource.java:76-99 — PUT update task (null clears).
pub async fn update_task(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    body: String,
) -> Result<Json<PlanItemInstanceRecord>, ApiError> {
    let update: CmmnHumanTaskUpdate =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(runtime.update_task(&plan_item_instance_id, update)?))
}

// Java: TaskResource.java:109-137 — POST task action; Java returns 200 with an
// empty body (@ResponseStatus(HttpStatus.OK), void return).
pub async fn execute_task_action(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: CmmnTaskActionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    runtime.execute_task_action(&plan_item_instance_id, request)?;
    Ok(StatusCode::OK)
}

// Java: TaskResource.java:149-174 — DELETE task; CMMN tasks are always 403.
pub async fn delete_task(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    runtime.delete_task(&plan_item_instance_id)?;
    Ok(StatusCode::NO_CONTENT)
}

// Java: TaskVariableCollectionResource.java:122-212 — POST batch create variables.
pub async fn create_task_variables(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    body: String,
) -> Result<(StatusCode, Json<Vec<VariableInstanceRecord>>), ApiError> {
    let requests: Vec<CmmnTaskVariableRequest> =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    if requests.is_empty() {
        return Err(ApiError::bad_request(
            "Request didn't contain a list of variables to create.",
        ));
    }

    // Java requires every variable in the same scope (TaskVariableCollectionResource.java:167-172).
    let mut shared_scope: Option<VariableScope> = None;
    let mut updates = Vec::with_capacity(requests.len());
    for request in &requests {
        let name = request
            .name
            .clone()
            .ok_or_else(|| ApiError::bad_request("Variable name is required".to_string()))?;
        let scope = task_variable_scope(request.scope.as_deref())?;
        if let Some(previous) = shared_scope {
            if previous != scope {
                return Err(ApiError::bad_request(
                    "Only allowed to update multiple variables in the same scope.".to_string(),
                ));
            }
        }
        shared_scope = Some(scope);
        // Java: creating an existing variable in the same scope → 409
        // (TaskVariableCollectionResource.java:174-176 hasVariableOnScope).
        if load_plan_item_variable(
            runtime.as_ref(),
            &plan_item_instance_id,
            &name,
            Some(scope),
        )
        .is_ok()
        {
            return Err(ApiError::Conflict(format!(
                "Variable '{name}' is already present on task '{plan_item_instance_id}'."
            )));
        }
        updates.push(variable_update_from_task_request(request, name)?);
    }

    let scope = shared_scope.expect("non-empty requests checked above");
    let records = if scope == VariableScope::Local {
        // Java: TaskVariableCollectionResource.java:188-190 —
        // taskService.setVariablesLocal for LOCAL scope.
        runtime.set_task_variables_local(&plan_item_instance_id, updates.clone())?;
        task_variables_after_set(runtime.as_ref(), &plan_item_instance_id, &updates)?
    } else {
        let plan_item = load_plan_item_instance(runtime.as_ref(), &plan_item_instance_id)?;
        runtime.create_task_variables(&plan_item_instance_id, updates.clone())?;
        variables_after_set(runtime.as_ref(), &plan_item.case_instance_id, &updates)?
    };
    Ok((StatusCode::CREATED, Json(records)))
}

// Java: TaskVariableCollectionResource.java:219-228 — DELETE all local variables, 204.
pub async fn delete_task_variables(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    runtime.delete_task_variables(&plan_item_instance_id)?;
    Ok(StatusCode::NO_CONTENT)
}

// Java: TaskVariableResource.java:94-130 — PUT single variable. The scope
// defaults to LOCAL when omitted (TaskVariableBaseResource.java:210-213).
pub async fn update_task_variable(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    uri: Uri,
    body: String,
) -> Result<Json<VariableInstanceRecord>, ApiError> {
    let scope = requested_variable_scope(&uri)?.unwrap_or(VariableScope::Local);
    let update = parse_single_variable_update(&body, &variable_name)?;
    // PUT is an update — the variable must already exist in scope
    // (TaskVariableBaseResource.java:229-231).
    load_plan_item_variable(
        runtime.as_ref(),
        &plan_item_instance_id,
        &variable_name,
        Some(scope),
    )?;
    match scope {
        // Java: TaskVariableBaseResource.java:241-242 — setVariableLocal for LOCAL.
        VariableScope::Local => {
            runtime.set_task_variables_local(&plan_item_instance_id, vec![update])?;
        }
        VariableScope::Global => {
            runtime.update_task_variable(&plan_item_instance_id, &variable_name, update)?;
        }
    }
    Ok(Json(normalize_cmmn_variable_record(load_plan_item_variable(
        runtime.as_ref(),
        &plan_item_instance_id,
        &variable_name,
        Some(scope),
    )?)))
}

// Java: TaskVariableResource.java:138-167 — DELETE single variable. The scope
// defaults to LOCAL when omitted (TaskVariableResource.java:147-150).
pub async fn delete_task_variable(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    uri: Uri,
) -> Result<StatusCode, ApiError> {
    let scope = requested_variable_scope(&uri)?.unwrap_or(VariableScope::Local);
    // Java checks hasVariableOnScope before remove (TaskVariableResource.java:152-154).
    load_plan_item_variable(
        runtime.as_ref(),
        &plan_item_instance_id,
        &variable_name,
        Some(scope),
    )?;
    match scope {
        // Java: TaskVariableResource.java:160-161 — removeVariableLocal for LOCAL.
        VariableScope::Local => {
            runtime.remove_task_variable_local(&plan_item_instance_id, &variable_name)?;
        }
        VariableScope::Global => {
            runtime.delete_task_variable(&plan_item_instance_id, &variable_name)?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_plan_item_variables(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    uri: Uri,
) -> Result<Json<Vec<VariableInstanceRecord>>, ApiError> {
    Ok(Json(
        list_variables_for_plan_item(
            runtime.as_ref(),
            &plan_item_instance_id,
            requested_variable_scope(&uri)?,
        )?
        .into_iter()
        .map(normalize_cmmn_variable_record)
        .collect(),
    ))
}

pub async fn get_plan_item_variable(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    uri: Uri,
) -> Result<Json<VariableInstanceRecord>, ApiError> {
    Ok(Json(normalize_cmmn_variable_record(
        load_plan_item_variable(
            runtime.as_ref(),
            &plan_item_instance_id,
            &variable_name,
            requested_variable_scope(&uri)?,
        )?,
    )))
}

pub async fn get_plan_item_variable_data(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    uri: Uri,
) -> Result<Response, ApiError> {
    super::process_instances::variable_data_response(
        load_plan_item_variable(
            runtime.as_ref(),
            &plan_item_instance_id,
            &variable_name,
            requested_variable_scope(&uri)?,
        )?
        .value,
    )
}

pub async fn update_plan_item_variable_data(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    uri: Uri,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    set_plan_item_variable_data(
        runtime.as_ref(),
        &plan_item_instance_id,
        &variable_name,
        requested_variable_scope(&uri)?,
        body.as_ref(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_plan_item_variables_async(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    set_plan_item_variables_async(
        runtime.as_ref(),
        &plan_item_instance_id,
        parse_variable_updates(&body)?,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_plan_item_variables_async(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    set_plan_item_variables_async(
        runtime.as_ref(),
        &plan_item_instance_id,
        parse_variable_updates(&body)?,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_plan_item_variable_async(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
    body: String,
) -> Result<StatusCode, ApiError> {
    set_plan_item_variables_async(
        runtime.as_ref(),
        &plan_item_instance_id,
        vec![parse_single_variable_update(&body, &variable_name)?],
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn complete_plan_item_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    runtime.complete_plan_item_instance(&plan_item_instance_id)?;
    Ok(Json(json!({ "status": "completed" })))
}

pub async fn reactivate_plan_item_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    runtime.reactivate_plan_item_instance(&plan_item_instance_id)?;
    Ok(Json(json!({ "status": "reactivated" })))
}

pub async fn disable_plan_item_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    runtime.disable_plan_item_instance(&plan_item_instance_id)?;
    Ok(Json(json!({ "status": "disabled" })))
}

pub async fn enable_plan_item_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    runtime.enable_plan_item_instance(&plan_item_instance_id)?;
    Ok(Json(json!({ "status": "enabled" })))
}

pub async fn list_variable_instances(
    Extension(runtime): Extension<DynCmmnRuntime>,
    uri: Uri,
) -> Result<Json<PagedResponse<VariableInstanceRecord>>, ApiError> {
    let query: VariableInstanceQueryParams = parse_query(&uri)?;
    Ok(Json(list_variable_instances_for_query(
        runtime.as_ref(),
        query,
    )?))
}

pub async fn query_variable_instances(
    Extension(runtime): Extension<DynCmmnRuntime>,
    body: String,
) -> Result<Json<PagedResponse<VariableInstanceRecord>>, ApiError> {
    let query: VariableInstanceQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(list_variable_instances_for_query(
        runtime.as_ref(),
        query,
    )?))
}

pub async fn get_variable_instance(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(variable_instance_id): Path<String>,
) -> Result<Json<VariableInstanceRecord>, ApiError> {
    Ok(Json(runtime.get_variable_instance(&variable_instance_id)?))
}

pub async fn get_variable_instance_data(
    Extension(runtime): Extension<DynCmmnRuntime>,
    Path(variable_instance_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let variable = runtime.get_variable_instance(&variable_instance_id)?;
    Ok(Json(variable.value))
}

fn load_case_instance(
    runtime: &dyn CmmnRuntimeApi,
    case_instance_id: &str,
) -> Result<CaseInstanceRecord, ApiError> {
    let response = runtime.list_case_instances(CaseInstanceQuery {
        id: Some(case_instance_id.to_string()),
        paging: PagingQuery {
            start: 0,
            size: Some(1),
        },
        ..CaseInstanceQuery::default()
    })?;

    response.data.into_iter().next().ok_or_else(|| {
        ApiError::NotFound(format!("Case instance '{case_instance_id}' was not found"))
    })
}

fn load_plan_item_instance(
    runtime: &dyn CmmnRuntimeApi,
    plan_item_instance_id: &str,
) -> Result<PlanItemInstanceRecord, ApiError> {
    let response = runtime.list_plan_item_instances(PlanItemInstanceQuery {
        id: Some(plan_item_instance_id.to_string()),
        paging: PagingQuery {
            start: 0,
            size: Some(1),
        },
        ..PlanItemInstanceQuery::default()
    })?;

    response.data.into_iter().next().ok_or_else(|| {
        ApiError::NotFound(format!(
            "Plan item instance '{plan_item_instance_id}' was not found"
        ))
    })
}

fn plan_item_query_from_params(
    query: PlanItemInstanceQueryParams,
) -> Result<PlanItemInstanceQuery, ApiError> {
    let id = single_alias_value(
        query.id,
        "id",
        query.plan_item_instance_id,
        "planItemInstanceId",
    )?;
    // Java `Integer.valueOf` on the priority params throws → 400
    // (TaskCollectionResource.java:149-159).
    let priority = query
        .priority
        .as_deref()
        .map(parse_priority_param)
        .transpose()?;
    let min_priority = query
        .minimum_priority
        .as_deref()
        .map(parse_priority_param)
        .transpose()?;
    let max_priority = query
        .maximum_priority
        .as_deref()
        .map(parse_priority_param)
        .transpose()?;
    // Java `RequestUtil.getDate` on created/due params throws → 400
    // (RequestUtil.java:76-86).
    let created_on = parse_optional_flowable_date("createdOn", query.created_on.as_deref())?;
    let created_before =
        parse_optional_flowable_date("createdBefore", query.created_before.as_deref())?;
    let created_after =
        parse_optional_flowable_date("createdAfter", query.created_after.as_deref())?;
    let due_date = parse_optional_flowable_date("dueDate", query.due_date.as_deref())?;
    let due_before = parse_optional_flowable_date("dueBefore", query.due_before.as_deref())?;
    let due_after = parse_optional_flowable_date("dueAfter", query.due_after.as_deref())?;
    // Java `getDelegationState` (TaskBaseResource.java:74-86): only pending/resolved.
    let delegation_state = match query.delegation_state.as_deref() {
        None => None,
        Some("pending") => Some("pending".to_string()),
        Some("resolved") => Some("resolved".to_string()),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Illegal value for delegationState: {other}"
            )));
        }
    };
    Ok(PlanItemInstanceQuery {
        paging: PagingQuery {
            start: query.start,
            size: query.size,
        },
        id,
        case_instance_id: query.case_instance_id,
        case_instance_ids: query.case_instance_ids.unwrap_or_default(),
        case_definition_id: query.case_definition_id,
        stage_instance_id: query.stage_instance_id,
        plan_item_definition_id: query.plan_item_definition_id,
        plan_item_definition_type: query.plan_item_definition_type,
        plan_item_definition_types: query.plan_item_definition_types.unwrap_or_default(),
        element_id: query.element_id,
        include_ended: query.include_ended,
        include_local_variables: query.include_local_variables,
        state: query.state,
        name: query.name,
        name_like: query.name_like,
        name_like_ignore_case: query.name_like_ignore_case,
        assignee: query.assignee,
        assignee_like: query.assignee_like,
        owner: query.owner,
        owner_like: query.owner_like,
        unassigned: query.unassigned,
        delegation_state,
        category: query.category,
        category_in: query.category_in.unwrap_or_default(),
        category_not_in: query.category_not_in.unwrap_or_default(),
        without_category: query.without_category,
        task_definition_key: query.task_definition_key,
        task_definition_key_like: query.task_definition_key_like,
        priority,
        min_priority,
        max_priority,
        created_on,
        created_before,
        created_after,
        due_date,
        due_before,
        due_after,
        without_due_date: query.without_due_date,
        case_definition_key: query.case_definition_key,
        case_definition_key_like: query.case_definition_key_like,
        case_definition_key_like_ignore_case: query.case_definition_key_like_ignore_case,
        active: query.active,
        // Java GET `candidateGroups` (CSV) and POST `candidateGroupIn` (array) are
        // the same TaskQuery.candidateGroupIn filter.
        candidate_user: query.candidate_user,
        candidate_group: query.candidate_group,
        candidate_group_in: query.candidate_groups.or(query.candidate_group_in).unwrap_or_default(),
        candidate_or_assigned: query.candidate_or_assigned,
        ignore_assignee: query.ignore_assignee,
        scope_id: query.scope_id,
        include_task_local_variables: query.include_task_local_variables,
        include_process_variables: query.include_process_variables,
        // Java PlanItemInstanceBaseResource.java:122-124 — case instance vars join.
        case_instance_variable_conditions: parse_query_variables(
            query.case_instance_variables.as_deref().unwrap_or(&[]),
        )?,
        // Java PlanItemInstanceBaseResource.java:118-120 (local plan-item vars)
        // + TaskBaseResource.java:308-310 (taskVariables). Empty-local convention:
        // any non-empty set → empty result in the adapter.
        local_variable_conditions: {
            let mut local = parse_query_variables(query.variables.as_deref().unwrap_or(&[]))?;
            local.extend(parse_query_variables(
                query.task_variables.as_deref().unwrap_or(&[]),
            )?);
            local
        },
        sort: query.sort,
        order: query.order,
        task_only: false,
    })
}

/// Convert REST QueryVariable list to engine conditions with Java validation
/// (BaseCaseInstanceResource.java:292-376 / TaskBaseResource.java:360-444).
fn parse_query_variables(
    variables: &[RestQueryVariable],
) -> Result<Vec<QueryVariableCondition>, ApiError> {
    variables
        .iter()
        .map(parse_one_query_variable)
        .collect()
}

fn parse_one_query_variable(variable: &RestQueryVariable) -> Result<QueryVariableCondition, ApiError> {
    let operation = parse_query_variable_operation(variable)?;
    let value = variable.value.as_ref().ok_or_else(|| {
        ApiError::bad_request(format!(
            "Variable value is missing for variable: {}",
            variable.name.as_deref().unwrap_or("null")
        ))
    })?;

    // nameLess only equals — BaseCaseInstanceResource.java:301-308.
    let name_less = variable.name.is_none();
    if name_less && operation != QueryVariableOperation::Equals {
        return Err(ApiError::bad_request(
            "Value-only query (without a variable-name) is only supported when using 'equals' operation.",
        ));
    }

    // ignoreCase / like require string query values
    // (BaseCaseInstanceResource.java:320-354).
    if matches!(
        operation,
        QueryVariableOperation::EqualsIgnoreCase
            | QueryVariableOperation::NotEqualsIgnoreCase
            | QueryVariableOperation::Like
            | QueryVariableOperation::LikeIgnoreCase
    ) && !value.is_string()
    {
        let kind = match operation {
            QueryVariableOperation::Like | QueryVariableOperation::LikeIgnoreCase => "like",
            _ => "ignoring casing",
        };
        return Err(ApiError::bad_request(format!(
            "Only string variable values are supported for {kind}, but was: {}",
            json_value_type_name(value)
        )));
    }

    // Bool/null banned from comparison ops — AbstractVariableQueryImpl.java:303-316.
    if matches!(
        operation,
        QueryVariableOperation::GreaterThan
            | QueryVariableOperation::GreaterThanOrEquals
            | QueryVariableOperation::LessThan
            | QueryVariableOperation::LessThanOrEquals
    ) && (value.is_null() || value.is_boolean())
    {
        return Err(ApiError::bad_request(
            "Booleans and null cannot be used in comparison variable query conditions",
        ));
    }

    Ok(QueryVariableCondition {
        name: variable.name.clone(),
        operation,
        value: value.clone(),
    })
}

fn parse_query_variable_operation(
    variable: &RestQueryVariable,
) -> Result<QueryVariableOperation, ApiError> {
    // QueryVariable.java:88-95 / BaseCaseInstanceResource.java:294-296.
    match variable.operation.as_deref() {
        None => Err(ApiError::bad_request(format!(
            "Variable operation is missing for variable: {}",
            variable.name.as_deref().unwrap_or("null")
        ))),
        // Full enum QueryVariable.java:75-76.
        Some("equals") => Ok(QueryVariableOperation::Equals),
        Some("notEquals") => Ok(QueryVariableOperation::NotEquals),
        Some("equalsIgnoreCase") => Ok(QueryVariableOperation::EqualsIgnoreCase),
        Some("notEqualsIgnoreCase") => Ok(QueryVariableOperation::NotEqualsIgnoreCase),
        Some("like") => Ok(QueryVariableOperation::Like),
        Some("likeIgnoreCase") => Ok(QueryVariableOperation::LikeIgnoreCase),
        Some("greaterThan") => Ok(QueryVariableOperation::GreaterThan),
        Some("greaterThanOrEquals") => Ok(QueryVariableOperation::GreaterThanOrEquals),
        Some("lessThan") => Ok(QueryVariableOperation::LessThan),
        Some("lessThanOrEquals") => Ok(QueryVariableOperation::LessThanOrEquals),
        Some(other) => Err(ApiError::bad_request(format!(
            "Unsupported variable query operation: {other}"
        ))),
    }
}

fn json_value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "double",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn parse_priority_param(value: &str) -> Result<i64, ApiError> {
    value.parse::<i64>().map_err(|_| {
        ApiError::bad_request(format!("Invalid priority value '{value}': must be an integer"))
    })
}

fn deserialize_string_list<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(value)) => Some(split_csv(&value)),
        Some(serde_json::Value::Array(values)) => Some(
            values
                .into_iter()
                .map(|value| match value {
                    serde_json::Value::String(value) => value,
                    other => other.to_string(),
                })
                .flat_map(|value| split_csv(&value))
                .collect(),
        ),
        Some(other) => Some(split_csv(&other.to_string())),
    })
}

/// Java `csvToList` (TaskCollectionResource.java:419-424): comma-separated list.
fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn list_variable_instances_for_query(
    runtime: &dyn CmmnRuntimeApi,
    query: VariableInstanceQueryParams,
) -> Result<PagedResponse<VariableInstanceRecord>, ApiError> {
    let paging = PagingQuery {
        start: query.start,
        size: query.size,
    };
    let plan_item_instance_id = single_alias_value(
        query.task_id,
        "taskId",
        query.plan_item_instance_id,
        "planItemInstanceId",
    )?;
    let mut variable_query = VariableInstanceQuery {
        paging,
        id: query.id,
        case_instance_id: query.case_instance_id,
        scope_id: query.scope_id,
        variable_name: query.variable_name.or(query.name),
        variable_name_like: query.variable_name_like,
        exclude_task_variables: query.exclude_task_variables.unwrap_or(false),
        exclude_local_variables: query.exclude_local_variables.unwrap_or(false),
    };

    if let Some(plan_item_instance_id) = plan_item_instance_id {
        let plan_item = load_plan_item_instance(runtime, &plan_item_instance_id)?;
        if variable_query
            .case_instance_id
            .as_deref()
            .is_some_and(|case_instance_id| case_instance_id != plan_item.case_instance_id)
        {
            return Ok(variable_query.paging.paginate(Vec::new()));
        }
        variable_query.case_instance_id = Some(plan_item.case_instance_id);
    }

    runtime.list_variable_instances(variable_query)
}

fn list_variables_for_case_instance(
    runtime: &dyn CmmnRuntimeApi,
    case_instance_id: &str,
) -> Result<Vec<VariableInstanceRecord>, ApiError> {
    load_case_instance(runtime, case_instance_id)?;
    Ok(runtime
        .list_variable_instances(VariableInstanceQuery {
            case_instance_id: Some(case_instance_id.to_string()),
            ..VariableInstanceQuery::default()
        })?
        .data)
}

fn load_case_instance_variable(
    runtime: &dyn CmmnRuntimeApi,
    case_instance_id: &str,
    variable_name: &str,
) -> Result<VariableInstanceRecord, ApiError> {
    list_variables_for_case_instance(runtime, case_instance_id)?
        .into_iter()
        .find(|variable| variable.name == variable_name)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Case instance '{case_instance_id}' variable '{variable_name}' was not found"
            ))
        })
}

fn list_variables_for_plan_item(
    runtime: &dyn CmmnRuntimeApi,
    plan_item_instance_id: &str,
    scope: Option<VariableScope>,
) -> Result<Vec<VariableInstanceRecord>, ApiError> {
    let plan_item = load_plan_item_instance(runtime, plan_item_instance_id)?;
    match scope {
        // Java: scope=local → only the task's own variables
        // (TaskVariableCollectionResource.java:85-87 → addLocalVariables).
        Some(VariableScope::Local) => runtime.list_task_variables_local(plan_item_instance_id),
        Some(VariableScope::Global) => {
            list_variables_for_case_instance(runtime, &plan_item.case_instance_id)
        }
        // Java: scope omitted → local variables first, then global fills only
        // gaps (TaskVariableCollectionResource.java:76-96; addGlobalVariables
        // line 238 keeps the existing local entry). Local shadows case.
        None => {
            let mut merged = runtime.list_task_variables_local(plan_item_instance_id)?;
            let global = list_variables_for_case_instance(runtime, &plan_item.case_instance_id)?;
            for variable in global {
                if !merged.iter().any(|existing| existing.name == variable.name) {
                    merged.push(variable);
                }
            }
            Ok(merged)
        }
    }
}

fn load_plan_item_variable(
    runtime: &dyn CmmnRuntimeApi,
    plan_item_instance_id: &str,
    variable_name: &str,
    scope: Option<VariableScope>,
) -> Result<VariableInstanceRecord, ApiError> {
    list_variables_for_plan_item(runtime, plan_item_instance_id, scope)?
        .into_iter()
        .find(|variable| variable.name == variable_name)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Plan item instance '{plan_item_instance_id}' variable '{variable_name}' was not found"
            ))
        })
}

fn normalize_cmmn_variable_record(mut variable: VariableInstanceRecord) -> VariableInstanceRecord {
    if let Some(variable_type) = super::process_instances::variable_data_type(&variable.value) {
        variable.variable_type = variable_type;
        variable.value = Value::Null;
    }
    variable
}

fn set_case_variables_async(
    runtime: &dyn CmmnRuntimeApi,
    case_instance_id: &str,
    updates: Vec<CmmnVariableUpdate>,
) -> Result<(), ApiError> {
    load_case_instance(runtime, case_instance_id)?;
    runtime.set_case_instance_variables(case_instance_id, updates)
}

fn variables_after_set(
    runtime: &dyn CmmnRuntimeApi,
    case_instance_id: &str,
    updates: &[CmmnVariableUpdate],
) -> Result<Vec<VariableInstanceRecord>, ApiError> {
    let mut records = Vec::with_capacity(updates.len());
    for update in updates {
        records.push(normalize_cmmn_variable_record(load_case_instance_variable(
            runtime,
            case_instance_id,
            &update.name,
        )?));
    }
    Ok(records)
}

/// Read-back of LOCAL task variables after a set — the POST create response
/// returns the written variables (TaskVariableCollectionResource.java:203-207).
fn task_variables_after_set(
    runtime: &dyn CmmnRuntimeApi,
    plan_item_instance_id: &str,
    updates: &[CmmnVariableUpdate],
) -> Result<Vec<VariableInstanceRecord>, ApiError> {
    let mut records = Vec::with_capacity(updates.len());
    for update in updates {
        records.push(normalize_cmmn_variable_record(load_plan_item_variable(
            runtime,
            plan_item_instance_id,
            &update.name,
            Some(VariableScope::Local),
        )?));
    }
    Ok(records)
}

fn set_plan_item_variables_async(
    runtime: &dyn CmmnRuntimeApi,
    plan_item_instance_id: &str,
    updates: Vec<CmmnVariableUpdate>,
) -> Result<(), ApiError> {
    let plan_item = load_plan_item_instance(runtime, plan_item_instance_id)?;
    runtime.set_case_instance_variables(&plan_item.case_instance_id, updates)
}

fn set_case_variable_data(
    runtime: &dyn CmmnRuntimeApi,
    case_instance_id: &str,
    variable_name: &str,
    bytes: &[u8],
) -> Result<(), ApiError> {
    let variable = load_case_instance_variable(runtime, case_instance_id, variable_name)?;
    let variable_type =
        super::process_instances::variable_data_type(&variable.value).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "CMMN case instance '{case_instance_id}' variable '{variable_name}' is not a binary, bytes, or serializable variable"
            ))
        })?;
    runtime.set_case_instance_variables(
        case_instance_id,
        vec![CmmnVariableUpdate {
            name: variable_name.to_string(),
            value: super::process_instances::encode_variable_data(&variable_type, bytes),
        }],
    )
}

fn set_plan_item_variable_data(
    runtime: &dyn CmmnRuntimeApi,
    plan_item_instance_id: &str,
    variable_name: &str,
    scope: Option<VariableScope>,
    bytes: &[u8],
) -> Result<(), ApiError> {
    let plan_item = load_plan_item_instance(runtime, plan_item_instance_id)?;
    if scope == Some(VariableScope::Local) {
        // Java TaskVariableDataResource writes the local task variable; the
        // variable must already exist locally with a binary/bytes/serializable
        // type (TaskVariableBaseResource.setVariable isNew=false path).
        let variable =
            load_plan_item_variable(runtime, plan_item_instance_id, variable_name, Some(VariableScope::Local))?;
        let variable_type =
            super::process_instances::variable_data_type(&variable.value).ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "Plan item instance '{plan_item_instance_id}' variable '{variable_name}' is not a binary, bytes, or serializable variable"
                ))
            })?;
        return runtime.set_task_variables_local(
            plan_item_instance_id,
            vec![CmmnVariableUpdate {
                name: variable_name.to_string(),
                value: super::process_instances::encode_variable_data(&variable_type, bytes),
            }],
        );
    }
    let variable =
        load_case_instance_variable(runtime, &plan_item.case_instance_id, variable_name)?;
    let variable_type =
        super::process_instances::variable_data_type(&variable.value).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Plan item instance '{plan_item_instance_id}' variable '{variable_name}' is not a binary, bytes, or serializable variable"
            ))
        })?;
    runtime.set_case_instance_variables(
        &plan_item.case_instance_id,
        vec![CmmnVariableUpdate {
            name: variable_name.to_string(),
            value: super::process_instances::encode_variable_data(&variable_type, bytes),
        }],
    )
}

fn parse_variable_updates(body: &str) -> Result<Vec<CmmnVariableUpdate>, ApiError> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let requests: Vec<VariableRequest> = if value.is_array() {
        serde_json::from_value(value).map_err(|error| ApiError::BadRequest(error.to_string()))?
    } else {
        vec![
            serde_json::from_value(value)
                .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        ]
    };

    requests
        .into_iter()
        .map(|request| {
            let name = request
                .name
                .clone()
                .ok_or_else(|| ApiError::BadRequest("Variable name is required".to_string()))?;
            variable_update_from_request(request, name)
        })
        .collect()
}

fn parse_single_variable_update(
    body: &str,
    variable_name: &str,
) -> Result<CmmnVariableUpdate, ApiError> {
    let request: VariableRequest =
        serde_json::from_str(body).map_err(|error| ApiError::BadRequest(error.to_string()))?;
    if let Some(name) = request.name.as_deref()
        && name != variable_name
    {
        return Err(ApiError::BadRequest(format!(
            "Request variable name '{}' does not match path variable '{}'",
            name, variable_name
        )));
    }
    variable_update_from_request(request, variable_name.to_string())
}

fn variable_update_from_request(
    request: VariableRequest,
    name: String,
) -> Result<CmmnVariableUpdate, ApiError> {
    let value = storage_value_for_cmmn_variable_request(&request)?;
    Ok(CmmnVariableUpdate { name, value })
}

/// Task-variable request carries the same value/type semantics as
/// `VariableRequest` plus a `scope` field; only the shared fields matter here.
fn variable_update_from_task_request(
    request: &CmmnTaskVariableRequest,
    name: String,
) -> Result<CmmnVariableUpdate, ApiError> {
    variable_update_from_request(
        VariableRequest {
            name: Some(name.clone()),
            variable_type: request.variable_type.clone(),
            value: request.value.clone(),
        },
        name,
    )
}

/// Java `RestVariable.getScopeFromString` with the Java default of LOCAL when
/// omitted (TaskVariableCollectionResource.java:164-166).
fn task_variable_scope(scope: Option<&str>) -> Result<VariableScope, ApiError> {
    match scope {
        None => Ok(VariableScope::Local),
        Some(s) if s.eq_ignore_ascii_case("local") => Ok(VariableScope::Local),
        Some(s) if s.eq_ignore_ascii_case("global") => Ok(VariableScope::Global),
        Some(s) => Err(ApiError::bad_request(format!(
            "Unsupported variable scope '{s}'"
        ))),
    }
}

fn storage_value_for_cmmn_variable_request(request: &VariableRequest) -> Result<Value, ApiError> {
    let Some(variable_type) = request.variable_type.as_deref() else {
        return Ok(request.value.clone());
    };
    match variable_type.to_ascii_lowercase().as_str() {
        "binary" | "bytes" => {
            if !request.value.is_null() {
                return Err(ApiError::BadRequest(format!(
                    "Variable type '{}' metadata must use null value; write bytes with the variable data endpoint",
                    variable_type
                )));
            }
            Ok(super::process_instances::encode_binary_variable(
                &variable_type.to_ascii_lowercase(),
                &[],
            ))
        }
        "serializable" => {
            if !request.value.is_null() {
                return Err(ApiError::BadRequest(format!(
                    "Variable type '{}' metadata must use null value; write object data with the variable data endpoint",
                    variable_type
                )));
            }
            Ok(super::process_instances::encode_variable_data(
                "serializable",
                &[],
            ))
        }
        _ => Ok(request.value.clone()),
    }
}

impl IdentityLinkRequest {
    fn into_command(self) -> Result<CmmnIdentityLinkCreateCommand, ApiError> {
        let link_type = self.link_type.ok_or_else(|| {
            ApiError::BadRequest("The identity link type is required.".to_string())
        })?;
        match (self.user, self.group) {
            (Some(user), None) => Ok(CmmnIdentityLinkCreateCommand {
                user: Some(user),
                group: None,
                link_type,
            }),
            (None, Some(group)) => Ok(CmmnIdentityLinkCreateCommand {
                user: None,
                group: Some(group),
                link_type,
            }),
            (None, None) => Err(ApiError::BadRequest(
                "Either user or group is required.".to_string(),
            )),
            (Some(_), Some(_)) => Err(ApiError::BadRequest(
                "Only one of user or group is allowed.".to_string(),
            )),
        }
    }
}

fn normalize_identity_link_family(family: &str) -> Result<&'static str, ApiError> {
    if family.eq_ignore_ascii_case("users") {
        Ok("users")
    } else if family.eq_ignore_ascii_case("groups") {
        Ok("groups")
    } else {
        Err(ApiError::BadRequest(format!(
            "Unsupported identity link family '{}'",
            family
        )))
    }
}

fn identity_link_matches_family(
    link: &CmmnIdentityLinkRecord,
    family: &str,
    identity_id: &str,
) -> bool {
    match family {
        "users" => link.user.as_deref() == Some(identity_id),
        "groups" => link.group.as_deref() == Some(identity_id),
        _ => false,
    }
}

fn retain_identity_link_family(links: &mut Vec<CmmnIdentityLinkRecord>, family: &str) {
    match family {
        "users" => links.retain(|link| link.user.is_some()),
        "groups" => links.retain(|link| link.group.is_some()),
        _ => {}
    }
}

fn single_alias_value(
    first: Option<String>,
    first_name: &str,
    second: Option<String>,
    second_name: &str,
) -> Result<Option<String>, ApiError> {
    match (first, second) {
        (Some(first), Some(second)) if first != second => Err(ApiError::bad_request(format!(
            "Only one of {first_name} or {second_name} can be used"
        ))),
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariableScope {
    Local,
    Global,
}

fn requested_variable_scope(uri: &Uri) -> Result<Option<VariableScope>, ApiError> {
    let query: VariableScopeQueryParams = parse_query(uri)?;
    match query.scope.as_deref() {
        None => Ok(None),
        Some(scope) if scope.eq_ignore_ascii_case("local") => Ok(Some(VariableScope::Local)),
        Some(scope) if scope.eq_ignore_ascii_case("global") => Ok(Some(VariableScope::Global)),
        Some(scope) => Err(ApiError::bad_request(format!(
            "Unsupported CMMN variable scope '{scope}'"
        ))),
    }
}

pub async fn list_historic_case_instances(
    Extension(history): Extension<DynCmmnHistory>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricCaseInstanceRecord>>, ApiError> {
    let query: HistoricCaseInstanceQueryParams = parse_query(&uri)?;
    Ok(Json(history.list_historic_case_instances(
        historic_case_instance_query_from_params(query)?,
    )?))
}

pub async fn query_historic_case_instances(
    Extension(history): Extension<DynCmmnHistory>,
    body: String,
) -> Result<Json<PagedResponse<HistoricCaseInstanceRecord>>, ApiError> {
    let query: HistoricCaseInstanceQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(history.list_historic_case_instances(
        historic_case_instance_query_from_params(query)?,
    )?))
}

pub async fn get_historic_case_instance(
    Extension(history): Extension<DynCmmnHistory>,
    Path(case_instance_id): Path<String>,
) -> Result<Json<HistoricCaseInstanceRecord>, ApiError> {
    Ok(Json(load_historic_case_instance(
        history.as_ref(),
        &case_instance_id,
    )?))
}

pub async fn delete_historic_case_instance(
    Extension(history): Extension<DynCmmnHistory>,
    Path(case_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    history.delete_historic_case_instance(&case_instance_id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn migrate_historic_case_instance(
    Extension(history): Extension<DynCmmnHistory>,
    Path(case_instance_id): Path<String>,
    body: String,
) -> Result<Json<Value>, ApiError> {
    let request: CmmnMigrationRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    history.migrate_historic_case_instance(&case_instance_id, request.into_command()?)?;
    Ok(Json(json!({ "status": "migrated" })))
}

pub async fn bulk_delete_historic_case_instances(
    Extension(history): Extension<DynCmmnHistory>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let request: BulkCaseInstanceActionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let (action, instance_ids) = request.require_instance_ids()?;
    if instance_ids.is_empty() {
        return Err(ApiError::bad_request("historic case instanceIds are empty"));
    }
    if action != "delete" {
        return Err(ApiError::bad_request(format!(
            "Illegal action: '{action}'."
        )));
    }

    history.bulk_delete_historic_case_instances(instance_ids)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_historic_stage_overview(
    Extension(history): Extension<DynCmmnHistory>,
    Path(case_instance_id): Path<String>,
) -> Result<Json<Vec<StageOverviewRecord>>, ApiError> {
    Ok(Json(
        history.get_historic_stage_overview(&case_instance_id)?,
    ))
}

pub async fn list_historic_case_instance_identity_links(
    Extension(history): Extension<DynCmmnHistory>,
    Path(case_instance_id): Path<String>,
) -> Result<Json<Vec<CmmnIdentityLinkRecord>>, ApiError> {
    Ok(Json(history.list_historic_case_instance_identity_links(
        &case_instance_id,
    )?))
}

pub async fn get_historic_case_instance_variable_data(
    Extension(history): Extension<DynCmmnHistory>,
    Path((case_instance_id, variable_name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        load_historic_case_variable(history.as_ref(), &case_instance_id, &variable_name)?.value,
    ))
}

pub async fn list_historic_plan_item_instances(
    Extension(history): Extension<DynCmmnHistory>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricPlanItemInstanceRecord>>, ApiError> {
    let query: HistoricPlanItemInstanceQueryParams = parse_query(&uri)?;
    Ok(Json(history.list_historic_plan_item_instances(
        historic_plan_item_query_from_params(query)?,
    )?))
}

pub async fn query_historic_plan_item_instances(
    Extension(history): Extension<DynCmmnHistory>,
    body: String,
) -> Result<Json<PagedResponse<HistoricPlanItemInstanceRecord>>, ApiError> {
    let query: HistoricPlanItemInstanceQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(history.list_historic_plan_item_instances(
        historic_plan_item_query_from_params(query)?,
    )?))
}

pub async fn get_historic_plan_item_instance(
    Extension(history): Extension<DynCmmnHistory>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<HistoricPlanItemInstanceRecord>, ApiError> {
    Ok(Json(load_historic_plan_item_instance(
        history.as_ref(),
        &plan_item_instance_id,
    )?))
}

pub async fn get_historic_task_form(
    Extension(history): Extension<DynCmmnHistory>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        history.get_historic_task_form(&plan_item_instance_id)?,
    ))
}

pub async fn list_historic_task_identity_links(
    Extension(history): Extension<DynCmmnHistory>,
    Path(plan_item_instance_id): Path<String>,
) -> Result<Json<Vec<CmmnIdentityLinkRecord>>, ApiError> {
    Ok(Json(history.list_historic_task_identity_links(
        &plan_item_instance_id,
    )?))
}

pub async fn get_historic_task_variable_data(
    Extension(history): Extension<DynCmmnHistory>,
    Path((plan_item_instance_id, variable_name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let plan_item = load_historic_plan_item_instance(history.as_ref(), &plan_item_instance_id)?;
    Ok(Json(
        load_historic_case_variable(
            history.as_ref(),
            &plan_item.case_instance_id,
            &variable_name,
        )?
        .value,
    ))
}

pub async fn list_historic_milestone_instances(
    Extension(history): Extension<DynCmmnHistory>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricMilestoneInstanceRecord>>, ApiError> {
    let query: HistoricMilestoneInstanceQueryParams = parse_query(&uri)?;
    Ok(Json(list_historic_milestone_instances_for_query(
        history.as_ref(),
        query.into(),
    )?))
}

pub async fn query_historic_milestone_instances(
    Extension(history): Extension<DynCmmnHistory>,
    body: String,
) -> Result<Json<PagedResponse<HistoricMilestoneInstanceRecord>>, ApiError> {
    let query: HistoricMilestoneInstanceQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(list_historic_milestone_instances_for_query(
        history.as_ref(),
        query.into(),
    )?))
}

pub async fn get_historic_milestone_instance(
    Extension(history): Extension<DynCmmnHistory>,
    Path(milestone_instance_id): Path<String>,
) -> Result<Json<HistoricMilestoneInstanceRecord>, ApiError> {
    Ok(Json(load_historic_milestone_instance(
        history.as_ref(),
        &milestone_instance_id,
    )?))
}

pub async fn list_historic_variable_instances(
    Extension(history): Extension<DynCmmnHistory>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricVariableInstanceRecord>>, ApiError> {
    let query: HistoricVariableInstanceQueryParams = parse_query(&uri)?;
    Ok(Json(list_historic_variable_instances_for_query(
        history.as_ref(),
        query,
    )?))
}

pub async fn query_historic_variable_instances(
    Extension(history): Extension<DynCmmnHistory>,
    body: String,
) -> Result<Json<PagedResponse<HistoricVariableInstanceRecord>>, ApiError> {
    let query: HistoricVariableInstanceQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(list_historic_variable_instances_for_query(
        history.as_ref(),
        query,
    )?))
}

pub async fn get_historic_variable_instance_data(
    Extension(history): Extension<DynCmmnHistory>,
    Path(variable_instance_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let variable = history.get_historic_variable_instance(&variable_instance_id)?;
    Ok(Json(variable.value))
}

fn load_historic_case_instance(
    history: &dyn CmmnHistoryApi,
    case_instance_id: &str,
) -> Result<HistoricCaseInstanceRecord, ApiError> {
    let response = history.list_historic_case_instances(HistoricCaseInstanceQuery {
        id: Some(case_instance_id.to_string()),
        paging: PagingQuery {
            start: 0,
            size: Some(1),
        },
        ..HistoricCaseInstanceQuery::default()
    })?;

    response.data.into_iter().next().ok_or_else(|| {
        ApiError::NotFound(format!(
            "Historic case instance '{case_instance_id}' was not found"
        ))
    })
}

fn load_historic_plan_item_instance(
    history: &dyn CmmnHistoryApi,
    plan_item_instance_id: &str,
) -> Result<HistoricPlanItemInstanceRecord, ApiError> {
    let response = history.list_historic_plan_item_instances(HistoricPlanItemInstanceQuery {
        id: Some(plan_item_instance_id.to_string()),
        paging: PagingQuery {
            start: 0,
            size: Some(1),
        },
        ..HistoricPlanItemInstanceQuery::default()
    })?;

    response.data.into_iter().next().ok_or_else(|| {
        ApiError::NotFound(format!(
            "Historic plan item instance '{plan_item_instance_id}' was not found"
        ))
    })
}

/// Java `HistoricTaskInstanceCollectionResource.getHistoricProcessInstances`
/// (HistoricTaskInstanceCollectionResource.java:97-306). `taskId` is an alias of
/// the Rust `id`/`planItemInstanceId` on this shared handler; dates go through
/// `RequestUtil.getDate` (400 on a malformed value).
fn historic_plan_item_query_from_params(
    query: HistoricPlanItemInstanceQueryParams,
) -> Result<HistoricPlanItemInstanceQuery, ApiError> {
    let id = single_alias_value(
        query.id,
        "id",
        query.plan_item_instance_id,
        "planItemInstanceId",
    )?;
    let id = single_alias_value(id, "id", query.task_id, "taskId")?;
    Ok(HistoricPlanItemInstanceQuery {
        paging: PagingQuery {
            start: query.start,
            size: query.size,
        },
        id,
        case_instance_id: query.case_instance_id,
        case_definition_id: query.case_definition_id,
        plan_item_definition_id: query.plan_item_definition_id,
        state: query.state,
        name: query.task_name,
        name_like: query.task_name_like,
        name_like_ignore_case: query.task_name_like_ignore_case,
        task_definition_key: query.task_definition_key,
        task_definition_key_like: query.task_definition_key_like,
        assignee: query.task_assignee,
        assignee_like: query.task_assignee_like,
        owner: query.task_owner,
        owner_like: query.task_owner_like,
        category: query.task_category,
        delete_reason: query.task_delete_reason,
        created_before: parse_optional_flowable_date(
            "taskCreatedBefore",
            query.task_created_before.as_deref(),
        )?,
        created_after: parse_optional_flowable_date(
            "taskCreatedAfter",
            query.task_created_after.as_deref(),
        )?,
        completed_before: parse_optional_flowable_date(
            "taskCompletedBefore",
            query.task_completed_before.as_deref(),
        )?,
        completed_after: parse_optional_flowable_date(
            "taskCompletedAfter",
            query.task_completed_after.as_deref(),
        )?,
        finished: query.finished,
        candidate_group: query.task_candidate_group,
        involved_user: query.task_involved_user,
        // Java only honours the flag when it parses to true
        // (HistoricTaskInstanceCollectionResource.java:289-291).
        ignore_assignee: query.ignore_task_assignee.unwrap_or(false),
        sort: query.sort,
        order: query.order,
    })
}

fn list_historic_variable_instances_for_query(
    history: &dyn CmmnHistoryApi,
    query: HistoricVariableInstanceQueryParams,
) -> Result<PagedResponse<HistoricVariableInstanceRecord>, ApiError> {
    let paging = PagingQuery {
        start: query.start,
        size: query.size,
    };
    let plan_item_instance_id = single_alias_value(
        query.task_id,
        "taskId",
        query.plan_item_instance_id,
        "planItemInstanceId",
    )?;
    let mut variable_query = HistoricVariableInstanceQuery {
        paging,
        id: query.id,
        case_instance_id: query.case_instance_id,
        scope_id: query.scope_id,
        variable_name: query.variable_name.or(query.name),
        variable_name_like: query.variable_name_like,
        exclude_task_variables: query.exclude_task_variables.unwrap_or(false),
        exclude_local_variables: query.exclude_local_variables.unwrap_or(false),
    };

    if let Some(plan_item_instance_id) = plan_item_instance_id {
        let plan_item = load_historic_plan_item_instance(history, &plan_item_instance_id)?;
        if variable_query
            .case_instance_id
            .as_deref()
            .is_some_and(|case_instance_id| case_instance_id != plan_item.case_instance_id)
        {
            return Ok(variable_query.paging.paginate(Vec::new()));
        }
        variable_query.case_instance_id = Some(plan_item.case_instance_id);
    }

    history.list_historic_variable_instances(variable_query)
}

fn load_historic_milestone_instance(
    history: &dyn CmmnHistoryApi,
    milestone_instance_id: &str,
) -> Result<HistoricMilestoneInstanceRecord, ApiError> {
    let response = history.list_historic_milestone_instances(HistoricMilestoneInstanceQuery {
        id: Some(milestone_instance_id.to_string()),
        paging: PagingQuery {
            start: 0,
            size: Some(1),
        },
        ..HistoricMilestoneInstanceQuery::default()
    })?;

    response.data.into_iter().next().ok_or_else(|| {
        ApiError::NotFound(format!(
            "Historic milestone instance '{milestone_instance_id}' was not found"
        ))
    })
}

fn list_historic_milestone_instances_for_query(
    history: &dyn CmmnHistoryApi,
    mut query: HistoricMilestoneInstanceQuery,
) -> Result<PagedResponse<HistoricMilestoneInstanceRecord>, ApiError> {
    let paging = query.paging;
    let milestone_name = query.milestone_name.clone();
    let reached_before =
        parse_optional_flowable_date("reachedBefore", query.reached_before.as_deref())?;
    let reached_after =
        parse_optional_flowable_date("reachedAfter", query.reached_after.as_deref())?;
    let requires_post_filter =
        milestone_name.is_some() || reached_before.is_some() || reached_after.is_some();

    if requires_post_filter {
        query.paging = PagingQuery {
            start: 0,
            size: None,
        };
    }

    let mut response = history.list_historic_milestone_instances(query)?;
    if let Some(milestone_name) = milestone_name {
        response
            .data
            .retain(|milestone| milestone.name == milestone_name);
    }
    if let Some(reached_before) = reached_before {
        response.data.retain(|milestone| {
            parse_flowable_date(&milestone.time)
                .map(|time| time < reached_before)
                .unwrap_or(false)
        });
    }
    if let Some(reached_after) = reached_after {
        response.data.retain(|milestone| {
            parse_flowable_date(&milestone.time)
                .map(|time| time > reached_after)
                .unwrap_or(false)
        });
    }

    if requires_post_filter {
        return Ok(paging.paginate(response.data));
    }
    Ok(response)
}

fn parse_optional_flowable_date(
    parameter_name: &str,
    value: Option<&str>,
) -> Result<Option<DateTime<Utc>>, ApiError> {
    value
        .map(parse_flowable_date)
        .transpose()
        .map_err(|_| ApiError::bad_request(format!("Invalid date value for {parameter_name}")))
}

fn parse_flowable_date(value: &str) -> Result<DateTime<Utc>, ()> {
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Ok(date.with_timezone(&Utc));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.3f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(date) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(date.and_utc());
        }
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|date| date.and_utc())
        .ok_or(())
}

fn load_historic_case_variable(
    history: &dyn CmmnHistoryApi,
    case_instance_id: &str,
    variable_name: &str,
) -> Result<HistoricVariableInstanceRecord, ApiError> {
    load_historic_case_instance(history, case_instance_id)?;
    history
        .list_historic_variable_instances(HistoricVariableInstanceQuery {
            case_instance_id: Some(case_instance_id.to_string()),
            variable_name: Some(variable_name.to_string()),
            paging: PagingQuery {
                start: 0,
                size: Some(1),
            },
            ..HistoricVariableInstanceQuery::default()
        })?
        .data
        .into_iter()
        .next()
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic case instance '{case_instance_id}' variable '{variable_name}' was not found"
            ))
        })
}
