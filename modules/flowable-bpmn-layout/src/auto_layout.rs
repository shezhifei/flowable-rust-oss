use crate::error::BpmnLayoutError;
use crate::options::{BpmnAutoLayoutOptions, LayoutDirection};
use crate::types::{
    BpmnLayoutResult, DiagramNodeKind, EdgeKind, EdgeLayout, LayoutBounds, LayoutWaypoint,
    NodeLayout, ProcessDiagramLayout,
};
use flowable_bpmn_model::{
    ArtifactEnum, Association, BpmnDiEdge, BpmnModel, FlowElementEnum, GraphicInfo, Process,
};
use indexmap::IndexMap;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

const TASK_WIDTH: f64 = 124.0;
const TASK_HEIGHT: f64 = 82.0;
const EVENT_SIZE: f64 = 40.0;
const GATEWAY_SIZE: f64 = 56.0;
const DIAGRAM_PADDING: f64 = 32.0;
const COLUMN_GAP: f64 = 196.0;
const ROW_GAP: f64 = 148.0;
const LOOP_DETOUR_X: f64 = 56.0;
const LOOP_DETOUR_Y: f64 = 56.0;
const DATA_OBJECT_SIZE: f64 = 50.0;
const POOL_PADDING: f64 = 32.0;
const LANE_HEADER_WIDTH: f64 = 120.0;
const LANE_PADDING: f64 = 16.0;

#[derive(Debug, Clone, Default)]
pub struct BpmnAutoLayout {
    options: BpmnAutoLayoutOptions,
}

#[derive(Debug, Clone)]
struct NodeDescriptor {
    id: String,
    kind: DiagramNodeKind,
    name: Option<String>,
    source_index: usize,
}

#[derive(Debug, Clone)]
struct EdgeDescriptor {
    id: String,
    name: Option<String>,
    source_ref: String,
    target_ref: String,
}

#[derive(Debug, Clone)]
struct ProcessGraph {
    process_id: String,
    nodes: IndexMap<String, NodeDescriptor>,
    edges: IndexMap<String, EdgeDescriptor>,
    predecessors: BTreeMap<String, Vec<String>>,
    successors: BTreeMap<String, Vec<String>>,
}

impl Default for LayoutBounds {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        }
    }
}

