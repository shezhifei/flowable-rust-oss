mod auto_layout;
mod error;
mod options;
mod types;

use flowable_bpmn_model::{ArtifactEnum, BpmnModel, FlowElementEnum, GraphicInfo};

pub use auto_layout::BpmnAutoLayout;
pub use error::BpmnLayoutError;
pub use options::{BpmnAutoLayoutOptions, LayoutDirection};
pub use types::{
    BpmnLayoutResult, DiagramNodeKind, EdgeLayout, LayoutBounds, LayoutWaypoint, NodeLayout,
    ProcessDiagramLayout,
};

pub fn ensure_layout(model: &mut BpmnModel) -> Result<(), BpmnLayoutError> {
    let generated = BpmnAutoLayout::new().generate(model)?.into_model();
    for (id, bounds) in generated.location_map {
        model.location_map.entry(id).or_insert(bounds);
    }
    for (id, bounds) in generated.label_location_map {
        model.label_location_map.entry(id).or_insert(bounds);
    }
    for (id, waypoints) in generated.flow_location_map {
        model.flow_location_map.entry(id).or_insert(waypoints);
    }
    for (id, edge) in generated.edge_map {
        model.edge_map.entry(id).or_insert(edge);
    }

    let mut locations = model.location_map.clone();
    let mut top_level_slot = 0;
    for process in &model.processes {
        fill_container_layout(
            &process.flow_elements,
            &process.artifacts,
            &mut locations,
            None,
            &mut top_level_slot,
        );
    }
    fill_artifact_layout(
        &model.global_artifacts,
        &mut locations,
        None,
        &mut top_level_slot,
    );
    model.location_map = locations;
    Ok(())
}

fn fill_container_layout(
    elements: &[FlowElementEnum],
    artifacts: &[ArtifactEnum],
    locations: &mut indexmap::IndexMap<String, GraphicInfo>,
    parent_id: Option<&str>,
    outer_slot: &mut usize,
) {
    let direct_nodes = elements
        .iter()
        .filter(|element| !matches!(element, FlowElementEnum::SequenceFlow(_)))
        .count();
    if let Some(parent_id) = parent_id
        && direct_nodes > 0
        && let Some(parent) = locations.get_mut(parent_id)
    {
        let columns = direct_nodes.min(3);
        let rows = direct_nodes.div_ceil(3);
        parent.width = parent.width.max(48.0 + columns as f64 * 132.0);
        parent.height = parent.height.max(68.0 + rows as f64 * 92.0);
    }

    let parent = parent_id.and_then(|id| locations.get(id).cloned());
    let mut local_slot = 0;
    for element in elements {
        let Some((id, width, height)) = element_layout(element) else {
            continue;
        };
        if !locations.contains_key(id) {
            let slot = if parent.is_some() {
                let slot = local_slot;
                local_slot += 1;
                slot
            } else {
                let slot = *outer_slot;
                *outer_slot += 1;
                slot
            };
            let (origin_x, origin_y, column_width, row_height) = parent
                .as_ref()
                .map(|bounds| (bounds.x + 28.0, bounds.y + 42.0, 132.0, 92.0))
                .unwrap_or((80.0, 80.0, 180.0, 130.0));
            locations.insert(
                id.to_string(),
                GraphicInfo {
                    x: origin_x + (slot % 3) as f64 * column_width,
                    y: origin_y + (slot / 3) as f64 * row_height,
                    width,
                    height,
                    expanded: Some(true),
                    ..GraphicInfo::default()
                },
            );
        }

        match element {
            FlowElementEnum::SubProcess(value) => fill_container_layout(
                &value.flow_elements,
                &value.artifacts,
                locations,
                Some(id),
                outer_slot,
            ),
            FlowElementEnum::Transaction(value) => fill_container_layout(
                &value.sub_process.flow_elements,
                &value.sub_process.artifacts,
                locations,
                Some(id),
                outer_slot,
            ),
            FlowElementEnum::EventSubProcess(value) => fill_container_layout(
                &value.sub_process.flow_elements,
                &value.sub_process.artifacts,
                locations,
                Some(id),
                outer_slot,
            ),
            FlowElementEnum::AdhocSubProcess(value) => fill_container_layout(
                &value.sub_process.flow_elements,
                &value.sub_process.artifacts,
                locations,
                Some(id),
                outer_slot,
            ),
            _ => {}
        }
    }
    fill_artifact_layout(artifacts, locations, parent.as_ref(), outer_slot);
}

