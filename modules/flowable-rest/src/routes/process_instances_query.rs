use super::process_instances::{ProcessInstanceResponse, to_process_instance_response};
use crate::common::{PagedResponse, PagingQuery, parse_query, parse_rfc3339_datetime};
use crate::error::ApiError;
use crate::query_variable::{
    QueryVariableOperation, validate_name_less_equals, validate_operation_value, value_matches,
};
use axum::{Extension, Json, extract::Path, http::Uri};
use chrono::{DateTime, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::variable_service::VariableInstance;
use flowable_engine::persistence::runtime_store::{EventSubscriptionKind, RuntimeEventWaitState};
use flowable_engine::repository::process_definition::ProcessDefinition;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::task::Task;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::variable_types::rest_variable_type;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProcessInstanceListQuery {
    start: usize,
    size: Option<usize>,
    sort: Option<String>,
    order: Option<String>,
    id: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    // Java ProcessInstanceQueryRequest.java:33 `processInstanceIds` — POST-only
    // in Java (the GET handler never reads it); accepted on GET as a superset.
    #[serde(
        rename = "processInstanceIds",
        deserialize_with = "deserialize_optional_string_list"
    )]
    process_instance_ids: Option<Vec<String>>,
    #[serde(rename = "processDefinitionId")]
    process_definition_id: Option<String>,
    // Java ProcessInstanceQueryRequest.java:44 `processDefinitionIds`.
    #[serde(
        rename = "processDefinitionIds",
        deserialize_with = "deserialize_optional_string_list"
    )]
    process_definition_ids: Option<Vec<String>>,
    // Java ProcessInstanceQueryRequest.java:34-36 `processInstanceName*` — the
    // legacy GET-style `name*` names are kept as primary with the Java POST body
    // names as serde aliases so both JSON spellings work (P111 dual-name).
    #[serde(rename = "name", alias = "processInstanceName")]
    name: Option<String>,
    #[serde(rename = "nameLike", alias = "processInstanceNameLike")]
    name_like: Option<String>,
    #[serde(rename = "nameLikeIgnoreCase", alias = "processInstanceNameLikeIgnoreCase")]
    name_like_ignore_case: Option<String>,
    #[serde(rename = "processDefinitionName")]
    process_definition_name: Option<String>,
    #[serde(rename = "processDefinitionNameLike")]
    process_definition_name_like: Option<String>,
    #[serde(rename = "processDefinitionNameLikeIgnoreCase")]
    process_definition_name_like_ignore_case: Option<String>,
    #[serde(rename = "processDefinitionKey")]
    process_definition_key: Option<String>,
    #[serde(rename = "processDefinitionKeyLike")]
    process_definition_key_like: Option<String>,
    #[serde(rename = "processDefinitionKeyLikeIgnoreCase")]
    process_definition_key_like_ignore_case: Option<String>,
    // Java ProcessInstanceQueryRequest.java:48-49 `processDefinitionKeys` /
    // `excludeProcessDefinitionKeys` — POST-only in Java.
    #[serde(
        rename = "processDefinitionKeys",
        deserialize_with = "deserialize_optional_string_list"
    )]
    process_definition_keys: Option<Vec<String>>,
    #[serde(
        rename = "excludeProcessDefinitionKeys",
        deserialize_with = "deserialize_optional_string_list"
    )]
    exclude_process_definition_keys: Option<Vec<String>>,
    #[serde(rename = "processDefinitionVersion")]
    process_definition_version: Option<i32>,
    // Java ProcessInstanceCollectionResource.java:171-181 processDefinitionCategory*
    // and :187 processDefinitionEngineVersion — joined through the process
    // definition repository entry (ProcessInstance has no category column).
    #[serde(rename = "processDefinitionCategory")]
    process_definition_category: Option<String>,
    #[serde(rename = "processDefinitionCategoryLike")]
    process_definition_category_like: Option<String>,
    #[serde(rename = "processDefinitionCategoryLikeIgnoreCase")]
    process_definition_category_like_ignore_case: Option<String>,
    #[serde(rename = "processDefinitionEngineVersion")]
    process_definition_engine_version: Option<String>,
    // Java ProcessInstanceQueryRequest.java:59-60 `deploymentId`/`deploymentIdIn` —
    // POST-only; also joined through the process definition.
    #[serde(rename = "deploymentId")]
    deployment_id: Option<String>,
    #[serde(
        rename = "deploymentIdIn",
        deserialize_with = "deserialize_optional_string_list"
    )]
    deployment_id_in: Option<Vec<String>>,
    // Java ProcessInstanceQueryRequest.java:37-42 `processBusinessKey*` /
    // `processBusinessStatus*` — legacy GET-style names kept as primary with
    // the Java POST body names as serde aliases (P111 dual-name).
    #[serde(rename = "businessKey", alias = "processBusinessKey")]
    business_key: Option<String>,
    #[serde(rename = "businessKeyLike", alias = "processBusinessKeyLike")]
    business_key_like: Option<String>,
    #[serde(rename = "businessKeyLikeIgnoreCase", alias = "processBusinessKeyLikeIgnoreCase")]
    business_key_like_ignore_case: Option<String>,
    #[serde(rename = "businessStatus", alias = "processBusinessStatus")]
    business_status: Option<String>,
    #[serde(rename = "businessStatusLike", alias = "processBusinessStatusLike")]
    business_status_like: Option<String>,
    #[serde(rename = "businessStatusLikeIgnoreCase", alias = "processBusinessStatusLikeIgnoreCase")]
    business_status_like_ignore_case: Option<String>,
    // Java ProcessInstanceQueryRequest.java:58-59: CMMN scope semantics,
    // accepted without effect in the BPMN-only store (tasks.rs:206-210
    // accept-but-documented precedent).
    #[serde(rename = "rootScopeId")]
    root_scope_id: Option<String>,
    #[serde(rename = "parentScopeId")]
    parent_scope_id: Option<String>,
    // Java ProcessInstanceCollectionResource.java:247-257: call-activity
    // hierarchy filters, resolved through the executions store.
    #[serde(rename = "superProcessInstanceId")]
    super_process_instance_id: Option<String>,
    #[serde(rename = "subProcessInstanceId")]
    sub_process_instance_id: Option<String>,
    #[serde(rename = "excludeSubprocesses")]
    exclude_subprocesses: Option<bool>,
    // Java ProcessInstanceCollectionResource.java:227 activeActivityId and
    // ProcessInstanceQueryRequest.java:66 activeActivityIds — active-activity
    // filtering over the executions store.
    #[serde(rename = "activeActivityId")]
    active_activity_id: Option<String>,
    #[serde(
        rename = "activeActivityIds",
        deserialize_with = "deserialize_optional_string_list"
    )]
    active_activity_ids: Option<Vec<String>>,
    #[serde(rename = "startedBy")]
    started_by: Option<String>,
    #[serde(rename = "involvedUser")]
    involved_user: Option<String>,
    // Java ProcessInstanceQueryRequest.java:78: CMMN parent-case semantics,
    // accepted without effect in the BPMN-only store (tasks.rs precedent).
    #[serde(rename = "parentCaseInstanceId")]
    parent_case_instance_id: Option<String>,
    #[serde(rename = "startedBefore")]
    started_before: Option<String>,
    #[serde(rename = "startedAfter")]
    started_after: Option<String>,
    suspended: Option<bool>,
    #[serde(rename = "callbackId")]
    callback_id: Option<String>,
    // Java ProcessInstanceCollectionResource.java:271-273 callbackIds (GET,
    // RequestUtil.parseToSet) and ProcessInstanceQueryRequest.java:77 (POST Set).
    #[serde(
        rename = "callbackIds",
        deserialize_with = "deserialize_optional_string_list"
    )]
    callback_ids: Option<Vec<String>>,
    #[serde(rename = "callbackType")]
    callback_type: Option<String>,
    #[serde(rename = "referenceId")]
    reference_id: Option<String>,
    #[serde(rename = "referenceType")]
    reference_type: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    #[serde(rename = "tenantIdLikeIgnoreCase")]
    tenant_id_like_ignore_case: Option<String>,
    #[serde(rename = "withoutTenantId")]
    without_tenant_id: Option<bool>,
    variables: Option<Vec<QueryVariable>>,
    #[serde(rename = "processInstanceVariables")]
    process_instance_variables: Option<Vec<QueryVariable>>,
    #[serde(rename = "includeProcessVariables")]
    include_process_variables: Option<bool>,
    // Java ProcessInstanceCollectionResource.java:263-265 includeProcessVariablesNames
    // (RequestUtil.parseToList) — response assembly: include only the named
    // process variables, implying includeProcessVariables.
    #[serde(
        rename = "includeProcessVariablesNames",
        deserialize_with = "deserialize_optional_string_list"
    )]
    include_process_variables_names: Option<Vec<String>>,
}

