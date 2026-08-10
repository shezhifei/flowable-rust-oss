//! Admin display-json assembly from `BpmnModel` DI (Java `DisplayJsonClientResource`)
//! and from the CMMN case model (Java `CmmnDisplayJsonClientResource`).

use flowable_bpmn_model::model::{Activity, BpmnModel, FlowElement, FlowElementEnum, GraphicInfo};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCasePlanModel, CmmnCaseTask, CmmnDecisionTask, CmmnEventListener, CmmnHumanTask,
    CmmnMilestone, CmmnPlanItem, CmmnProcessTask, CmmnStage,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Build admin UI display JSON for a process definition model.
pub fn build_process_definition_display(model: &BpmnModel) -> Value {
    build_display(model, None, None)
}

/// Build display JSON with runtime highlighting for an instance.
pub fn build_process_instance_display(
    model: &BpmnModel,
    completed: &[String],
    current: &[String],
) -> Value {
    let completed_set: HashSet<_> = completed.iter().cloned().collect();
    let current_set: HashSet<_> = current.iter().cloned().collect();
    let mut node = build_display(model, Some(&completed_set), Some(&current_set));
    if let Some(obj) = node.as_object_mut() {
        obj.insert("completedActivities".into(), json!(completed));
        obj.insert("currentActivities".into(), json!(current));
        let flows = gather_completed_flows(model, completed, current);
        obj.insert("completedSequenceFlows".into(), json!(flows));
    }
    node
}

/// Historic-only highlighting (no current activities).
pub fn build_history_display(model: &BpmnModel, completed: &[String]) -> Value {
    let completed_set: HashSet<_> = completed.iter().cloned().collect();
    let mut node = build_display(model, Some(&completed_set), None);
    if let Some(obj) = node.as_object_mut() {
        obj.insert("completedActivities".into(), json!(completed));
        let flows = gather_completed_flows(model, completed, &[]);
        obj.insert("completedSequenceFlows".into(), json!(flows));
    }
    node
}

fn build_display(
    model: &BpmnModel,
    completed: Option<&HashSet<String>>,
    current: Option<&HashSet<String>>,
) -> Value {
    if model.location_map.is_empty() {
        return json!({});
    }

    let mut diagram_x = f64::MAX;
    let mut diagram_y = f64::MAX;
    let mut diagram_right = 0.0_f64;
    let mut diagram_bottom = 0.0_f64;
    let mut first = true;

    let mut pools = Vec::new();
    for pool in &model.pools {
        let id = pool
            .base_element
            .id
            .clone()
            .unwrap_or_else(|| "pool".into());
        if let Some(gi) = model.location_map.get(&id) {
            let mut pool_node = json!({
                "id": id,
                "name": pool.name,
            });
            fill_graphic(&mut pool_node, gi, true);
            pools.push(pool_node);
            expand_diagram(
                gi,
                &mut diagram_x,
                &mut diagram_y,
                &mut diagram_right,
                &mut diagram_bottom,
                &mut first,
            );
        }
    }

    let mut elements = Vec::new();
    let mut flows = Vec::new();

    let processes: Vec<&flowable_bpmn_model::model::Process> = if model.processes.is_empty() {
        model.main_process.iter().collect()
    } else {
        model.processes.iter().collect()
    };

    for process in processes {
        process_elements(
            &process.flow_elements,
            model,
            &mut elements,
            &mut flows,
            completed,
            current,
            &mut diagram_x,
            &mut diagram_y,
            &mut diagram_right,
            &mut diagram_bottom,
            &mut first,
        );
    }

    if first {
        diagram_x = 0.0;
        diagram_y = 0.0;
    }

    let mut display = json!({
        "elements": elements,
        "flows": flows,
        "collapsed": [],
        "diagramBeginX": diagram_x,
        "diagramBeginY": diagram_y,
        "diagramWidth": diagram_right,
        "diagramHeight": diagram_bottom,
    });
    if !pools.is_empty() {
        display
            .as_object_mut()
            .unwrap()
            .insert("pools".into(), json!(pools));
    }
    display
}

