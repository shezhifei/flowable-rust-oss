use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionAttribute {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionElement {
    #[serde(flatten)]
    pub base_element: BaseElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_text: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub child_elements: IndexMap<String, Vec<ExtensionElement>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BaseElement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub xml_row_number: i32,
    pub xml_column_number: i32,
    pub extension_elements: IndexMap<String, Vec<ExtensionElement>>,
    pub attributes: IndexMap<String, Vec<ExtensionAttribute>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowElement {
    #[serde(flatten)]
    pub base_element: BaseElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    pub execution_listeners: Vec<FlowableListener>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldExtension {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub field_name: Option<String>,
    pub string_value: Option<String>,
    pub expression: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowableListener {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub event: Option<String>,
    pub implementation_type: Option<String>,
    pub implementation: Option<String>,
    pub on_transaction: Option<String>,
    pub custom_properties_resolver_implementation_type: Option<String>,
    pub custom_properties_resolver_implementation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_extensions: Vec<FieldExtension>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowNode {
    #[serde(flatten)]
    pub flow_element: FlowElement,
    pub asynchronous: bool,
    pub asynchronous_leave: bool,
    pub not_exclusive: bool,
    pub asynchronous_leave_not_exclusive: bool,
    pub exclusive: bool,
    pub asynchronous_leave_exclusive: bool,
    pub incoming_flows: Vec<SequenceFlow>,
    pub outgoing_flows: Vec<SequenceFlow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    #[serde(flatten)]
    pub flow_node: FlowNode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_job_retry_time_cycle_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_flow: Option<String>,
    pub is_for_compensation: bool,
    pub for_compensation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_characteristics: Option<MultiInstanceLoopCharacteristics>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_input_associations: Vec<DataAssociation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_output_associations: Vec<DataAssociation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub map_exceptions: Vec<MapExceptionEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundary_events: Vec<BoundaryEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_extensions: Vec<FieldExtension>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MapExceptionEntry {
    pub class_name: Option<String>,
    pub error_code: Option<String>,
    pub and_children: bool,
    pub root_cause: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataAssociation {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub source_ref: Option<String>,
    pub target_ref: Option<String>,
    pub transformation: Option<String>,
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MultiInstanceLoopCharacteristics {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub input_data_item: Option<String>,
    pub collection_string: Option<String>,
    pub handler: Option<CollectionHandler>,
    pub loop_cardinality: Option<String>,
    pub completion_condition: Option<String>,
    pub element_variable: Option<String>,
    pub element_index_variable: Option<String>,
    pub sequential: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregations: Option<VariableAggregationDefinitions>,
    pub no_wait_states_async_leave: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionHandler {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub implementation_type: Option<String>,
    pub implementation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VariableAggregationDefinitions {
    pub aggregations: Vec<VariableAggregationDefinition>,
    pub overview_aggregations: Vec<VariableAggregationDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VariableAggregationDefinition {
    pub target: Option<String>,
    pub target_expression: Option<String>,
    pub implementation_type: Option<String>,
    pub implementation: Option<String>,
    pub create_overview_variable: bool,
    pub store_as_transient_variable: bool,
    pub definitions: Vec<VariableAggregationDefinitionVariable>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VariableAggregationDefinitionVariable {
    pub source: Option<String>,
    pub source_expression: Option<String>,
    pub target: Option<String>,
    pub target_expression: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BpmnModel {
    pub definitions_attributes: IndexMap<String, Vec<ExtensionAttribute>>,
    pub processes: Vec<Process>,
    pub location_map: IndexMap<String, GraphicInfo>,
    pub label_location_map: IndexMap<String, GraphicInfo>,
    pub flow_location_map: IndexMap<String, Vec<GraphicInfo>>,
    pub edge_map: IndexMap<String, BpmnDiEdge>,
    pub signals: Vec<Signal>,
    pub pools: Vec<Pool>,
    pub imports: Vec<Import>,
    pub interfaces: Vec<Interface>,
    pub global_artifacts: Vec<ArtifactEnum>,
    pub resources: Vec<Resource>,
    pub target_namespace: Option<String>,
    pub source_system_id: Option<String>,
    pub user_task_form_types: Option<Vec<String>>,
    pub start_event_form_types: Option<Vec<String>>,
    pub exporter: Option<String>,
    pub exporter_version: Option<String>,
    pub messages: Vec<Message>,
    pub errors: HashMap<String, String>,
    pub namespaces: IndexMap<String, String>,
    pub data_stores: IndexMap<String, DataStore>,
    pub message_flows: IndexMap<String, MessageFlow>,
    pub escalations: Vec<Escalation>,
    pub item_definitions: IndexMap<String, ItemDefinition>,
    pub main_process: Option<Process>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageFlow {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub source_ref: Option<String>,
    pub target_ref: Option<String>,
    pub message_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Escalation {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub escalation_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub structure_ref: Option<String>,
    pub item_kind: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_collection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataStore {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub data_state: Option<String>,
    pub item_subject_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BpmnDiEdge {
    pub id: Option<String>,
    pub waypoints: Vec<GraphicInfo>,
    pub source_docker_info: Option<GraphicInfo>,
    pub target_docker_info: Option<GraphicInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphicInfo {
    pub x: f64,
    pub y: f64,
    pub height: f64,
    pub width: f64,
    pub xml_row_number: i32,
    pub xml_column_number: i32,
    pub rotation: f64,
    pub expanded: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FormProperty {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub expression: Option<String>,
    pub variable: Option<String>,
    #[serde(rename = "type")]
    pub property_type: Option<String>,
    pub default_expression: Option<String>,
    pub date_pattern: Option<String>,
    pub readable: bool,
    pub writeable: bool,
    pub required: bool,
    pub form_values: Vec<FormValue>,
}

impl Default for FormProperty {
    fn default() -> Self {
        Self {
            base_element: BaseElement::default(),
            name: None,
            expression: None,
            variable: None,
            property_type: None,
            default_expression: None,
            date_pattern: None,
            readable: true,
            writeable: true,
            required: false,
            form_values: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FormValue {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Signal {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub item_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(flatten)]
    pub flow_node: FlowNode,
    pub event_definitions: Vec<EventDefinitionEnum>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "eventDefinitionType", rename_all = "camelCase")]
pub enum EventDefinitionEnum {
    TimerEventDefinition(TimerEventDefinition),
    ErrorEventDefinition(ErrorEventDefinition),
    MessageEventDefinition(MessageEventDefinition),
    SignalEventDefinition(SignalEventDefinition),
    CancelEventDefinition(CancelEventDefinition),
    CompensateEventDefinition(CompensateEventDefinition),
    ConditionalEventDefinition(ConditionalEventDefinition),
    LinkEventDefinition(LinkEventDefinition),
    EscalationEventDefinition(EscalationEventDefinition),
    /// Flowable extension: `variableListenerEventDefinition` on event start /
    /// intermediate / boundary events.
    VariableListenerEventDefinition(VariableListenerEventDefinition),
    TerminateEventDefinition(TerminateEventDefinition),
}

/// Java `org.flowable.bpmn.model.VariableListenerEventDefinition`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VariableListenerEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub variable_name: Option<String>,
    /// `all` | `create` | `update` | `delete` | `update-create` (Java constants).
    pub variable_change_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CancelEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
}

/// Java `org.flowable.bpmn.model.TerminateEventDefinition`: only valid on end
/// events; `terminateAll` / `terminateMultiInstance` parsed by
/// `TerminateEventDefinitionParser`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminateEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub terminate_all: bool,
    pub terminate_multi_instance: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CompensateEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub activity_ref: Option<String>,
    pub wait_for_completion: bool,
}

impl Default for CompensateEventDefinition {
    fn default() -> Self {
        Self {
            base_element: BaseElement::default(),
            activity_ref: None,
            wait_for_completion: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConditionalEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub condition_expression: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EscalationEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub escalation_ref: Option<String>,
    pub escalation_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub message_ref: Option<String>,
    pub message_expression: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignalEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub signal_ref: Option<String>,
    pub signal_expression: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TimerEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_cycle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    pub calendar_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEventDefinition {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub error_code: Option<String>,
    pub error_ref: Option<String>,
    pub error_variable_name: Option<String>,
    pub error_variable_local_scope: bool,
    pub error_variable_transient: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Pool {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub process_ref: Option<String>,
    pub executable: bool,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            base_element: BaseElement::default(),
            name: None,
            process_ref: None,
            executable: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Import {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub import_type: Option<String>,
    pub location: Option<String>,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Interface {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub implementation_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Association {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub source_ref: Option<String>,
    pub target_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub association_direction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TextAnnotation {
    #[serde(flatten)]
    pub base_element: BaseElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    #[serde(flatten)]
    pub base_element: BaseElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_value_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "artifactType", rename_all = "camelCase")]
pub enum ArtifactEnum {
    Association(Association),
    TextAnnotation(TextAnnotation),
    Group(Group),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct ValuedDataObject {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    pub documentation: Option<String>,
    pub execution_listeners: Vec<FlowableListener>,
    pub item_subject_ref: ItemDefinition,
    /// Typed value produced at convert time (Java `ValuedDataObject#setValue`
    /// subclasses convert Long/Double/Boolean/Date/…; expressions are **not**
    /// evaluated). Runtime copies this into variables as-is.
    pub value: Option<serde_json::Value>,
    #[serde(rename = "type")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_object_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Lane {
    #[serde(flatten)]
    pub base_element: BaseElement,
    pub name: Option<String>,
    #[serde(skip_serializing)]
    #[schemars(skip)]
    pub parent_process: Option<Process>,
    pub flow_references: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Process {
    #[serde(flatten)]
    pub base_element: BaseElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub executable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_specification: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub execution_listeners: Vec<FlowableListener>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lanes: Vec<Lane>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_objects: Vec<ValuedDataObject>,
    pub candidate_starter_users: Vec<String>,
    pub candidate_starter_groups: Vec<String>,
    pub event_listeners: Vec<String>,
    pub enable_eager_execution_tree_fetching: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flow_elements: Vec<FlowElementEnum>,
    pub flow_element_map: IndexMap<String, FlowElementEnum>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactEnum>,
    pub artifact_map: IndexMap<String, ArtifactEnum>,
    #[serde(skip)]
    pub associations: Vec<Association>,
}

impl Default for Process {
    fn default() -> Self {
        Self {
            base_element: BaseElement::default(),
            name: None,
            executable: true,
            documentation: None,
            io_specification: None,
            execution_listeners: Vec::new(),
            lanes: Vec::new(),
            data_objects: Vec::new(),
            candidate_starter_users: Vec::new(),
            candidate_starter_groups: Vec::new(),
            event_listeners: Vec::new(),
            enable_eager_execution_tree_fetching: false,
            flow_elements: Vec::new(),
            flow_element_map: IndexMap::new(),
            artifacts: Vec::new(),
            artifact_map: IndexMap::new(),
            associations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubProcess {
    #[serde(flatten)]
    pub activity: Activity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flow_elements: Vec<FlowElementEnum>,
    pub flow_element_map: IndexMap<String, FlowElementEnum>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactEnum>,
    pub artifact_map: IndexMap<String, ArtifactEnum>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_objects: Vec<ValuedDataObject>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub triggered_by_event: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    #[serde(flatten)]
    pub sub_process: SubProcess,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventSubProcess {
    #[serde(flatten)]
    pub sub_process: SubProcess,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdhocSubProcess {
    #[serde(flatten)]
    pub sub_process: SubProcess,
    pub completion_condition: Option<String>,
    pub ordering: Option<String>,
    pub cancel_remaining_instances: bool,
}

impl Default for AdhocSubProcess {
    fn default() -> Self {
        Self {
            sub_process: SubProcess::default(),
            completion_condition: None,
            ordering: None,
            cancel_remaining_instances: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "subProcessType", rename_all = "camelCase")]
pub enum SubProcessEnum {
    SubProcess(SubProcess),
    Transaction(Transaction),
    EventSubProcess(EventSubProcess),
    AdhocSubProcess(AdhocSubProcess),
}

impl SubProcessEnum {
    pub fn sub_process_mut(&mut self) -> &mut SubProcess {
        match self {
            SubProcessEnum::SubProcess(s) => s,
            SubProcessEnum::Transaction(t) => &mut t.sub_process,
            SubProcessEnum::EventSubProcess(e) => &mut e.sub_process,
            SubProcessEnum::AdhocSubProcess(a) => &mut a.sub_process,
        }
    }

    pub fn sub_process(&self) -> &SubProcess {
        match self {
            SubProcessEnum::SubProcess(s) => s,
            SubProcessEnum::Transaction(t) => &t.sub_process,
            SubProcessEnum::EventSubProcess(e) => &e.sub_process,
            SubProcessEnum::AdhocSubProcess(a) => &a.sub_process,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "elementType", rename_all = "camelCase")]
pub enum FlowElementEnum {
    SequenceFlow(SequenceFlow),
    Task(Task),
    UserTask(UserTask),
    ServiceTask(ServiceTask),
    /// Java `CaseServiceTask` (CaseServiceTask.java:21-32).
    CaseServiceTask(CaseServiceTask),
    /// Java `SendTask` (SendTask.java:20-24).
    SendTask(SendTask),
    ScriptTask(ScriptTask),
    ManualTask(ManualTask),
    ReceiveTask(ReceiveTask),
    BusinessRuleTask(BusinessRuleTask),
    StartEvent(StartEvent),
    EndEvent(EndEvent),
    ExclusiveGateway(ExclusiveGateway),
    ParallelGateway(ParallelGateway),
    InclusiveGateway(InclusiveGateway),
    EventBasedGateway(EventBasedGateway),
    ComplexGateway(ComplexGateway),
    IntermediateCatchEvent(IntermediateCatchEvent),
    IntermediateThrowEvent(IntermediateThrowEvent),
    SubProcess(SubProcess),
    Transaction(Transaction),
    EventSubProcess(EventSubProcess),
    AdhocSubProcess(AdhocSubProcess),
    CallActivity(CallActivity),
    ValuedDataObject(ValuedDataObject),
    BoundaryEvent(BoundaryEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct BoundaryEvent {
    #[serde(flatten)]
    pub event: Event,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached_to_ref_id: Option<String>,
    pub cancel_activity: bool,
    pub in_parameters: Vec<IOParameter>,
    pub out_parameters: Vec<IOParameter>,
}

impl Default for BoundaryEvent {
    fn default() -> Self {
        Self {
            event: Event::default(),
            attached_to_ref_id: None,
            cancel_activity: true,
            in_parameters: Vec::new(),
            out_parameters: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct CallActivity {
    #[serde(flatten)]
    pub activity: Activity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub called_element: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub called_element_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub called_element_binding: Option<String>,
    pub inherit_variables: bool,
    pub use_local_scope_for_out_parameters: bool,
    pub complete_async: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_to_default_tenant: Option<bool>,
    pub inherit_business_key: bool,
    pub same_deployment: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_key: Option<String>,
    /// Java `CallActivity.processInstanceName` — expression/literal naming the child PI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_instance_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_instance_id_variable_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_parameters: Vec<IOParameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub out_parameters: Vec<IOParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IOParameter {
    #[serde(flatten)]
    pub base_element: BaseElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_expression: Option<String>,
    pub transient: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SequenceFlow {
    #[serde(flatten)]
    pub flow_element: FlowElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_expression: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<GraphicInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    #[serde(flatten)]
    pub activity: Activity,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserTask {
    #[serde(flatten)]
    pub task: Task,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<String>,
    pub same_deployment: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_form_fields: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id_variable_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_completer_variable_name: Option<String>,
    pub extended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_calendar_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_users: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form_properties: Vec<FormProperty>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_listeners: Vec<FlowableListener>,
}

impl Default for UserTask {
    fn default() -> Self {
        Self {
            task: Task::default(),
            assignee: None,
            owner: None,
            priority: None,
            form_key: None,
            category: None,
            extension_id: None,
            same_deployment: true,
            validate_form_fields: None,
            skip_expression: None,
            task_id_variable_name: None,
            task_completer_variable_name: None,
            extended: false,
            due_date: None,
            business_calendar_name: None,
            candidate_users: Vec::new(),
            candidate_groups: Vec::new(),
            form_properties: Vec::new(),
            task_listeners: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HttpHandlerScriptInfo {
    pub language: Option<String>,
    pub script: Option<String>,
    pub result_variable: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HttpHandlerDefinition {
    pub implementation: Option<String>,
    pub implementation_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_extensions: Vec<FieldExtension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_info: Option<HttpHandlerScriptInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceTask {
    #[serde(flatten)]
    pub task: Task,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementation_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_form_fields: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_variable_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_in_same_transaction: Option<bool>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    /// Java `ExternalWorkerServiceTask.topic` / `flowable:topic` on
    /// `flowable:type="external-worker"` service tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Java `ExternalWorkerServiceTask.doNotIncludeVariables` /
    /// `flowable:doNotIncludeVariables` — when true and no in-parameters are
    /// defined, fetch returns an empty variable map (DefaultInternalJobManager.java:102-103).
    #[serde(default, skip_serializing_if = "is_false")]
    pub do_not_include_variables: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_event_type: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub send_synchronously: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_expression: Option<String>,
    pub use_local_scope_for_result_variable: bool,
    pub triggerable: bool,
    pub extended: bool,
    pub store_result_variable_as_transient: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_parameters: Vec<IOParameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub out_parameters: Vec<IOParameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_in_parameters: Vec<IOParameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_out_parameters: Vec<IOParameter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_request_handler: Option<HttpHandlerDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_response_handler: Option<HttpHandlerDefinition>,
}

/// Java `org.flowable.bpmn.model.CaseServiceTask` (CaseServiceTask.java:21-32).
/// XML shape is still `<serviceTask flowable:type="case" …>`
/// (ServiceTaskXMLConverter.java:123-124, ServiceTask.CASE_TASK = "case").
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CaseServiceTask {
    #[serde(flatten)]
    pub service_task: ServiceTask,
    /// Java `CaseServiceTask.caseDefinitionKey` — literal or EL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_definition_key: Option<String>,
    /// Java `CaseServiceTask.caseInstanceName`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_instance_name: Option<String>,
    /// Java `CaseServiceTask.sameDeployment` (default false in Java model).
    #[serde(default, skip_serializing_if = "is_false")]
    pub same_deployment: bool,
    /// Java `CaseServiceTask.businessKey` expression/literal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_key: Option<String>,
    /// Java `CaseServiceTask.inheritBusinessKey`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub inherit_business_key: bool,
    /// Java `CaseServiceTask.fallbackToDefaultTenant`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fallback_to_default_tenant: bool,
    /// Java `CaseServiceTask.caseInstanceIdVariableName`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_instance_id_variable_name: Option<String>,
}

impl CaseServiceTask {
    /// Always-true marker used by untagged serde discrimination helpers and
    /// converter construction. Task type is forced to Java `ServiceTask.CASE_TASK`.
    pub fn ensure_case_type(mut self) -> Self {
        self.service_task.task_type = Some("case".to_string());
        self
    }

    pub fn in_parameters(&self) -> &[IOParameter] {
        &self.service_task.in_parameters
    }

    pub fn out_parameters(&self) -> &[IOParameter] {
        &self.service_task.out_parameters
    }

    pub fn activity_id(&self) -> Option<&str> {
        self.service_task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref()
    }
}

/// Java `org.flowable.bpmn.model.SendTask` (SendTask.java:20-24) — the BPMN
/// `<sendTask>` element (`SendTaskXMLConverter.java:42-55`).
///
/// Wraps the shared [`ServiceTask`] shape so the mail / dmn execution helpers
/// (`ServiceTaskActivityBehavior`) and the deployment validator reuse exactly
/// the same field-extension and IO-parameter logic — Java's sendTask only adds
/// `type`, `implementationType`, `operationRef` on top of `TaskWithFieldExtensions`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendTask {
    #[serde(flatten)]
    pub service_task: ServiceTask,
    /// Java `SendTask.operationRef` (SendTask.java:24) — webservice only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScriptTask {
    #[serde(flatten)]
    pub task: Task,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_variable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_expression: Option<String>,
    pub auto_store_variables: bool,
    pub do_not_include_variables: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_parameters: Vec<IOParameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub out_parameters: Vec<IOParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualTask {
    #[serde(flatten)]
    pub task: Task,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveTask {
    #[serde(flatten)]
    pub task: Task,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_expression: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BusinessRuleTask {
    #[serde(flatten)]
    pub task: Task,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_ref: Option<String>,
    pub result_variable_name: Option<String>,
    pub exclude: bool,
    pub rule_names: Vec<String>,
    pub input_variables: Vec<String>,
    pub class_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartEvent {
    #[serde(flatten)]
    pub event: Event,
    pub initiator: Option<String>,
    pub form_key: Option<String>,
    pub same_deployment: bool,
    #[serde(alias = "isInterrupting")]
    pub interrupting: bool,
    /// Java `StartEvent.formProperties`: a start form declared inline with
    /// `flowable:formProperty` rather than through a `formKey` reference.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form_properties: Vec<FormProperty>,
}

impl Default for StartEvent {
    fn default() -> Self {
        Self {
            event: Event::default(),
            initiator: None,
            form_key: None,
            same_deployment: true,
            interrupting: true,
            form_properties: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndEvent {
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntermediateCatchEvent {
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Gateway {
    #[serde(flatten)]
    pub flow_node: FlowNode,
    pub default_flow: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExclusiveGateway {
    #[serde(flatten)]
    pub gateway: Gateway,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ParallelGateway {
    #[serde(flatten)]
    pub gateway: Gateway,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InclusiveGateway {
    #[serde(flatten)]
    pub gateway: Gateway,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventBasedGateway {
    #[serde(flatten)]
    pub gateway: Gateway,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_gateway_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComplexGateway {
    #[serde(flatten)]
    pub gateway: Gateway,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_condition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntermediateThrowEvent {
    #[serde(flatten)]
    pub event: Event,
}
