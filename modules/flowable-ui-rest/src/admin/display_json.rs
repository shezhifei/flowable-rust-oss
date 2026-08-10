//! Admin display-json assembly from `BpmnModel` DI (Java `DisplayJsonClientResource`).

use flowable_bpmn_model::model::{Activity, BpmnModel, FlowElement, FlowElementEnum, GraphicInfo};
use serde_json::{json, Value};
use std::collections::HashSet;

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
}