fn process_elements(
    list: &[FlowElementEnum],
    model: &BpmnModel,
    elements: &mut Vec<Value>,
    flows: &mut Vec<Value>,
    completed: Option<&HashSet<String>>,
    current: Option<&HashSet<String>>,
    diagram_x: &mut f64,
    diagram_y: &mut f64,
    diagram_right: &mut f64,
    diagram_bottom: &mut f64,
    first: &mut bool,
) {
    for el in list {
        match el {
            FlowElementEnum::SequenceFlow(sf) => {
                let fe = &sf.flow_element;
                let id = fe.base_element.id.clone().unwrap_or_default();
                let mut node = json!({
                    "id": id,
                    "type": "sequenceFlow",
                    "sourceRef": sf.source_ref,
                    "targetRef": sf.target_ref,
                    "name": fe.name,
                });
                if let Some(c) = completed {
                    node.as_object_mut()
                        .unwrap()
                        .insert("completed".into(), json!(c.contains(&id)));
                }
                let waypoints = model
                    .flow_location_map
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                let mut wp = Vec::new();
                for gi in &waypoints {
                    let mut p = json!({});
                    fill_graphic(&mut p, gi, false);
                    wp.push(p);
                    expand_diagram(gi, diagram_x, diagram_y, diagram_right, diagram_bottom, first);
                }
                node.as_object_mut()
                    .unwrap()
                    .insert("waypoints".into(), json!(wp));
                flows.push(node);
            }
            other => {
                let (id, name, type_name, nested) = element_meta(other);
                let mut node = json!({
                    "id": id,
                    "name": name,
                    "type": type_name,
                });
                if let Some(c) = completed {
                    node.as_object_mut()
                        .unwrap()
                        .insert("completed".into(), json!(c.contains(&id)));
                }
                if let Some(c) = current {
                    node.as_object_mut()
                        .unwrap()
                        .insert("current".into(), json!(c.contains(&id)));
                }
                if let Some(gi) = model.location_map.get(&id) {
                    fill_graphic(&mut node, gi, true);
                    expand_diagram(gi, diagram_x, diagram_y, diagram_right, diagram_bottom, first);
                }
                elements.push(node);
                if let Some(children) = nested {
                    process_elements(
                        children,
                        model,
                        elements,
                        flows,
                        completed,
                        current,
                        diagram_x,
                        diagram_y,
                        diagram_right,
                        diagram_bottom,
                        first,
                    );
                }
            }
        }
    }
}

fn fe_meta(fe: &FlowElement) -> (String, Option<String>) {
    (
        fe.base_element.id.clone().unwrap_or_default(),
        fe.name.clone(),
    )
}

fn act_meta(act: &Activity) -> (String, Option<String>) {
    fe_meta(&act.flow_node.flow_element)
}

