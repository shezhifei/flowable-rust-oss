use crate::agenda::FlowableEngineAgenda;
use crate::agenda::continue_process_operation::{
    find_flow_element, flow_element_id, flow_element_type,
};
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::el::expression::Expression;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::MultiInstanceLoopCharacteristics;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

pub struct MultiInstanceActivityBehavior {
    inner_behavior: Box<dyn ActivityBehavior>,
    mi_characteristics: MultiInstanceLoopCharacteristics,
    /// Java `SequentialMultiInstanceBehavior#continueSequentialMultiInstance`
    /// branches on `instanceof SubProcess`: each round creates a **new** scope
    /// child (`setScope(true)`). Non-SubProcess sequential MI reuses one child.
    inner_is_subprocess: bool,
}

impl MultiInstanceActivityBehavior {
    pub fn new(
        inner_behavior: Box<dyn ActivityBehavior>,
        mi_characteristics: MultiInstanceLoopCharacteristics,
        inner_is_subprocess: bool,
    ) -> Self {
        Self {
            inner_behavior,
            mi_characteristics,
            inner_is_subprocess,
        }
    }
}

impl ActivityBehavior for MultiInstanceActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // Resolve collection when configured (needed for elementVariable even when
        // loopCardinality drives the instance count).
        let collection_items = if self.uses_collection() {
            self.resolve_collection_items(command_context, execution)?
        } else {
            None
        };

        // Java `MultiInstanceActivityBehavior#resolveNrOfInstances` (lines 450-461):
        // loopCardinality expression takes precedence over collection size.
        let loop_cardinality = if self.mi_characteristics.loop_cardinality.is_some() {
            Some(self.resolve_loop_cardinality(command_context, execution)?)
        } else if let Some(ref items) = collection_items {
            Some(items.len() as i32)
        } else {
            None
        };

        let Some(loop_cardinality) = loop_cardinality else {
            return self.inner_behavior.execute(execution, command_context);
        };

        // Java `ContinueProcessOperation#executeMultiInstanceSynchronous`:
        // materialize a dedicated MI root before creating instances. Re-entry
        // (sequential continue) already carries `is_multi_instance_root`.
        materialize_multi_instance_root(execution, command_context)?;
        // Java keeps the MI root inactive while instance children run. Re-entry
        // via `plan_continue_process_operation` may have set is_active=true;
        // restore the inactive flag on every MI execute.
        if execution.is_active {
            execution.is_active = false;
            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);
        }

        if loop_cardinality <= 0 {
            // Java: `nrOfInstances == 0` → `cleanupMiRoot(execution)`.
            // Zero instances: COMPLETED (not WITH_CONDITION).
            cleanup_mi_root_and_leave(execution, command_context, false);
            return Ok(());
        }

        if self.mi_characteristics.sequential {
            // Sequential MI: Create child execution for the current index. The
            // resume index is the number of completed instances — Java keeps
            // `loopCounter` only on the child instance execution
            // (`ContinueMultiInstanceOperation` → `setVariableLocal`), never on
            // the MI root, so there is no root-side counter to read back.
            let current_index = execution
                .process_variable("nrOfCompletedInstances")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            self.execute_sequential(
                execution,
                command_context,
                current_index,
                loop_cardinality,
                collection_items.as_deref(),
            )?;
        } else {
            // Parallel MI: Create all child executions at once
            self.execute_parallel(
                execution,
                command_context,
                loop_cardinality,
                collection_items.as_deref(),
            )?;
        }

        Ok(())
    }
}

impl MultiInstanceActivityBehavior {
    /// Java `MultiInstanceActivityBehavior#usesCollection` (line 555-557).
    fn uses_collection(&self) -> bool {
        self.mi_characteristics.input_data_item.is_some()
            || self.mi_characteristics.collection_string.is_some()
    }

