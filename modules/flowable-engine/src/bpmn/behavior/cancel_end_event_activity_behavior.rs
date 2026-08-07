use crate::agenda::FlowableEngineAgenda;
use crate::agenda::continue_process_operation::flow_element_id;
use crate::bpmn::behavior::intermediate_throw_event_activity_behavior::{
    collect_activity_ids_transitively, container_flow_elements,
};
use crate::bpmn::behavior::multi_instance_support::delete_execution_tree_with_reason;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use std::collections::{HashMap, HashSet};

pub struct CancelEndEventActivityBehavior;

impl Default for CancelEndEventActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelEndEventActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

/// The enclosing transaction scope of a cancel end event, resolved from the
/// execution tree and the BPMN model (Java `CancelEndEventActivityBehavior#
/// execute` 43-77 walks up to the sub-process scope execution).
struct CancelScope {
    /// The scope execution row of the enclosing transaction sub-process.
    tx_execution: Execution,
    /// All activity ids transitively inside the transaction: only their
    /// compensation subscriptions belong to this cancel.
    scope_activity_ids: HashSet<String>,
    /// Activity ids inside nested containers that are STILL ACTIVE at cancel
    /// time. Java destroys those child scopes without running their handlers
    /// (`TransactionSubProcessTest.testNestedCancelOuter`): their
    /// subscriptions are removed, never compensated.
    excluded_activity_ids: HashSet<String>,
    /// The single cancel boundary event attached to the transaction.
    cancel_boundary_id: Option<String>,
}

/// Recursively find the id of the cancel boundary event attached to
/// `attached_to_id`. Nested transactions have their boundary inside the
/// enclosing container, so a top-level scan is not enough.
pub(crate) fn find_cancel_boundary_id(
    flow_elements: &[FlowElementEnum],
    attached_to_id: &str,
) -> Option<String> {
    for element in flow_elements {
        if let FlowElementEnum::BoundaryEvent(boundary) = element
            && boundary.attached_to_ref_id.as_deref() == Some(attached_to_id)
            && let [EventDefinitionEnum::CancelEventDefinition(_)] =
                boundary.event.event_definitions.as_slice()
        {
            return boundary
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .clone();
        }
        if let Some(nested) = container_flow_elements(element)
            && let Some(found) = find_cancel_boundary_id(nested, attached_to_id)
        {
            return Some(found);
        }
    }
    None
}

/// Collect the transitive activity ids of every nested container that still
/// has a live execution: those scopes are destroyed with the transaction,
/// their subscriptions must NOT be compensated (Java deletes the child scope
/// executions via `deleteChildExecutions(TRANSACTION_CANCELED)`).
fn collect_active_nested_container_ids(
    flow_elements: &[FlowElementEnum],
    executions: &HashMap<String, Execution>,
    process_instance_id: Option<&str>,
    excluded: &mut HashSet<String>,
) {
    for element in flow_elements {
        let Some(nested) = container_flow_elements(element) else {
            continue;
        };
        let container_id = flow_element_id(element);
        let container_active = container_id.is_some_and(|container_id| {
            executions.values().any(|execution| {
                execution.activity_id.as_deref() == Some(container_id)
                    && !execution.is_ended
                    && execution.process_instance_id.as_deref() == process_instance_id
            })
        });
        if container_active {
            if let Some(container_id) = container_id {
                excluded.insert(container_id.to_string());
            }
            collect_activity_ids_transitively(nested, excluded);
        } else {
            collect_active_nested_container_ids(nested, executions, process_instance_id, excluded);
        }
    }
}