fn fill_artifact_layout(
    artifacts: &[ArtifactEnum],
    locations: &mut indexmap::IndexMap<String, GraphicInfo>,
    parent: Option<&GraphicInfo>,
    slot: &mut usize,
) {
    for artifact in artifacts {
        let (id, width, height) = match artifact {
            ArtifactEnum::Association(_) => continue,
            ArtifactEnum::TextAnnotation(value) => {
                let Some(id) = value.base_element.id.as_deref() else {
                    continue;
                };
                (id, 180.0, 64.0)
            }
            ArtifactEnum::Group(value) => {
                let Some(id) = value.base_element.id.as_deref() else {
                    continue;
                };
                (id, 320.0, 180.0)
            }
        };
        if locations.contains_key(id) {
            continue;
        }
        let (origin_x, origin_y) = parent
            .map(|bounds| (bounds.x + 24.0, bounds.y + bounds.height + 24.0))
            .unwrap_or((80.0, 500.0));
        locations.insert(
            id.to_string(),
            GraphicInfo {
                x: origin_x + (*slot % 3) as f64 * 200.0,
                y: origin_y + (*slot / 3) as f64 * 100.0,
                width,
                height,
                expanded: Some(true),
                ..GraphicInfo::default()
            },
        );
        *slot += 1;
    }
}

fn element_layout(element: &FlowElementEnum) -> Option<(&str, f64, f64)> {
    let (id, size) = match element {
        FlowElementEnum::SequenceFlow(_) => return None,
        FlowElementEnum::Task(value) => (
            value
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::UserTask(value) => (
            value
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::ServiceTask(value) => (
            value
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::CaseServiceTask(value) => (
            value
                .service_task
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::SendTask(value) => (
            value
                .service_task
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::ScriptTask(value) => (
            value
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::ManualTask(value) => (
            value
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::ReceiveTask(value) => (
            value
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::BusinessRuleTask(value) => (
            value
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::StartEvent(value) => (
            value
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (42.0, 42.0),
        ),
        FlowElementEnum::EndEvent(value) => (
            value
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (42.0, 42.0),
        ),
        FlowElementEnum::ExclusiveGateway(value) => (
            value
                .gateway
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (64.0, 64.0),
        ),
        FlowElementEnum::ParallelGateway(value) => (
            value
                .gateway
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (64.0, 64.0),
        ),
        FlowElementEnum::InclusiveGateway(value) => (
            value
                .gateway
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (64.0, 64.0),
        ),
        FlowElementEnum::EventBasedGateway(value) => (
            value
                .gateway
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (64.0, 64.0),
        ),
        FlowElementEnum::ComplexGateway(value) => (
            value
                .gateway
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (64.0, 64.0),
        ),
        FlowElementEnum::IntermediateCatchEvent(value) => (
            value
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (42.0, 42.0),
        ),
        FlowElementEnum::IntermediateThrowEvent(value) => (
            value
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (42.0, 42.0),
        ),
        FlowElementEnum::SubProcess(value) => (
            value
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (220.0, 140.0),
        ),
        FlowElementEnum::Transaction(value) => (
            value
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (220.0, 140.0),
        ),
        FlowElementEnum::EventSubProcess(value) => (
            value
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (220.0, 140.0),
        ),
        FlowElementEnum::AdhocSubProcess(value) => (
            value
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (220.0, 140.0),
        ),
        FlowElementEnum::CallActivity(value) => (
            value
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (140.0, 80.0),
        ),
        FlowElementEnum::ValuedDataObject(value) => {
            (value.base_element.id.as_deref(), (48.0, 62.0))
        }
        FlowElementEnum::BoundaryEvent(value) => (
            value
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref(),
            (36.0, 36.0),
        ),
    };
    id.map(|id| (id, size.0, size.1))
}