impl ProcessInstanceListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

pub(crate) async fn list_process_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ProcessInstanceResponse>>, ApiError> {
    let query: ProcessInstanceListQuery = parse_query(&uri)?;
    Ok(Json(query_process_instances_from_store(engine, query)?))
}

pub(crate) async fn query_process_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<ProcessInstanceResponse>>, ApiError> {
    let mut query: ProcessInstanceListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: ProcessInstanceListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);
    query.include_process_variables = url_query
        .include_process_variables
        .or(query.include_process_variables);

    Ok(Json(query_process_instances_from_store(engine, query)?))
}

fn query_process_instances_from_store(
    engine: Arc<ProcessEngine>,
    query: ProcessInstanceListQuery,
) -> Result<PagedResponse<ProcessInstanceResponse>, ApiError> {
    let store = engine.get_runtime_store();
    let mut instances = store
        .db_store()
        .find_all::<flowable_engine::runtime::process_instance::ProcessInstance>(
            "process_instances",
        )
        .unwrap();

    if let Some(process_instance_id) = query.id.as_deref().or(query.process_instance_id.as_deref())
    {
        instances.retain(|instance| instance.id == process_instance_id);
    }
    // Java ProcessInstanceQueryRequest.java:33 `processInstanceIds`.
    if let Some(process_instance_ids) = query.process_instance_ids.as_ref() {
        let process_instance_ids = process_instance_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        instances.retain(|instance| process_instance_ids.contains(instance.id.as_str()));
    }
    if let Some(process_definition_id) = query.process_definition_id.as_deref() {
        instances.retain(|instance| instance.process_definition_id == process_definition_id);
    }
    // Java ProcessInstanceQueryRequest.java:44 `processDefinitionIds`.
    if let Some(process_definition_ids) = query.process_definition_ids.as_ref() {
        let process_definition_ids = process_definition_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        instances.retain(|instance| {
            process_definition_ids.contains(instance.process_definition_id.as_str())
        });
    }
    if let Some(name) = query.name.as_deref() {
        instances.retain(|instance| instance.name.as_deref() == Some(name));
    }
    if let Some(name_like) = query.name_like.as_deref() {
        instances.retain(|instance| string_field_like(instance.name.as_deref(), name_like));
    }
    if let Some(name_like_ignore_case) = query.name_like_ignore_case.as_deref() {
        instances.retain(|instance| {
            string_field_contains_ignore_case(instance.name.as_deref(), name_like_ignore_case)
        });
    }
    if let Some(process_definition_name) = query.process_definition_name.as_deref() {
        instances.retain(|instance| {
            instance.process_definition_name.as_deref() == Some(process_definition_name)
        });
    }
    if let Some(process_definition_name_like) = query.process_definition_name_like.as_deref() {
        instances.retain(|instance| {
            string_field_like(
                instance.process_definition_name.as_deref(),
                process_definition_name_like,
            )
        });
    }
    if let Some(process_definition_name_like_ignore_case) =
        query.process_definition_name_like_ignore_case.as_deref()
    {
        instances.retain(|instance| {
            string_field_contains_ignore_case(
                instance.process_definition_name.as_deref(),
                process_definition_name_like_ignore_case,
            )
        });
    }
    if let Some(process_definition_key) = query.process_definition_key.as_deref() {
        instances.retain(|instance| instance.process_definition_key == process_definition_key);
    }
    if let Some(process_definition_key_like) = query.process_definition_key_like.as_deref() {
        instances.retain(|instance| {
            sql_like_matches(
                process_definition_key_like,
                &instance.process_definition_key,
            )
        });
    }
    if let Some(process_definition_key_like_ignore_case) =
        query.process_definition_key_like_ignore_case.as_deref()
    {
        instances.retain(|instance| {
            sql_like_matches_ignore_case(
                process_definition_key_like_ignore_case,
                &instance.process_definition_key,
            )
        });
    }
    // Java ProcessInstanceQueryRequest.java:48-49.
    if let Some(process_definition_keys) = query.process_definition_keys.as_ref() {
        let process_definition_keys = process_definition_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        instances.retain(|instance| {
            process_definition_keys.contains(instance.process_definition_key.as_str())
        });
    }
    if let Some(exclude_process_definition_keys) = query.exclude_process_definition_keys.as_ref() {
        let exclude_process_definition_keys = exclude_process_definition_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        instances.retain(|instance| {
            !exclude_process_definition_keys.contains(instance.process_definition_key.as_str())
        });
    }
    if let Some(process_definition_version) = query.process_definition_version {
        instances
            .retain(|instance| instance.process_definition_version == process_definition_version);
    }
    // Java ProcessInstanceCollectionResource.java:171-181 processDefinitionCategory*,
    // :187 processDefinitionEngineVersion, ProcessInstanceQueryRequest.java:59-60
    // deploymentId/deploymentIdIn — all joined through the process definition
    // (the ProcessInstance entity carries no category/engine-version/deployment).
    let needs_process_definition_meta = query.process_definition_category.is_some()
        || query.process_definition_category_like.is_some()
        || query.process_definition_category_like_ignore_case.is_some()
        || query.process_definition_engine_version.is_some()
        || query.deployment_id.is_some()
        || query.deployment_id_in.is_some();
    if needs_process_definition_meta {
        let definitions = process_definition_meta_by_id(&engine)?;
        if let Some(category) = query.process_definition_category.as_deref() {
            instances.retain(|instance| {
                definitions
                    .get(&instance.process_definition_id)
                    .and_then(|definition| definition.category.as_deref())
                    == Some(category)
            });
        }
        if let Some(pattern) = query.process_definition_category_like.as_deref() {
            instances.retain(|instance| {
                definitions
                    .get(&instance.process_definition_id)
                    .and_then(|definition| definition.category.as_deref())
                    .is_some_and(|category| sql_like_matches(pattern, category))
            });
        }
        if let Some(pattern) = query.process_definition_category_like_ignore_case.as_deref() {
            let pattern = pattern.to_lowercase();
            instances.retain(|instance| {
                definitions
                    .get(&instance.process_definition_id)
                    .and_then(|definition| definition.category.as_deref())
                    .is_some_and(|category| sql_like_matches(&pattern, &category.to_lowercase()))
            });
        }
        if let Some(engine_version) = query.process_definition_engine_version.as_deref() {
            instances.retain(|instance| {
                definitions
                    .get(&instance.process_definition_id)
                    .and_then(|definition| definition.engine_version.as_deref())
                    == Some(engine_version)
            });
        }
        if let Some(deployment_id) = query.deployment_id.as_deref() {
            instances.retain(|instance| {
                definitions
                    .get(&instance.process_definition_id)
                    .and_then(|definition| definition.deployment_id.as_deref())
                    == Some(deployment_id)
            });
        }
        if let Some(deployment_ids) = query.deployment_id_in.as_ref() {
            let deployment_ids = deployment_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            instances.retain(|instance| {
                definitions
                    .get(&instance.process_definition_id)
                    .and_then(|definition| definition.deployment_id.as_deref())
                    .is_some_and(|deployment_id| deployment_ids.contains(deployment_id))
            });
        }
    }
    if let Some(business_key) = query.business_key.as_deref() {
        instances.retain(|instance| instance.business_key.as_deref() == Some(business_key));
    }
    if let Some(business_key_like) = query.business_key_like.as_deref() {
        instances.retain(|instance| {
            string_field_like(instance.business_key.as_deref(), business_key_like)
        });
    }
    if let Some(business_key_like_ignore_case) = query.business_key_like_ignore_case.as_deref() {
        instances.retain(|instance| {
            string_field_contains_ignore_case(
                instance.business_key.as_deref(),
                business_key_like_ignore_case,
            )
        });
    }
    if let Some(business_status) = query.business_status.as_deref() {
        instances.retain(|instance| instance.business_status.as_deref() == Some(business_status));
    }
    if let Some(business_status_like) = query.business_status_like.as_deref() {
        instances.retain(|instance| {
            string_field_like(instance.business_status.as_deref(), business_status_like)
        });
    }
    if let Some(business_status_like_ignore_case) =
        query.business_status_like_ignore_case.as_deref()
    {
        instances.retain(|instance| {
            string_field_contains_ignore_case(
                instance.business_status.as_deref(),
                business_status_like_ignore_case,
            )
        });
    }
    if let Some(started_by) = query.started_by.as_deref() {
        instances.retain(|instance| instance.start_user_id.as_deref() == Some(started_by));
    }
    if let Some(involved_user) = query.involved_user.as_deref() {
        let involved_process_instance_ids: HashSet<String> = engine
            .get_identity_link_service()
            .create_identity_link_query()
            .user_id(involved_user.to_string())
            .list()
            .map_err(|error| ApiError::InternalServerError(error.to_string()))?
            .into_iter()
            .filter_map(|link| link.process_instance_id)
            .collect();
        instances.retain(|instance| involved_process_instance_ids.contains(&instance.id));
    }
    if let Some(started_before) = query.started_before.as_deref() {
        let started_before = parse_query_datetime(started_before, "startedBefore")?;
        instances.retain(|instance| {
            instance
                .start_time
                .is_some_and(|time| time < started_before)
        });
    }
    if let Some(started_after) = query.started_after.as_deref() {
        let started_after = parse_query_datetime(started_after, "startedAfter")?;
        instances.retain(|instance| instance.start_time.is_some_and(|time| time > started_after));
    }
    if let Some(suspended) = query.suspended {
        instances.retain(|instance| instance.is_suspended == suspended);
    }
    // Java Execution.xml:897-906 activeActivityId / activeActivityIds and
    // Execution.xml:819-826 super/sub process-instance links — both resolved
    // over the executions store (shared load).
    let needs_execution_meta = query.active_activity_id.is_some()
        || query.active_activity_ids.is_some()
        || query.super_process_instance_id.is_some()
        || query.sub_process_instance_id.is_some();
    let executions = if needs_execution_meta {
        Some(
            store
                .db_store()
                .find_all::<Execution>("executions")
                .unwrap(),
        )
    } else {
        None
    };
    if let Some(executions) = executions.as_deref() {
        if let Some(active_activity_id) = query.active_activity_id.as_deref() {
            instances.retain(|instance| {
                executions.iter().any(|execution| {
                    execution.process_instance_id.as_deref() == Some(instance.id.as_str())
                        && execution.activity_id.as_deref() == Some(active_activity_id)
                        && !execution.is_ended
                        && !execution.is_suspended
                })
            });
        }
        if let Some(active_activity_ids) = query.active_activity_ids.as_ref() {
            let active_activity_ids = active_activity_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            instances.retain(|instance| {
                executions.iter().any(|execution| {
                    execution.process_instance_id.as_deref() == Some(instance.id.as_str())
                        && execution
                            .activity_id
                            .as_deref()
                            .is_some_and(|id| active_activity_ids.contains(id))
                        && !execution.is_ended
                        && !execution.is_suspended
                })
            });
        }
        if let Some(super_process_instance_id) = query.super_process_instance_id.as_deref() {
            // Java: RES.SUPER_EXEC_ IN (select ID_ from ACT_RU_EXECUTION where PROC_INST_ID_ = X).
            let super_execution_ids = executions
                .iter()
                .filter(|execution| {
                    execution.process_instance_id.as_deref() == Some(super_process_instance_id)
                })
                .map(|execution| execution.id.as_str())
                .collect::<HashSet<_>>();
            instances.retain(|instance| {
                instance
                    .super_execution_id
                    .as_deref()
                    .is_some_and(|id| super_execution_ids.contains(id))
            });
        }
        if let Some(sub_process_instance_id) = query.sub_process_instance_id.as_deref() {
            // Java: RES.ID_ = (select PROC_INST_ID_ from EXECUTION where ID_ =
            //   (select SUPER_EXEC_ from EXECUTION where ID_ = X)).
            let parent_process_instance_id = executions
                .iter()
                .find(|execution| execution.id == sub_process_instance_id)
                .and_then(|execution| execution.super_execution_id.as_deref())
                .and_then(|super_execution_id| {
                    executions.iter().find(|execution| execution.id == super_execution_id)
                })
                .and_then(|execution| execution.process_instance_id.as_deref());
            instances.retain(|instance| {
                Some(instance.id.as_str()) == parent_process_instance_id
            });
        }
    }
    // Java Execution.xml:827-829 excludeSubprocesses — no super execution.
    if query.exclude_subprocesses == Some(true) {
        instances.retain(|instance| instance.super_execution_id.is_none());
    }
    if let Some(callback_id) = query.callback_id.as_deref() {
        instances.retain(|instance| instance.callback_id.as_deref() == Some(callback_id));
    }
    // Java Execution.xml:841-846 callbackIds.
    if let Some(callback_ids) = query.callback_ids.as_ref() {
        let callback_ids = callback_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        instances.retain(|instance| {
            instance
                .callback_id
                .as_deref()
                .is_some_and(|id| callback_ids.contains(id))
        });
    }
    if let Some(callback_type) = query.callback_type.as_deref() {
        instances.retain(|instance| instance.callback_type.as_deref() == Some(callback_type));
    }
    if let Some(reference_id) = query.reference_id.as_deref() {
        instances.retain(|instance| instance.reference_id.as_deref() == Some(reference_id));
    }
    if let Some(reference_type) = query.reference_type.as_deref() {
        instances.retain(|instance| instance.reference_type.as_deref() == Some(reference_type));
    }
    // accept-but-documented (tasks.rs:206-210 precedent): CMMN scope semantics
    // — `rootScopeId`/`parentScopeId`/`parentCaseInstanceId` are accepted for
    // JSON parity but have no effect in the BPMN-only store.
    let _ = (
        query.root_scope_id.as_deref(),
        query.parent_scope_id.as_deref(),
        query.parent_case_instance_id.as_deref(),
    );
    if query.without_tenant_id.unwrap_or(false) && query.tenant_id.is_some() {
        return Err(ApiError::BadRequest(
            "tenantId and withoutTenantId cannot be used together".to_string(),
        ));
    }
    if let Some(tenant_id) = query.tenant_id.as_deref() {
        instances.retain(|instance| instance.tenant_id.as_deref() == Some(tenant_id));
    }
    if let Some(tenant_id_like) = query.tenant_id_like.as_deref() {
        instances
            .retain(|instance| string_field_like(instance.tenant_id.as_deref(), tenant_id_like));
    }
    if let Some(tenant_id_like_ignore_case) = query.tenant_id_like_ignore_case.as_deref() {
        instances.retain(|instance| {
            string_field_contains_ignore_case(
                instance.tenant_id.as_deref(),
                tenant_id_like_ignore_case,
            )
        });
    }
    if query.without_tenant_id.unwrap_or(false) {
        instances.retain(|instance| instance.tenant_id.is_none());
    }
    if let Some(variables) = query.variables.as_ref() {
        let variable_instances = engine
            .get_variable_service()
            .create_variable_instance_query()
            .list()?;
        apply_process_instance_variable_filters(&mut instances, &variable_instances, variables)?;
    }
    if let Some(process_instance_variables) = query.process_instance_variables.as_ref() {
        let variable_instances = engine
            .get_variable_service()
            .create_variable_instance_query()
            .list()?;
        apply_process_instance_variable_filters(
            &mut instances,
            &variable_instances,
            process_instance_variables,
        )?;
    }

    sort_process_instances(
        &mut instances,
        query.sort.as_deref(),
        query.order.as_deref(),
    )?;

    let include_process_variables = query.include_process_variables == Some(true);
    let included_process_variable_names = query.include_process_variables_names.as_ref().map(
        |names| names.iter().map(String::as_str).collect::<HashSet<_>>(),
    );
    let result = instances
        .into_iter()
        .map(|instance| {
            if include_process_variables || included_process_variable_names.is_some() {
                to_process_instance_response_with_process_variables(
                    &engine,
                    instance,
                    included_process_variable_names.as_ref(),
                )
            } else {
                Ok(to_process_instance_response(instance))
            }
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(query.paging().paginate(result))
}

fn to_process_instance_response_with_process_variables(
    engine: &ProcessEngine,
    instance: flowable_engine::runtime::process_instance::ProcessInstance,
    included_process_variable_names: Option<&HashSet<&str>>,
) -> Result<ProcessInstanceResponse, ApiError> {
    let mut variables = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()?
        .into_iter()
        .filter(|variable| variable.process_instance_id == instance.id.as_str())
        .filter(|variable| {
            included_process_variable_names.is_none_or(|names| {
                names.contains(variable.name.as_str())
            })
        })
        .map(|variable| {
            let mut response =
                super::process_instances::to_rest_variable_response(variable.name, variable.value);
            response.scope = "global".to_string();
            response
        })
        .collect::<Vec<_>>();
    variables.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(ProcessInstanceResponse {
        variables: Some(variables),
        ..to_process_instance_response(instance)
    })
}

fn apply_process_instance_variable_filters(
    instances: &mut Vec<flowable_engine::runtime::process_instance::ProcessInstance>,
    variable_instances: &[VariableInstance],
    variables: &[QueryVariable],
) -> Result<(), ApiError> {
    for variable in variables {
        let operation = parse_query_variable_operation(variable)?;
        let value = variable_value(variable)?;
        let name = variable.name.as_deref();

        validate_name_less_equals(name, operation)?;
        validate_operation_value(operation, value)?;

        instances.retain(|instance| {
            variable_instances
                .iter()
                .filter(|candidate| candidate.process_instance_id == instance.id.as_str())
                .any(|candidate| variable_instance_matches(candidate, name, operation, value))
        });
    }

    Ok(())
}

fn process_definition_meta_by_id(
    engine: &ProcessEngine,
) -> Result<HashMap<String, ProcessDefinition>, ApiError> {
    Ok(engine
        .get_repository_service()
        .get_process_definitions()?
        .into_iter()
        .map(|definition| (definition.id.clone(), definition))
        .collect())
}

fn string_field_like(value: Option<&str>, pattern: &str) -> bool {
    value.is_some_and(|value| sql_like_matches(pattern, value))
}

fn string_field_contains_ignore_case(value: Option<&str>, pattern: &str) -> bool {
    value.is_some_and(|value| sql_like_matches_ignore_case(pattern, value))
}

fn sql_like_matches_ignore_case(pattern: &str, value: &str) -> bool {
    sql_like_matches(&pattern.to_lowercase(), &value.to_lowercase())
}

/// Delegates to the shared O(pattern × value) matcher with the 512-char cap
/// (`routes::tasks::sql_like_matches`); the former recursive matcher here had
/// exponential worst cases on `%`-heavy patterns.
fn sql_like_matches(pattern: &str, value: &str) -> bool {
    crate::routes::tasks::sql_like_matches(pattern, value)
}

fn parse_query_datetime(value: &str, field: &str) -> Result<DateTime<Utc>, ApiError> {
    parse_rfc3339_datetime(value, field)
}

fn sort_process_instances(
    instances: &mut [flowable_engine::runtime::process_instance::ProcessInstance],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    match sort {
        None | Some("id") => instances.sort_by(|left, right| left.id.cmp(&right.id)),
        Some("name") => {
            instances.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)))
        }
        Some("processDefinitionId") => instances.sort_by(|left, right| {
            left.process_definition_id
                .cmp(&right.process_definition_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("processDefinitionKey") => instances.sort_by(|left, right| {
            left.process_definition_key
                .cmp(&right.process_definition_key)
                .then(left.id.cmp(&right.id))
        }),
        Some("businessKey") => instances.sort_by(|left, right| {
            left.business_key
                .cmp(&right.business_key)
                .then(left.id.cmp(&right.id))
        }),
        Some("startTime") => instances.sort_by(|left, right| {
            left.start_time
                .cmp(&right.start_time)
                .then(left.id.cmp(&right.id))
        }),
        Some("tenantId") => instances.sort_by(|left, right| {
            left.tenant_id
                .cmp(&right.tenant_id)
                .then(left.id.cmp(&right.id))
        }),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported process instance sort field '{other}'"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => instances.reverse(),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported process instance sort order '{other}'"
            )));
        }
    }

    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ExecutionListQuery {
    start: usize,
    size: Option<usize>,
    sort: Option<String>,
    order: Option<String>,
    id: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    /// P133: Java `ActivityInstanceCollectionResource.java:72-73` filters by
    /// activity instance id. Rust activity-instances are projected from
    /// executions (`id`/`executionId` == activity instance id), so this is a
    /// pure alias for the execution id filter.
    #[serde(rename = "activityInstanceId")]
    activity_instance_id: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(
        rename = "processInstanceIds",
        deserialize_with = "deserialize_optional_string_list"
    )]
    process_instance_ids: Option<Vec<String>>,
    #[serde(rename = "processInstanceBusinessKey")]
    process_instance_business_key: Option<String>,
    #[serde(rename = "processDefinitionId")]
    process_definition_id: Option<String>,
    #[serde(rename = "processDefinitionKey")]
    process_definition_key: Option<String>,
    #[serde(rename = "activityId")]
    activity_id: Option<String>,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
    #[serde(rename = "messageEventSubscriptionName")]
    message_event_subscription_name: Option<String>,
    #[serde(rename = "signalEventSubscriptionName")]
    signal_event_subscription_name: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    #[serde(rename = "withoutTenantId")]
    without_tenant_id: Option<bool>,
    variables: Option<Vec<QueryVariable>>,
    #[serde(rename = "processInstanceVariables")]
    process_instance_variables: Option<Vec<QueryVariable>>,
}