    /// Java `MultiInstanceActivityBehavior#resolveLoopCardinality` (lines 563-575).
    /// Evaluates the loopCardinality text as EL; Number → int, String → parse int,
    /// otherwise `FlowableIllegalArgumentException` → BadRequest.
    fn resolve_loop_cardinality(
        &self,
        command_context: &mut CommandContext,
        execution: &Execution,
    ) -> Result<i32, FlowableError> {
        let expr_text = self
            .mi_characteristics
            .loop_cardinality
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                FlowableError::BadRequest(
                    "Could not resolve loopCardinality expression: empty".to_string(),
                )
            })?;

        // Java uses expressionManager.createExpression + getValue(execution).
        // Evaluation walks the VariableScope parent chain (P4-7a evaluation_execution).
        // SimpleExpression requires `${…}`; bare literals (e.g. "5") are wrapped
        // so they compile the same way JUEL treats bare expression text.
        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);
        let value = evaluate_mi_expression(&expr_text, &evaluation_execution);

        match value {
            // Number → intValue() (covers Long/Integer/Double from EL)
            Some(Value::Number(n)) => {
                if let Some(i) = n.as_i64() {
                    Ok(i as i32)
                } else if let Some(u) = n.as_u64() {
                    Ok(u as i32)
                } else if let Some(f) = n.as_f64() {
                    Ok(f as i32)
                } else {
                    Err(FlowableError::BadRequest(format!(
                        "Could not resolve loopCardinality expression '{}': not a number nor number String",
                        expr_text
                    )))
                }
            }
            // String → Integer.valueOf
            Some(Value::String(s)) => s.trim().parse::<i32>().map_err(|_| {
                FlowableError::BadRequest(format!(
                    "Could not resolve loopCardinality expression '{}': not a number nor number String",
                    expr_text
                ))
            }),
            // Boolean / null / missing / other → same Java error
            _ => Err(FlowableError::BadRequest(format!(
                "Could not resolve loopCardinality expression '{}': not a number nor number String",
                expr_text
            ))),
        }
    }

    /// Java `MultiInstanceActivityBehavior#resolveAndValidateCollection` (lines 483-553)
    /// and `#resolveCollection` (lines 541-553).
    ///
    /// Order: inputDataItem (collection expression) before collectionString —
    /// Java `AbstractActivityBpmnParseHandler` lines 60-68; was inverted in Rust.
    fn resolve_collection_items(
        &self,
        command_context: &mut CommandContext,
        execution: &Execution,
    ) -> Result<Option<Vec<Value>>, FlowableError> {
        // Java resolveCollection:541-553 — collectionExpression (from inputDataItem)
        // first, then collectionVariable (unused here), then collectionString.
        let obj = if let Some(input) = self.mi_characteristics.input_data_item.as_ref() {
            // Java AbstractActivityBpmnParseHandler:61-62 always wraps inputDataItem
            // as an expression (bare names and ${…} both evaluate as EL).
            let evaluation_execution =
                crate::engine::variable_service::evaluation_execution(command_context, execution);
            evaluate_mi_expression(input.trim(), &evaluation_execution)
        } else if let Some(cs) = self.mi_characteristics.collection_string.as_ref() {
            // collectionString is a raw string that may name a variable
            // (Java MultiInstanceActivityBehavior.java:549-550).
            Some(Value::String(cs.trim().to_string()))
        } else {
            return Ok(None);
        };

        // Java resolveAndValidateCollection:488-508
        match obj {
            Some(Value::Array(items)) => Ok(Some(items)),
            Some(Value::String(name)) => {
                // String result → treat as variable name and re-resolve.
                // Parent-scope walk via find_execution_variable (parity with
                // execution.getVariable which walks VariableScope chain).
                let store = command_context.runtime_store.clone();
                let resolved = crate::engine::variable_service::find_execution_variable(
                    &store,
                    &mut command_context.session,
                    &execution.id,
                    &name,
                );
                match resolved {
                    Some((_, Value::Array(items))) => Ok(Some(items)),
                    Some((_, other)) => Err(FlowableError::BadRequest(format!(
                        "Variable '{}':{} is not a Collection",
                        name, other
                    ))),
                    // Java MultiInstanceActivityBehavior.java:500-501
                    None => Err(FlowableError::BadRequest(format!(
                        "Variable '{}' was not found",
                        name
                    ))),
                }
            }
            Some(other) => Err(FlowableError::BadRequest(format!(
                "Couldn't resolve collection expression, variable reference or string (got {})",
                crate::engine::variable_service::variable_type_name(&other)
            ))),
            // Expression evaluated to null / missing variable
            None => {
                let hint = self
                    .mi_characteristics
                    .input_data_item
                    .as_deref()
                    .or(self.mi_characteristics.collection_string.as_deref())
                    .unwrap_or("");
                Err(FlowableError::BadRequest(format!(
                    "Couldn't resolve collection expression ({}), variable reference or string",
                    hint
                )))
            }
        }
    }

    /// Java parity: instance variables are execution-LOCAL on the child
    /// instance execution (`ContinueMultiInstanceOperation` →
    /// `setVariableLocal(elementIndexVariable, …)` and
    /// `executeOriginalBehavior` → `setLoopVariable` → `setVariableLocal`).
    fn apply_instance_variables(&self, child: &mut Execution, index: i32, item: Option<Value>) {
        child.set_local_variable("loopCounter".to_string(), index.into());
        if let Some(index_variable) = &self.mi_characteristics.element_index_variable {
            child.set_local_variable(index_variable.clone(), index.into());
        }
        if let (Some(element_variable), Some(item)) =
            (&self.mi_characteristics.element_variable, item)
        {
            child.set_local_variable(element_variable.clone(), item);
        }
    }

    fn execute_sequential(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        index: i32,
        total: i32,
        collection_items: Option<&[Value]>,
    ) -> Result<(), crate::error::FlowableError> {
        // Java branches on SubProcess vs non-SubProcess in
        // `continueSequentialMultiInstance` (lines 106–124).
        if self.inner_is_subprocess {
            return self.execute_sequential_subprocess(
                execution,
                command_context,
                index,
                total,
                collection_items,
            );
        }

        // Non-SubProcess (e.g. userTask): reuse one instance child for every
        // round — clear locals, bump loopCounter, re-execute. Child id is
        // stable; ended rows do not accumulate between rounds (P6-A).
        let mut child = match find_reusable_sequential_child(command_context, &execution.id) {
            Some(existing) => existing,
            None => create_sequential_instance_child(execution, command_context, false),
        };

        let mut current_index = index;
        while current_index < total {
            // Set MI bookkeeping variables on the MI root execution. Java
            // parity: `SequentialMultiInstanceBehavior#createInstances` /
            // `#leave` write them via `setLoopVariable` → `setVariableLocal`.
            execution.set_local_variable("nrOfInstances".to_string(), total.into());
            execution.set_local_variable("nrOfActiveInstances".to_string(), 1.into());
            execution
                .set_local_variable("nrOfCompletedInstances".to_string(), current_index.into());

            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);

            // Java `continueSequentialMultiInstance` (non-SubProcess path):
            // delete all local variables except the nrOf* bookkeeping names
            // (those live on the MI root; the filter is defensive), then
            // re-apply instance variables for the next loopCounter.
            clear_sequential_instance_locals(&mut child);
            self.apply_instance_variables(
                &mut child,
                current_index,
                collection_items.and_then(|items| items.get(current_index as usize).cloned()),
            );
            child.is_active = true;
            child.is_ended = false;
            command_context
                .execution_entity_manager
                .update(&child, &mut command_context.session);

            // Java `ContinueMultiInstanceOperation#executeSynchronous` records
            // activity start for each MI instance before executing the inner
            // behavior.
            record_mi_child_activity_start(command_context, &child);

            self.inner_behavior.execute(&mut child, command_context)?;

            if child_has_wait_state(command_context, &child) {
                command_context
                    .execution_entity_manager
                    .update(&child, &mut command_context.session);
                return Ok(());
            }

            // Synchronous completion of one round: record activity end before
            // moving to the next iteration (Java `continueSequentialMultiInstance`
            // calls recordActivityEnd before clearing locals and re-executing).
            record_mi_child_activity_end(command_context, &child);

            // Synchronous completion of one round: keep the child alive for
            // the next iteration (Java does not end it between rounds).
            let nr_of_completed = current_index + 1;
            execution
                .set_local_variable("nrOfCompletedInstances".to_string(), nr_of_completed.into());
            // Sequential MI keeps one active instance while more rounds remain;
            // Java leave does not zero nrOfActiveInstances between rounds.
            execution.set_local_variable("nrOfActiveInstances".to_string(), 1.into());

            if multi_instance_completion_condition_satisfied(
                command_context,
                &self.mi_characteristics,
                execution,
            )? {
                record_mi_child_activity_end(command_context, &child);
                end_sequential_instance_child(command_context, &mut child);
                // Java `super.leave` → `cleanupMiRoot` with condition satisfied.
                // SequentialMultiInstanceBehavior.java:92-93.
                cleanup_mi_root_and_leave(execution, command_context, true);
                return Ok(());
            }

            current_index += 1;
        }

        // Completed all loops: end the reused child, then cleanupMiRoot leave.
        record_mi_child_activity_end(command_context, &child);
        end_sequential_instance_child(command_context, &mut child);
        // SequentialMultiInstanceBehavior.java:95-96 — all rounds done without condition.
        cleanup_mi_root_and_leave(execution, command_context, false);

        Ok(())
    }

    /// Java SubProcess sequential MI: each entry creates **one** new scope child
    /// (`setScope(true)`), runs the embedded SubProcess, and returns. Nested
    /// wait states and further rounds are driven by agenda / leave
    /// (`leave_sequential_subprocess_mi_instance`), not by an in-line while
    /// loop — nested activities are planned on the agenda and only become
    /// observable after `execute` returns.
    fn execute_sequential_subprocess(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        index: i32,
        total: i32,
        collection_items: Option<&[Value]>,
    ) -> Result<(), crate::error::FlowableError> {
        if index >= total {
            cleanup_mi_root_and_leave(execution, command_context, false);
            return Ok(());
        }

        execution.set_local_variable("nrOfInstances".to_string(), total.into());
        execution.set_local_variable("nrOfActiveInstances".to_string(), 1.into());
        execution.set_local_variable("nrOfCompletedInstances".to_string(), index.into());
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        // Always a fresh child (never reuse). Java continue path also sets
        // scope=true before executeOriginalBehavior; SubProcess.execute sets
        // it again defensively.
        let mut child = create_sequential_instance_child(execution, command_context, true);
        self.apply_instance_variables(
            &mut child,
            index,
            collection_items.and_then(|items| items.get(index as usize).cloned()),
        );
        command_context
            .execution_entity_manager
            .update(&child, &mut command_context.session);

        self.inner_behavior.execute(&mut child, command_context)?;
        // Nested wait / end is handled by agenda ops and
        // `leave_sequential_subprocess_mi_instance` on SubProcess scope leave.
        Ok(())
    }

    fn execute_parallel(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        total: i32,
        collection_items: Option<&[Value]>,
    ) -> Result<(), crate::error::FlowableError> {
        // Java parity: `ParallelMultiInstanceBehavior#createInstances` writes
        // the nrOf* bookkeeping via `setLoopVariable` → `setVariableLocal` on
        // the MI root execution.
        execution.set_local_variable("nrOfInstances".to_string(), total.into());
        execution.set_local_variable("nrOfActiveInstances".to_string(), total.into());
        execution.set_local_variable("nrOfCompletedInstances".to_string(), 0.into());
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        let mut completed_instances = 0;
        let mut active_instances = total;
        for i in 0..total {
            let mut child = execution.clone();
            child.id = uuid::Uuid::new_v4().to_string();
            child.parent_id = Some(execution.id.clone());
            child.is_active = true;
            child.is_scope = false;
            // Instance children are never MI roots (Java `createChildExecution`).
            child.is_multi_instance_root = false;
            // Java parity: instance executions are `createChildExecution` —
            // they start with empty variable maps and resolve inherited names
            // through the parent VariableScope chain. Snapshot-copying the MI
            // root would freeze nrOf* into the child and shadow the live
            // values (same defect class as P4-6C / P4-7b).
            child.variables.clear();
            child.local_variables.clear();
            child.transient_variables.clear();
            self.apply_instance_variables(
                &mut child,
                i,
                collection_items.and_then(|items| items.get(i as usize).cloned()),
            );

            command_context
                .execution_entity_manager
                .insert(&child, &mut command_context.session);

            record_mi_child_activity_start(command_context, &child);

            self.inner_behavior.execute(&mut child, command_context)?;

            if child_has_wait_state(command_context, &child) {
                command_context
                    .execution_entity_manager
                    .update(&child, &mut command_context.session);
                continue;
            }

            record_mi_child_activity_end(command_context, &child);

            child.is_active = false;
            child.is_ended = true;
            command_context
                .execution_entity_manager
                .update(&child, &mut command_context.session);

            completed_instances += 1;
            active_instances -= 1;
            execution.set_local_variable(
                "nrOfCompletedInstances".to_string(),
                completed_instances.into(),
            );
            execution
                .set_local_variable("nrOfActiveInstances".to_string(), active_instances.into());
            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);

            if multi_instance_completion_condition_satisfied(
                command_context,
                &self.mi_characteristics,
                execution,
            )? {
                break;
            }
        }

        let with_condition = multi_instance_completion_condition_satisfied(
            command_context,
            &self.mi_characteristics,
            execution,
        )?;
        if completed_instances == total || with_condition {
            // Java ParallelMultiInstanceBehavior.java:302-319.
            cleanup_mi_root_and_leave(execution, command_context, with_condition);
            return Ok(());
        }

        // Parallel wait-state: MI root stays inactive (already false from
        // materialization); children hold the wait states.
        execution.is_active = false;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        Ok(())
    }
}