impl ActivityBehavior for CancelEndEventActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let scope = Self::resolve_cancel_scope(execution, command_context);

        let has_compensation = match (&execution.process_instance_id, &scope) {
            (Some(pi_id), Some(scope)) => {
                Self::compensate_transaction_scope(execution, command_context, pi_id.clone(), scope)
            }
            (Some(pi_id), None) => {
                // Fallback (no model / no enclosing transaction resolvable,
                // e.g. behaviors driven outside a deployed definition):
                // legacy process-wide compensation.
                Self::compensate_process_wide(execution, command_context, pi_id.clone())
            }
            (None, _) => false,
        };

        // Destroy every execution inside the cancelled transaction scope
        // except the cancel end event's own row — pending compensation
        // children hang under it (Java `deleteChildExecutions(subProcess-
        // Execution, notToDeleteExecutions, TRANSACTION_CANCELED)`).
        if let Some(scope) = &scope {
            destroy_transaction_scope(command_context, &scope.tx_execution.id, &execution.id);
        }

        execution.is_ended = true;

        // If there are no compensation activities to run, trigger the cancel
        // boundary directly. When compensation IS present, the boundary will
        // be triggered after all compensation tasks complete (handled by
        // TakeOutgoingSequenceFlowsOperation).
        if !has_compensation {
            match &scope {
                Some(scope) => {
                    Self::trigger_resolved_cancel_boundary(execution, command_context, scope)
                }
                None => self.trigger_cancel_boundary(execution, command_context),
            }
        }

        Ok(())
    }
}

impl CancelEndEventActivityBehavior {
    /// Walk up the execution tree to the first ancestor whose activity is a
    /// transaction / sub-process scope and resolve the scope's activity-id
    /// membership plus its cancel boundary from the model.
    fn resolve_cancel_scope(
        execution: &Execution,
        command_context: &mut CommandContext,
    ) -> Option<CancelScope> {
        let process_definition_id = execution.process_definition_id.clone()?;

        let all_executions = command_context
            .runtime_store
            .snapshot_executions(&mut command_context.session);

        let model = command_context
            .deployment_manager
            .get_bpmn_model(&process_definition_id)?;
        let main_process = model.main_process.as_ref()?;

        // Java walks up until a SubProcess activity is found (transactions
        // are modelled as sub-process scopes at runtime).
        let mut cursor = execution.parent_id.clone();
        let mut tx_execution = None;
        while let Some(parent_id) = cursor {
            let parent = all_executions.get(&parent_id)?;
            if let Some(activity_id) = parent.activity_id.as_deref()
                && matches!(
                    main_process.flow_element_map.get(activity_id),
                    Some(FlowElementEnum::Transaction(_)) | Some(FlowElementEnum::SubProcess(_))
                )
            {
                tx_execution = Some(parent.clone());
                break;
            }
            cursor = parent.parent_id.clone();
        }
        let tx_execution = tx_execution?;
        let tx_activity_id = tx_execution.activity_id.clone()?;

        let scope_element = main_process.flow_element_map.get(tx_activity_id.as_str())?;
        let scope_elements = container_flow_elements(scope_element)?;

        let mut scope_activity_ids = HashSet::new();
        collect_activity_ids_transitively(scope_elements, &mut scope_activity_ids);

        let mut excluded_activity_ids = HashSet::new();
        collect_active_nested_container_ids(
            scope_elements,
            &all_executions,
            execution.process_instance_id.as_deref(),
            &mut excluded_activity_ids,
        );

        let cancel_boundary_id =
            find_cancel_boundary_id(&main_process.flow_elements, &tx_activity_id);

        Some(CancelScope {
            tx_execution,
            scope_activity_ids,
            excluded_activity_ids,
            cancel_boundary_id,
        })
    }