impl ExecutionListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryVariable {
    name: Option<String>,
    operation: Option<String>,
    value: Option<serde_json::Value>,
    /// Java `QueryVariable.type` (QueryVariable.java:66-71): accepted for JSON
    /// parity but not used for value conversion — matching is driven by the
    /// JSON value shape (P108 deviation, see query_variable.rs:21-27).
    #[serde(rename = "type")]
    _variable_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionResponse {
    id: String,
    process_instance_id: Option<String>,
    process_definition_id: Option<String>,
    activity_id: Option<String>,
    parent_id: Option<String>,
    is_active: bool,
    is_ended: bool,
    is_suspended: bool,
}

pub(crate) fn to_execution_response(execution: Execution) -> ExecutionResponse {
    ExecutionResponse {
        id: execution.id,
        process_instance_id: execution.process_instance_id,
        process_definition_id: execution.process_definition_id,
        activity_id: execution.activity_id,
        parent_id: execution.parent_id,
        is_active: execution.is_active,
        is_ended: execution.is_ended,
        is_suspended: execution.is_suspended,
    }
}

pub(crate) async fn query_executions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<ExecutionResponse>>, ApiError> {
    let mut query: ExecutionListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: ExecutionListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);

    Ok(Json(executions_for_query(engine, query)?))
}