fn element_meta(
    el: &FlowElementEnum,
) -> (String, Option<String>, &'static str, Option<&[FlowElementEnum]>) {
    match el {
        FlowElementEnum::StartEvent(e) => {
            let (id, name) = fe_meta(&e.event.flow_node.flow_element);
            (id, name, "StartEvent", None)
        }
        FlowElementEnum::EndEvent(e) => {
            let (id, name) = fe_meta(&e.event.flow_node.flow_element);
            (id, name, "EndEvent", None)
        }
        FlowElementEnum::UserTask(e) => {
            let (id, name) = act_meta(&e.task.activity);
            (id, name, "UserTask", None)
        }
        FlowElementEnum::ServiceTask(e) => {
            let (id, name) = act_meta(&e.task.activity);
            (id, name, "ServiceTask", None)
        }
        FlowElementEnum::ScriptTask(e) => {
            let (id, name) = act_meta(&e.task.activity);
            (id, name, "ScriptTask", None)
        }
        FlowElementEnum::ManualTask(e) => {
            let (id, name) = act_meta(&e.task.activity);
            (id, name, "ManualTask", None)
        }
        FlowElementEnum::ReceiveTask(e) => {
            let (id, name) = act_meta(&e.task.activity);
            (id, name, "ReceiveTask", None)
        }
        FlowElementEnum::SendTask(e) => {
            let (id, name) = act_meta(&e.service_task.task.activity);
            (id, name, "SendTask", None)
        }
        FlowElementEnum::BusinessRuleTask(e) => {
            let (id, name) = act_meta(&e.task.activity);
            (id, name, "BusinessRuleTask", None)
        }
        FlowElementEnum::Task(e) => {
            let (id, name) = act_meta(&e.activity);
            (id, name, "Task", None)
        }
        FlowElementEnum::ExclusiveGateway(e) => {
            let (id, name) = fe_meta(&e.gateway.flow_node.flow_element);
            (id, name, "ExclusiveGateway", None)
        }
        FlowElementEnum::ParallelGateway(e) => {
            let (id, name) = fe_meta(&e.gateway.flow_node.flow_element);
            (id, name, "ParallelGateway", None)
        }
        FlowElementEnum::InclusiveGateway(e) => {
            let (id, name) = fe_meta(&e.gateway.flow_node.flow_element);
            (id, name, "InclusiveGateway", None)
        }
        FlowElementEnum::EventBasedGateway(e) => {
            let (id, name) = fe_meta(&e.gateway.flow_node.flow_element);
            (id, name, "EventBasedGateway", None)
        }
        FlowElementEnum::ComplexGateway(e) => {
            let (id, name) = fe_meta(&e.gateway.flow_node.flow_element);
            (id, name, "ComplexGateway", None)
        }
        FlowElementEnum::BoundaryEvent(e) => {
            let (id, name) = fe_meta(&e.event.flow_node.flow_element);
            (id, name, "BoundaryEvent", None)
        }
        FlowElementEnum::IntermediateCatchEvent(e) => {
            let (id, name) = fe_meta(&e.event.flow_node.flow_element);
            (id, name, "IntermediateCatchEvent", None)
        }
        FlowElementEnum::IntermediateThrowEvent(e) => {
            let (id, name) = fe_meta(&e.event.flow_node.flow_element);
            (id, name, "ThrowEvent", None)
        }
        FlowElementEnum::CallActivity(e) => {
            let (id, name) = act_meta(&e.activity);
            (id, name, "CallActivity", None)
        }
        FlowElementEnum::SubProcess(e) => {
            let (id, name) = act_meta(&e.activity);
            (id, name, "SubProcess", Some(e.flow_elements.as_slice()))
        }
        FlowElementEnum::Transaction(e) => {
            let (id, name) = act_meta(&e.sub_process.activity);
            (
                id,
                name,
                "Transaction",
                Some(e.sub_process.flow_elements.as_slice()),
            )
        }
        FlowElementEnum::EventSubProcess(e) => {
            let (id, name) = act_meta(&e.sub_process.activity);
            (
                id,
                name,
                "EventSubProcess",
                Some(e.sub_process.flow_elements.as_slice()),
            )
        }
        FlowElementEnum::AdhocSubProcess(e) => {
            let (id, name) = act_meta(&e.sub_process.activity);
            (
                id,
                name,
                "AdhocSubProcess",
                Some(e.sub_process.flow_elements.as_slice()),
            )
        }
        FlowElementEnum::CaseServiceTask(e) => {
            let (id, name) = act_meta(&e.service_task.task.activity);
            (id, name, "ServiceTask", None)
        }
        FlowElementEnum::ValuedDataObject(e) => {
            let id = e.base_element.id.clone().unwrap_or_default();
            (id, e.name.clone(), "DataObject", None)
        }
        FlowElementEnum::SequenceFlow(e) => {
            let (id, name) = fe_meta(&e.flow_element);
            (id, name, "sequenceFlow", None)
        }
    }
}

fn fill_graphic(node: &mut Value, gi: &GraphicInfo, include_wh: bool) {
    let obj = node.as_object_mut().unwrap();
    obj.insert("x".into(), json!(gi.x));
    obj.insert("y".into(), json!(gi.y));
    if include_wh {
        obj.insert("width".into(), json!(gi.width));
        obj.insert("height".into(), json!(gi.height));
    }
}

fn expand_diagram(
    gi: &GraphicInfo,
    diagram_x: &mut f64,
    diagram_y: &mut f64,
    diagram_right: &mut f64,
    diagram_bottom: &mut f64,
    first: &mut bool,
) {
    let right = gi.x + gi.width.max(0.0);
    let bottom = gi.y + gi.height.max(0.0);
    if *first || gi.x < *diagram_x {
        *diagram_x = gi.x;
    }
    if *first || gi.y < *diagram_y {
        *diagram_y = gi.y;
    }
    if right > *diagram_right {
        *diagram_right = right;
    }
    if bottom > *diagram_bottom {
        *diagram_bottom = bottom;
    }
    *first = false;
}