/// Java `ContinueProcessOperation#createMultiInstanceRootExecution`:
/// replace the arriving execution with a dedicated inactive MI root child of
/// the same parent. Instance children hang under that root.
///
/// Rust adaptation when the arriver **is** the process-instance scope row
/// (`parent_id == None`): Java always arrives on a non-PI child token, so it
/// can delete the arriver. Rust sequential flow keeps the PI as the token —
/// we must not delete the PI. Instead create the MI root as a child of the PI
/// and leave the PI inactive (same tree shape: PI → MI root → instances).
pub(crate) fn materialize_multi_instance_root(
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<(), FlowableError> {
    if execution.is_multi_instance_root {
        return Ok(());
    }

    if execution.is_process_instance_scope_execution() {
        let mut mi_root = new_child_execution(execution, Some(execution.id.clone()));
        mi_root.is_multi_instance_root = true;
        mi_root.is_active = false;
        mi_root.is_scope = false;
        mi_root.is_ended = false;
        // Process variables stay on the PI scope row; nrOf* are written local
        // on the MI root after materialization.
        mi_root.variables.clear();
        mi_root.local_variables.clear();
        mi_root.transient_variables.clear();

        // Clear the PI activity id so a later leave-child end under the PI does
        // not re-take the stale MI activity's outgoing flows (end-event scope
        // leave only fires when parent.activity_id is Some). The MI activity
        // lives on the dedicated MI root from here on.
        execution.is_active = false;
        execution.activity_id = None;
        execution.activity_name = None;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);
        command_context
            .execution_entity_manager
            .insert(&mi_root, &mut command_context.session);
        *execution = mi_root;
        return Ok(());
    }

    // Non-PI arriver (fork child, subprocess child, …): Java deletes the
    // arriver and creates a fresh MI root under the same parent.
    let parent_id = execution.parent_id.clone();
    let old_id = execution.id.clone();
    let mut mi_root = new_child_execution(execution, parent_id);
    mi_root.activity_id = execution.activity_id.clone();
    mi_root.activity_name = execution.activity_name.clone();
    mi_root.is_multi_instance_root = true;
    mi_root.is_active = false;
    mi_root.is_scope = false;
    mi_root.is_ended = false;
    mi_root.variables.clear();
    mi_root.local_variables.clear();
    mi_root.transient_variables.clear();

    command_context
        .runtime_store
        .delete_event_wait_state_by_execution_id(&old_id, &mut command_context.session);
    command_context
        .runtime_store
        .delete_boundary_event_states_by_host_execution_id(&old_id, &mut command_context.session);
    command_context
        .runtime_store
        .delete_timer_job_states_by_execution_id(&old_id, &mut command_context.session);
    command_context
        .execution_entity_manager
        .delete(&old_id, &mut command_context.session);
    command_context
        .execution_entity_manager
        .insert(&mi_root, &mut command_context.session);
    *execution = mi_root;
    Ok(())
}