pub(crate) async fn list_executions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ExecutionResponse>>, ApiError> {
    let query: ExecutionListQuery = parse_query(&uri)?;
    Ok(Json(executions_for_query(engine, query)?))
}

pub(crate) async fn get_execution(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
) -> Result<Json<ExecutionResponse>, ApiError> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let execution = store
        .find_execution(&execution_id, &mut session)
        .ok_or_else(|| ApiError::NotFound(format!("Execution '{}' was not found", execution_id)))?;

    Ok(Json(to_execution_response(execution)))
}

pub(crate) async fn get_execution_active_activities(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(execution_id): Path<String>,
) -> Result<Json<Vec<String>>, ApiError> {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store
        .find_execution(&execution_id, &mut session)
        .ok_or_else(|| ApiError::NotFound(format!("Execution '{}' was not found", execution_id)))?;

    let executions = runtime_store
        .db_store()
        .find_all::<Execution>("executions")
        .unwrap();
    let mut active_activity_ids = executions
        .iter()
        .filter(|execution| {
            !execution.is_ended
                && !execution.is_suspended
                && execution.activity_id.is_some()
                && is_execution_or_descendant(&executions, execution, &execution_id)
        })
        .filter_map(|execution| execution.activity_id.clone())
        .collect::<Vec<_>>();
    active_activity_ids.sort();
    active_activity_ids.dedup();

    Ok(Json(active_activity_ids))
}