    /// Compensate the completed activities of the cancelled transaction only.
    /// Subscriptions outside the transaction survive untouched; subscriptions
    /// of still-active nested scopes are removed without running the handler.
    fn compensate_transaction_scope(
        execution: &Execution,
        command_context: &mut CommandContext,
        pi_id: String,
        scope: &CancelScope,
    ) -> bool {
        let subscriptions = command_context
            .runtime_store
            .find_compensation_subscriptions_by_process_instance_id_newest_first(
                &pi_id,
                &mut command_context.session,
            );

        let mut has_compensation = false;
        for subscription in subscriptions {
            if !scope.scope_activity_ids.contains(&subscription.activity_id) {
                // Outside the cancelled transaction: not consumed.
                continue;
            }
            if scope
                .excluded_activity_ids
                .contains(&subscription.activity_id)
            {
                // Still-active nested scope: destroyed, never compensated
                // (Java testNestedCancelOuter).
                command_context
                    .runtime_store
                    .delete_compensation_subscription(
                        &subscription.id,
                        &mut command_context.session,
                    );
                continue;
            }

            let comp_execution = Execution {
                id: uuid::Uuid::new_v4().to_string(),
                process_instance_id: Some(pi_id.clone()),
                process_definition_id: execution.process_definition_id.clone(),
                activity_id: Some(subscription.compensation_activity_id.clone()),
                parent_id: Some(execution.id.clone()),
                is_active: true,
                // Handlers observe the scope-variable snapshot taken when the
                // compensated activity completed (P18-C / Java `ScopeUtil`).
                variables: subscription.variables_snapshot.clone(),
                ..Default::default()
            };

            command_context
                .execution_entity_manager
                .insert(&comp_execution, &mut command_context.session);
            command_context
                .agenda
                .plan_continue_process_operation(comp_execution);
            command_context
                .runtime_store
                .delete_compensation_subscription(&subscription.id, &mut command_context.session);
            has_compensation = true;
        }

        has_compensation
    }

    /// Legacy behavior for executions without a resolvable transaction scope:
    /// compensate every subscription of the process instance.
    fn compensate_process_wide(
        execution: &Execution,
        command_context: &mut CommandContext,
        pi_id: String,
    ) -> bool {
        let subs = command_context
            .runtime_store
            .find_compensation_subscriptions_by_process_instance_id_newest_first(
                &pi_id,
                &mut command_context.session,
            );
        let has = !subs.is_empty();
        for sub in subs {
            let comp_execution = Execution {
                id: uuid::Uuid::new_v4().to_string(),
                process_instance_id: Some(pi_id.clone()),
                process_definition_id: execution.process_definition_id.clone(),
                activity_id: Some(sub.compensation_activity_id.clone()),
                parent_id: Some(execution.id.clone()),
                ..Default::default()
            };

            command_context
                .execution_entity_manager
                .insert(&comp_execution, &mut command_context.session);
            command_context
                .agenda
                .plan_continue_process_operation(comp_execution);
        }
        command_context
            .runtime_store
            .delete_compensation_subscriptions_by_process_instance_id(
                &pi_id,
                &mut command_context.session,
            );
        has
    }