fn gather_completed_flows(
    model: &BpmnModel,
    completed: &[String],
    current: &[String],
) -> Vec<String> {
    let mut activities: Vec<String> = completed.to_vec();
    activities.extend(current.iter().cloned());
    let mut completed_flows = Vec::new();

    let processes: Vec<&flowable_bpmn_model::model::Process> = if model.processes.is_empty() {
        model.main_process.iter().collect()
    } else {
        model.processes.iter().collect()
    };

    for process in processes {
        for el in &process.flow_elements {
            if let FlowElementEnum::SequenceFlow(sf) = el {
                let Some(src) = sf.source_ref.as_ref() else {
                    continue;
                };
                let Some(tgt) = sf.target_ref.as_ref() else {
                    continue;
                };
                if let Some(idx) = activities.iter().position(|a| a == src) {
                    if idx + 1 < activities.len() && activities[idx + 1] == *tgt {
                        if let Some(id) = &sf.flow_element.base_element.id {
                            completed_flows.push(id.clone());
                        }
                    }
                }
            }
        }
    }
    completed_flows
}

// ---------------------------------------------------------------------------
// CMMN display JSON (Java `CmmnDisplayJsonClientResource`)
// ---------------------------------------------------------------------------
//
// The Rust CMMN converter does not parse CMMNDI, so the engine model carries
// no graphic info of its own; the caller passes whatever graphic info it has
// (keyed by element id) and, mirroring the Java resource, an empty map yields
// an empty display object (`processCaseElements` only runs when the Java
// model's location map is non-empty).
//
// `#[allow(dead_code)]` on this section: `task/display_json.rs` includes this
// file via `#[path]` and only uses the BPMN builders today, so the CMMN
// builders (kept pub for the task side to reuse) would warn there.

/// Build admin UI display JSON for a case definition model.
#[allow(dead_code)]
pub fn build_case_definition_display(
    case: &CmmnCase,
    graphics: &HashMap<String, GraphicInfo>,
) -> Value {
    build_case_display(case, graphics, None, None, None)
}

/// Build display JSON with runtime highlighting for a case instance.
///
/// `completed` / `current` / `available` hold *plan item definition ids*
/// (Java matches plan item instances on `planItemDefinitionId`).
#[allow(dead_code)]
pub fn build_case_instance_display(
    case: &CmmnCase,
    graphics: &HashMap<String, GraphicInfo>,
    completed: &[String],
    current: &[String],
    available: &[String],
) -> Value {
    let completed_set: HashSet<_> = completed.iter().cloned().collect();
    let current_set: HashSet<_> = current.iter().cloned().collect();
    let available_set: HashSet<_> = available.iter().cloned().collect();
    let mut node = build_case_display(
        case,
        graphics,
        Some(&completed_set),
        Some(&current_set),
        Some(&available_set),
    );
    if let Some(obj) = node.as_object_mut() {
        if !obj.is_empty() {
            obj.insert("completedActivities".into(), json!(completed));
            obj.insert("currentActivities".into(), json!(current));
            obj.insert("availableActivities".into(), json!(available));
        }
    }
    node
}

#[allow(dead_code)]
fn build_case_display(
    case: &CmmnCase,
    graphics: &HashMap<String, GraphicInfo>,
    completed: Option<&HashSet<String>>,
    current: Option<&HashSet<String>>,
    available: Option<&HashSet<String>>,
) -> Value {
    if graphics.is_empty() {
        return json!({});
    }

    let mut diagram_x = f64::MAX;
    let mut diagram_y = f64::MAX;
    let mut diagram_right = 0.0_f64;
    let mut diagram_bottom = 0.0_f64;
    let mut first = true;

    let mut elements = Vec::new();
    // The Rust CMMN model has no associations, so there are no flows to emit.
    let flows: Vec<Value> = Vec::new();

    let plan_model = &case.case_plan_model;
    let mut plan_model_node = json!({
        "id": plan_model.id,
        "name": plan_model.name,
        "type": "PlanModel",
    });
    if let Some(gi) = graphics.get(&plan_model.id) {
        fill_graphic(&mut plan_model_node, gi, true);
        expand_diagram(
            gi,
            &mut diagram_x,
            &mut diagram_y,
            &mut diagram_right,
            &mut diagram_bottom,
            &mut first,
        );
    }
    elements.push(plan_model_node);

    process_cmmn_container(
        plan_model,
        graphics,
        &mut elements,
        completed,
        current,
        available,
        &mut diagram_x,
        &mut diagram_y,
        &mut diagram_right,
        &mut diagram_bottom,
        &mut first,
    );

    if first {
        diagram_x = 0.0;
        diagram_y = 0.0;
    }

    json!({
        "elements": elements,
        "flows": flows,
        "diagramBeginX": diagram_x,
        "diagramBeginY": diagram_y,
        "diagramWidth": diagram_right,
        "diagramHeight": diagram_bottom,
    })
}