/// Java `MultiInstanceActivityBehavior#cleanupMiRoot`:
/// delete MI root + all children, create a fresh leave execution under the
/// MI root's parent, take outgoing sequence flows on that leave execution.
///
/// `completed_with_condition` selects between
/// `MULTI_INSTANCE_ACTIVITY_COMPLETED` and
/// `MULTI_INSTANCE_ACTIVITY_COMPLETED_WITH_CONDITION`
/// (Java `SequentialMultiInstanceBehavior.java:90-97` /
/// `ParallelMultiInstanceBehavior.java:302-319`).
pub(crate) fn cleanup_mi_root_and_leave(
    mi_body_execution: &Execution,
    command_context: &mut CommandContext,
    completed_with_condition: bool,
) {
    let Some(mi_root) = resolve_multi_instance_root(command_context, mi_body_execution) else {
        // Fallback: not under an MI root — take outgoing on the given execution.
        let mut leave = mi_body_execution.clone();
        leave.is_active = true;
        leave.is_ended = false;
        leave.is_multi_instance_root = false;
        command_context
            .execution_entity_manager
            .update(&leave, &mut command_context.session);
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(leave);
        return;
    };

    // P119: emit MULTI_INSTANCE_ACTIVITY_COMPLETED(*) before the root is
    // deleted (Java `sendCompletedEvent` / `sendCompletedWithConditionEvent`
    // runs immediately before `super.leave` → `cleanupMiRoot`).
    let activity_id_for_event = mi_root.activity_id.clone().unwrap_or_default();
    let activity_type = mi_root
        .activity_id
        .as_deref()
        .and_then(|aid| {
            let pd = mi_root.process_definition_id.as_deref()?;
            let model = command_context.deployment_manager.get_bpmn_model(pd)?;
            let process = model.main_process.as_ref()?;
            let fe = process.flow_element_map.get(aid)?;
            Some(crate::agenda::continue_process_operation::flow_element_type(fe).to_string())
        })
        .unwrap_or_else(|| "activity".to_string());
    crate::engine::event_dispatcher::dispatch_multi_instance_activity_completed(
        command_context,
        &activity_id_for_event,
        &activity_type,
        mi_root.process_instance_id.as_deref(),
        Some(&mi_root.id),
        mi_root.process_definition_id.as_deref(),
        completed_with_condition,
    );

    let parent_id = mi_root.parent_id.clone();
    let activity_id = mi_root.activity_id.clone();
    let activity_name = mi_root.activity_name.clone();
    let mi_root_id = mi_root.id.clone();

    // Promote non-bookkeeping variables from the MI root onto its parent before
    // the root is deleted. Java `cleanupMiRoot` / variable aggregation writes
    // completed aggregates onto `multiInstanceRootExecution.getParent()`.
    promote_mi_root_variables_to_parent(command_context, &mi_root);

    // Delete MI root tree (instance scopes + nested SubProcess children).
    // Java `deleteChildExecutions` is recursive; SubProcess MI nests tasks under
    // the instance scope child, so a one-level delete leaves orphans.
    delete_execution_tree(command_context, &mi_root_id);

    // Fresh leave execution under the MI root's parent (may be PI or fork scope).
    let mut leave = new_child_execution(&mi_root, parent_id);
    leave.activity_id = activity_id;
    leave.activity_name = activity_name;
    leave.is_multi_instance_root = false;
    leave.is_active = true;
    leave.is_ended = false;
    leave.is_scope = false;
    leave.variables.clear();
    leave.local_variables.clear();
    leave.transient_variables.clear();

    command_context
        .execution_entity_manager
        .insert(&leave, &mut command_context.session);
    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(leave);
}