    /// Trigger the resolved cancel boundary of the transaction (no
    /// compensation handlers to wait for).
    fn trigger_resolved_cancel_boundary(
        execution: &Execution,
        command_context: &mut CommandContext,
        scope: &CancelScope,
    ) {
        let Some(b_id) = scope.cancel_boundary_id.clone() else {
            return;
        };

        // Cancel path does not go through `execute_boundary_trigger`, so
        // delete the one-shot cancel boundary subscription here (P13).
        if let Some(pi_id) = execution
            .process_instance_id
            .as_deref()
            .or(scope.tx_execution.process_instance_id.as_deref())
        {
            command_context.runtime_store.delete_boundary_event_state(
                &b_id,
                pi_id,
                &mut command_context.session,
            );
        }

        let mut boundary_exec = scope.tx_execution.clone();
        boundary_exec.activity_id = Some(b_id);
        command_context
            .execution_entity_manager
            .update(&boundary_exec, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(boundary_exec.clone());
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(boundary_exec);
    }

    /// Legacy path: find the transaction's cancel boundary event via the
    /// direct parent execution and trigger it.
    fn trigger_cancel_boundary(&self, execution: &Execution, command_context: &mut CommandContext) {
        let process_def_id = match execution.process_definition_id.as_ref() {
            Some(id) => id,
            None => return,
        };

        // Walk up to find the transaction execution
        let all_executions = command_context
            .runtime_store
            .snapshot_executions(&mut command_context.session);

        // The cancel end event's parent should be the transaction's child execution
        let parent_id = match &execution.parent_id {
            Some(id) => id,
            None => return,
        };

        // Find the transaction execution (could be parent or grandparent)
        let tx_exec = all_executions.get(parent_id);
        if tx_exec.is_none() {
            return;
        }
        let tx_exec = tx_exec.unwrap();

        let tx_activity_id = match tx_exec.activity_id.as_deref() {
            Some(id) => id,
            None => return,
        };

        let model = match command_context
            .deployment_manager
            .get_bpmn_model(process_def_id)
        {
            Some(m) => m,
            None => return,
        };
        let main_process = match model.main_process.as_ref() {
            Some(p) => p,
            None => return,
        };

        // Find the cancel boundary event attached to this transaction
        let cancel_boundary_id =
            find_cancel_boundary_id(&main_process.flow_elements, tx_activity_id);

        if let Some(b_id) = cancel_boundary_id {
            // Cancel path does not go through `execute_boundary_trigger`, so
            // delete the one-shot cancel boundary subscription here (P13).
            if let Some(pi_id) = execution
                .process_instance_id
                .as_deref()
                .or(tx_exec.process_instance_id.as_deref())
            {
                command_context.runtime_store.delete_boundary_event_state(
                    &b_id,
                    pi_id,
                    &mut command_context.session,
                );
            }

            let mut boundary_exec = tx_exec.clone();
            boundary_exec.activity_id = Some(b_id);
            command_context
                .execution_entity_manager
                .update(&boundary_exec, &mut command_context.session);
            command_context
                .agenda
                .plan_continue_process_operation(boundary_exec.clone());
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(boundary_exec);
        }
    }
}

/// Delete every execution inside the transaction scope except the cancel end
/// event's own row (and its ancestor chain, which parents it).
fn destroy_transaction_scope(
    command_context: &mut CommandContext,
    tx_execution_id: &str,
    keep_execution_id: &str,
) {
    let all_executions = command_context
        .runtime_store
        .snapshot_executions(&mut command_context.session);

    // Ancestor chain of the kept row up to (excluding) the transaction row:
    // those rows must survive too, we only prune their other children.
    let mut keep_chain = HashSet::new();
    keep_chain.insert(keep_execution_id.to_string());
    let mut cursor = all_executions
        .get(keep_execution_id)
        .and_then(|execution| execution.parent_id.clone());
    while let Some(id) = cursor {
        if id == tx_execution_id {
            break;
        }
        cursor = all_executions
            .get(&id)
            .and_then(|execution| execution.parent_id.clone());
        keep_chain.insert(id);
    }

    delete_children_except(
        command_context,
        tx_execution_id,
        keep_execution_id,
        &keep_chain,
    );
}

fn delete_children_except(
    command_context: &mut CommandContext,
    parent_id: &str,
    keep_execution_id: &str,
    keep_chain: &HashSet<String>,
) {
    let child_ids: Vec<String> = command_context
        .execution_entity_manager
        .find_child_executions_by_parent_execution_id(parent_id, &mut command_context.session)
        .into_iter()
        .map(|child| child.id)
        .collect();
    for child_id in child_ids {
        if child_id == keep_execution_id {
            // The cancel end event's own row survives: freshly planned
            // compensation children hang under it.
            continue;
        }
        if keep_chain.contains(&child_id) {
            delete_children_except(command_context, &child_id, keep_execution_id, keep_chain);
        } else {
            // Java `CancelEndEventActivityBehavior#deleteChildExecutions`
            // passes `DeleteReason.TRANSACTION_CANCELED` into
            // `deleteExecutionAndRelatedData`.
            delete_execution_tree_with_reason(
                command_context,
                &child_id,
                Some(crate::history::delete_reason::TRANSACTION_CANCELED),
            );
        }
    }
}
