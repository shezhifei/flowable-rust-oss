use flowable_bpmn_model::BpmnModel;
use indexmap::IndexMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutWaypoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagramNodeKind {
    StartEvent,
    EndEvent,
    IntermediateCatchEvent,
    IntermediateThrowEvent,
    Task,
    UserTask,
    ServiceTask,
    ScriptTask,
    ManualTask,
    ReceiveTask,
    BusinessRuleTask,
    ExclusiveGateway,
    ParallelGateway,
    InclusiveGateway,
    EventBasedGateway,
    ComplexGateway,
    CallActivity,
    SubProcess,
    Transaction,
    EventSubProcess,
    AdhocSubProcess,
    BoundaryEvent,
    DataObject,
    Pool,
    Lane,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeLayout {
    pub element_id: String,
    pub kind: DiagramNodeKind,
    pub name: Option<String>,
    pub bounds: LayoutBounds,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeLayout {
    pub element_id: String,
    pub source_ref: String,
    pub target_ref: String,
    pub name: Option<String>,
    pub waypoints: Vec<LayoutWaypoint>,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeKind {
    #[default]
    SequenceFlow,
    Association,
    MessageFlow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessDiagramLayout {
    pub process_id: String,
    pub bounds: LayoutBounds,
    pub nodes: IndexMap<String, NodeLayout>,
    pub edges: IndexMap<String, EdgeLayout>,
    pub message_flows: IndexMap<String, EdgeLayout>,
}

#[derive(Debug, Clone)]
pub struct BpmnLayoutResult {
    pub bpmn_model: BpmnModel,
    pub diagram: ProcessDiagramLayout,
}

impl BpmnLayoutResult {
    pub fn into_model(self) -> BpmnModel {
        self.bpmn_model
    }
}