fn is_execution_or_descendant(
    executions: &[Execution],
    execution: &Execution,
    ancestor_id: &str,
) -> bool {
    let mut current = Some(execution.id.as_str());
    let mut visited = HashSet::new();

    while let Some(current_id) = current {
        if current_id == ancestor_id {
            return true;
        }
        if !visited.insert(current_id.to_string()) {
            return false;
        }
        current = executions
            .iter()
            .find(|candidate| candidate.id == current_id)
            .and_then(|candidate| candidate.parent_id.as_deref());
    }

    false
}

fn executions_for_query(
    engine: Arc<ProcessEngine>,
    query: ExecutionListQuery,
) -> Result<PagedResponse<ExecutionResponse>, ApiError> {
    let mut executions = engine
        .get_runtime_store()
        .db_store()
        .find_all::<Execution>("executions")
        .unwrap();

    if query.without_tenant_id.unwrap_or(false) && query.tenant_id.is_some() {
        return Err(ApiError::bad_request(
            "tenantId and withoutTenantId cannot be used together",
        ));
    }

    // P133: activityInstanceId aliases execution id (ActivityInstanceCollectionResource.java:72-73)
    if let Some(execution_id) = query
        .id
        .as_deref()
        .or(query.execution_id.as_deref())
        .or(query.activity_instance_id.as_deref())
    {
        executions.retain(|execution| execution.id == execution_id);
    }
    if let Some(process_instance_id) = query.process_instance_id.as_deref() {
        executions.retain(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                || execution.root_process_instance_id.as_deref() == Some(process_instance_id)
        });
    }
    if let Some(process_instance_ids) = query.process_instance_ids.as_ref() {
        let process_instance_ids = process_instance_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        executions.retain(|execution| {
            execution
                .process_instance_id
                .as_deref()
                .is_some_and(|id| process_instance_ids.contains(id))
                || execution
                    .root_process_instance_id
                    .as_deref()
                    .is_some_and(|id| process_instance_ids.contains(id))
        });
    }
    if let Some(process_definition_id) = query.process_definition_id.as_deref() {
        executions.retain(|execution| {
            execution.process_definition_id.as_deref() == Some(process_definition_id)
        });
    }
    if let Some(process_definition_key) = query.process_definition_key.as_deref() {
        executions.retain(|execution| {
            execution.process_definition_key.as_deref() == Some(process_definition_key)
        });
    }
    if let Some(process_instance_business_key) = query.process_instance_business_key.as_deref() {
        let process_instances_by_id = engine
            .get_runtime_store()
            .db_store()
            .find_all::<flowable_engine::runtime::process_instance::ProcessInstance>(
                "process_instances",
            )
            .unwrap()
            .into_iter()
            .map(|instance| (instance.id.clone(), instance))
            .collect::<HashMap<_, _>>();
        executions.retain(|execution| {
            execution
                .process_instance_id
                .as_deref()
                .or(execution.root_process_instance_id.as_deref())
                .and_then(|id| process_instances_by_id.get(id))
                .and_then(|instance| instance.business_key.as_deref())
                == Some(process_instance_business_key)
        });
    }
    if let Some(activity_id) = query.activity_id.as_deref() {
        executions.retain(|execution| execution.activity_id.as_deref() == Some(activity_id));
    }
    if let Some(parent_id) = query.parent_id.as_deref() {
        executions.retain(|execution| execution.parent_id.as_deref() == Some(parent_id));
    }
    if let Some(tenant_id) = query.tenant_id.as_deref() {
        executions.retain(|execution| execution.tenant_id.as_deref() == Some(tenant_id));
    }
    if let Some(tenant_id_like) = query.tenant_id_like.as_deref() {
        executions
            .retain(|execution| string_field_like(execution.tenant_id.as_deref(), tenant_id_like));
    }
    if query.without_tenant_id.unwrap_or(false) {
        executions.retain(|execution| execution.tenant_id.is_none());
    }
    if query.message_event_subscription_name.is_some()
        || query.signal_event_subscription_name.is_some()
    {
        let event_wait_states = engine
            .get_runtime_store()
            .db_store()
            .find_all::<RuntimeEventWaitState>("event_wait_states")
            .unwrap();
        if let Some(message_name) = query.message_event_subscription_name.as_deref() {
            executions.retain(|execution| {
                execution_has_event_subscription(
                    &event_wait_states,
                    &execution.id,
                    &EventSubscriptionKind::Message,
                    message_name,
                )
            });
        }
        if let Some(signal_name) = query.signal_event_subscription_name.as_deref() {
            executions.retain(|execution| {
                execution_has_event_subscription(
                    &event_wait_states,
                    &execution.id,
                    &EventSubscriptionKind::Signal,
                    signal_name,
                )
            });
        }
    }
    if let Some(variables) = query.variables.as_ref() {
        let variable_instances = engine
            .get_variable_service()
            .create_variable_instance_query()
            .list()?;
        apply_execution_variable_filters(
            &mut executions,
            &variable_instances,
            variables,
            VariableQueryScope::Execution,
        )?;
    }
    if let Some(process_instance_variables) = query.process_instance_variables.as_ref() {
        let variable_instances = engine
            .get_variable_service()
            .create_variable_instance_query()
            .list()?;
        apply_execution_variable_filters(
            &mut executions,
            &variable_instances,
            process_instance_variables,
            VariableQueryScope::ProcessInstance,
        )?;
    }

    sort_executions(
        &mut executions,
        query.sort.as_deref(),
        query.order.as_deref(),
    )?;
    let result = executions.into_iter().map(to_execution_response).collect();
    Ok(query.paging().paginate(result))
}