/// Bookkeeping locals that belong only on the MI root and must not leak to the
/// parent on leave (Java does not promote `nrOf*`).
const MI_ROOT_BOOKKEEPING: &[&str] = &[
    "nrOfInstances",
    "nrOfCompletedInstances",
    "nrOfActiveInstances",
];

fn promote_mi_root_variables_to_parent(command_context: &mut CommandContext, mi_root: &Execution) {
    let Some(parent_id) = mi_root.parent_id.as_deref() else {
        return;
    };
    let Some(mut parent) = command_context
        .execution_entity_manager
        .find_by_id(parent_id, &mut command_context.session)
    else {
        return;
    };

    // Prefer the live MI root row (may have aggregation writes not in the
    // caller's snapshot).
    let live_root = command_context
        .execution_entity_manager
        .find_by_id(&mi_root.id, &mut command_context.session)
        .unwrap_or_else(|| mi_root.clone());

    for (name, value) in live_root.variables.iter() {
        if MI_ROOT_BOOKKEEPING.contains(&name.as_str()) {
            continue;
        }
        parent.set_process_variable(name.clone(), value.clone());
    }
    for (name, value) in live_root.local_variables.iter() {
        if MI_ROOT_BOOKKEEPING.contains(&name.as_str()) {
            continue;
        }
        // Aggregations may be stored as process vars on the MI root; if they
        // landed in local_variables, promote as process vars on the parent so
        // process-level reads (and the PI scope row) see them.
        parent.set_process_variable(name.clone(), value.clone());
    }

    command_context
        .execution_entity_manager
        .update(&parent, &mut command_context.session);
}

/// Walk to the multi-instance root for `execution` (self or ancestor).
pub(crate) fn resolve_multi_instance_root(
    command_context: &mut CommandContext,
    execution: &Execution,
) -> Option<Execution> {
    if execution.is_multi_instance_root {
        return command_context
            .execution_entity_manager
            .find_by_id(&execution.id, &mut command_context.session)
            .or_else(|| Some(execution.clone()));
    }
    let mut current_parent = execution.parent_id.clone();
    while let Some(parent_id) = current_parent {
        let parent = command_context
            .execution_entity_manager
            .find_by_id(&parent_id, &mut command_context.session)?;
        if parent.is_multi_instance_root {
            return Some(parent);
        }
        current_parent = parent.parent_id.clone();
    }
    None
}

/// Java parity (`ContinueProcessOperation#executeMultiInstanceSynchronous`
/// 221–233 + `ContinueMultiInstanceOperation`): boundary events of a
/// multi-instance activity attach once, on the MI root execution — instance
/// children never register boundary events. Returns the MI root id when
/// `execution` is a direct MI instance child, otherwise the execution's own
/// id (non-MI path unchanged).
pub(crate) fn boundary_host_execution_id(
    command_context: &mut CommandContext,
    execution: &Execution,
) -> String {
    if let Some(parent_id) = execution.parent_id.as_deref() {
        let parent_is_mi_root = command_context
            .execution_entity_manager
            .find_by_id(parent_id, &mut command_context.session)
            .map(|parent| parent.is_multi_instance_root)
            .unwrap_or(false);
        if parent_is_mi_root {
            return parent_id.to_string();
        }
    }
    execution.id.clone()
}

fn new_child_execution(template: &Execution, parent_id: Option<String>) -> Execution {
    Execution {
        id: Uuid::new_v4().to_string(),
        parent_id,
        super_execution_id: template.super_execution_id.clone(),
        root_process_instance_id: template.root_process_instance_id.clone(),
        process_instance_id: template.process_instance_id.clone(),
        process_definition_id: template.process_definition_id.clone(),
        process_definition_key: template.process_definition_key.clone(),
        process_definition_name: template.process_definition_name.clone(),
        process_definition_version: template.process_definition_version,
        activity_id: template.activity_id.clone(),
        activity_name: template.activity_name.clone(),
        name: None,
        description: None,
        is_suspended: template.is_suspended,
        is_ended: false,
        is_active: true,
        is_concurrent: false,
        is_scope: false,
        is_multi_instance_root: false,
        reference_id: None,
        reference_type: None,
        tenant_id: template.tenant_id.clone(),
        variables: HashMap::new(),
        local_variables: HashMap::new(),
        transient_variables: HashMap::new(),
        non_interrupting_event_subprocess_path: false,
    }
}

pub(crate) fn delete_execution_related_runtime_data(command_context: &mut CommandContext, execution_id: &str) {
    if let Some(task) = command_context
        .task_entity_manager
        .find_by_execution_id(execution_id, &mut command_context.session)
    {
        command_context
            .task_entity_manager
            .delete(&task.id, &mut command_context.session);
    }
    command_context
        .runtime_store
        .delete_event_wait_state_by_execution_id(execution_id, &mut command_context.session);
    command_context
        .runtime_store
        .delete_boundary_event_states_by_host_execution_id(
            execution_id,
            &mut command_context.session,
        );
    command_context
        .runtime_store
        .delete_timer_job_states_by_execution_id(execution_id, &mut command_context.session);
}

