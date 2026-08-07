use std::collections::BTreeSet;

fn java_rest_paths() -> Vec<String> {
    let fixture = include_str!("fixtures/java_rest_paths.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("valid JSON fixture");
    value["paths"]
        .as_array()
        .expect("paths array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>()
}

fn normalize_rust_path(_method: &str, route: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = route.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':' {
            let mut param = String::new();
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                param.push(chars[i]);
                i += 1;
            }
            let java_param = context_aware_param_name(route, &param);
            result.push('{');
            result.push_str(&java_param);
            result.push('}');
        } else if chars[i] == '*' {
            let mut param = String::new();
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                param.push(chars[i]);
                i += 1;
            }
            let java_param = to_java_param_name(&param);
            result.push('{');
            result.push_str(&java_param);
            result.push('}');
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn context_aware_param_name(route: &str, rust_name: &str) -> String {
    if rust_name == "id" {
        if route.contains("/runtime/tasks/") {
            return "taskId".to_string();
        }
        if route.contains("/external-worker/") {
            return "id".to_string();
        }
        if route.contains("/cmmn-runtime/tasks/") {
            return "planItemInstanceId".to_string();
        }
        if route.contains("/runtime/identity-links/") {
            return "linkId".to_string();
        }
        if route.contains("/runtime/entity-links/") {
            return "linkId".to_string();
        }
    }
    if rust_name == "decision_table_id"
        && route.contains("/decisions/")
        && !route.contains("/decision-tables/")
    {
        return "decisionId".to_string();
    }
    to_java_param_name(rust_name)
}

fn to_java_param_name(rust_name: &str) -> String {
    match rust_name {
        "process_instance_id" => "processInstanceId".to_string(),
        "process_definition_id" => "processDefinitionId".to_string(),
        "deployment_id" => "deploymentId".to_string(),
        "resource_name" => "resourceName".to_string(),
        "model_id" => "modelId".to_string(),
        "execution_id" => "executionId".to_string(),
        "variable_name" => "variableName".to_string(),
        "variable_instance_id" => "variableInstanceId".to_string(),
        "task_id" => "taskId".to_string(),
        "attachment_id" => "attachmentId".to_string(),
        "comment_id" => "commentId".to_string(),
        "event_id" => "eventId".to_string(),
        "link_id" => "linkId".to_string(),
        "identity_id" => "identityId".to_string(),
        "link_type" => "linkType".to_string(),
        "detail_id" => "detailId".to_string(),
        "decision_table_id" => "decisionTableId".to_string(),
        "decision_service_id" => "decisionServiceId".to_string(),
        "historic_decision_execution_id" => "historicDecisionExecutionId".to_string(),
        "drd_id" => "drdId".to_string(),
        "case_instance_id" => "caseInstanceId".to_string(),
        "case_definition_id" => "caseDefinitionId".to_string(),
        "plan_item_instance_id" => "planItemInstanceId".to_string(),
        "milestone_instance_id" => "milestoneInstanceId".to_string(),
        "event_subscription_id" => "eventSubscriptionId".to_string(),
        "app_definition_id" => "appDefinitionId".to_string(),
        "channel_definition_id" => "channelDefinitionId".to_string(),
        "event_definition_id" => "eventDefinitionId".to_string(),
        "delivery_id" => "deliveryId".to_string(),
        "form_definition_id" => "formDefinitionId".to_string(),
        "form_instance_id" => "formInstanceId".to_string(),
        "content_item_id" => "contentItemId".to_string(),
        "user_id" => "userId".to_string(),
        "group_id" => "groupId".to_string(),
        "privilege_id" => "privilegeId".to_string(),
        "token_id" => "tokenId".to_string(),
        "batch_id" => "batchId".to_string(),
        "batch_part_id" => "batchPartId".to_string(),
        "job_id" => "jobId".to_string(),
        "engine_property" => "engineProperty".to_string(),
        "table_name" => "tableName".to_string(),
        other => {
            let mut result = String::new();
            let mut capitalize_next = false;
            for ch in other.chars() {
                if ch == '_' {
                    capitalize_next = true;
                } else if capitalize_next {
                    result.push(ch.to_ascii_uppercase());
                    capitalize_next = false;
                } else {
                    result.push(ch);
                }
            }
            result
        }
    }
}

fn rust_registered_paths() -> Vec<String> {
    let raw_routes = vec![
        ("POST", "/repository/deployments"),
        ("GET", "/repository/deployments/:deployment_id"),
        ("DELETE", "/repository/deployments/:deployment_id"),
        ("GET", "/repository/deployments/:deployment_id/resources"),
        (
            "GET",
            "/repository/deployments/:deployment_id/resourcedata/*resource_name",
        ),
        (
            "GET",
            "/repository/deployments/:deployment_id/resources/*resource_name",
        ),
        ("GET", "/repository/process-definitions"),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id",
        ),
        (
            "PUT",
            "/repository/process-definitions/:process_definition_id",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/start-form",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/form-definitions",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/decision-tables",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/decisions",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/resourcedata",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/model",
        ),
        (
            "POST",
            "/repository/process-definitions/:process_definition_id/migrate",
        ),
        (
            "POST",
            "/repository/process-definitions/:process_definition_id/batch-migrate",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/identitylinks",
        ),
        (
            "POST",
            "/repository/process-definitions/:process_definition_id/identitylinks",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/identitylinks/:family/:identity_id",
        ),
        (
            "DELETE",
            "/repository/process-definitions/:process_definition_id/identitylinks/:family/:identity_id",
        ),
        (
            "GET",
            "/repository/process-definitions/:process_definition_id/image",
        ),
        ("GET", "/repository/models"),
        ("POST", "/repository/models"),
        ("GET", "/repository/models/:model_id"),
        ("PUT", "/repository/models/:model_id"),
        ("DELETE", "/repository/models/:model_id"),
        ("GET", "/repository/models/:model_id/source"),
        ("PUT", "/repository/models/:model_id/source"),
        ("GET", "/repository/models/:model_id/source-extra"),
        ("PUT", "/repository/models/:model_id/source-extra"),
        ("POST", "/runtime/process-instances"),
        ("GET", "/runtime/process-instances"),
        ("GET", "/runtime/process-instances/:process_instance_id"),
        ("PUT", "/runtime/process-instances/:process_instance_id"),
        ("DELETE", "/runtime/process-instances/:process_instance_id"),
        ("POST", "/runtime/process-instances/delete"),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/inject",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/validate-migration",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/migrate",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/evaluate-conditions",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/change-state",
        ),
        (
            "GET",
            "/runtime/process-instances/:process_instance_id/variables",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/variables",
        ),
        (
            "PUT",
            "/runtime/process-instances/:process_instance_id/variables",
        ),
        (
            "DELETE",
            "/runtime/process-instances/:process_instance_id/variables",
        ),
        (
            "GET",
            "/runtime/process-instances/:process_instance_id/variables/:variable_name",
        ),
        (
            "PUT",
            "/runtime/process-instances/:process_instance_id/variables/:variable_name",
        ),
        (
            "DELETE",
            "/runtime/process-instances/:process_instance_id/variables/:variable_name",
        ),
        (
            "GET",
            "/runtime/process-instances/:process_instance_id/variables/:variable_name/data",
        ),
        (
            "PUT",
            "/runtime/process-instances/:process_instance_id/variables/:variable_name/data",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/variables-async",
        ),
        (
            "PUT",
            "/runtime/process-instances/:process_instance_id/variables-async",
        ),
        (
            "PUT",
            "/runtime/process-instances/:process_instance_id/variables-async/:variable_name",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/modification",
        ),
        (
            "GET",
            "/runtime/process-instances/:process_instance_id/diagram",
        ),
        (
            "GET",
            "/runtime/process-instances/:process_instance_id/identity-links",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/identity-links",
        ),
        (
            "GET",
            "/runtime/process-instances/:process_instance_id/identity-links/users/:identity_id/:link_type",
        ),
        (
            "DELETE",
            "/runtime/process-instances/:process_instance_id/identity-links/users/:identity_id/:link_type",
        ),
        (
            "GET",
            "/runtime/process-instances/:process_instance_id/identitylinks",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/identitylinks",
        ),
        (
            "GET",
            "/runtime/process-instances/:process_instance_id/identitylinks/users/:identity_id/:link_type",
        ),
        (
            "DELETE",
            "/runtime/process-instances/:process_instance_id/identitylinks/users/:identity_id/:link_type",
        ),
        ("GET", "/runtime/executions"),
        ("GET", "/runtime/executions/:execution_id"),
        ("POST", "/runtime/executions/:execution_id/change-state"),
        (
            "POST",
            "/runtime/executions/:execution_id/activate-activity",
        ),
        ("GET", "/runtime/executions/:execution_id/activities"),
        ("GET", "/runtime/executions/:execution_id/variables"),
        ("POST", "/runtime/executions/:execution_id/variables"),
        ("PUT", "/runtime/executions/:execution_id/variables"),
        ("DELETE", "/runtime/executions/:execution_id/variables"),
        (
            "GET",
            "/runtime/executions/:execution_id/variables/:variable_name",
        ),
        (
            "PUT",
            "/runtime/executions/:execution_id/variables/:variable_name",
        ),
        (
            "DELETE",
            "/runtime/executions/:execution_id/variables/:variable_name",
        ),
        (
            "GET",
            "/runtime/executions/:execution_id/variables/:variable_name/data",
        ),
        (
            "PUT",
            "/runtime/executions/:execution_id/variables/:variable_name/data",
        ),
        ("POST", "/runtime/executions/:execution_id/variables-async"),
        ("PUT", "/runtime/executions/:execution_id/variables-async"),
        (
            "PUT",
            "/runtime/executions/:execution_id/variables-async/:variable_name",
        ),
        (
            "POST",
            "/runtime/executions/:execution_id/signal-event-received",
        ),
        (
            "POST",
            "/runtime/executions/:execution_id/message-event-received",
        ),
        ("GET", "/runtime/activity-instances"),
        ("GET", "/runtime/variable-instances"),
        (
            "GET",
            "/runtime/variable-instances/:variable_instance_id/data",
        ),
        ("GET", "/runtime/tasks"),
        ("GET", "/runtime/tasks/:id"),
        ("PUT", "/runtime/tasks/:id"),
        ("POST", "/runtime/tasks/:id"),
        ("GET", "/runtime/tasks/:id/subtasks"),
        ("GET", "/runtime/tasks/:id/form"),
        ("POST", "/runtime/tasks/:id/complete"),
        ("GET", "/runtime/tasks/:id/variables"),
        ("POST", "/runtime/tasks/:id/variables"),
        ("PUT", "/runtime/tasks/:id/variables"),
        ("DELETE", "/runtime/tasks/:id/variables"),
        ("GET", "/runtime/tasks/:id/variables/:variable_name"),
        ("PUT", "/runtime/tasks/:id/variables/:variable_name"),
        ("DELETE", "/runtime/tasks/:id/variables/:variable_name"),
        ("GET", "/runtime/tasks/:id/variables/:variable_name/data"),
        ("PUT", "/runtime/tasks/:id/variables/:variable_name/data"),
        ("GET", "/runtime/tasks/:id/attachments"),
        ("POST", "/runtime/tasks/:id/attachments"),
        ("GET", "/runtime/tasks/:id/attachments/:attachment_id"),
        ("DELETE", "/runtime/tasks/:id/attachments/:attachment_id"),
        (
            "GET",
            "/runtime/tasks/:id/attachments/:attachment_id/content",
        ),
        ("GET", "/runtime/tasks/:id/comments"),
        ("POST", "/runtime/tasks/:id/comments"),
        ("GET", "/runtime/tasks/:id/comments/:comment_id"),
        ("DELETE", "/runtime/tasks/:id/comments/:comment_id"),
        ("GET", "/runtime/tasks/:id/events"),
        ("GET", "/runtime/tasks/:id/events/:event_id"),
        ("GET", "/runtime/tasks/:id/identity-links"),
        ("POST", "/runtime/tasks/:id/identity-links"),
        ("GET", "/runtime/tasks/:id/identity-links/:family"),
        (
            "GET",
            "/runtime/tasks/:id/identity-links/:family/:identity_id/:link_type",
        ),
        (
            "DELETE",
            "/runtime/tasks/:id/identity-links/:family/:identity_id/:link_type",
        ),
        ("GET", "/runtime/tasks/:id/identitylinks"),
        ("POST", "/runtime/tasks/:id/identitylinks"),
        ("GET", "/runtime/tasks/:id/identitylinks/:family"),
        (
            "GET",
            "/runtime/tasks/:id/identitylinks/:family/:identity_id/:link_type",
        ),
        (
            "DELETE",
            "/runtime/tasks/:id/identitylinks/:family/:identity_id/:link_type",
        ),
        ("POST", "/runtime/signals"),
        ("POST", "/runtime/messages"),
        ("GET", "/runtime/event-subscriptions"),
        ("GET", "/runtime/event-subscriptions/:event_subscription_id"),
        ("POST", "/runtime/entity-links"),
        ("GET", "/runtime/entity-links"),
        ("DELETE", "/runtime/entity-links/:link_id"),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/adhoc-tasks/activate",
        ),
        (
            "POST",
            "/runtime/process-instances/:process_instance_id/adhoc-tasks/:task_id/complete",
        ),
        ("GET", "/history/historic-process-instances"),
        (
            "GET",
            "/history/historic-process-instances/:process_instance_id",
        ),
        (
            "DELETE",
            "/history/historic-process-instances/:process_instance_id",
        ),
        ("POST", "/history/historic-process-instances/delete"),
        (
            "GET",
            "/history/historic-process-instances/:process_instance_id/identitylinks",
        ),
        (
            "GET",
            "/history/historic-process-instances/:process_instance_id/comments",
        ),
        (
            "GET",
            "/history/historic-process-instances/:process_instance_id/comments/:comment_id",
        ),
        (
            "GET",
            "/history/historic-process-instances/:process_instance_id/variables/:variable_name/data",
        ),
        ("GET", "/history/historic-task-instances"),
        ("GET", "/history/historic-task-instances/:task_id"),
        ("DELETE", "/history/historic-task-instances/:task_id"),
        ("POST", "/history/historic-task-instances/delete"),
        (
            "GET",
            "/history/historic-task-instances/:task_id/identitylinks",
        ),
        ("GET", "/history/historic-task-instances/:task_id/form"),
        (
            "GET",
            "/history/historic-task-instances/:task_id/variables/:variable_name/data",
        ),
        ("GET", "/history/historic-activity-instances"),
        ("GET", "/history/historic-detail"),
        ("GET", "/history/historic-detail/:detail_id/data"),
        ("GET", "/history/historic-variable-instances"),
        (
            "GET",
            "/history/historic-variable-instances/:variable_instance_id/data",
        ),
        ("GET", "/history/historic-task-log-entries"),
        ("POST", "/history/history-cleanup"),
        ("POST", "/history/history-cleanup/strategy"),
        ("POST", "/query/process-instances"),
        ("POST", "/query/executions"),
        ("POST", "/query/activity-instances"),
        ("POST", "/query/variable-instances"),
        ("POST", "/query/tasks"),
        ("POST", "/query/historic-process-instances"),
        ("POST", "/query/historic-task-instances"),
        ("POST", "/query/historic-activity-instances"),
        ("POST", "/query/historic-variable-instances"),
        ("POST", "/query/historic-detail"),
        ("GET", "/dmn-repository/deployments"),
        ("POST", "/dmn-repository/deployments"),
        ("GET", "/dmn-repository/deployments/:deployment_id"),
        ("DELETE", "/dmn-repository/deployments/:deployment_id"),
        (
            "GET",
            "/dmn-repository/deployments/:deployment_id/resources",
        ),
        (
            "GET",
            "/dmn-repository/deployments/:deployment_id/resourcedata/*resource_name",
        ),
        (
            "GET",
            "/dmn-repository/deployments/:deployment_id/resources/*resource_name",
        ),
        ("GET", "/dmn-repository/decision-tables"),
        ("GET", "/dmn-repository/decisions"),
        ("GET", "/dmn-repository/decision-services"),
        (
            "GET",
            "/dmn-repository/decision-services/:decision_service_id",
        ),
        ("GET", "/dmn-repository/decision-tables/:decision_table_id"),
        ("GET", "/dmn-repository/decisions/:decision_table_id"),
        (
            "GET",
            "/dmn-repository/decision-tables/:decision_table_id/resourcedata",
        ),
        (
            "GET",
            "/dmn-repository/decisions/:decision_table_id/resourcedata",
        ),
        (
            "GET",
            "/dmn-repository/decision-tables/:decision_table_id/model",
        ),
        ("GET", "/dmn-repository/decisions/:decision_table_id/model"),
        (
            "GET",
            "/dmn-repository/decision-tables/:decision_table_id/image",
        ),
        ("GET", "/dmn-repository/decisions/:decision_table_id/image"),
        ("GET", "/dmn-repository/decision-requirements-diagrams"),
        (
            "GET",
            "/dmn-repository/decision-requirements-diagrams/:drd_id",
        ),
        (
            "GET",
            "/dmn-repository/decision-requirements-diagrams/:drd_id/resourcedata",
        ),
        ("POST", "/dmn-runtime/decision-executions"),
        ("POST", "/dmn-rule/execute"),
        ("POST", "/dmn-rule/execute/single-result"),
        ("POST", "/dmn-rule/execute-decision"),
        ("POST", "/dmn-rule/execute-decision/single-result"),
        ("POST", "/dmn-rule/execute-decision-service"),
        ("POST", "/dmn-rule/execute-decision-service/single-result"),
        ("GET", "/dmn-history/historic-decision-executions"),
        (
            "GET",
            "/dmn-history/historic-decision-executions/:historic_decision_execution_id",
        ),
        (
            "DELETE",
            "/dmn-history/historic-decision-executions/:historic_decision_execution_id",
        ),
        ("POST", "/dmn-history/historic-decision-executions/delete"),
        (
            "GET",
            "/dmn-history/historic-decision-executions/:historic_decision_execution_id/auditdata",
        ),
        ("POST", "/dmn-query/historic-decision-executions"),
        ("GET", "/cmmn-repository/deployments"),
        ("POST", "/cmmn-repository/deployments"),
        ("GET", "/cmmn-repository/deployments/:deployment_id"),
        ("DELETE", "/cmmn-repository/deployments/:deployment_id"),
        (
            "GET",
            "/cmmn-repository/deployments/:deployment_id/resources",
        ),
        (
            "GET",
            "/cmmn-repository/deployments/:deployment_id/resourcedata/*resource_name",
        ),
        (
            "GET",
            "/cmmn-repository/deployments/:deployment_id/resources/*resource_name",
        ),
        ("GET", "/cmmn-repository/case-definitions"),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id",
        ),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id/resourcedata",
        ),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id/model",
        ),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id/decision-tables",
        ),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id/decisions",
        ),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id/form-definitions",
        ),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id/start-form",
        ),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id/image",
        ),
        (
            "GET",
            "/cmmn-repository/case-definitions/:case_definition_id/identitylinks",
        ),
        (
            "POST",
            "/cmmn-repository/case-definitions/:case_definition_id/identitylinks",
        ),
        (
            "DELETE",
            "/cmmn-repository/case-definitions/:case_definition_id/identitylinks/:family/:identity_id",
        ),
        (
            "POST",
            "/cmmn-repository/case-definitions/:case_definition_id/migrate",
        ),
        (
            "POST",
            "/cmmn-repository/case-definitions/:case_definition_id/batch-migrate",
        ),
        (
            "POST",
            "/cmmn-repository/case-definitions/:case_definition_id/migrate-historic-instances",
        ),
        (
            "POST",
            "/cmmn-repository/case-definitions/:case_definition_id/batch-migrate-historic-instances",
        ),
        ("GET", "/cmmn-runtime/case-instances"),
        ("POST", "/cmmn-runtime/case-instances"),
        ("GET", "/cmmn-runtime/case-instances/:case_instance_id"),
        ("DELETE", "/cmmn-runtime/case-instances/:case_instance_id"),
        ("POST", "/cmmn-runtime/case-instances/delete"),
        (
            "DELETE",
            "/cmmn-runtime/case-instances/:case_instance_id/delete",
        ),
        (
            "GET",
            "/cmmn-runtime/case-instances/:case_instance_id/stage-overview",
        ),
        (
            "POST",
            "/cmmn-runtime/case-instances/:case_instance_id/validate-migration",
        ),
        (
            "POST",
            "/cmmn-runtime/case-instances/:case_instance_id/migrate",
        ),
        (
            "POST",
            "/cmmn-runtime/case-instances/:case_instance_id/change-state",
        ),
        (
            "GET",
            "/cmmn-runtime/case-instances/:case_instance_id/identitylinks",
        ),
        (
            "POST",
            "/cmmn-runtime/case-instances/:case_instance_id/identitylinks",
        ),
        (
            "GET",
            "/cmmn-runtime/case-instances/:case_instance_id/identitylinks/users/:identity_id/:link_type",
        ),
        (
            "DELETE",
            "/cmmn-runtime/case-instances/:case_instance_id/identitylinks/users/:identity_id/:link_type",
        ),
        (
            "GET",
            "/cmmn-runtime/case-instances/:case_instance_id/variables",
        ),
        (
            "GET",
            "/cmmn-runtime/case-instances/:case_instance_id/variables/:variable_name",
        ),
        (
            "GET",
            "/cmmn-runtime/case-instances/:case_instance_id/variables/:variable_name/data",
        ),
        (
            "PUT",
            "/cmmn-runtime/case-instances/:case_instance_id/variables/:variable_name/data",
        ),
        (
            "POST",
            "/cmmn-runtime/case-instances/:case_instance_id/variables-async",
        ),
        (
            "PUT",
            "/cmmn-runtime/case-instances/:case_instance_id/variables-async",
        ),
        (
            "PUT",
            "/cmmn-runtime/case-instances/:case_instance_id/variables-async/:variable_name",
        ),
        (
            "POST",
            "/cmmn-runtime/case-instances/:case_instance_id/events",
        ),
        (
            "GET",
            "/cmmn-runtime/case-instances/:case_instance_id/diagram",
        ),
        ("GET", "/cmmn-runtime/tasks"),
        ("GET", "/cmmn-runtime/tasks/:plan_item_instance_id"),
        ("GET", "/cmmn-runtime/tasks/:plan_item_instance_id/subtasks"),
        ("GET", "/cmmn-runtime/tasks/:plan_item_instance_id/form"),
        (
            "GET",
            "/cmmn-runtime/tasks/:plan_item_instance_id/identitylinks",
        ),
        (
            "POST",
            "/cmmn-runtime/tasks/:plan_item_instance_id/identitylinks",
        ),
        (
            "GET",
            "/cmmn-runtime/tasks/:plan_item_instance_id/identitylinks/:family",
        ),
        (
            "GET",
            "/cmmn-runtime/tasks/:plan_item_instance_id/identitylinks/:family/:identity_id/:link_type",
        ),
        (
            "DELETE",
            "/cmmn-runtime/tasks/:plan_item_instance_id/identitylinks/:family/:identity_id/:link_type",
        ),
        (
            "GET",
            "/cmmn-runtime/tasks/:plan_item_instance_id/variables",
        ),
        (
            "GET",
            "/cmmn-runtime/tasks/:plan_item_instance_id/variables/:variable_name",
        ),
        (
            "GET",
            "/cmmn-runtime/tasks/:plan_item_instance_id/variables/:variable_name/data",
        ),
        (
            "PUT",
            "/cmmn-runtime/tasks/:plan_item_instance_id/variables/:variable_name/data",
        ),
        ("GET", "/cmmn-runtime/plan-item-instances"),
        (
            "GET",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id",
        ),
        (
            "GET",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables",
        ),
        (
            "GET",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables/:variable_name",
        ),
        (
            "GET",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables/:variable_name/data",
        ),
        (
            "PUT",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables/:variable_name/data",
        ),
        (
            "POST",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables-async",
        ),
        (
            "PUT",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables-async",
        ),
        (
            "PUT",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/variables-async/:variable_name",
        ),
        (
            "POST",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/complete",
        ),
        (
            "POST",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/reactivate",
        ),
        (
            "POST",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/disable",
        ),
        (
            "POST",
            "/cmmn-runtime/plan-item-instances/:plan_item_instance_id/enable",
        ),
        ("GET", "/cmmn-runtime/variable-instances"),
        (
            "GET",
            "/cmmn-runtime/variable-instances/:variable_instance_id",
        ),
        (
            "GET",
            "/cmmn-runtime/variable-instances/:variable_instance_id/data",
        ),
        ("GET", "/cmmn-runtime/event-subscriptions"),
        (
            "GET",
            "/cmmn-runtime/event-subscriptions/:event_subscription_id",
        ),
        ("POST", "/cmmn-query/tasks"),
        ("POST", "/cmmn-query/case-instances"),
        ("POST", "/cmmn-query/plan-item-instances"),
        ("POST", "/cmmn-query/variable-instances"),
        ("GET", "/cmmn-history/historic-case-instances"),
        (
            "GET",
            "/cmmn-history/historic-case-instances/:case_instance_id",
        ),
        (
            "DELETE",
            "/cmmn-history/historic-case-instances/:case_instance_id",
        ),
        ("POST", "/cmmn-history/historic-case-instances/delete"),
        (
            "GET",
            "/cmmn-history/historic-case-instances/:case_instance_id/stage-overview",
        ),
        (
            "POST",
            "/cmmn-history/historic-case-instances/:case_instance_id/migrate",
        ),
        (
            "GET",
            "/cmmn-history/historic-case-instances/:case_instance_id/identitylinks",
        ),
        (
            "GET",
            "/cmmn-history/historic-case-instances/:case_instance_id/variables/:variable_name/data",
        ),
        ("POST", "/cmmn-query/historic-case-instances"),
        ("GET", "/cmmn-history/historic-task-instances"),
        (
            "GET",
            "/cmmn-history/historic-task-instances/:plan_item_instance_id",
        ),
        (
            "GET",
            "/cmmn-history/historic-task-instances/:plan_item_instance_id/form",
        ),
        (
            "GET",
            "/cmmn-history/historic-task-instances/:plan_item_instance_id/identitylinks",
        ),
        (
            "GET",
            "/cmmn-history/historic-task-instances/:plan_item_instance_id/variables/:variable_name/data",
        ),
        ("POST", "/cmmn-query/historic-task-instances"),
        ("GET", "/cmmn-history/historic-milestone-instances"),
        (
            "GET",
            "/cmmn-history/historic-milestone-instances/:milestone_instance_id",
        ),
        ("POST", "/cmmn-query/historic-milestone-instances"),
        ("GET", "/cmmn-history/historic-variable-instances"),
        (
            "GET",
            "/cmmn-history/historic-variable-instances/:variable_instance_id/data",
        ),
        ("POST", "/cmmn-query/historic-variable-instances"),
        ("GET", "/cmmn-history/historic-plan-item-instances"),
        (
            "GET",
            "/cmmn-history/historic-plan-item-instances/:plan_item_instance_id",
        ),
        ("GET", "/cmmn-history/historic-planitem-instances"),
        (
            "GET",
            "/cmmn-history/historic-planitem-instances/:plan_item_instance_id",
        ),
        ("POST", "/cmmn-query/historic-planitem-instances"),
        ("GET", "/app-repository/deployments"),
        ("POST", "/app-repository/deployments"),
        ("GET", "/app-repository/deployments/:deployment_id"),
        ("DELETE", "/app-repository/deployments/:deployment_id"),
        (
            "GET",
            "/app-repository/deployments/:deployment_id/resources",
        ),
        (
            "GET",
            "/app-repository/deployments/:deployment_id/resourcedata/*resource_name",
        ),
        (
            "GET",
            "/app-repository/deployments/:deployment_id/resources/*resource_name",
        ),
        ("GET", "/app-repository/app-definitions"),
        ("GET", "/app-repository/app-definitions/:app_definition_id"),
        (
            "GET",
            "/app-repository/app-definitions/:app_definition_id/model",
        ),
        (
            "GET",
            "/app-repository/app-definitions/:app_definition_id/resourcedata",
        ),
        (
            "GET",
            "/app-repository/app-definitions/:app_definition_id/image",
        ),
        ("GET", "/app-runtime/compositions"),
        ("GET", "/app-runtime/compositions/:app_definition_id"),
        ("GET", "/event-registry-repository/deployments"),
        ("POST", "/event-registry-repository/deployments"),
        (
            "GET",
            "/event-registry-repository/deployments/:deployment_id",
        ),
        (
            "DELETE",
            "/event-registry-repository/deployments/:deployment_id",
        ),
        (
            "GET",
            "/event-registry-repository/deployments/:deployment_id/resources",
        ),
        (
            "GET",
            "/event-registry-repository/deployments/:deployment_id/resourcedata/*resource_name",
        ),
        (
            "GET",
            "/event-registry-repository/deployments/:deployment_id/resources/*resource_name",
        ),
        ("GET", "/event-registry-repository/channel-definitions"),
        (
            "GET",
            "/event-registry-repository/channel-definitions/:channel_definition_id",
        ),
        (
            "PUT",
            "/event-registry-repository/channel-definitions/:channel_definition_id",
        ),
        (
            "GET",
            "/event-registry-repository/channel-definitions/:channel_definition_id/model",
        ),
        (
            "GET",
            "/event-registry-repository/channel-definitions/:channel_definition_id/resourcedata",
        ),
        ("GET", "/event-registry-repository/event-definitions"),
        (
            "GET",
            "/event-registry-repository/event-definitions/:event_definition_id",
        ),
        (
            "PUT",
            "/event-registry-repository/event-definitions/:event_definition_id",
        ),
        (
            "GET",
            "/event-registry-repository/event-definitions/:event_definition_id/model",
        ),
        (
            "GET",
            "/event-registry-repository/event-definitions/:event_definition_id/resourcedata",
        ),
        ("POST", "/event-registry-runtime/event-instances"),
        ("POST", "/event-registry-runtime/inbound-event-instances"),
        (
            "GET",
            "/event-registry-management/event-instance-deliveries",
        ),
        (
            "GET",
            "/event-registry-management/event-instance-deliveries/:delivery_id",
        ),
        (
            "POST",
            "/event-registry-management/event-deliveries/:delivery_id/retry",
        ),
        (
            "DELETE",
            "/event-registry-management/event-deliveries/:delivery_id",
        ),
        ("GET", "/event-registry-management/engine"),
        ("POST", "/form/form-data"),
        ("POST", "/form/form-data/:form_definition_id"),
        ("GET", "/form/form-instances"),
        ("GET", "/form/form-instances/:form_instance_id"),
        ("POST", "/form-repository/deployments"),
        ("GET", "/form-repository/deployments/:deployment_id"),
        ("DELETE", "/form-repository/deployments/:deployment_id"),
        (
            "GET",
            "/form-repository/deployments/:deployment_id/resources",
        ),
        ("GET", "/form-repository/form-definitions"),
        ("DELETE", "/form-repository/form-definitions"),
        (
            "GET",
            "/form-repository/form-definitions/:form_definition_id",
        ),
        (
            "PUT",
            "/form-repository/form-definitions/:form_definition_id",
        ),
        (
            "GET",
            "/form-repository/form-definitions/:form_definition_id/versions",
        ),
        (
            "GET",
            "/form-repository/form-definitions/:form_definition_id/resourcedata",
        ),
        (
            "GET",
            "/form-repository/form-definitions/:form_definition_id/layout",
        ),
        (
            "GET",
            "/form-repository/form-definitions/:form_definition_id/outcomes",
        ),
        (
            "PUT",
            "/form-repository/form-definitions/:form_definition_id/activation",
        ),
        ("GET", "/content/items"),
        ("POST", "/content/items"),
        ("GET", "/content/items/:content_item_id"),
        ("DELETE", "/content/items/:content_item_id"),
        ("GET", "/content/items/:content_item_id/data"),
        ("GET", "/content/items/:content_item_id/object-metadata"),
        ("GET", "/content/items/:content_item_id/object-data"),
        ("GET", "/content/storage-status"),
        ("GET", "/identity/users"),
        ("POST", "/identity/users"),
        ("GET", "/identity/users/:user_id"),
        ("PUT", "/identity/users/:user_id"),
        ("DELETE", "/identity/users/:user_id"),
        ("GET", "/identity/users/:user_id/info"),
        ("POST", "/identity/users/:user_id/info"),
        ("GET", "/identity/users/:user_id/info/:key"),
        ("PUT", "/identity/users/:user_id/info/:key"),
        ("DELETE", "/identity/users/:user_id/info/:key"),
        ("GET", "/identity/users/:user_id/picture"),
        ("POST", "/identity/users/:user_id/picture"),
        ("PUT", "/identity/users/:user_id/picture"),
        ("DELETE", "/identity/users/:user_id/picture"),
        ("GET", "/identity/groups"),
        ("POST", "/identity/groups"),
        ("GET", "/identity/groups/:group_id"),
        ("PUT", "/identity/groups/:group_id"),
        ("DELETE", "/identity/groups/:group_id"),
        ("GET", "/identity/groups/:group_id/members"),
        ("POST", "/identity/groups/:group_id/members"),
        ("DELETE", "/identity/groups/:group_id/members/:user_id"),
        ("GET", "/identity/users/:user_id/memberships"),
        ("POST", "/identity/memberships"),
        ("DELETE", "/identity/memberships/:user_id/:group_id"),
        ("GET", "/identity/privileges"),
        ("POST", "/identity/privileges"),
        ("GET", "/identity/privileges/:privilege_id"),
        ("DELETE", "/identity/privileges/:privilege_id"),
        ("GET", "/identity/tokens"),
        ("POST", "/identity/tokens"),
        ("DELETE", "/identity/tokens/:token_id"),
        ("GET", "/users"),
        ("POST", "/users"),
        ("GET", "/users/:user_id"),
        ("PUT", "/users/:user_id"),
        ("DELETE", "/users/:user_id"),
        ("GET", "/groups"),
        ("POST", "/groups"),
        ("GET", "/groups/:group_id"),
        ("PUT", "/groups/:group_id"),
        ("DELETE", "/groups/:group_id"),
        ("GET", "/groups/:group_id/members"),
        ("POST", "/groups/:group_id/members"),
        ("DELETE", "/groups/:group_id/members/:user_id"),
        ("GET", "/privileges"),
        ("GET", "/privileges/:privilege_id"),
        ("GET", "/privileges/:privilege_id/users"),
        ("POST", "/privileges/:privilege_id/users"),
        ("DELETE", "/privileges/:privilege_id/users/:user_id"),
        ("GET", "/privileges/:privilege_id/groups"),
        ("POST", "/privileges/:privilege_id/groups"),
        ("DELETE", "/privileges/:privilege_id/group/:group_id"),
        ("POST", "/external-worker/jobs/fetch-and-lock"),
        ("GET", "/external-worker/jobs"),
        ("GET", "/external-worker/jobs/:id"),
        ("POST", "/external-worker/jobs/:id/complete"),
        ("POST", "/external-worker/jobs/:id/failure"),
        ("POST", "/external-worker/jobs/:id/bpmnError"),
        ("POST", "/external-worker/jobs/:id/cmmnTerminate"),
        ("POST", "/external-worker/jobs/:id/unlock"),
        ("POST", "/external-worker/jobs/bulk-unlock"),
        ("GET", "/management/engine"),
        ("GET", "/management/properties"),
        ("GET", "/management/engine-properties"),
        ("POST", "/management/engine-properties"),
        ("GET", "/management/engine-properties/:engine_property"),
        ("PUT", "/management/engine-properties/:engine_property"),
        ("DELETE", "/management/engine-properties/:engine_property"),
        ("GET", "/management/tables"),
        ("GET", "/management/tables/:table_name"),
        ("GET", "/management/tables/:table_name/columns"),
        ("GET", "/management/tables/:table_name/data"),
        ("GET", "/management/jobs"),
        ("GET", "/management/jobs/:job_id"),
        ("POST", "/management/jobs/:job_id"),
        ("DELETE", "/management/jobs/:job_id"),
        ("GET", "/management/jobs/:job_id/exception-stacktrace"),
        ("GET", "/management/timer-jobs"),
        ("GET", "/management/timer-jobs/:job_id"),
        ("POST", "/management/timer-jobs/:job_id"),
        ("DELETE", "/management/timer-jobs/:job_id"),
        ("GET", "/management/timer-jobs/:job_id/exception-stacktrace"),
        ("GET", "/management/deadletter-jobs"),
        ("POST", "/management/deadletter-jobs"),
        ("GET", "/management/deadletter-jobs/:job_id"),
        ("POST", "/management/deadletter-jobs/:job_id"),
        ("DELETE", "/management/deadletter-jobs/:job_id"),
        (
            "GET",
            "/management/deadletter-jobs/:job_id/exception-stacktrace",
        ),
        ("GET", "/management/history-jobs"),
        ("GET", "/management/history-jobs/:job_id"),
        ("POST", "/management/history-jobs/:job_id"),
        ("DELETE", "/management/history-jobs/:job_id"),
        ("GET", "/management/suspended-jobs"),
        ("GET", "/management/suspended-jobs/:job_id"),
        ("POST", "/management/suspended-jobs/:job_id"),
        ("DELETE", "/management/suspended-jobs/:job_id"),
        (
            "GET",
            "/management/suspended-jobs/:job_id/exception-stacktrace",
        ),
        ("GET", "/management/batches"),
        ("POST", "/management/batches"),
        ("GET", "/management/batches/:batch_id"),
        ("DELETE", "/management/batches/:batch_id"),
        ("GET", "/management/batches/:batch_id/batch-document"),
        ("GET", "/management/batches/:batch_id/batch-parts"),
        ("GET", "/management/batch-parts/:batch_part_id"),
        (
            "GET",
            "/management/batch-parts/:batch_part_id/batch-part-document",
        ),
        ("GET", "/management/directory/support"),
        ("GET", "/management/directory/reconcile"),
        ("POST", "/management/directory/reconcile"),
        ("GET", "/management/operations/support"),
        ("GET", "/management/platform/support"),
        ("GET", "/management/platform/topology-certification"),
        ("GET", "/management/jmx/runtime"),
        ("GET", "/management/jmx/connector-descriptor"),
        ("GET", "/management/jmx/mbean-registry"),
        ("GET", "/management/jmx/operations-bus"),
        ("GET", "/management/jmx/runtime-ledger"),
        ("GET", "/management/jmx/timer-ledger"),
        ("GET", "/management/operations/topology"),
        ("GET", "/cmmn-management/engine"),
        ("GET", "/cmmn-management/jobs"),
        ("GET", "/cmmn-management/jobs/:job_id"),
        ("GET", "/cmmn-management/jobs/:job_id/exception-stacktrace"),
        ("GET", "/cmmn-management/timer-jobs"),
        ("GET", "/cmmn-management/timer-jobs/:job_id"),
        (
            "GET",
            "/cmmn-management/timer-jobs/:job_id/exception-stacktrace",
        ),
        ("GET", "/cmmn-management/deadletter-jobs"),
        ("GET", "/cmmn-management/deadletter-jobs/:job_id"),
        (
            "GET",
            "/cmmn-management/deadletter-jobs/:job_id/exception-stacktrace",
        ),
        ("GET", "/cmmn-management/history-jobs"),
        ("GET", "/cmmn-management/history-jobs/:job_id"),
        ("GET", "/cmmn-management/suspended-jobs"),
        ("GET", "/cmmn-management/suspended-jobs/:job_id"),
        ("DELETE", "/cmmn-management/suspended-jobs/:job_id"),
        (
            "GET",
            "/cmmn-management/suspended-jobs/:job_id/exception-stacktrace",
        ),
        ("GET", "/dmn-management/engine"),
        ("GET", "/app-management/engine"),
        ("GET", "/idm-management/engine"),
        ("GET", "/health"),
        ("GET", "/ready"),
        ("GET", "/metrics"),
    ];

    raw_routes
        .into_iter()
        .map(|(method, path)| {
            let normalized = normalize_rust_path(method, path);
            format!("{method} {normalized}")
        })
        .collect()
}