#[derive(Debug, Clone, Copy)]
enum VariableQueryScope {
    Execution,
    ProcessInstance,
}

fn apply_execution_variable_filters(
    executions: &mut Vec<Execution>,
    variable_instances: &[VariableInstance],
    variables: &[QueryVariable],
    scope: VariableQueryScope,
) -> Result<(), ApiError> {
    for variable in variables {
        let operation = parse_query_variable_operation(variable)?;
        let value = variable_value(variable)?;
        let name = variable.name.as_deref();

        validate_name_less_equals(name, operation)?;
        validate_operation_value(operation, value)?;

        executions.retain(|execution| {
            variables_for_execution_scope(variable_instances, execution, scope)
                .into_iter()
                .any(|candidate| variable_instance_matches(candidate, name, operation, value))
        });
    }

    Ok(())
}

fn parse_query_variable_operation(
    variable: &QueryVariable,
) -> Result<QueryVariableOperation, ApiError> {
    match variable.operation.as_deref() {
        None => Err(ApiError::bad_request(format!(
            "Variable operation is missing for variable: {}",
            variable.name.as_deref().unwrap_or("null")
        ))),
        Some(name) => QueryVariableOperation::from_friendly_name(name).ok_or_else(|| {
            ApiError::bad_request(format!("Unsupported variable query operation: {name}"))
        }),
    }
}

fn variable_value(variable: &QueryVariable) -> Result<&serde_json::Value, ApiError> {
    variable.value.as_ref().ok_or_else(|| {
        ApiError::bad_request(format!(
            "Variable value is missing for variable: {}",
            variable.name.as_deref().unwrap_or("null")
        ))
    })
}

fn variables_for_execution_scope<'a>(
    variable_instances: &'a [VariableInstance],
    execution: &Execution,
    scope: VariableQueryScope,
) -> Vec<&'a VariableInstance> {
    match scope {
        VariableQueryScope::Execution => variable_instances
            .iter()
            .filter(|variable| variable.execution_id == execution.id)
            .collect(),
        VariableQueryScope::ProcessInstance => {
            let process_instance_id = execution
                .process_instance_id
                .as_deref()
                .or(execution.root_process_instance_id.as_deref());
            variable_instances
                .iter()
                .filter(|variable| {
                    Some(variable.process_instance_id.as_str()) == process_instance_id
                })
                .collect()
        }
    }
}

fn variable_instance_matches(
    variable: &VariableInstance,
    expected_name: Option<&str>,
    operation: QueryVariableOperation,
    expected_value: &serde_json::Value,
) -> bool {
    if expected_name.is_some_and(|name| variable.name != name) {
        return false;
    }

    value_matches(&variable.value, operation, expected_value)
}