/// Fire `ACTIVITY_MESSAGE_CANCELLED` for every message subscription still
/// attached to `execution_id` (intermediate wait state + boundary host).
/// Java: `ExecutionEntityManagerImpl.deleteEventSubScriptions` (1050-1075).
fn dispatch_message_cancelled_for_execution(
    command_context: &mut CommandContext,
    execution_id: &str,
) {
    use crate::persistence::runtime_store::EventSubscriptionKind;

    let process_definition_id = command_context
        .runtime_store
        .find_execution(execution_id, &mut command_context.session)
        .and_then(|e| e.process_definition_id);

    if let Some(wait) = command_context
        .runtime_store
        .find_event_wait_state_by_execution_id(execution_id, &mut command_context.session)
    {
        if let Some(sub) = wait.event_subscription.as_ref() {
            if sub.kind == EventSubscriptionKind::Message {
                let activity_id = wait.activity_id.as_deref().unwrap_or("");
                crate::engine::event_dispatcher::dispatch_activity_message_cancelled(
                    command_context,
                    activity_id,
                    &sub.event_ref,
                    Some(&wait.process_instance_id),
                    Some(execution_id),
                    process_definition_id.as_deref(),
                );
            }
        }
    }

    let boundaries = command_context
        .runtime_store
        .find_boundary_event_states_by_host_execution_id(
            execution_id,
            &mut command_context.session,
        );
    for boundary in boundaries {
        if boundary.event_subscription.kind == EventSubscriptionKind::Message {
            crate::engine::event_dispatcher::dispatch_activity_message_cancelled(
                command_context,
                &boundary.boundary_event_id,
                &boundary.event_subscription.event_ref,
                Some(&boundary.process_instance_id),
                Some(execution_id),
                process_definition_id.as_deref(),
            );
        }
    }
}

fn child_has_wait_state(command_context: &mut CommandContext, child: &Execution) -> bool {
    if command_context
        .task_entity_manager
        .find_by_execution_id(&child.id, &mut command_context.session)
        .is_some()
        || command_context
            .runtime_store
            .find_event_wait_state_by_execution_id(&child.id, &mut command_context.session)
            .is_some()
    {
        return true;
    }
    // SubProcess / Transaction instance children are scope rows: the wait
    // state (nested userTask / receiveTask / …) hangs under a descendant, not
    // on the instance child itself. After `SubProcessActivityBehavior::execute`
    // returns, a nested start-event child is already inserted — treat any
    // open descendant as an outstanding wait so parallel MI does not
    // prematurely `cleanupMiRoot`.
    !command_context
        .execution_entity_manager
        .find_child_executions_by_parent_execution_id(&child.id, &mut command_context.session)
        .is_empty()
}

/// Create a sequential MI instance child under the MI root.
/// `is_scope` is true for SubProcess (Java `setScope(true)` on continue).
fn create_sequential_instance_child(
    mi_root: &Execution,
    command_context: &mut CommandContext,
    is_scope: bool,
) -> Execution {
    let mut child = mi_root.clone();
    child.id = Uuid::new_v4().to_string();
    child.parent_id = Some(mi_root.id.clone());
    child.is_active = true;
    child.is_ended = false;
    child.is_scope = is_scope;
    // Instance children are never MI roots (Java `createChildExecution`).
    child.is_multi_instance_root = false;
    // Java parity: instance executions start with empty variable maps and
    // resolve inherited names through the parent VariableScope chain.
    child.variables.clear();
    child.local_variables.clear();
    child.transient_variables.clear();
    command_context
        .execution_entity_manager
        .insert(&child, &mut command_context.session);
    child
}

/// Recursively delete an execution and all descendants (Java
/// `deleteChildExecutions` + `deleteExecutionAndRelatedData` with no reason).
pub(crate) fn delete_execution_tree(command_context: &mut CommandContext, root_id: &str) {
    delete_execution_tree_with_reason(command_context, root_id, None);
}

/// Java `deleteChildExecutions` + `deleteExecutionAndRelatedData(reason)`:
/// ends open historic activities with `delete_reason` before stripping runtime
/// data and deleting each execution row. Pass `None` for a normal destroy
/// (MI leave, scope end without cancel semantics).
pub(crate) fn delete_execution_tree_with_reason(
    command_context: &mut CommandContext,
    root_id: &str,
    delete_reason: Option<&str>,
) {
    let child_ids: Vec<String> = command_context
        .execution_entity_manager
        .find_child_executions_by_parent_execution_id(root_id, &mut command_context.session)
        .into_iter()
        .map(|c| c.id)
        .collect();
    for child_id in child_ids {
        delete_execution_tree_with_reason(command_context, &child_id, delete_reason);
    }
    delete_execution_and_related_data(command_context, root_id, delete_reason);
}

/// Java `ExecutionEntityManager.deleteExecutionAndRelatedData(execution, deleteReason, …)`:
/// record activity end (with reason) when the execution has a current activity,
/// then strip related runtime data and delete the execution row.
///
/// Does **not** delete the process-instance scope row: callers that need to
/// retire a PI-as-host must strip runtime state separately (boundary / event
/// subprocess interrupting paths).
pub(crate) fn delete_execution_and_related_data(
    command_context: &mut CommandContext,
    execution_id: &str,
    delete_reason: Option<&str>,
) {
    // P119: MULTI_INSTANCE_ACTIVITY_CANCELLED when cancelling an MI root
    // (Java `ExecutionEntityManagerImpl.dispatchExecutionCancelled` →
    // `dispatchMultiInstanceActivityCancelled` at lines 755-756 / 777-785).
    // Only fire on cancel paths (delete_reason present); normal MI leave uses
    // COMPLETED instead via cleanup_mi_root_and_leave.
    if delete_reason.is_some() {
        if let Some(execution) = command_context
            .runtime_store
            .find_execution(execution_id, &mut command_context.session)
        {
            if execution.is_multi_instance_root {
                if let Some(activity_id) = execution.activity_id.as_deref() {
                    let activity_type = execution
                        .process_definition_id
                        .as_deref()
                        .and_then(|pd| {
                            let model = command_context.deployment_manager.get_bpmn_model(pd)?;
                            let process = model.main_process.as_ref()?;
                            let fe = process.flow_element_map.get(activity_id)?;
                            Some(
                                crate::agenda::continue_process_operation::flow_element_type(fe)
                                    .to_string(),
                            )
                        })
                        .unwrap_or_else(|| "activity".to_string());
                    crate::engine::event_dispatcher::dispatch_multi_instance_activity_cancelled(
                        command_context,
                        activity_id,
                        &activity_type,
                        execution.process_instance_id.as_deref(),
                        Some(&execution.id),
                        execution.process_definition_id.as_deref(),
                    );
                }
            }
        }
    }
    // P125: ACTIVITY_MESSAGE_CANCELLED for message subscriptions removed with
    // the execution (Java ExecutionEntityManagerImpl.deleteEventSubScriptions
    // 1050-1075 / ACTIVITY_MESSAGE_CANCELLED at 1063-1066). Normal message
    // receive deletes the subscription outside this path and must not fire.
    dispatch_message_cancelled_for_execution(command_context, execution_id);
    record_activity_end_for_execution(command_context, execution_id, delete_reason);
    delete_execution_related_runtime_data(command_context, execution_id);
    command_context
        .runtime_store
        .delete_event_subprocess_event_subscriptions_by_scope_execution_id(
            execution_id,
            &mut command_context.session,
        );
    command_context
        .execution_entity_manager
        .delete(execution_id, &mut command_context.session);
}