/// Shared shape of `CmmnCasePlanModel` and `CmmnStage` (both are Java `Stage`s
/// holding plan items and plan item definitions).
#[allow(dead_code)]
trait PlanItemContainer {
    fn plan_items(&self) -> &[CmmnPlanItem];
    fn stages(&self) -> &[CmmnStage];
    fn human_tasks(&self) -> &[CmmnHumanTask];
    fn decision_tasks(&self) -> &[CmmnDecisionTask];
    fn process_tasks(&self) -> &[CmmnProcessTask];
    fn case_tasks(&self) -> &[CmmnCaseTask];
    fn milestones(&self) -> &[CmmnMilestone];
    fn event_listeners(&self) -> &[CmmnEventListener];
}

impl PlanItemContainer for CmmnCasePlanModel {
    fn plan_items(&self) -> &[CmmnPlanItem] {
        &self.plan_items
    }
    fn stages(&self) -> &[CmmnStage] {
        &self.stages
    }
    fn human_tasks(&self) -> &[CmmnHumanTask] {
        &self.human_tasks
    }
    fn decision_tasks(&self) -> &[CmmnDecisionTask] {
        &self.decision_tasks
    }
    fn process_tasks(&self) -> &[CmmnProcessTask] {
        &self.process_tasks
    }
    fn case_tasks(&self) -> &[CmmnCaseTask] {
        &self.case_tasks
    }
    fn milestones(&self) -> &[CmmnMilestone] {
        &self.milestones
    }
    fn event_listeners(&self) -> &[CmmnEventListener] {
        &self.event_listeners
    }
}

impl PlanItemContainer for CmmnStage {
    fn plan_items(&self) -> &[CmmnPlanItem] {
        &self.plan_items
    }
    fn stages(&self) -> &[CmmnStage] {
        &self.stages
    }
    fn human_tasks(&self) -> &[CmmnHumanTask] {
        &self.human_tasks
    }
    fn decision_tasks(&self) -> &[CmmnDecisionTask] {
        &self.decision_tasks
    }
    fn process_tasks(&self) -> &[CmmnProcessTask] {
        &self.process_tasks
    }
    fn case_tasks(&self) -> &[CmmnCaseTask] {
        &self.case_tasks
    }
    fn milestones(&self) -> &[CmmnMilestone] {
        &self.milestones
    }
    fn event_listeners(&self) -> &[CmmnEventListener] {
        &self.event_listeners
    }
}

/// Java plan item definition simple class name (e.g. `HumanTask`, `Stage`) plus
/// the definition name, looked up by `PlanItem.definition_ref`.
#[allow(dead_code)]
fn cmmn_definition_meta(container: &dyn PlanItemContainer, definition_ref: &str) -> (&'static str, Option<String>) {
    if let Some(d) = container.human_tasks().iter().find(|d| d.id == definition_ref) {
        return ("HumanTask", Some(d.name.clone()));
    }
    if let Some(d) = container.stages().iter().find(|d| d.id == definition_ref) {
        return ("Stage", Some(d.name.clone()));
    }
    if let Some(d) = container.decision_tasks().iter().find(|d| d.id == definition_ref) {
        return ("DecisionTask", Some(d.name.clone()));
    }
    if let Some(d) = container.process_tasks().iter().find(|d| d.id == definition_ref) {
        return ("ProcessTask", Some(d.name.clone()));
    }
    if let Some(d) = container.case_tasks().iter().find(|d| d.id == definition_ref) {
        return ("CaseTask", Some(d.name.clone()));
    }
    if let Some(d) = container.milestones().iter().find(|d| d.id == definition_ref) {
        return ("Milestone", Some(d.name.clone()));
    }
    if let Some(d) = container.event_listeners().iter().find(|d| d.id == definition_ref) {
        let type_name = if d.event_type == "timer" {
            "TimerEventListener"
        } else {
            "UserEventListener"
        };
        return (type_name, d.name.clone());
    }
    ("PlanItem", None)
}