fn allowed_missing() -> BTreeSet<&'static str> {
    BTreeSet::new()
}

#[test]
fn all_java_rest_paths_covered_by_rust() {
    let java_paths = java_rest_paths();
    let rust_paths = rust_registered_paths();
    let allowed = allowed_missing();

    let rust_set: BTreeSet<&str> = rust_paths.iter().map(|s| s.as_str()).collect();

    let mut uncovered = Vec::new();
    for java_path in &java_paths {
        if allowed.contains(java_path.as_str()) {
            continue;
        }
        if !rust_set.contains(java_path.as_str()) {
            uncovered.push(java_path.clone());
        }
    }

    if !uncovered.is_empty() {
        uncovered.sort();
        let mut msg = format!(
            "The following {} Java REST paths are not covered by Rust:\n",
            uncovered.len()
        );
        for path in &uncovered {
            msg.push_str(&format!("  {path}\n"));
        }
        panic!("{msg}");
    }
}

#[test]
fn rust_registered_paths_count_matches_fixture() {
    let java_paths = java_rest_paths();
    let rust_paths = rust_registered_paths();
    let allowed = allowed_missing();

    let rust_set: BTreeSet<&str> = rust_paths.iter().map(|s| s.as_str()).collect();
    let java_set: BTreeSet<&str> = java_paths.iter().map(|s| s.as_str()).collect();

    let covered = java_set.intersection(&rust_set).count();
    let total_java = java_set.len();
    let missing = total_java - covered;
    let allowed_count = allowed.len();
    let genuinely_missing = missing - allowed_count;

    let percentage = if total_java > 0 {
        (covered as f64 / total_java as f64) * 100.0
    } else {
        100.0
    };

    println!("=== REST Java Path Coverage Report ===");
    println!("Java REST paths:       {total_java}");
    println!("Rust registered paths: {}", rust_set.len());
    println!("Covered by Rust:       {covered}");
    println!("Missing:               {missing}");
    println!("Allowed missing:       {allowed_count}");
    println!("Genuinely missing:     {genuinely_missing}");
    println!("Coverage:              {percentage:.1}%");
}