/// Ends the open historic activity for `execution_id` when it has an
/// `activity_id`. Mirrors Java
/// `ActivityInstanceEntityManager.recordActivityEnd(execution, deleteReason)`.
///
/// Safe to call when the execution row is missing or has no activity (no-op).
/// Unlike Java we do not gate on `isActive`: wait-state hosts (userTask /
/// intermediate catch) are inactive but still have open historic activities
/// that must receive the cancel reason.
pub(crate) fn record_activity_end_for_execution(
    command_context: &mut CommandContext,
    execution_id: &str,
    delete_reason: Option<&str>,
) {
    let Some(execution) = command_context
        .runtime_store
        .find_execution(execution_id, &mut command_context.session)
    else {
        return;
    };
    if execution.is_multi_instance_root {
        return;
    }
    let Some(activity_id) = execution.activity_id.as_deref() else {
        return;
    };
    if activity_id.is_empty() {
        return;
    }
    command_context.history_manager.record_activity_end(
        execution_id,
        activity_id,
        delete_reason,
        &mut command_context.session,
    );
}

/// Java `EndExecutionOperation#handleMultiInstanceSubProcess` +
/// `SequentialMultiInstanceBehavior#leave` / parallel leave for an embedded
/// SubProcess instance.
///
/// Called when the SubProcess scope row ends (inner end event) and its parent
/// is the dedicated MI root. Destroys the completed scope (DestroyScope), then
/// either continues the next sequential round, waits for parallel siblings, or
/// `cleanupMiRoot`.
/// Returns `Ok(true)` when the leave was handled as multi-instance SubProcess
/// leave; `Ok(false)` when the caller should fall through to normal SubProcess
/// outgoing leave.
pub(crate) fn leave_sequential_subprocess_mi_instance(
    scope_execution: &Execution,
    command_context: &mut CommandContext,
) -> Result<bool, FlowableError> {
    let Some(mi_root_id) = scope_execution.parent_id.clone() else {
        return Ok(false);
    };
    let Some(mut mi_root) = command_context
        .execution_entity_manager
        .find_by_id(&mi_root_id, &mut command_context.session)
    else {
        return Ok(false);
    };
    if !mi_root.is_multi_instance_root {
        return Ok(false);
    }

    let Some(mi) = resolve_mi_loop_characteristics(command_context, &mi_root) else {
        // MI root flag set but characteristics missing — still not normal leave.
        return Ok(false);
    };

    let nr_of_instances = mi_root
        .process_variable("nrOfInstances")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let nr_of_completed = mi_root
        .process_variable("nrOfCompletedInstances")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        + 1;

    mi_root.set_local_variable("nrOfCompletedInstances".to_string(), nr_of_completed.into());

    if mi.sequential {
        // Sequential: one active instance while rounds remain.
        mi_root.set_local_variable("nrOfActiveInstances".to_string(), 1.into());
        command_context
            .execution_entity_manager
            .update(&mi_root, &mut command_context.session);

        let complete_condition =
            multi_instance_completion_condition_satisfied(command_context, &mi, &mi_root)?;
        let more_rounds = nr_of_completed < nr_of_instances;

        // Java DestroyScope on the completed SubProcess scope before continue/leave.
        let scope_id = scope_execution.id.clone();
        delete_execution_tree(command_context, &scope_id);

        if complete_condition || !more_rounds {
            // SequentialMultiInstanceBehavior.java:90-97.
            cleanup_mi_root_and_leave(&mi_root, command_context, complete_condition);
            return Ok(true);
        }

        // Next sequential SubProcess round: re-enter MI on the root (creates a
        // fresh scope child in `execute_sequential_subprocess`).
        command_context
            .agenda
            .plan_continue_process_operation(mi_root);
        return Ok(true);
    }

    // Parallel SubProcess instance leave: destroy this scope, wait for siblings.
    let nr_of_active = mi_root
        .process_variable("nrOfActiveInstances")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .saturating_sub(1);
    mi_root.set_local_variable("nrOfActiveInstances".to_string(), nr_of_active.into());
    command_context
        .execution_entity_manager
        .update(&mi_root, &mut command_context.session);

    let scope_id = scope_execution.id.clone();
    delete_execution_tree(command_context, &scope_id);

    let complete_condition =
        multi_instance_completion_condition_satisfied(command_context, &mi, &mi_root)?;
    if complete_condition || nr_of_completed >= nr_of_instances || nr_of_active <= 0 {
        // Parallel MultiInstance leave with optional completion condition.
        cleanup_mi_root_and_leave(&mi_root, command_context, complete_condition);
    }
    Ok(true)
}

/// Look up `multiInstanceLoopCharacteristics` for the MI root's activity.
pub(crate) fn resolve_mi_loop_characteristics(
    command_context: &mut CommandContext,
    mi_root: &Execution,
) -> Option<MultiInstanceLoopCharacteristics> {
    let activity_id = mi_root.activity_id.as_deref()?;
    let pd_id = mi_root.process_definition_id.as_deref()?;
    let model = command_context.deployment_manager.get_bpmn_model(pd_id)?;
    let process = model.main_process.as_ref()?;
    find_loop_characteristics_in_elements(&process.flow_elements, activity_id)
}