#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
fn process_cmmn_container(
    container: &dyn PlanItemContainer,
    graphics: &HashMap<String, GraphicInfo>,
    elements: &mut Vec<Value>,
    completed: Option<&HashSet<String>>,
    current: Option<&HashSet<String>>,
    available: Option<&HashSet<String>>,
    diagram_x: &mut f64,
    diagram_y: &mut f64,
    diagram_right: &mut f64,
    diagram_bottom: &mut f64,
    first: &mut bool,
) {
    for plan_item in container.plan_items() {
        let (type_name, definition_name) = cmmn_definition_meta(container, &plan_item.definition_ref);
        let name = plan_item.name.clone().or(definition_name);
        let mut node = json!({
            "id": plan_item.id,
            "name": name,
            "type": type_name,
            "planItemDefinitionId": plan_item.definition_ref,
        });
        // Java highlights on planItemDefinitionId, not the plan item id.
        if let Some(c) = completed {
            node.as_object_mut()
                .unwrap()
                .insert("completed".into(), json!(c.contains(&plan_item.definition_ref)));
        }
        if let Some(c) = current {
            node.as_object_mut()
                .unwrap()
                .insert("current".into(), json!(c.contains(&plan_item.definition_ref)));
        }
        if let Some(c) = available {
            node.as_object_mut()
                .unwrap()
                .insert("available".into(), json!(c.contains(&plan_item.definition_ref)));
        }
        if let Some(gi) = graphics.get(&plan_item.id) {
            fill_graphic(&mut node, gi, true);
            expand_diagram(gi, diagram_x, diagram_y, diagram_right, diagram_bottom, first);
        }
        elements.push(node);

        for criterion_id in plan_item
            .entry_criterion_ids
            .iter()
            .map(|id| (id, "EntryCriterion"))
            .chain(plan_item.exit_criterion_ids.iter().map(|id| (id, "ExitCriterion")))
        {
            let mut criterion_node = json!({
                "id": criterion_id.0,
                "type": criterion_id.1,
            });
            if let Some(gi) = graphics.get(criterion_id.0) {
                fill_graphic(&mut criterion_node, gi, true);
                expand_diagram(gi, diagram_x, diagram_y, diagram_right, diagram_bottom, first);
            }
            elements.push(criterion_node);
        }

        // Recurse into stages referenced by this plan item.
        if type_name == "Stage" {
            if let Some(stage) = container
                .stages()
                .iter()
                .find(|s| s.id == plan_item.definition_ref)
            {
                process_cmmn_container(
                    stage,
                    graphics,
                    elements,
                    completed,
                    current,
                    available,
                    diagram_x,
                    diagram_y,
                    diagram_right,
                    diagram_bottom,
                    first,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowable_bpmn_model::model::{Process, UserTask};

    #[test]
    fn empty_without_di() {
        let model = BpmnModel::default();
        let v = build_process_definition_display(&model);
        assert!(v.as_object().unwrap().is_empty());
    }

    #[test]
    fn builds_elements_from_location_map() {
        let mut model = BpmnModel::default();
        model.location_map.insert(
            "task1".into(),
            GraphicInfo {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 80.0,
                ..Default::default()
            },
        );
        let mut process = Process::default();
        let mut ut = UserTask::default();
        ut.task.activity.flow_node.flow_element.base_element.id = Some("task1".into());
        ut.task.activity.flow_node.flow_element.name = Some("Do it".into());
        process
            .flow_elements
            .push(FlowElementEnum::UserTask(ut));
        model.processes.push(process);
        let v = build_process_definition_display(&model);
        assert_eq!(v["elements"].as_array().unwrap().len(), 1);
        assert_eq!(v["elements"][0]["type"], "UserTask");
        assert_eq!(v["elements"][0]["x"], 10.0);
    }

    fn sample_case() -> CmmnCase {
        let mut plan_model = CmmnCasePlanModel::new("planModel1", "My Case");
        plan_model
            .human_tasks
            .push(CmmnHumanTask::new("humanTaskDef", "Do work"));
        plan_model.plan_items.push(CmmnPlanItem {
            id: "planItem1".into(),
            definition_ref: "humanTaskDef".into(),
            name: None,
            entry_criterion_ids: vec!["entry1".into()],
            exit_criterion_ids: Vec::new(),
            manual_activation_rule: None,
            repetition_rule: None,
            required_rule: None,
            parent_completion_rule: None,
            completion_neutral_rule: None,
        });
        let mut stage = CmmnStage::new("stageDef", "Stage One");
        stage
            .milestones
            .push(CmmnMilestone::new("milestoneDef", "Milestone"));
        stage.plan_items.push(CmmnPlanItem {
            id: "planItem2".into(),
            definition_ref: "milestoneDef".into(),
            name: None,
            entry_criterion_ids: Vec::new(),
            exit_criterion_ids: Vec::new(),
            manual_activation_rule: None,
            repetition_rule: None,
            required_rule: None,
            parent_completion_rule: None,
            completion_neutral_rule: None,
        });
        plan_model.stages.push(stage);
        plan_model.plan_items.push(CmmnPlanItem {
            id: "planItem3".into(),
            definition_ref: "stageDef".into(),
            name: None,
            entry_criterion_ids: Vec::new(),
            exit_criterion_ids: vec!["exit1".into()],
            manual_activation_rule: None,
            repetition_rule: None,
            required_rule: None,
            parent_completion_rule: None,
            completion_neutral_rule: None,
        });
        CmmnCase {
            id: "case1".into(),
            key: "case-key".into(),
            name: "My Case".into(),
            description: None,
            case_plan_model: plan_model,
            lifecycle_listeners: Vec::new(),
            plan_item_lifecycle_listeners: Default::default(),
            start_event_type: None,
            start_correlation_configuration: None,
            start_correlation_parameters: Vec::new(),
        }
    }

    fn sample_graphics() -> HashMap<String, GraphicInfo> {
        let mut graphics = HashMap::new();
        graphics.insert(
            "planModel1".into(),
            GraphicInfo {
                x: 20.0,
                y: 30.0,
                width: 600.0,
                height: 400.0,
                ..Default::default()
            },
        );
        graphics.insert(
            "planItem1".into(),
            GraphicInfo {
                x: 60.0,
                y: 80.0,
                width: 100.0,
                height: 80.0,
                ..Default::default()
            },
        );
        graphics
    }

    #[test]
    fn cmmn_empty_without_graphic_info() {
        let case = sample_case();
        let v = build_case_definition_display(&case, &HashMap::new());
        assert!(v.as_object().unwrap().is_empty());
    }

    #[test]
    fn cmmn_builds_plan_items_and_criteria() {
        let case = sample_case();
        let v = build_case_definition_display(&case, &sample_graphics());
        let elements = v["elements"].as_array().unwrap();
        // PlanModel + human task + entry criterion + stage + nested milestone + exit criterion
        assert_eq!(elements.len(), 6);
        assert_eq!(elements[0]["type"], "PlanModel");
        assert_eq!(elements[0]["x"], 20.0);
        let human = elements.iter().find(|e| e["id"] == "planItem1").unwrap();
        assert_eq!(human["type"], "HumanTask");
        assert_eq!(human["name"], "Do work");
        assert_eq!(human["planItemDefinitionId"], "humanTaskDef");
        assert!(elements.iter().any(|e| e["id"] == "entry1" && e["type"] == "EntryCriterion"));
        assert!(elements.iter().any(|e| e["id"] == "exit1" && e["type"] == "ExitCriterion"));
        // Nested stage contents are flattened into the same elements list.
        let milestone = elements.iter().find(|e| e["id"] == "planItem2").unwrap();
        assert_eq!(milestone["type"], "Milestone");
        // Definition variant carries no highlight flags.
        assert!(human.get("completed").is_none());
        assert_eq!(v["diagramBeginX"], 20.0);
        assert_eq!(v["diagramWidth"], 620.0);
    }

    #[test]
    fn cmmn_instance_highlighting_matches_plan_item_definition_id() {
        let case = sample_case();
        let v = build_case_instance_display(
            &case,
            &sample_graphics(),
            &["milestoneDef".to_string()],
            &["humanTaskDef".to_string()],
            &["stageDef".to_string()],
        );
        let elements = v["elements"].as_array().unwrap();
        let human = elements.iter().find(|e| e["id"] == "planItem1").unwrap();
        assert_eq!(human["current"], true);
        assert_eq!(human["completed"], false);
        let milestone = elements.iter().find(|e| e["id"] == "planItem2").unwrap();
        assert_eq!(milestone["completed"], true);
        let stage = elements.iter().find(|e| e["id"] == "planItem3").unwrap();
        assert_eq!(stage["available"], true);
        assert_eq!(v["completedActivities"], json!(["milestoneDef"]));
        assert_eq!(v["currentActivities"], json!(["humanTaskDef"]));
        assert_eq!(v["availableActivities"], json!(["stageDef"]));
    }
}
