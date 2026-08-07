use crate::agenda::FlowableEngineAgenda;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{
    EventDefinitionEnum, FlowElementEnum, VariableListenerEventDefinition,
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableEventType {
    Create,
    Update,
    Delete,
}

impl VariableEventType {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "create" => Some(Self::Create),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            // Java `update-create` matches either create or update.
            "update-create" => Some(Self::Update),
            "all" => None, // handled as empty/wildcard list
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

pub struct VariableListenerEventBehavior {
    pub variable_name: Option<String>,
    pub variable_events: Vec<VariableEventType>,
}

impl VariableListenerEventBehavior {
    pub fn new(variable_name: Option<String>, variable_events: Vec<VariableEventType>) -> Self {
        Self {
            variable_name,
            variable_events,
        }
    }

    pub fn from_definition(def: &VariableListenerEventDefinition) -> Self {
        let events = match def.variable_change_type.as_deref() {
            None | Some("") | Some("all") => vec![],
            Some("update-create") => vec![VariableEventType::Create, VariableEventType::Update],
            Some(other) => VariableEventType::from_str(other).into_iter().collect(),
        };
        Self::new(def.variable_name.clone(), events)
    }

    pub fn matches_event(&self, variable_name: &str, event_type: &VariableEventType) -> bool {
        let name_matches = self
            .variable_name
            .as_ref()
            .is_none_or(|name| name == variable_name);

        let event_matches =
            self.variable_events.is_empty() || self.variable_events.contains(event_type);

        name_matches && event_matches
    }
}

impl Default for VariableListenerEventBehavior {
    fn default() -> Self {
        Self::new(None, vec![])
    }
}

impl ActivityBehavior for VariableListenerEventBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(execution.clone());

        Ok(())
    }
}