fn find_loop_characteristics_in_elements(
    flow_elements: &[flowable_bpmn_model::model::FlowElementEnum],
    activity_id: &str,
) -> Option<MultiInstanceLoopCharacteristics> {
    use flowable_bpmn_model::model::FlowElementEnum;
    for el in flow_elements {
        match el {
            FlowElementEnum::SubProcess(sp)
                if sp
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id) =>
            {
                return sp.activity.loop_characteristics.clone();
            }
            FlowElementEnum::UserTask(ut)
                if ut
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id) =>
            {
                return ut.task.activity.loop_characteristics.clone();
            }
            FlowElementEnum::ServiceTask(st)
                if st
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id) =>
            {
                return st.task.activity.loop_characteristics.clone();
            }
            FlowElementEnum::CaseServiceTask(st)
                if st
                    .service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id) =>
            {
                return st.service_task.task.activity.loop_characteristics.clone();
            }
            FlowElementEnum::CallActivity(ca)
                if ca
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id) =>
            {
                return ca.activity.loop_characteristics.clone();
            }
            FlowElementEnum::Transaction(t) => {
                let sp = &t.sub_process;
                if sp
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id)
                {
                    return sp.activity.loop_characteristics.clone();
                }
                if let Some(found) =
                    find_loop_characteristics_in_elements(&sp.flow_elements, activity_id)
                {
                    return Some(found);
                }
            }
            FlowElementEnum::AdhocSubProcess(t) => {
                let sp = &t.sub_process;
                if sp
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id)
                {
                    return sp.activity.loop_characteristics.clone();
                }
                if let Some(found) =
                    find_loop_characteristics_in_elements(&sp.flow_elements, activity_id)
                {
                    return Some(found);
                }
            }
            FlowElementEnum::SubProcess(sp) => {
                if let Some(found) =
                    find_loop_characteristics_in_elements(&sp.flow_elements, activity_id)
                {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

/// Java sequential MI keeps a single non-ended instance child under the MI
/// root. On re-entry (after a wait-state complete) that child is reused.
/// Only for non-SubProcess sequential MI (P6-A).
fn find_reusable_sequential_child(
    command_context: &mut CommandContext,
    mi_root_id: &str,
) -> Option<Execution> {
    command_context
        .execution_entity_manager
        .find_child_executions_by_parent_execution_id(mi_root_id, &mut command_context.session)
        .into_iter()
        .find(|child| !child.is_ended)
}

/// Java `continueSequentialMultiInstance` (non-SubProcess): remove every local
/// variable except the nrOf* bookkeeping names (those belong on the MI root;
/// the filter matches Java's defensive set).
fn clear_sequential_instance_locals(child: &mut Execution) {
    const PRESERVE: &[&str] = &[
        "nrOfInstances",
        "nrOfCompletedInstances",
        "nrOfActiveInstances",
    ];
    child
        .local_variables
        .retain(|name, _| PRESERVE.contains(&name.as_str()));
    // Instance vars are local-only after P5-B; still clear the process map so a
    // prior round cannot leave a shadowing snapshot on the reused child.
    child.variables.clear();
    child.transient_variables.clear();
}

pub(crate) fn record_mi_child_activity_start(
    command_context: &mut CommandContext,
    child: &Execution,
) {
    let Some(activity_id) = child.activity_id.as_deref() else {
        return;
    };
    let Some(process_def_id) = child.process_definition_id.as_deref() else {
        return;
    };
    let Some(process_instance_id) = child.process_instance_id.as_deref() else {
        return;
    };
    let Some(bpmn_model) = command_context
        .deployment_manager
        .get_bpmn_model(process_def_id)
    else {
        return;
    };
    let Some(main_process) = bpmn_model.main_process.as_ref() else {
        return;
    };
    let Some(flow_element) = find_flow_element(main_process, activity_id) else {
        return;
    };
    let activity_id_str = flow_element_id(flow_element).unwrap_or("<unknown>");
    let activity_type = flow_element_type(flow_element);
    command_context.history_manager.record_activity_start(
        activity_id_str,
        None,
        activity_type,
        process_instance_id,
        &child.id,
        &mut command_context.session,
    );
}

pub(crate) fn record_mi_child_activity_end(
    command_context: &mut CommandContext,
    child: &Execution,
) {
    let Some(activity_id) = child.activity_id.as_deref() else {
        return;
    };
    command_context.history_manager.record_activity_end(
        &child.id,
        activity_id,
        None,
        &mut command_context.session,
    );
}

fn end_sequential_instance_child(command_context: &mut CommandContext, child: &mut Execution) {
    child.is_active = false;
    child.is_ended = true;
    command_context
        .execution_entity_manager
        .update(child, &mut command_context.session);
}

/// Evaluate MI loopCardinality / collection expression text.
///
/// Java `expressionManager.createExpression(text)` accepts both `${…}` and bare
/// tokens (literals / variable names). `SimpleExpression` only compiles `${…}`,
/// so bare text is wrapped as `${text}` before evaluation.
fn evaluate_mi_expression(text: &str, scope: &Execution) -> Option<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let el_text = if trimmed.starts_with("${") && trimmed.ends_with('}') {
        trimmed.to_string()
    } else {
        // e.g. "5" → "${5}", "approvers" → "${approvers}"
        format!("${{{}}}", trimmed)
    };
    crate::el::expression::SimpleExpression::new(el_text).get_value(scope)
}

fn multi_instance_completion_condition_satisfied(
    command_context: &mut CommandContext,
    mi: &MultiInstanceLoopCharacteristics,
    execution: &Execution,
) -> Result<bool, FlowableError> {
    let Some(condition) = &mi.completion_condition else {
        return Ok(false);
    };

    // Java parity: the completion condition is evaluated with
    // `expressionManager.createExpression(…).getValue(execution)`, and EL
    // variable resolution walks the VariableScope parent chain. Evaluate on the
    // P4-7a evaluation execution so process-level names (e.g. a threshold held
    // by the process-instance row) resolve.
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);
    match crate::el::expression::SimpleExpression::new(condition.clone())
        .get_value(&evaluation_execution)
    {
        Some(Value::Bool(value)) => Ok(value),
        Some(value) => Err(FlowableError::Generic(format!(
            "Multi-instance completionCondition must evaluate to a boolean, got {value}"
        ))),
        None => Ok(false),
    }
}