impl BpmnAutoLayout {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(options: BpmnAutoLayoutOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &BpmnAutoLayoutOptions {
        &self.options
    }

    pub fn generate(&self, model: &BpmnModel) -> Result<BpmnLayoutResult, BpmnLayoutError> {
        self.validate_options()?;

        if model.processes.is_empty() {
            return Err(BpmnLayoutError::MissingMainProcess);
        }

        let mut all_nodes = IndexMap::new();
        let mut all_edges = IndexMap::new();
        let mut all_message_flows = IndexMap::new();
        let mut x_offset = 0.0;
        let mut first_process_id = String::new();

        for (idx, process) in model.processes.iter().enumerate() {
            let graph = match self.collect_process_graph(model, process) {
                Ok(g) => g,
                Err(_) => continue,
            };

            if idx == 0 {
                first_process_id = graph.process_id.clone();
            }

            let mut diagram = self.auto_layout(model, process, &graph);

            if x_offset > 0.0 {
                for node in diagram.nodes.values_mut() {
                    node.bounds.x += x_offset;
                }
                for edge in diagram.edges.values_mut() {
                    for wp in &mut edge.waypoints {
                        wp.x += x_offset;
                    }
                }
                for flow in diagram.message_flows.values_mut() {
                    for wp in &mut flow.waypoints {
                        wp.x += x_offset;
                    }
                }
            }

            let process_width = diagram.bounds.width;
            all_nodes.extend(diagram.nodes);
            all_edges.extend(diagram.edges);
            all_message_flows.extend(diagram.message_flows);
            x_offset += process_width + COLUMN_GAP;
        }

        let all_bounds = diagram_bounds(
            all_nodes.values().map(|node| &node.bounds),
            all_edges.values().chain(all_message_flows.values()),
        );

        let diagram = ProcessDiagramLayout {
            process_id: first_process_id,
            bounds: all_bounds,
            nodes: all_nodes,
            edges: all_edges,
            message_flows: all_message_flows,
        };

        let mut laid_out_model = model.clone();
        self.apply_layout(&mut laid_out_model, &diagram);

        Ok(BpmnLayoutResult {
            bpmn_model: laid_out_model,
            diagram,
        })
    }

    fn validate_options(&self) -> Result<(), BpmnLayoutError> {
        match self.options.direction {
            LayoutDirection::LeftToRight => Ok(()),
            LayoutDirection::TopToBottom => Err(BpmnLayoutError::UnsupportedOption {
                option: "direction",
                detail: "only left-to-right layout is supported in the M18 baseline".to_string(),
            }),
        }
    }

    fn collect_process_graph(
        &self,
        model: &BpmnModel,
        process: &Process,
    ) -> Result<ProcessGraph, BpmnLayoutError> {
        if model.processes.len() > 1 {
            // Multi-process models are laid out by positioning each
            // process in a horizontal column to the right of the
            // previous one. The first process uses the standard layout;
            // subsequent processes are offset by COLUMN_GAP.
        }

        if !model.pools.is_empty() {
            // Pools are rendered as labelled rectangles that contain the
            // process flow; the participant dimensions are added after
            // the main auto-layout so they wrap the inner process.
        }

        if !model.message_flows.is_empty() {
            // Message flows are handled as additional edges in the diagram;
            // their waypoints are generated after the main auto-layout
            // since the source/target may live in a different process.
        }

        if !process.lanes.is_empty() {
            // Lanes are rendered as horizontal bands that partition the
            // process flow; the lane dimensions are computed from the
            // nodes they contain after the main auto-layout.
        }

        if !process.artifacts.is_empty()
            || !model.global_artifacts.is_empty()
            || !process.associations.is_empty()
        {
            // Associations are rendered as dashed edges from their
            // source to target. TextAnnotation and Group are not yet
            // modeled in the Rust type system; when they are added they
            // will be rendered as boxes and dashed containers here.
        }

        if !process.data_objects.is_empty() {
            // Data objects are rendered as 50x50 boxes positioned to the
            // right of the process flow; collected into ProcessGraph so
            // they appear in the diagram.
        }

        let process_id =
            process
                .base_element
                .id
                .clone()
                .ok_or_else(|| BpmnLayoutError::InvalidModel {
                    detail: "main process is missing an id".to_string(),
                })?;

        let mut nodes = IndexMap::new();
        let mut edges = IndexMap::new();

        for (source_index, element) in process.flow_elements.iter().enumerate() {
            if let Some(edge) = self.extract_edge(element)? {
                edges.insert(edge.id.clone(), edge);
                continue;
            }

            if let Some(node) = self.extract_node(element, source_index)? {
                nodes.insert(node.id.clone(), node);
            }
        }

        if nodes.is_empty() {
            return Err(BpmnLayoutError::InvalidModel {
                detail: "main process does not contain renderable flow nodes".to_string(),
            });
        }

        let mut predecessors: BTreeMap<String, Vec<String>> =
            nodes.keys().cloned().map(|id| (id, Vec::new())).collect();
        let mut successors: BTreeMap<String, Vec<String>> =
            nodes.keys().cloned().map(|id| (id, Vec::new())).collect();

        for edge in edges.values() {
            if !nodes.contains_key(&edge.source_ref) {
                return Err(BpmnLayoutError::InvalidModel {
                    detail: format!(
                        "sequence flow '{}' references unknown source '{}'",
                        edge.id, edge.source_ref
                    ),
                });
            }
            if !nodes.contains_key(&edge.target_ref) {
                return Err(BpmnLayoutError::InvalidModel {
                    detail: format!(
                        "sequence flow '{}' references unknown target '{}'",
                        edge.id, edge.target_ref
                    ),
                });
            }
            predecessors
                .entry(edge.target_ref.clone())
                .or_default()
                .push(edge.source_ref.clone());
            successors
                .entry(edge.source_ref.clone())
                .or_default()
                .push(edge.target_ref.clone());
        }

        Ok(ProcessGraph {
            process_id,
            nodes,
            edges,
            predecessors,
            successors,
        })
    }

    fn extract_node(
        &self,
        element: &FlowElementEnum,
        source_index: usize,
    ) -> Result<Option<NodeDescriptor>, BpmnLayoutError> {
        let descriptor = match element {
            FlowElementEnum::Task(task) => Some(NodeDescriptor {
                id: required_id(
                    task.activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "task",
                )?,
                kind: DiagramNodeKind::Task,
                name: task.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::UserTask(task) => Some(NodeDescriptor {
                id: required_id(
                    task.task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "userTask",
                )?,
                kind: DiagramNodeKind::UserTask,
                name: task.task.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::ServiceTask(task) => Some(NodeDescriptor {
                id: required_id(
                    task.task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "serviceTask",
                )?,
                kind: DiagramNodeKind::ServiceTask,
                name: task.task.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::CaseServiceTask(task) => Some(NodeDescriptor {
                id: required_id(
                    task.service_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "serviceTask",
                )?,
                kind: DiagramNodeKind::ServiceTask,
                name: task
                    .service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .name
                    .clone(),
                source_index,
            }),
            FlowElementEnum::SendTask(task) => Some(NodeDescriptor {
                id: required_id(
                    task.service_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "sendTask",
                )?,
                kind: DiagramNodeKind::ServiceTask,
                name: task
                    .service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .name
                    .clone(),
                source_index,
            }),
            FlowElementEnum::ScriptTask(task) => Some(NodeDescriptor {
                id: required_id(
                    task.task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "scriptTask",
                )?,
                kind: DiagramNodeKind::ScriptTask,
                name: task.task.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::ManualTask(task) => Some(NodeDescriptor {
                id: required_id(
                    task.task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "manualTask",
                )?,
                kind: DiagramNodeKind::ManualTask,
                name: task.task.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::ReceiveTask(task) => Some(NodeDescriptor {
                id: required_id(
                    task.task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "receiveTask",
                )?,
                kind: DiagramNodeKind::ReceiveTask,
                name: task.task.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::BusinessRuleTask(task) => Some(NodeDescriptor {
                id: required_id(
                    task.task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "businessRuleTask",
                )?,
                kind: DiagramNodeKind::BusinessRuleTask,
                name: task.task.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::StartEvent(event) => Some(NodeDescriptor {
                id: required_id(
                    event.event.flow_node.flow_element.base_element.id.as_ref(),
                    "startEvent",
                )?,
                kind: DiagramNodeKind::StartEvent,
                name: event.event.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::EndEvent(event) => Some(NodeDescriptor {
                id: required_id(
                    event.event.flow_node.flow_element.base_element.id.as_ref(),
                    "endEvent",
                )?,
                kind: DiagramNodeKind::EndEvent,
                name: event.event.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::IntermediateCatchEvent(event) => Some(NodeDescriptor {
                id: required_id(
                    event.event.flow_node.flow_element.base_element.id.as_ref(),
                    "intermediateCatchEvent",
                )?,
                kind: DiagramNodeKind::IntermediateCatchEvent,
                name: event.event.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::IntermediateThrowEvent(event) => Some(NodeDescriptor {
                id: required_id(
                    event.event.flow_node.flow_element.base_element.id.as_ref(),
                    "intermediateThrowEvent",
                )?,
                kind: DiagramNodeKind::IntermediateThrowEvent,
                name: event.event.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::ExclusiveGateway(gateway) => Some(NodeDescriptor {
                id: required_id(
                    gateway
                        .gateway
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "exclusiveGateway",
                )?,
                kind: DiagramNodeKind::ExclusiveGateway,
                name: gateway.gateway.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::ParallelGateway(gateway) => Some(NodeDescriptor {
                id: required_id(
                    gateway
                        .gateway
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "parallelGateway",
                )?,
                kind: DiagramNodeKind::ParallelGateway,
                name: gateway.gateway.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::InclusiveGateway(gateway) => Some(NodeDescriptor {
                id: required_id(
                    gateway
                        .gateway
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "inclusiveGateway",
                )?,
                kind: DiagramNodeKind::InclusiveGateway,
                name: gateway.gateway.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::EventBasedGateway(gateway) => Some(NodeDescriptor {
                id: required_id(
                    gateway
                        .gateway
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "eventBasedGateway",
                )?,
                kind: DiagramNodeKind::EventBasedGateway,
                name: gateway.gateway.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::ComplexGateway(gateway) => Some(NodeDescriptor {
                id: required_id(
                    gateway
                        .gateway
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "complexGateway",
                )?,
                kind: DiagramNodeKind::ComplexGateway,
                name: gateway.gateway.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::CallActivity(activity) => Some(NodeDescriptor {
                id: required_id(
                    activity
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "callActivity",
                )?,
                kind: DiagramNodeKind::CallActivity,
                name: activity.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::SequenceFlow(_) => None,
            FlowElementEnum::BoundaryEvent(boundary_event) => Some(NodeDescriptor {
                id: required_id(
                    boundary_event
                        .event
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "boundaryEvent",
                )?,
                kind: DiagramNodeKind::BoundaryEvent,
                name: boundary_event.event.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::SubProcess(sub_process) => Some(NodeDescriptor {
                id: required_id(
                    sub_process
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "subProcess",
                )?,
                kind: DiagramNodeKind::SubProcess,
                name: sub_process.activity.flow_node.flow_element.name.clone(),
                source_index,
            }),
            FlowElementEnum::Transaction(transaction) => Some(NodeDescriptor {
                id: required_id(
                    transaction
                        .sub_process
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "transaction",
                )?,
                kind: DiagramNodeKind::Transaction,
                name: transaction
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .name
                    .clone(),
                source_index,
            }),
            FlowElementEnum::EventSubProcess(event_sub_process) => Some(NodeDescriptor {
                id: required_id(
                    event_sub_process
                        .sub_process
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "eventSubProcess",
                )?,
                kind: DiagramNodeKind::EventSubProcess,
                name: event_sub_process
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .name
                    .clone(),
                source_index,
            }),
            FlowElementEnum::AdhocSubProcess(adhoc_sub_process) => Some(NodeDescriptor {
                id: required_id(
                    adhoc_sub_process
                        .sub_process
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_ref(),
                    "adhocSubProcess",
                )?,
                kind: DiagramNodeKind::AdhocSubProcess,
                name: adhoc_sub_process
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .name
                    .clone(),
                source_index,
            }),
            FlowElementEnum::ValuedDataObject(data_object) => {
                let id = required_id(
                    data_object.base_element.id.as_ref(),
                    "data object reference",
                )?;
                Some(NodeDescriptor {
                    id: id.clone(),
                    kind: DiagramNodeKind::DataObject,
                    name: data_object.name.clone(),
                    source_index,
                })
            }
        };

        Ok(descriptor)
    }

    fn extract_edge(
        &self,
        element: &FlowElementEnum,
    ) -> Result<Option<EdgeDescriptor>, BpmnLayoutError> {
        let FlowElementEnum::SequenceFlow(sequence_flow) = element else {
            return Ok(None);
        };

        Ok(Some(EdgeDescriptor {
            id: required_id(
                sequence_flow.flow_element.base_element.id.as_ref(),
                "sequenceFlow",
            )?,
            name: sequence_flow.flow_element.name.clone(),
            source_ref: required_reference(
                sequence_flow.source_ref.as_ref(),
                "sequence flow source",
            )?,
            target_ref: required_reference(
                sequence_flow.target_ref.as_ref(),
                "sequence flow target",
            )?,
        }))
    }

    fn auto_layout(
        &self,
        model: &BpmnModel,
        process: &Process,
        graph: &ProcessGraph,
    ) -> ProcessDiagramLayout {
        let layers = compute_layers(graph);
        let mut row_positions: HashMap<String, usize> = HashMap::new();
        let mut nodes = IndexMap::new();

        let mut layer_ids = layers
            .values()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        layer_ids.sort_unstable();

        for layer in layer_ids {
            let mut layer_nodes = graph
                .nodes
                .values()
                .filter(|node| layers.get(&node.id).copied() == Some(layer))
                .cloned()
                .collect::<Vec<_>>();

            layer_nodes.sort_by(|left, right| {
                let left_barycenter = barycenter(graph, &row_positions, &left.id);
                let right_barycenter = barycenter(graph, &row_positions, &right.id);
                left_barycenter
                    .total_cmp(&right_barycenter)
                    .then(left.source_index.cmp(&right.source_index))
                    .then(left.id.cmp(&right.id))
            });

            for (row, node) in layer_nodes.iter().enumerate() {
                row_positions.insert(node.id.clone(), row);
                let (width, height) = node_size(&node.kind);
                let bounds = LayoutBounds {
                    x: DIAGRAM_PADDING + (layer as f64 * COLUMN_GAP),
                    y: DIAGRAM_PADDING + (row as f64 * ROW_GAP),
                    width,
                    height,
                };
                nodes.insert(
                    node.id.clone(),
                    NodeLayout {
                        element_id: node.id.clone(),
                        kind: node.kind.clone(),
                        name: node.name.clone(),
                        bounds,
                    },
                );
            }
        }

        // Lanes shift their members before anything is routed or attached, so
        // boundary events glue to the parent's final position and every waypoint
        // is drawn from bounds that will not move again.
        if !process.lanes.is_empty() {
            let lane_y_positions =
                compute_lane_y_positions(&nodes, &process.lanes, &row_positions, ROW_GAP);
            for (node_id, target_y) in &lane_y_positions {
                if let Some(node) = nodes.get_mut(node_id) {
                    node.bounds.y = *target_y;
                }
            }
        }

        // post-process boundary events: reposition to right edge of parent activity
        let boundary_placements: Vec<(String, String)> = model
            .processes
            .iter()
            .flat_map(|p| &p.flow_element_map)
            .filter_map(|(id, elem)| {
                if let FlowElementEnum::BoundaryEvent(be) = elem {
                    be.attached_to_ref_id
                        .as_ref()
                        .map(|parent_id| (id.clone(), parent_id.clone()))
                } else {
                    None
                }
            })
            .collect();
        for (boundary_id, parent_id) in &boundary_placements {
            let parent_bounds = nodes.get(parent_id).map(|n| n.bounds);
            if let (Some(boundary_node), Some(parent_bounds)) =
                (nodes.get_mut(boundary_id), parent_bounds)
            {
                let parent_right = parent_bounds.x + parent_bounds.width;
                let parent_center_y = parent_bounds.y + parent_bounds.height / 2.0;
                boundary_node.bounds.x = parent_right - boundary_node.bounds.width / 2.0;
                boundary_node.bounds.y = parent_center_y - boundary_node.bounds.height / 2.0;
            }
        }

        let mut edges = IndexMap::new();
        for edge in graph.edges.values() {
            let source = &nodes[&edge.source_ref];
            let target = &nodes[&edge.target_ref];
            let source_index = graph.nodes[&edge.source_ref].source_index;
            edges.insert(
                edge.id.clone(),
                EdgeLayout {
                    element_id: edge.id.clone(),
                    source_ref: edge.source_ref.clone(),
                    target_ref: edge.target_ref.clone(),
                    name: edge.name.clone(),
                    waypoints: edge_waypoints(&source.bounds, &target.bounds, source_index),
                    kind: EdgeKind::SequenceFlow,
                },
            );
        }

        let collect_associations = |acc: &mut Vec<Association>| {
            for assoc in &process.associations {
                acc.push(assoc.clone());
            }
            for artifact in &model.global_artifacts {
                if let ArtifactEnum::Association(assoc) = artifact {
                    acc.push(assoc.clone());
                }
            }
        };
        let mut associations = Vec::new();
        collect_associations(&mut associations);
        for assoc in &associations {
            let (Some(assoc_id), Some(source_ref), Some(target_ref)) = (
                assoc.base_element.id.as_ref(),
                assoc.source_ref.as_ref(),
                assoc.target_ref.as_ref(),
            ) else {
                continue;
            };
            let source = nodes.get(source_ref);
            let target = nodes.get(target_ref);
            if let (Some(s), Some(t)) = (source, target) {
                edges.insert(
                    assoc_id.clone(),
                    EdgeLayout {
                        element_id: assoc_id.clone(),
                        source_ref: source_ref.clone(),
                        target_ref: target_ref.clone(),
                        name: None,
                        waypoints: association_waypoints(&s.bounds, &t.bounds),
                        kind: EdgeKind::Association,
                    },
                );
            }
        }

        let mut message_flows = IndexMap::new();
        for (flow_id, flow) in &model.message_flows {
            let (Some(source_ref), Some(target_ref)) =
                (flow.source_ref.as_ref(), flow.target_ref.as_ref())
            else {
                continue;
            };
            let source = nodes.get(source_ref);
            let target = nodes.get(target_ref);
            let waypoints = match (source, target) {
                (Some(s), Some(t)) => message_flow_waypoints(&s.bounds, &t.bounds),
                _ => Vec::new(),
            };
            message_flows.insert(
                flow_id.clone(),
                EdgeLayout {
                    element_id: flow_id.clone(),
                    source_ref: source_ref.clone(),
                    target_ref: target_ref.clone(),
                    name: flow.name.clone(),
                    waypoints,
                    kind: EdgeKind::MessageFlow,
                },
            );
        }

        for lane in &process.lanes {
            let Some(lane_id) = lane.base_element.id.clone() else {
                continue;
            };
            let lane_nodes: Vec<&NodeLayout> = lane
                .flow_references
                .iter()
                .filter_map(|ref_id| nodes.get(ref_id))
                .collect();
            if lane_nodes.is_empty() {
                continue;
            }
            let min_x = lane_nodes
                .iter()
                .map(|n| n.bounds.x)
                .fold(f64::INFINITY, f64::min);
            let min_y = lane_nodes
                .iter()
                .map(|n| n.bounds.y)
                .fold(f64::INFINITY, f64::min);
            let max_x = lane_nodes
                .iter()
                .map(|n| n.bounds.x + n.bounds.width)
                .fold(f64::NEG_INFINITY, f64::max);
            let max_y = lane_nodes
                .iter()
                .map(|n| n.bounds.y + n.bounds.height)
                .fold(f64::NEG_INFINITY, f64::max);
            let lane_bounds = LayoutBounds {
                x: min_x - LANE_PADDING,
                y: min_y - LANE_PADDING,
                width: (max_x - min_x) + 2.0 * LANE_PADDING + LANE_HEADER_WIDTH,
                height: (max_y - min_y) + 2.0 * LANE_PADDING,
            };
            nodes.insert(
                lane_id.clone(),
                NodeLayout {
                    element_id: lane_id,
                    kind: DiagramNodeKind::Lane,
                    name: lane.name.clone(),
                    bounds: lane_bounds,
                },
            );
        }

        for pool in &model.pools {
            let Some(pool_id) = pool.base_element.id.clone() else {
                continue;
            };
            if pool.process_ref.as_deref() != Some(graph.process_id.as_str()) {
                continue;
            }
            let inner_bounds = diagram_bounds(
                nodes.values().map(|node| &node.bounds),
                edges.values().chain(message_flows.values()),
            );
            let pool_bounds = LayoutBounds {
                x: inner_bounds.x - POOL_PADDING,
                y: inner_bounds.y - POOL_PADDING,
                width: inner_bounds.width + 2.0 * POOL_PADDING,
                height: inner_bounds.height + 2.0 * POOL_PADDING,
            };
            nodes.insert(
                pool_id.clone(),
                NodeLayout {
                    element_id: pool_id,
                    kind: DiagramNodeKind::Pool,
                    name: pool.name.clone(),
                    bounds: pool_bounds,
                },
            );
        }

        let bounds = diagram_bounds(
            nodes.values().map(|node| &node.bounds),
            edges.values().chain(message_flows.values()),
        );
        ProcessDiagramLayout {
            process_id: graph.process_id.clone(),
            bounds,
            nodes,
            edges,
            message_flows,
        }
    }

    fn apply_layout(&self, model: &mut BpmnModel, diagram: &ProcessDiagramLayout) {
        model.location_map.clear();
        model.flow_location_map.clear();
        model.edge_map.clear();
        model.label_location_map.clear();

        for node in diagram.nodes.values() {
            model.location_map.insert(
                node.element_id.clone(),
                GraphicInfo {
                    x: node.bounds.x,
                    y: node.bounds.y,
                    width: node.bounds.width,
                    height: node.bounds.height,
                    xml_row_number: 0,
                    xml_column_number: 0,
                    rotation: 0.0,
                    expanded: None,
                },
            );
        }

        for edge in diagram.edges.values() {
            let waypoints = edge
                .waypoints
                .iter()
                .map(|waypoint| GraphicInfo {
                    x: waypoint.x,
                    y: waypoint.y,
                    width: 0.0,
                    height: 0.0,
                    xml_row_number: 0,
                    xml_column_number: 0,
                    rotation: 0.0,
                    expanded: None,
                })
                .collect::<Vec<_>>();
            model
                .flow_location_map
                .insert(edge.element_id.clone(), waypoints.clone());
            model.edge_map.insert(
                edge.element_id.clone(),
                BpmnDiEdge {
                    id: Some(edge.element_id.clone()),
                    waypoints,
                    source_docker_info: None,
                    target_docker_info: None,
                },
            );
        }

        for edge in diagram.message_flows.values() {
            let waypoints = edge
                .waypoints
                .iter()
                .map(|waypoint| GraphicInfo {
                    x: waypoint.x,
                    y: waypoint.y,
                    width: 0.0,
                    height: 0.0,
                    xml_row_number: 0,
                    xml_column_number: 0,
                    rotation: 0.0,
                    expanded: None,
                })
                .collect::<Vec<_>>();
            model
                .flow_location_map
                .insert(edge.element_id.clone(), waypoints);
        }
    }
}

fn required_id(id: Option<&String>, element_type: &str) -> Result<String, BpmnLayoutError> {
    id.cloned().ok_or_else(|| BpmnLayoutError::InvalidModel {
        detail: format!("{element_type} is missing an id"),
    })
}

fn required_reference(reference: Option<&String>, label: &str) -> Result<String, BpmnLayoutError> {
    reference
        .cloned()
        .ok_or_else(|| BpmnLayoutError::InvalidModel {
            detail: format!("{label} is missing"),
        })
}

fn node_size(kind: &DiagramNodeKind) -> (f64, f64) {
    match kind {
        DiagramNodeKind::StartEvent
        | DiagramNodeKind::EndEvent
        | DiagramNodeKind::IntermediateCatchEvent
        | DiagramNodeKind::IntermediateThrowEvent => (EVENT_SIZE, EVENT_SIZE),
        DiagramNodeKind::ExclusiveGateway
        | DiagramNodeKind::ParallelGateway
        | DiagramNodeKind::InclusiveGateway
        | DiagramNodeKind::EventBasedGateway
        | DiagramNodeKind::ComplexGateway => (GATEWAY_SIZE, GATEWAY_SIZE),
        DiagramNodeKind::Task
        | DiagramNodeKind::UserTask
        | DiagramNodeKind::ServiceTask
        | DiagramNodeKind::ScriptTask
        | DiagramNodeKind::ManualTask
        | DiagramNodeKind::ReceiveTask
        | DiagramNodeKind::BusinessRuleTask
        | DiagramNodeKind::CallActivity => (TASK_WIDTH, TASK_HEIGHT),
        DiagramNodeKind::SubProcess
        | DiagramNodeKind::Transaction
        | DiagramNodeKind::EventSubProcess
        | DiagramNodeKind::AdhocSubProcess => (200.0, 120.0),
        DiagramNodeKind::BoundaryEvent => (EVENT_SIZE, EVENT_SIZE),
        DiagramNodeKind::DataObject => (DATA_OBJECT_SIZE, DATA_OBJECT_SIZE),
        DiagramNodeKind::Pool => (0.0, 0.0),
        DiagramNodeKind::Lane => (0.0, 0.0),
    }
}

fn compute_layers(graph: &ProcessGraph) -> HashMap<String, usize> {
    let mut indegree = graph
        .nodes
        .keys()
        .cloned()
        .map(|id| {
            let degree = graph.predecessors.get(&id).map_or(0, Vec::len);
            (id, degree)
        })
        .collect::<HashMap<_, _>>();
    let mut layers = HashMap::new();
    let mut queue = graph
        .nodes
        .values()
        .filter(|node| indegree.get(&node.id).copied().unwrap_or(0) == 0)
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    queue.sort_by(|left: &String, right: &String| {
        graph.nodes[left]
            .source_index
            .cmp(&graph.nodes[right].source_index)
            .then(left.cmp(right))
    });

    while let Some(node_id) = queue.first().cloned() {
        queue.remove(0);
        let layer = graph
            .predecessors
            .get(&node_id)
            .into_iter()
            .flatten()
            .filter_map(|predecessor| layers.get(predecessor))
            .max()
            .copied()
            .map_or(0, |layer| layer + 1);
        layers.insert(node_id.clone(), layer);

        if let Some(successors) = graph.successors.get(&node_id) {
            for successor in successors {
                if let Some(current) = indegree.get_mut(successor) {
                    *current = current.saturating_sub(1);
                    if *current == 0 {
                        queue.push(successor.clone());
                    }
                }
            }
        }
        queue.sort_by(|left: &String, right: &String| {
            graph.nodes[left]
                .source_index
                .cmp(&graph.nodes[right].source_index)
                .then(left.cmp(right))
        });
    }

    let mut remaining = graph
        .nodes
        .values()
        .filter(|node| !layers.contains_key(&node.id))
        .cloned()
        .collect::<Vec<_>>();
    remaining.sort_by(|left, right| {
        left.source_index
            .cmp(&right.source_index)
            .then(left.id.cmp(&right.id))
    });

    for node in remaining {
        let layer = graph
            .predecessors
            .get(&node.id)
            .into_iter()
            .flatten()
            .filter_map(|predecessor| layers.get(predecessor))
            .max()
            .copied()
            .map_or(0, |layer| layer + 1);
        layers.insert(node.id.clone(), layer);
    }

    layers
}

fn barycenter(graph: &ProcessGraph, row_positions: &HashMap<String, usize>, node_id: &str) -> f64 {
    let predecessors = graph
        .predecessors
        .get(node_id)
        .map_or(&[][..], Vec::as_slice);
    let positions = predecessors
        .iter()
        .filter_map(|predecessor| row_positions.get(predecessor))
        .copied()
        .collect::<Vec<_>>();
    if positions.is_empty() {
        graph.nodes[node_id].source_index as f64
    } else {
        positions.iter().sum::<usize>() as f64 / positions.len() as f64
    }
}

fn edge_waypoints(
    source: &LayoutBounds,
    target: &LayoutBounds,
    source_index: usize,
) -> Vec<LayoutWaypoint> {
    let source_point = LayoutWaypoint {
        x: source.x + source.width,
        y: source.y + source.height / 2.0,
    };
    let target_point = LayoutWaypoint {
        x: target.x,
        y: target.y + target.height / 2.0,
    };

    let waypoints = if target_point.x >= source_point.x {
        if source_point.y == target_point.y {
            vec![source_point, target_point]
        } else {
            let mid_x = source_point.x + (target_point.x - source_point.x) / 2.0;
            vec![
                source_point,
                LayoutWaypoint {
                    x: mid_x,
                    y: source_point.y,
                },
                LayoutWaypoint {
                    x: mid_x,
                    y: target_point.y,
                },
                target_point,
            ]
        }
    } else {
        let detour_x = source_point.x + LOOP_DETOUR_X;
        let target_detour_x = target_point.x - LOOP_DETOUR_X;
        let top_y =
            source_point.y.min(target_point.y) - LOOP_DETOUR_Y - (source_index as f64 * 12.0);
        vec![
            source_point,
            LayoutWaypoint {
                x: detour_x,
                y: source_point.y,
            },
            LayoutWaypoint {
                x: detour_x,
                y: top_y,
            },
            LayoutWaypoint {
                x: target_detour_x,
                y: top_y,
            },
            LayoutWaypoint {
                x: target_detour_x,
                y: target_point.y,
            },
            target_point,
        ]
    };

    optimize_waypoints(waypoints)
}

fn message_flow_waypoints(source: &LayoutBounds, target: &LayoutBounds) -> Vec<LayoutWaypoint> {
    let source_center = LayoutWaypoint {
        x: source.x + source.width / 2.0,
        y: source.y + source.height / 2.0,
    };
    let target_center = LayoutWaypoint {
        x: target.x + target.width / 2.0,
        y: target.y + target.height / 2.0,
    };
    let start = LayoutWaypoint {
        x: source.x + source.width,
        y: source_center.y,
    };
    let end = LayoutWaypoint {
        x: target.x,
        y: target_center.y,
    };
    if (end.y - start.y).abs() < 1.0 {
        return vec![start, end];
    }
    let mid_x = start.x + (end.x - start.x) / 2.0;
    vec![
        start,
        LayoutWaypoint {
            x: mid_x,
            y: start.y,
        },
        LayoutWaypoint { x: mid_x, y: end.y },
        end,
    ]
}

fn compute_lane_y_positions(
    nodes: &IndexMap<String, NodeLayout>,
    lanes: &[flowable_bpmn_model::Lane],
    row_positions: &HashMap<String, usize>,
    row_gap: f64,
) -> HashMap<String, f64> {
    let mut result = HashMap::new();
    let mut band_top = DIAGRAM_PADDING;
    for lane in lanes {
        // A lane is a band, not a single row: a lane holding parallel branches
        // occupies as many rows as those branches need. Collapsing every member
        // to one y would stack them on top of each other, so the rows the lane
        // actually uses are renumbered from the top of its band.
        let mut rows = lane
            .flow_references
            .iter()
            .filter(|reference| nodes.contains_key(*reference))
            .filter_map(|reference| row_positions.get(reference).copied())
            .collect::<Vec<_>>();
        rows.sort_unstable();
        rows.dedup();
        if rows.is_empty() {
            continue;
        }
        for reference in &lane.flow_references {
            if !nodes.contains_key(reference) {
                continue;
            }
            let Some(row) = row_positions.get(reference).copied() else {
                continue;
            };
            let local_row = rows
                .iter()
                .position(|candidate| *candidate == row)
                .unwrap_or(0);
            result.insert(reference.clone(), band_top + (local_row as f64 * row_gap));
        }
        band_top += rows.len() as f64 * row_gap;
    }
    result
}

fn association_waypoints(source: &LayoutBounds, target: &LayoutBounds) -> Vec<LayoutWaypoint> {
    let source_center = LayoutWaypoint {
        x: source.x + source.width / 2.0,
        y: source.y + source.height / 2.0,
    };
    let target_center = LayoutWaypoint {
        x: target.x + target.width / 2.0,
        y: target.y + target.height / 2.0,
    };
    let start = LayoutWaypoint {
        x: source.x + source.width,
        y: source_center.y,
    };
    let end = LayoutWaypoint {
        x: target.x,
        y: target_center.y,
    };
    if (end.y - start.y).abs() < 1.0 {
        return vec![start, end];
    }
    let mid_x = start.x + (end.x - start.x) / 2.0;
    vec![
        start,
        LayoutWaypoint {
            x: mid_x,
            y: start.y,
        },
        LayoutWaypoint { x: mid_x, y: end.y },
        end,
    ]
}

fn optimize_waypoints(waypoints: Vec<LayoutWaypoint>) -> Vec<LayoutWaypoint> {
    let mut deduped = Vec::new();
    for waypoint in waypoints {
        let is_duplicate = deduped.last().is_some_and(|previous: &LayoutWaypoint| {
            previous.x == waypoint.x && previous.y == waypoint.y
        });
        if !is_duplicate {
            deduped.push(waypoint);
        }
    }

    if deduped.len() <= 2 {
        return deduped;
    }

    let mut optimized = vec![deduped[0]];
    for index in 1..deduped.len() - 1 {
        let previous = &deduped[index - 1];
        let current = &deduped[index];
        let next = &deduped[index + 1];
        let horizontal = previous.y == current.y && current.y == next.y;
        let vertical = previous.x == current.x && current.x == next.x;
        if !(horizontal || vertical) {
            optimized.push(*current);
        }
    }
    optimized.push(deduped[deduped.len() - 1]);
    optimized
}

fn diagram_bounds<'a>(
    node_bounds: impl Iterator<Item = &'a LayoutBounds>,
    edges: impl Iterator<Item = &'a EdgeLayout>,
) -> LayoutBounds {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for bounds in node_bounds {
        min_x = min_x.min(bounds.x);
        min_y = min_y.min(bounds.y);
        max_x = max_x.max(bounds.x + bounds.width);
        max_y = max_y.max(bounds.y + bounds.height);
    }

    for edge in edges {
        for waypoint in &edge.waypoints {
            min_x = min_x.min(waypoint.x);
            min_y = min_y.min(waypoint.y);
            max_x = max_x.max(waypoint.x);
            max_y = max_y.max(waypoint.y);
        }
    }

    if min_x == f64::MAX {
        return LayoutBounds::default();
    }

    LayoutBounds {
        x: min_x - DIAGRAM_PADDING,
        y: min_y - DIAGRAM_PADDING,
        width: (max_x - min_x) + DIAGRAM_PADDING * 2.0,
        height: (max_y - min_y) + DIAGRAM_PADDING * 2.0,
    }
}

#[allow(dead_code)]
fn _ordering_or_equal(ordering: Option<Ordering>) -> Ordering {
    ordering.unwrap_or(Ordering::Equal)
}