fn execution_has_event_subscription(
    event_wait_states: &[RuntimeEventWaitState],
    execution_id: &str,
    kind: &EventSubscriptionKind,
    event_ref: &str,
) -> bool {
    event_wait_states.iter().any(|wait_state| {
        wait_state.execution_id == execution_id
            && wait_state
                .event_subscription
                .as_ref()
                .is_some_and(|subscription| {
                    &subscription.kind == kind && subscription.event_ref == event_ref
                })
    })
}

fn sort_executions(
    executions: &mut [Execution],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    match sort {
        None | Some("processInstanceId") => executions.sort_by(|left, right| {
            left.process_instance_id
                .cmp(&right.process_instance_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("id") => executions.sort_by(|left, right| left.id.cmp(&right.id)),
        Some("processDefinitionId") => executions.sort_by(|left, right| {
            left.process_definition_id
                .cmp(&right.process_definition_id)
                .then(left.id.cmp(&right.id))
        }),
        Some("processDefinitionKey") => executions.sort_by(|left, right| {
            left.process_definition_key
                .cmp(&right.process_definition_key)
                .then(left.id.cmp(&right.id))
        }),
        Some("tenantId") => executions.sort_by(|left, right| {
            left.tenant_id
                .cmp(&right.tenant_id)
                .then(left.id.cmp(&right.id))
        }),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported execution sort field '{other}'"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => executions.reverse(),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported execution sort order '{other}'"
            )));
        }
    }

    Ok(())
}

fn deserialize_optional_string_list<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(value)) => Some(split_string_list(&value)),
        Some(serde_json::Value::Array(values)) => Some(
            values
                .into_iter()
                .map(|value| match value {
                    serde_json::Value::String(value) => value,
                    other => other.to_string(),
                })
                .flat_map(|value| split_string_list(&value))
                .collect(),
        ),
        Some(other) => Some(split_string_list(&other.to_string())),
    })
}

fn split_string_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityInstanceResponse {
    id: String,
    activity_id: String,
    activity_name: Option<String>,
    process_instance_id: Option<String>,
    execution_id: String,
}

pub(crate) async fn query_activity_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<ActivityInstanceResponse>>, ApiError> {
    let mut query: ExecutionListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: ExecutionListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);
    let paging = query.paging();
    let executions_page = executions_for_query(engine, query)?;
    let activity_instances = executions_page
        .data
        .into_iter()
        .filter_map(|execution| {
            execution
                .activity_id
                .clone()
                .map(|activity_id| ActivityInstanceResponse {
                    id: execution.id.clone(),
                    activity_id,
                    activity_name: None,
                    process_instance_id: execution.process_instance_id,
                    execution_id: execution.id,
                })
        })
        .collect();

    Ok(Json(paging.paginate(activity_instances)))
}

pub(crate) async fn list_activity_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ActivityInstanceResponse>>, ApiError> {
    let query: ExecutionListQuery = parse_query(&uri)?;
    let paging = query.paging();
    let executions_page = executions_for_query(engine, query)?;
    let activity_instances = executions_page
        .data
        .into_iter()
        .filter_map(|execution| {
            execution
                .activity_id
                .clone()
                .map(|activity_id| ActivityInstanceResponse {
                    id: execution.id.clone(),
                    activity_id,
                    activity_name: None,
                    process_instance_id: execution.process_instance_id,
                    execution_id: execution.id,
                })
        })
        .collect();

    Ok(Json(paging.paginate(activity_instances)))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct VariableInstanceListQuery {
    start: usize,
    size: Option<usize>,
    sort: Option<String>,
    order: Option<String>,
    #[serde(rename = "processInstanceId")]
    process_instance_id: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    #[serde(rename = "variableName", alias = "name")]
    variable_name: Option<String>,
    #[serde(rename = "variableNameLike")]
    variable_name_like: Option<String>,
    #[serde(rename = "variableType", alias = "type")]
    variable_type: Option<String>,
    #[serde(rename = "excludeTaskVariables")]
    exclude_task_variables: Option<bool>,
    #[serde(rename = "excludeLocalVariables")]
    exclude_local_variables: Option<bool>,
}

impl VariableInstanceListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VariableInstanceResponse {
    id: String,
    name: String,
    #[serde(rename = "type")]
    variable_type: String,
    value: serde_json::Value,
    execution_id: String,
    process_instance_id: String,
    task_id: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimeVariableInstance {
    id: String,
    execution_id: String,
    process_instance_id: String,
    task_id: Option<String>,
    name: String,
    value: serde_json::Value,
    variable_type: String,
}

fn to_runtime_variable_instance(variable: VariableInstance) -> RuntimeVariableInstance {
    let variable_type = rest_variable_type(&variable.value).to_string();
    RuntimeVariableInstance {
        id: variable.id,
        execution_id: variable.execution_id,
        process_instance_id: variable.process_instance_id,
        task_id: None,
        name: variable.name,
        value: variable.value,
        variable_type,
    }
}

fn task_local_variable_instances(tasks: Vec<Task>) -> Vec<RuntimeVariableInstance> {
    tasks
        .into_iter()
        .flat_map(|task| {
            let task_id = task.id;
            let execution_id = task.execution_id;
            let process_instance_id = task.process_instance_id;
            task.local_variables
                .into_iter()
                .map(move |(name, value)| RuntimeVariableInstance {
                    id: format!("task:{task_id}:{name}"),
                    execution_id: execution_id.clone(),
                    process_instance_id: process_instance_id.clone(),
                    task_id: Some(task_id.clone()),
                    variable_type: rest_variable_type(&value).to_string(),
                    name,
                    value,
                })
        })
        .collect()
}

fn to_variable_instance_response(variable: RuntimeVariableInstance) -> VariableInstanceResponse {
    VariableInstanceResponse {
        id: variable.id,
        name: variable.name,
        variable_type: variable.variable_type,
        value: variable.value,
        execution_id: variable.execution_id,
        process_instance_id: variable.process_instance_id,
        task_id: variable.task_id,
    }
}

pub(crate) async fn list_variable_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<VariableInstanceResponse>>, ApiError> {
    let query: VariableInstanceListQuery = parse_query(&uri)?;
    Ok(Json(variable_instances_for_query(engine, query)?))
}

pub(crate) async fn query_variable_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<VariableInstanceResponse>>, ApiError> {
    let mut query: VariableInstanceListQuery =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let url_query: VariableInstanceListQuery = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);

    Ok(Json(variable_instances_for_query(engine, query)?))
}

fn variable_instances_for_query(
    engine: Arc<ProcessEngine>,
    query: VariableInstanceListQuery,
) -> Result<PagedResponse<VariableInstanceResponse>, ApiError> {
    let mut variables = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()?
        .into_iter()
        .map(to_runtime_variable_instance)
        .collect::<Vec<_>>();

    if query.exclude_task_variables != Some(true) && query.exclude_local_variables != Some(true) {
        variables.extend(task_local_variable_instances(
            engine.get_task_service().create_task_query().list()?,
        ));
    }

    if let Some(process_instance_id) = query.process_instance_id.as_deref() {
        variables.retain(|variable| variable.process_instance_id == process_instance_id);
    }
    if let Some(execution_id) = query.execution_id.as_deref() {
        variables.retain(|variable| variable.execution_id == execution_id);
    }
    if let Some(task_id) = query.task_id.as_deref() {
        variables.retain(|variable| variable.task_id.as_deref() == Some(task_id));
    }
    if let Some(variable_name) = query.variable_name.as_deref() {
        variables.retain(|variable| variable.name == variable_name);
    }
    if let Some(variable_name_like) = query.variable_name_like.as_deref() {
        variables.retain(|variable| variable.name.contains(variable_name_like));
    }
    if let Some(variable_type) = query.variable_type.as_deref() {
        variables.retain(|variable| variable.variable_type == variable_type);
    }

    sort_variable_instances(
        &mut variables,
        query.sort.as_deref(),
        query.order.as_deref(),
    )?;

    let result = variables
        .into_iter()
        .map(to_variable_instance_response)
        .collect();

    Ok(query.paging().paginate(result))
}

fn sort_variable_instances(
    variables: &mut [RuntimeVariableInstance],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    let descending = match order {
        None | Some("asc") => false,
        Some("desc") => true,
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported variable instance sort order '{other}'"
            )));
        }
    };

    match sort {
        None | Some("variableName") | Some("name") => variables.sort_by(|left, right| {
            let primary = left.name.cmp(&right.name);
            if descending {
                primary.reverse().then(left.id.cmp(&right.id))
            } else {
                primary.then(left.id.cmp(&right.id))
            }
        }),
        Some("variableType") | Some("type") => variables.sort_by(|left, right| {
            let primary = left.variable_type.cmp(&right.variable_type);
            if descending {
                primary.reverse().then(left.id.cmp(&right.id))
            } else {
                primary.then(left.id.cmp(&right.id))
            }
        }),
        Some("processInstanceId") => variables.sort_by(|left, right| {
            let primary = left.process_instance_id.cmp(&right.process_instance_id);
            if descending {
                primary.reverse().then(left.id.cmp(&right.id))
            } else {
                primary.then(left.id.cmp(&right.id))
            }
        }),
        Some("executionId") => variables.sort_by(|left, right| {
            let primary = left.execution_id.cmp(&right.execution_id);
            if descending {
                primary.reverse().then(left.id.cmp(&right.id))
            } else {
                primary.then(left.id.cmp(&right.id))
            }
        }),
        Some("taskId") => variables.sort_by(|left, right| {
            let primary = left.task_id.cmp(&right.task_id);
            if descending {
                primary.reverse().then(left.id.cmp(&right.id))
            } else {
                primary.then(left.id.cmp(&right.id))
            }
        }),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported variable instance sort field '{other}'"
            )));
        }
    }

    Ok(())
}