/// Evaluates variable-listener event subprocess start events after a variable
/// mutation (Java `EvaluateVariableListenerEventDefinitionsOperation`, model-scan
/// simplified path without generic "variable" event subscriptions).
pub fn evaluate_variable_listener_event_subprocesses(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    variable_name: &str,
    event_type: &VariableEventType,
) -> Result<(), FlowableError> {
    let process_instance = {
        let (store, session) = command_context.store_and_session();
        store.find_process_instance(process_instance_id, session)
    };
    let Some(process_instance) = process_instance else {
        return Ok(());
    };
    if process_instance.is_ended || process_instance.is_suspended {
        return Ok(());
    }

    let process_definition_id = process_instance.process_definition_id.clone();
    let bpmn_model = match command_context
        .deployment_manager
        .get_bpmn_model(&process_definition_id)
    {
        Some(m) => m,
        None => return Ok(()),
    };
    let Some(main_process) = bpmn_model.main_process.as_ref() else {
        return Ok(());
    };

    // Collect matching event subprocesses first so we don't hold borrows across
    // mutation of command_context.
    let mut matches: Vec<(String, String, bool)> = Vec::new();
    for flow_element in &main_process.flow_elements {
        let (sub_process, event_subprocess_id) = match flow_element {
            FlowElementEnum::EventSubProcess(esp) => {
                let id = esp
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .clone()
                    .unwrap_or_default();
                (&esp.sub_process, id)
            }
            FlowElementEnum::SubProcess(sub) if sub.triggered_by_event => {
                let id = sub
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .clone()
                    .unwrap_or_default();
                (sub, id)
            }
            _ => continue,
        };

        for child in &sub_process.flow_elements {
            let FlowElementEnum::StartEvent(start_event) = child else {
                continue;
            };
            for def in &start_event.event.event_definitions {
                let EventDefinitionEnum::VariableListenerEventDefinition(vl) = def else {
                    continue;
                };
                let behavior = VariableListenerEventBehavior::from_definition(vl);
                if !behavior.matches_event(variable_name, event_type) {
                    continue;
                }
                let start_id = start_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .clone()
                    .unwrap_or_default();
                matches.push((event_subprocess_id.clone(), start_id, start_event.interrupting));
            }
        }
    }

    if matches.is_empty() {
        return Ok(());
    }

    let parent_execution = {
        let (store, session) = command_context.store_and_session();
        store.find_execution(process_instance_id, session)
    };
    let Some(parent_execution) = parent_execution else {
        return Ok(());
    };

    for (event_subprocess_id, start_event_id, interrupting) in matches {
        if interrupting {
            // Java EventSubProcessVariableListenerStartEventActivityBehavior#trigger
            // deletes sibling children of the parent; also clear runtime data on
            // the process-instance row itself (Rust flat execution trees).
            let child_ids: Vec<String> = command_context
                .execution_entity_manager
                .find_child_executions_by_parent_execution_id(
                    process_instance_id,
                    &mut command_context.session,
                )
                .into_iter()
                .map(|c| c.id)
                .collect();
            // Java `EventSubProcessVariableListenerlStartEventActivityBehavior`:
            // `EVENT_SUBPROCESS_INTERRUPTING + "(" + startEvent.getId() + ")"`.
            let delete_reason =
                crate::history::delete_reason::event_subprocess_interrupting(&start_event_id);
            for child_id in child_ids {
                crate::bpmn::behavior::multi_instance_support::delete_execution_tree_with_reason(
                    command_context,
                    &child_id,
                    Some(&delete_reason),
                );
            }
            // Flat-tree host may reuse the PI row: end its open activity with
            // the same reason, then strip runtime state (do not delete the PI).
            crate::bpmn::behavior::multi_instance_support::record_activity_end_for_execution(
                command_context,
                process_instance_id,
                Some(&delete_reason),
            );
            crate::bpmn::behavior::multi_instance_support::delete_execution_related_runtime_data(
                command_context,
                process_instance_id,
            );
        }

        let es_scope_id = Uuid::new_v4().to_string();
        let es_scope_execution = Execution {
            id: es_scope_id.clone(),
            parent_id: Some(process_instance_id.to_string()),
            process_instance_id: Some(process_instance_id.to_string()),
            process_definition_id: parent_execution.process_definition_id.clone(),
            activity_id: Some(event_subprocess_id),
            is_active: true,
            is_scope: true,
            variables: parent_execution.variables.clone(),
            ..Default::default()
        };
        command_context
            .execution_entity_manager
            .insert(&es_scope_execution, &mut command_context.session);

        let start_execution = Execution {
            id: Uuid::new_v4().to_string(),
            parent_id: Some(es_scope_id),
            process_instance_id: Some(process_instance_id.to_string()),
            process_definition_id: parent_execution.process_definition_id.clone(),
            activity_id: Some(start_event_id),
            is_active: true,
            is_scope: false,
            variables: parent_execution.variables.clone(),
            ..Default::default()
        };
        command_context
            .execution_entity_manager
            .insert(&start_execution, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(start_execution);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_event_type_from_str() {
        assert_eq!(
            VariableEventType::from_str("create"),
            Some(VariableEventType::Create)
        );
        assert_eq!(
            VariableEventType::from_str("CREATE"),
            Some(VariableEventType::Create)
        );
        assert_eq!(
            VariableEventType::from_str("update"),
            Some(VariableEventType::Update)
        );
        assert_eq!(
            VariableEventType::from_str("UPDATE"),
            Some(VariableEventType::Update)
        );
        assert_eq!(
            VariableEventType::from_str("delete"),
            Some(VariableEventType::Delete)
        );
        assert_eq!(
            VariableEventType::from_str("DELETE"),
            Some(VariableEventType::Delete)
        );
        assert_eq!(VariableEventType::from_str("invalid"), None);
    }

    #[test]
    fn test_variable_event_type_as_str() {
        assert_eq!(VariableEventType::Create.as_str(), "create");
        assert_eq!(VariableEventType::Update.as_str(), "update");
        assert_eq!(VariableEventType::Delete.as_str(), "delete");
    }

    #[test]
    fn test_matches_event_with_specific_variable_name() {
        let behavior = VariableListenerEventBehavior::new(
            Some("myVar".to_string()),
            vec![VariableEventType::Create, VariableEventType::Update],
        );

        assert!(behavior.matches_event("myVar", &VariableEventType::Create));
        assert!(behavior.matches_event("myVar", &VariableEventType::Update));
        assert!(!behavior.matches_event("myVar", &VariableEventType::Delete));
        assert!(!behavior.matches_event("otherVar", &VariableEventType::Create));
    }

    #[test]
    fn test_matches_event_with_wildcard_variable_name() {
        let behavior = VariableListenerEventBehavior::new(None, vec![VariableEventType::Delete]);

        assert!(!behavior.matches_event("anyVar", &VariableEventType::Create));
        assert!(!behavior.matches_event("anyVar", &VariableEventType::Update));
        assert!(behavior.matches_event("anyVar", &VariableEventType::Delete));
        assert!(behavior.matches_event("otherVar", &VariableEventType::Delete));
    }

    #[test]
    fn test_matches_event_with_empty_events_matches_all() {
        let behavior = VariableListenerEventBehavior::new(Some("myVar".to_string()), vec![]);

        assert!(behavior.matches_event("myVar", &VariableEventType::Create));
        assert!(behavior.matches_event("myVar", &VariableEventType::Update));
        assert!(behavior.matches_event("myVar", &VariableEventType::Delete));
    }

    #[test]
    fn test_default_behavior() {
        let behavior = VariableListenerEventBehavior::default();

        assert!(behavior.variable_name.is_none());
        assert!(behavior.variable_events.is_empty());
        assert!(behavior.matches_event("anyVar", &VariableEventType::Create));
        assert!(behavior.matches_event("anyVar", &VariableEventType::Update));
        assert!(behavior.matches_event("anyVar", &VariableEventType::Delete));
    }

    #[test]
    fn test_matches_event_case_sensitive_variable_name() {
        let behavior = VariableListenerEventBehavior::new(
            Some("MyVar".to_string()),
            vec![VariableEventType::Create],
        );

        assert!(behavior.matches_event("MyVar", &VariableEventType::Create));
        assert!(!behavior.matches_event("myvar", &VariableEventType::Create));
        assert!(!behavior.matches_event("MYVAR", &VariableEventType::Create));
    }
}