pub(crate) async fn get_variable_instance_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(variable_instance_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let variable = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()?
        .into_iter()
        .find(|variable| variable.id == variable_instance_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Variable instance '{}' was not found",
                variable_instance_id
            ))
        })?;

    Ok(Json(variable.value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn query_variable(body: Value) -> QueryVariable {
        serde_json::from_value(body).unwrap()
    }

    fn variable_instance(name: &str, value: Value) -> VariableInstance {
        VariableInstance {
            id: "id".to_string(),
            execution_id: "exec".to_string(),
            process_instance_id: "pi".to_string(),
            name: name.to_string(),
            value,
            variable_type: "string".to_string(),
        }
    }

    #[test]
    fn variable_parse_accepts_all_ten_operations() {
        for name in [
            "equals",
            "notEquals",
            "equalsIgnoreCase",
            "notEqualsIgnoreCase",
            "like",
            "likeIgnoreCase",
            "greaterThan",
            "greaterThanOrEquals",
            "lessThan",
            "lessThanOrEquals",
        ] {
            let variable = query_variable(json!({"name": "v", "operation": name, "value": "x"}));
            let parsed = parse_query_variable_operation(&variable).unwrap();
            assert_eq!(
                QueryVariableOperation::from_friendly_name(name),
                Some(parsed),
                "operation {name}"
            );
        }
    }

    #[test]
    fn variable_illegal_operation_is_400() {
        let variable = query_variable(json!({"name": "v", "operation": "bogusOp", "value": 1}));
        let error = parse_query_variable_operation(&variable).unwrap_err();
        assert!(matches!(
            error,
            ApiError::BadRequest(message) if message == "Unsupported variable query operation: bogusOp"
        ));
    }

    #[test]
    fn variable_nameless_non_equals_and_boolean_comparison_are_400() {
        // Same shared validators the PI/execution filter functions apply.
        let nameless_error = validate_name_less_equals(None, QueryVariableOperation::NotEquals)
            .unwrap_err();
        assert!(matches!(
            nameless_error,
            ApiError::BadRequest(message) if message ==
                "Value-only query (without a variable-name) is only supported when using 'equals' operation."
        ));
        assert!(validate_name_less_equals(Some("v"), QueryVariableOperation::NotEquals).is_ok());

        let bool_error = validate_operation_value(
            QueryVariableOperation::GreaterThanOrEquals,
            &json!(true),
        )
        .unwrap_err();
        assert!(matches!(
            bool_error,
            ApiError::BadRequest(message) if message ==
                "Booleans and null cannot be used in 'greater than or equal' condition"
        ));
        let null_error =
            validate_operation_value(QueryVariableOperation::LessThan, &Value::Null).unwrap_err();
        assert!(matches!(
            null_error,
            ApiError::BadRequest(message) if message ==
                "Booleans and null cannot be used in 'less than' condition"
        ));
    }

    #[test]
    fn variable_instance_like_greater_ignore_case_positive_and_miss() {
        // like: % wildcard matches, miss otherwise.
        assert!(variable_instance_matches(
            &variable_instance("v", json!("HelloWorld")),
            Some("v"),
            QueryVariableOperation::Like,
            &json!("Hello%")
        ));
        assert!(!variable_instance_matches(
            &variable_instance("v", json!("HelloWorld")),
            Some("v"),
            QueryVariableOperation::Like,
            &json!("Nope%")
        ));
        // greaterThan: numeric, miss on equal.
        assert!(variable_instance_matches(
            &variable_instance("n", json!(10)),
            Some("n"),
            QueryVariableOperation::GreaterThan,
            &json!(5)
        ));
        assert!(!variable_instance_matches(
            &variable_instance("n", json!(10)),
            Some("n"),
            QueryVariableOperation::GreaterThan,
            &json!(10)
        ));
        // equalsIgnoreCase: miss on differing value.
        assert!(variable_instance_matches(
            &variable_instance("s", json!("Hello")),
            Some("s"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("hello")
        ));
        assert!(!variable_instance_matches(
            &variable_instance("s", json!("Hello")),
            Some("s"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("world")
        ));
        // Expected-name mismatch never matches.
        assert!(!variable_instance_matches(
            &variable_instance("s", json!("Hello")),
            Some("other"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("hello")
        ));
    }
}
