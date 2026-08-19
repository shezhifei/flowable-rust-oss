use flowable_bpmn_model::*;
use quick_xml::{
    Writer,
    events::{BytesCData, BytesDecl, BytesEnd, BytesStart, BytesText, Event as XmlEvent},
};
use std::{
    error::Error,
    fmt::{Display, Formatter},
};

const BPMN_NS: &str = "http://www.omg.org/spec/BPMN/20100524/MODEL";
const FLOWABLE_NS: &str = "http://flowable.org/bpmn";
const BPMNDI_NS: &str = "http://www.omg.org/spec/BPMN/20100524/DI";
const DC_NS: &str = "http://www.omg.org/spec/DD/20100524/DC";
const DI_NS: &str = "http://www.omg.org/spec/DD/20100524/DI";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BpmnXmlWriteError(String);

impl Display for BpmnXmlWriteError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "failed to serialize BPMN XML: {}", self.0)
    }
}

impl Error for BpmnXmlWriteError {}

pub struct BpmnXmlWriter;

impl BpmnXmlWriter {
    pub fn new() -> Self {
        Self
    }

    pub fn write_model(&self, model: &BpmnModel) -> Result<String, BpmnXmlWriteError> {
        let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
        emit(
            &mut writer,
            XmlEvent::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)),
        )?;

        let mut definitions = BytesStart::new("definitions");
        definitions.push_attribute(("xmlns", BPMN_NS));
        definitions.push_attribute(("xmlns:flowable", FLOWABLE_NS));
        definitions.push_attribute(("xmlns:bpmndi", BPMNDI_NS));
        definitions.push_attribute(("xmlns:dc", DC_NS));
        definitions.push_attribute(("xmlns:di", DI_NS));
        for (prefix, namespace) in &model.namespaces {
            if prefix.is_empty()
                || matches!(
                    prefix.as_str(),
                    "flowable" | "bpmndi" | "dc" | "di" | "omgdc" | "omgdi"
                )
            {
                continue;
            }
            let name = format!("xmlns:{prefix}");
            definitions.push_attribute((name.as_str(), namespace.as_str()));
        }
        push_opt(
            &mut definitions,
            "targetNamespace",
            model.target_namespace.as_deref(),
        );
        push_opt(&mut definitions, "exporter", model.exporter.as_deref());
        push_opt(
            &mut definitions,
            "exporterVersion",
            model.exporter_version.as_deref(),
        );
        push_extension_attributes(&mut definitions, &model.definitions_attributes);
        emit(&mut writer, XmlEvent::Start(definitions))?;

        for import in &model.imports {
            write_import(&mut writer, import)?;
        }
        for item in model.item_definitions.values() {
            write_item_definition(&mut writer, item)?;
        }
        for message in &model.messages {
            write_message(&mut writer, message)?;
        }
        for signal in &model.signals {
            write_signal(&mut writer, signal)?;
        }
        for (id, code) in &model.errors {
            write_error(&mut writer, id, code)?;
        }
        for escalation in &model.escalations {
            write_escalation(&mut writer, escalation)?;
        }
        for store in model.data_stores.values() {
            write_data_store(&mut writer, store)?;
        }
        for process in &model.processes {
            write_process(&mut writer, process)?;
        }
        write_collaboration(&mut writer, model)?;
        write_diagram(&mut writer, model)?;

        emit(&mut writer, XmlEvent::End(BytesEnd::new("definitions")))?;
        String::from_utf8(writer.into_inner()).map_err(|error| BpmnXmlWriteError(error.to_string()))
    }
}

impl Default for BpmnXmlWriter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn write_bpmn_model(model: &BpmnModel) -> Result<String, BpmnXmlWriteError> {
    BpmnXmlWriter::new().write_model(model)
}

fn write_import(writer: &mut Writer<Vec<u8>>, value: &Import) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("import");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "importType", value.import_type.as_deref());
    push_opt(&mut node, "location", value.location.as_deref());
    push_opt(&mut node, "namespace", value.namespace.as_deref());
    emit(writer, XmlEvent::Empty(node))
}

fn write_item_definition(
    writer: &mut Writer<Vec<u8>>,
    value: &ItemDefinition,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("itemDefinition");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "structureRef", value.structure_ref.as_deref());
    push_opt(&mut node, "itemKind", value.item_kind.as_deref());
    push_true(&mut node, "isCollection", value.is_collection);
    emit(writer, XmlEvent::Empty(node))
}

fn write_message(writer: &mut Writer<Vec<u8>>, value: &Message) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("message");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "name", value.name.as_deref());
    push_opt(&mut node, "itemRef", value.item_ref.as_deref());
    emit(writer, XmlEvent::Empty(node))
}

fn write_signal(writer: &mut Writer<Vec<u8>>, value: &Signal) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("signal");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "name", value.name.as_deref());
    push_opt(&mut node, "flowable:scope", value.scope.as_deref());
    emit(writer, XmlEvent::Empty(node))
}

fn write_error(
    writer: &mut Writer<Vec<u8>>,
    id: &str,
    code: &str,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("error");
    node.push_attribute(("id", id));
    node.push_attribute(("errorCode", code));
    emit(writer, XmlEvent::Empty(node))
}

fn write_escalation(
    writer: &mut Writer<Vec<u8>>,
    value: &Escalation,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("escalation");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "name", value.name.as_deref());
    push_opt(
        &mut node,
        "escalationCode",
        value.escalation_code.as_deref(),
    );
    emit(writer, XmlEvent::Empty(node))
}

fn write_data_store(
    writer: &mut Writer<Vec<u8>>,
    value: &DataStore,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("dataStore");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "name", value.name.as_deref());
    push_opt(
        &mut node,
        "itemSubjectRef",
        value.item_subject_ref.as_deref(),
    );
    if let Some(state) = &value.data_state {
        emit(writer, XmlEvent::Start(node))?;
        text_element(writer, "dataState", state)?;
        emit(writer, XmlEvent::End(BytesEnd::new("dataStore")))
    } else {
        emit(writer, XmlEvent::Empty(node))
    }
}

fn write_process(writer: &mut Writer<Vec<u8>>, process: &Process) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("process");
    push_base(&mut node, &process.base_element);
    push_opt(&mut node, "name", process.name.as_deref());
    node.push_attribute(("isExecutable", bool_text(process.executable)));
    if !process.candidate_starter_users.is_empty() {
        node.push_attribute((
            "flowable:candidateStarterUsers",
            process.candidate_starter_users.join(",").as_str(),
        ));
    }
    if !process.candidate_starter_groups.is_empty() {
        node.push_attribute((
            "flowable:candidateStarterGroups",
            process.candidate_starter_groups.join(",").as_str(),
        ));
    }
    emit(writer, XmlEvent::Start(node))?;
    if let Some(documentation) = &process.documentation {
        text_element(writer, "documentation", documentation)?;
    }
    write_extensions(
        writer,
        &process.base_element,
        &process.execution_listeners,
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        &[],
    )?;
    write_lanes(writer, &process.lanes)?;
    for data in &process.data_objects {
        write_data_object(writer, data)?;
    }
    for element in &process.flow_elements {
        if !matches!(element, FlowElementEnum::ValuedDataObject(_)) {
            write_flow_element(writer, element)?;
        }
    }
    if process.artifacts.is_empty() {
        for association in &process.associations {
            write_association(writer, association)?;
        }
    } else {
        for artifact in &process.artifacts {
            write_artifact(writer, artifact)?;
        }
    }
    emit(writer, XmlEvent::End(BytesEnd::new("process")))
}

fn write_lanes(writer: &mut Writer<Vec<u8>>, lanes: &[Lane]) -> Result<(), BpmnXmlWriteError> {
    if lanes.is_empty() {
        return Ok(());
    }
    emit(writer, XmlEvent::Start(BytesStart::new("laneSet")))?;
    for lane in lanes {
        let mut node = BytesStart::new("lane");
        push_base(&mut node, &lane.base_element);
        push_opt(&mut node, "name", lane.name.as_deref());
        emit(writer, XmlEvent::Start(node))?;
        // Java's LaneExport writes the extensions ahead of the references.
        write_extension_elements_if_any(writer, &lane.base_element)?;
        for reference in &lane.flow_references {
            text_element(writer, "flowNodeRef", reference)?;
        }
        emit(writer, XmlEvent::End(BytesEnd::new("lane")))?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new("laneSet")))
}

fn write_flow_element(
    writer: &mut Writer<Vec<u8>>,
    element: &FlowElementEnum,
) -> Result<(), BpmnXmlWriteError> {
    match element {
        FlowElementEnum::SequenceFlow(v) => write_sequence_flow(writer, v),
        FlowElementEnum::Task(v) => write_task(writer, "task", &v.activity, &[]),
        FlowElementEnum::UserTask(v) => write_user_task(writer, v),
        FlowElementEnum::ServiceTask(v) => write_service_task(writer, "serviceTask", v, None),
        FlowElementEnum::CaseServiceTask(v) => {
            write_service_task(writer, "serviceTask", &v.service_task, Some("case"))
        }
        FlowElementEnum::SendTask(v) => {
            write_service_task(writer, "sendTask", &v.service_task, None)
        }
        FlowElementEnum::ScriptTask(v) => write_script_task(writer, v),
        FlowElementEnum::ManualTask(v) => write_task(writer, "manualTask", &v.task.activity, &[]),
        FlowElementEnum::ReceiveTask(v) => write_receive_task(writer, v),
        FlowElementEnum::BusinessRuleTask(v) => write_business_rule_task(writer, v),
        FlowElementEnum::StartEvent(v) => {
            write_event(writer, "startEvent", &v.event, &v.form_properties, |n| {
                push_opt(n, "flowable:initiator", v.initiator.as_deref());
                push_opt(n, "flowable:formKey", v.form_key.as_deref());
                if !v.interrupting {
                    n.push_attribute(("isInterrupting", "false"));
                }
            })
        }
        FlowElementEnum::EndEvent(v) => write_event(writer, "endEvent", &v.event, &[], |_| {}),
        FlowElementEnum::IntermediateCatchEvent(v) => {
            write_event(writer, "intermediateCatchEvent", &v.event, &[], |_| {})
        }
        FlowElementEnum::IntermediateThrowEvent(v) => {
            write_event(writer, "intermediateThrowEvent", &v.event, &[], |_| {})
        }
        FlowElementEnum::BoundaryEvent(v) => write_boundary_event(writer, v),
        FlowElementEnum::ExclusiveGateway(v) => {
            write_gateway(writer, "exclusiveGateway", &v.gateway)
        }
        FlowElementEnum::ParallelGateway(v) => write_gateway(writer, "parallelGateway", &v.gateway),
        FlowElementEnum::InclusiveGateway(v) => {
            write_gateway(writer, "inclusiveGateway", &v.gateway)
        }
        FlowElementEnum::EventBasedGateway(v) => {
            write_gateway(writer, "eventBasedGateway", &v.gateway)
        }
        FlowElementEnum::ComplexGateway(v) => write_complex_gateway(writer, v),
        FlowElementEnum::SubProcess(v) => write_subprocess(writer, "subProcess", v, None, |_| {}),
        FlowElementEnum::Transaction(v) => {
            write_subprocess(writer, "transaction", &v.sub_process, None, |_| {})
        }
        FlowElementEnum::EventSubProcess(v) => {
            write_subprocess(writer, "subProcess", &v.sub_process, None, |n| {
                n.push_attribute(("triggeredByEvent", "true"));
            })
        }
        FlowElementEnum::AdhocSubProcess(v) => write_subprocess(
            writer,
            "adHocSubProcess",
            &v.sub_process,
            v.completion_condition.as_deref(),
            |n| {
                push_opt(n, "ordering", v.ordering.as_deref());
                n.push_attribute((
                    "cancelRemainingInstances",
                    bool_text(v.cancel_remaining_instances),
                ));
            },
        ),
        FlowElementEnum::CallActivity(v) => write_call_activity(writer, v),
        FlowElementEnum::ValuedDataObject(v) => write_data_object(writer, v),
    }
}

fn write_sequence_flow(
    writer: &mut Writer<Vec<u8>>,
    value: &SequenceFlow,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = flow_start("sequenceFlow", &value.flow_element);
    push_opt(&mut node, "sourceRef", value.source_ref.as_deref());
    push_opt(&mut node, "targetRef", value.target_ref.as_deref());
    push_opt(
        &mut node,
        "flowable:skipExpression",
        value.skip_expression.as_deref(),
    );
    if value.condition_expression.is_none()
        && !has_extensions(
            &value.flow_element.base_element,
            &value.flow_element.execution_listeners,
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            &[],
        )
    {
        return emit(writer, XmlEvent::Empty(node));
    }
    emit(writer, XmlEvent::Start(node))?;
    write_flow_body(writer, &value.flow_element)?;
    if let Some(condition) = &value.condition_expression {
        let mut condition_node = BytesStart::new("conditionExpression");
        condition_node.push_attribute(("xsi:type", "tFormalExpression"));
        if let Some(language) = &value.condition_language {
            condition_node.push_attribute(("language", language.as_str()));
        }
        emit(writer, XmlEvent::Start(condition_node))?;
        emit(writer, XmlEvent::CData(BytesCData::new(condition)))?;
        emit(writer, XmlEvent::End(BytesEnd::new("conditionExpression")))?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new("sequenceFlow")))
}

fn write_user_task(
    writer: &mut Writer<Vec<u8>>,
    value: &UserTask,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = activity_start("userTask", &value.task.activity);
    push_opt(&mut node, "flowable:assignee", value.assignee.as_deref());
    push_opt(&mut node, "flowable:owner", value.owner.as_deref());
    push_opt(&mut node, "flowable:priority", value.priority.as_deref());
    push_opt(&mut node, "flowable:formKey", value.form_key.as_deref());
    push_opt(&mut node, "flowable:dueDate", value.due_date.as_deref());
    push_opt(
        &mut node,
        "flowable:businessCalendarName",
        value.business_calendar_name.as_deref(),
    );
    push_opt(&mut node, "flowable:category", value.category.as_deref());
    push_opt(
        &mut node,
        "flowable:skipExpression",
        value.skip_expression.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:validateFormFields",
        value.validate_form_fields.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:taskIdVariableName",
        value.task_id_variable_name.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:taskCompleterVariableName",
        value.task_completer_variable_name.as_deref(),
    );
    if !value.candidate_users.is_empty() {
        let joined = value.candidate_users.join(",");
        node.push_attribute(("flowable:candidateUsers", joined.as_str()));
    }
    if !value.candidate_groups.is_empty() {
        let joined = value.candidate_groups.join(",");
        node.push_attribute(("flowable:candidateGroups", joined.as_str()));
    }
    emit(writer, XmlEvent::Start(node))?;
    write_activity_body(
        writer,
        &value.task.activity,
        &value.task_listeners,
        &value.form_properties,
        &[],
        &[],
    )?;
    emit(writer, XmlEvent::End(BytesEnd::new("userTask")))
}

fn write_service_task(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    value: &ServiceTask,
    forced_type: Option<&str>,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = activity_start(name, &value.task.activity);
    if let (Some(kind), Some(implementation)) = (
        value.implementation_type.as_deref(),
        value.implementation.as_deref(),
    ) {
        let attribute = match kind {
            "class" => "flowable:class",
            "expression" => "flowable:expression",
            "delegateExpression" => "flowable:delegateExpression",
            _ => "flowable:class",
        };
        node.push_attribute((attribute, implementation));
    }
    push_opt(
        &mut node,
        "flowable:type",
        forced_type.or(value.task_type.as_deref()),
    );
    push_opt(
        &mut node,
        "flowable:resultVariableName",
        value.result_variable_name.as_deref(),
    );
    push_opt(&mut node, "flowable:topic", value.topic.as_deref());
    if let Some(parallel) = value.parallel_in_same_transaction {
        node.push_attribute(("flowable:parallelInSameTransaction", bool_text(parallel)));
    }
    push_opt(
        &mut node,
        "flowable:skipExpression",
        value.skip_expression.as_deref(),
    );
    push_true(&mut node, "flowable:triggerable", value.triggerable);
    push_true(
        &mut node,
        "flowable:useLocalScopeForResultVariable",
        value.use_local_scope_for_result_variable,
    );
    push_true(
        &mut node,
        "flowable:storeResultVariableAsTransient",
        value.store_result_variable_as_transient,
    );
    push_true(
        &mut node,
        "flowable:doNotIncludeVariables",
        value.do_not_include_variables,
    );
    emit(writer, XmlEvent::Start(node))?;
    write_activity_body(
        writer,
        &value.task.activity,
        &[],
        &[],
        &value.in_parameters,
        &value.out_parameters,
    )?;
    emit(writer, XmlEvent::End(BytesEnd::new(name)))
}

fn write_script_task(
    writer: &mut Writer<Vec<u8>>,
    value: &ScriptTask,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = activity_start("scriptTask", &value.task.activity);
    push_opt(&mut node, "scriptFormat", value.script_format.as_deref());
    push_opt(
        &mut node,
        "flowable:resultVariable",
        value.result_variable.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:skipExpression",
        value.skip_expression.as_deref(),
    );
    push_true(
        &mut node,
        "flowable:autoStoreVariables",
        value.auto_store_variables,
    );
    push_true(
        &mut node,
        "flowable:doNotIncludeVariables",
        value.do_not_include_variables,
    );
    emit(writer, XmlEvent::Start(node))?;
    write_activity_body(
        writer,
        &value.task.activity,
        &[],
        &[],
        &value.in_parameters,
        &value.out_parameters,
    )?;
    if let Some(script) = &value.script {
        text_element(writer, "script", script)?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new("scriptTask")))
}

fn write_receive_task(
    writer: &mut Writer<Vec<u8>>,
    value: &ReceiveTask,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = activity_start("receiveTask", &value.task.activity);
    push_opt(&mut node, "messageRef", value.message_ref.as_deref());
    push_opt(
        &mut node,
        "flowable:skipExpression",
        value.skip_expression.as_deref(),
    );
    emit(writer, XmlEvent::Start(node))?;
    write_activity_body(writer, &value.task.activity, &[], &[], &[], &[])?;
    emit(writer, XmlEvent::End(BytesEnd::new("receiveTask")))
}

fn write_business_rule_task(
    writer: &mut Writer<Vec<u8>>,
    value: &BusinessRuleTask,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = activity_start("businessRuleTask", &value.task.activity);
    push_opt(
        &mut node,
        "flowable:decisionTableReferenceKey",
        value.decision_ref.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:decisionRef",
        value.decision_ref.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:resultVariable",
        value.result_variable_name.as_deref(),
    );
    push_opt(&mut node, "flowable:class", value.class_name.as_deref());
    push_true(&mut node, "flowable:exclude", value.exclude);
    if !value.rule_names.is_empty() {
        let joined = value.rule_names.join(",");
        node.push_attribute(("flowable:rules", joined.as_str()));
    }
    if !value.input_variables.is_empty() {
        let joined = value.input_variables.join(",");
        node.push_attribute(("flowable:ruleVariablesInput", joined.as_str()));
    }
    emit(writer, XmlEvent::Start(node))?;
    write_activity_body(writer, &value.task.activity, &[], &[], &[], &[])?;
    emit(writer, XmlEvent::End(BytesEnd::new("businessRuleTask")))
}

fn write_task(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    activity: &Activity,
    fields: &[FieldExtension],
) -> Result<(), BpmnXmlWriteError> {
    let node = activity_start(name, activity);
    emit(writer, XmlEvent::Start(node))?;
    write_activity_body(writer, activity, &[], &[], &[], &[])?;
    if !fields.is_empty() {
        write_extensions(
            writer,
            &activity.flow_node.flow_element.base_element,
            &[],
            &[],
            fields,
            &[],
            &[],
            &[],
            None,
            &[],
        )?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new(name)))
}

fn write_gateway(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    value: &Gateway,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = flow_node_start(name, &value.flow_node);
    push_opt(&mut node, "default", value.default_flow.as_deref());
    emit(writer, XmlEvent::Start(node))?;
    write_flow_body(writer, &value.flow_node.flow_element)?;
    emit(writer, XmlEvent::End(BytesEnd::new(name)))
}

/// Writes an event and its definitions. `forms` is the inline start form —
/// non-empty only for `startEvent`, the one event type Java lets carry
/// `flowable:formProperty`.
fn write_event<F: FnOnce(&mut BytesStart<'_>)>(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    value: &flowable_bpmn_model::Event,
    forms: &[FormProperty],
    decorate: F,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = flow_node_start(name, &value.flow_node);
    decorate(&mut node);
    emit(writer, XmlEvent::Start(node))?;
    if let Some(documentation) = &value.flow_node.flow_element.documentation {
        text_element(writer, "documentation", documentation)?;
    }
    write_extensions(
        writer,
        &value.flow_node.flow_element.base_element,
        &value.flow_node.flow_element.execution_listeners,
        &[],
        &[],
        forms,
        &[],
        &[],
        None,
        &[],
    )?;
    for definition in &value.event_definitions {
        write_event_definition(writer, definition)?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new(name)))
}

fn write_boundary_event(
    writer: &mut Writer<Vec<u8>>,
    value: &BoundaryEvent,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = flow_node_start("boundaryEvent", &value.event.flow_node);
    push_opt(
        &mut node,
        "attachedToRef",
        value.attached_to_ref_id.as_deref(),
    );
    if !value.cancel_activity {
        node.push_attribute(("cancelActivity", "false"));
    }
    emit(writer, XmlEvent::Start(node))?;
    if let Some(documentation) = &value.event.flow_node.flow_element.documentation {
        text_element(writer, "documentation", documentation)?;
    }
    write_extensions(
        writer,
        &value.event.flow_node.flow_element.base_element,
        &value.event.flow_node.flow_element.execution_listeners,
        &[],
        &[],
        &[],
        &value.in_parameters,
        &value.out_parameters,
        None,
        &[],
    )?;
    for definition in &value.event.event_definitions {
        write_event_definition(writer, definition)?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new("boundaryEvent")))
}

fn write_event_definition(
    writer: &mut Writer<Vec<u8>>,
    value: &EventDefinitionEnum,
) -> Result<(), BpmnXmlWriteError> {
    match value {
        EventDefinitionEnum::TimerEventDefinition(v) => {
            let mut node = BytesStart::new("timerEventDefinition");
            push_base(&mut node, &v.base_element);
            // Java writes the calendar as an attribute and the parser only reads
            // it as one; a child element here would be dropped on reparse.
            push_opt(
                &mut node,
                "flowable:businessCalendarName",
                v.calendar_name.as_deref(),
            );
            emit(writer, XmlEvent::Start(node))?;
            write_extension_elements_if_any(writer, &v.base_element)?;
            if let Some(text) = &v.time_date {
                text_element(writer, "timeDate", text)?;
            }
            if let Some(text) = &v.time_duration {
                text_element(writer, "timeDuration", text)?;
            }
            if let Some(text) = &v.time_cycle {
                let mut cycle = BytesStart::new("timeCycle");
                push_opt(&mut cycle, "flowable:endDate", v.end_date.as_deref());
                emit(writer, XmlEvent::Start(cycle))?;
                emit(writer, XmlEvent::Text(BytesText::new(text)))?;
                emit(writer, XmlEvent::End(BytesEnd::new("timeCycle")))?;
            }
            emit(writer, XmlEvent::End(BytesEnd::new("timerEventDefinition")))
        }
        EventDefinitionEnum::ErrorEventDefinition(v) => empty_event_ref(
            writer,
            "errorEventDefinition",
            &v.base_element,
            "errorRef",
            v.error_ref.as_deref(),
        ),
        EventDefinitionEnum::MessageEventDefinition(v) => empty_event_ref(
            writer,
            "messageEventDefinition",
            &v.base_element,
            "messageRef",
            v.message_ref.as_deref(),
        ),
        EventDefinitionEnum::SignalEventDefinition(v) => {
            let mut node = BytesStart::new("signalEventDefinition");
            push_base(&mut node, &v.base_element);
            push_opt(&mut node, "signalRef", v.signal_ref.as_deref());
            // A signal can be selected by expression instead of by reference.
            push_opt(
                &mut node,
                "flowable:signalExpression",
                v.signal_expression.as_deref(),
            );
            emit(writer, XmlEvent::Empty(node))
        }
        EventDefinitionEnum::EscalationEventDefinition(v) => empty_event_ref(
            writer,
            "escalationEventDefinition",
            &v.base_element,
            "escalationRef",
            v.escalation_ref.as_deref(),
        ),
        EventDefinitionEnum::CancelEventDefinition(v) => {
            empty_event_ref(writer, "cancelEventDefinition", &v.base_element, "", None)
        }
        EventDefinitionEnum::CompensateEventDefinition(v) => empty_event_ref(
            writer,
            "compensateEventDefinition",
            &v.base_element,
            "activityRef",
            v.activity_ref.as_deref(),
        ),
        EventDefinitionEnum::ConditionalEventDefinition(v) => {
            let mut node = BytesStart::new("conditionalEventDefinition");
            push_base(&mut node, &v.base_element);
            emit(writer, XmlEvent::Start(node))?;
            if let Some(c) = &v.condition_expression {
                text_element(writer, "condition", c)?;
            }
            emit(
                writer,
                XmlEvent::End(BytesEnd::new("conditionalEventDefinition")),
            )
        }
        EventDefinitionEnum::TerminateEventDefinition(v) => {
            let mut node = BytesStart::new("terminateEventDefinition");
            push_base(&mut node, &v.base_element);
            push_true(&mut node, "flowable:terminateAll", v.terminate_all);
            push_true(
                &mut node,
                "flowable:terminateMultiInstance",
                v.terminate_multi_instance,
            );
            emit(writer, XmlEvent::Empty(node))
        }
        EventDefinitionEnum::LinkEventDefinition(v) => {
            let mut node = BytesStart::new("linkEventDefinition");
            push_base(&mut node, &v.base_element);
            push_opt(&mut node, "name", v.name.as_deref());
            push_opt(&mut node, "target", v.target.as_deref());
            emit(writer, XmlEvent::Empty(node))
        }
        EventDefinitionEnum::VariableListenerEventDefinition(v) => {
            let mut node = BytesStart::new("flowable:variableListenerEventDefinition");
            push_base(&mut node, &v.base_element);
            push_opt(&mut node, "variableName", v.variable_name.as_deref());
            push_opt(
                &mut node,
                "variableChangeType",
                v.variable_change_type.as_deref(),
            );
            emit(writer, XmlEvent::Empty(node))
        }
    }
}

fn empty_event_ref(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    base: &BaseElement,
    attribute: &'static str,
    value: Option<&str>,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new(name);
    push_base(&mut node, base);
    if !attribute.is_empty() {
        push_opt(&mut node, attribute, value);
    }
    emit(writer, XmlEvent::Empty(node))
}

fn write_subprocess<F: FnOnce(&mut BytesStart<'_>)>(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    value: &SubProcess,
    completion_condition: Option<&str>,
    decorate: F,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = activity_start(name, &value.activity);
    decorate(&mut node);
    emit(writer, XmlEvent::Start(node))?;
    write_activity_body(writer, &value.activity, &[], &[], &[], &[])?;
    if let Some(condition) = completion_condition {
        text_element(writer, "completionCondition", condition)?;
    }
    for data in &value.data_objects {
        write_data_object(writer, data)?;
    }
    for child in &value.flow_elements {
        if !matches!(child, FlowElementEnum::ValuedDataObject(_)) {
            write_flow_element(writer, child)?;
        }
    }
    for artifact in &value.artifacts {
        write_artifact(writer, artifact)?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new(name)))
}

fn write_complex_gateway(
    writer: &mut Writer<Vec<u8>>,
    value: &ComplexGateway,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = flow_node_start("complexGateway", &value.gateway.flow_node);
    push_opt(&mut node, "default", value.gateway.default_flow.as_deref());
    emit(writer, XmlEvent::Start(node))?;
    write_flow_body(writer, &value.gateway.flow_node.flow_element)?;
    if let Some(condition) = &value.activation_condition {
        let mut condition_node = BytesStart::new("activationCondition");
        condition_node.push_attribute(("xsi:type", "tFormalExpression"));
        emit(writer, XmlEvent::Start(condition_node))?;
        emit(writer, XmlEvent::CData(BytesCData::new(condition)))?;
        emit(writer, XmlEvent::End(BytesEnd::new("activationCondition")))?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new("complexGateway")))
}

fn write_call_activity(
    writer: &mut Writer<Vec<u8>>,
    value: &CallActivity,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = activity_start("callActivity", &value.activity);
    push_opt(&mut node, "calledElement", value.called_element.as_deref());
    push_opt(
        &mut node,
        "flowable:calledElementType",
        value.called_element_type.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:calledElementBinding",
        value.called_element_binding.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:businessKey",
        value.business_key.as_deref(),
    );
    push_true(
        &mut node,
        "flowable:inheritVariables",
        value.inherit_variables,
    );
    push_true(
        &mut node,
        "flowable:inheritBusinessKey",
        value.inherit_business_key,
    );
    push_true(&mut node, "flowable:completeAsync", value.complete_async);
    // Tri-state: absent means "inherit engine default", so an explicit `false`
    // has to survive the round-trip too.
    if let Some(fallback) = value.fallback_to_default_tenant {
        node.push_attribute(("flowable:fallbackToDefaultTenant", bool_text(fallback)));
    }
    emit(writer, XmlEvent::Start(node))?;
    write_activity_body(
        writer,
        &value.activity,
        &[],
        &[],
        &value.in_parameters,
        &value.out_parameters,
    )?;
    emit(writer, XmlEvent::End(BytesEnd::new("callActivity")))
}

fn write_data_object(
    writer: &mut Writer<Vec<u8>>,
    value: &ValuedDataObject,
) -> Result<(), BpmnXmlWriteError> {
    let tag = if value.data_object_ref.is_some() {
        "dataObjectReference"
    } else {
        "dataObject"
    };
    let mut node = BytesStart::new(tag);
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "name", value.name.as_deref());
    let item_ref = value
        .item_subject_ref
        .base_element
        .id
        .as_deref()
        .or(value.item_subject_ref.structure_ref.as_deref());
    push_opt(&mut node, "itemSubjectRef", item_ref);
    push_opt(&mut node, "flowable:type", value.data_type.as_deref());
    push_opt(&mut node, "dataObjectRef", value.data_object_ref.as_deref());
    // Java wraps both the value and any custom extension elements in a single
    // `extensionElements` block (ValuedDataObjectXMLConverter).
    let extensions = &value.base_element.extension_elements;
    if value.value.is_none() && extensions.is_empty() {
        return emit(writer, XmlEvent::Empty(node));
    }
    emit(writer, XmlEvent::Start(node))?;
    emit(
        writer,
        XmlEvent::Start(BytesStart::new("extensionElements")),
    )?;
    if let Some(value) = &value.value {
        let text = match value {
            serde_json::Value::String(v) => v.clone(),
            v => v.to_string(),
        };
        text_element(writer, "flowable:value", &text)?;
    }
    for values in extensions.values() {
        for extension in values {
            write_extension(writer, extension, &root_namespace_scope())?;
        }
    }
    emit(writer, XmlEvent::End(BytesEnd::new("extensionElements")))?;
    emit(writer, XmlEvent::End(BytesEnd::new(tag)))
}

fn write_association(
    writer: &mut Writer<Vec<u8>>,
    value: &Association,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("association");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "sourceRef", value.source_ref.as_deref());
    push_opt(&mut node, "targetRef", value.target_ref.as_deref());
    push_opt(
        &mut node,
        "associationDirection",
        value
            .association_direction
            .as_deref()
            .map(association_direction_xml),
    );
    write_artifact_node(writer, node, &value.base_element, None)
}

fn association_direction_xml(value: &str) -> &str {
    if value.eq_ignore_ascii_case("one") {
        "One"
    } else if value.eq_ignore_ascii_case("both") {
        "Both"
    } else if value.eq_ignore_ascii_case("none") {
        "None"
    } else {
        value
    }
}

fn write_artifact(
    writer: &mut Writer<Vec<u8>>,
    artifact: &ArtifactEnum,
) -> Result<(), BpmnXmlWriteError> {
    match artifact {
        ArtifactEnum::Association(value) => write_association(writer, value),
        ArtifactEnum::TextAnnotation(value) => write_text_annotation(writer, value),
        ArtifactEnum::Group(value) => write_group(writer, value),
    }
}

fn write_text_annotation(
    writer: &mut Writer<Vec<u8>>,
    value: &TextAnnotation,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("textAnnotation");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "textFormat", value.text_format.as_deref());
    write_artifact_node(writer, node, &value.base_element, value.text.as_deref())
}

fn write_group(writer: &mut Writer<Vec<u8>>, value: &Group) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("group");
    push_base(&mut node, &value.base_element);
    push_opt(
        &mut node,
        "categoryValueRef",
        value.category_value_ref.as_deref(),
    );
    write_artifact_node(writer, node, &value.base_element, None)
}

fn write_artifact_node(
    writer: &mut Writer<Vec<u8>>,
    node: BytesStart<'_>,
    base_element: &BaseElement,
    text: Option<&str>,
) -> Result<(), BpmnXmlWriteError> {
    let has_extensions = !base_element.extension_elements.is_empty();
    if text.is_none() && !has_extensions {
        return emit(writer, XmlEvent::Empty(node));
    }
    let name = String::from_utf8_lossy(node.local_name().as_ref()).into_owned();
    emit(writer, XmlEvent::Start(node))?;
    if let Some(text) = text {
        text_element(writer, "text", text)?;
    }
    write_extensions(
        writer,
        base_element,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        &[],
    )?;
    emit(writer, XmlEvent::End(BytesEnd::new(name)))
}

fn write_activity_body(
    writer: &mut Writer<Vec<u8>>,
    activity: &Activity,
    task_listeners: &[FlowableListener],
    form_properties: &[FormProperty],
    in_parameters: &[IOParameter],
    out_parameters: &[IOParameter],
) -> Result<(), BpmnXmlWriteError> {
    if let Some(documentation) = &activity.flow_node.flow_element.documentation {
        text_element(writer, "documentation", documentation)?;
    }
    write_extensions(
        writer,
        &activity.flow_node.flow_element.base_element,
        &activity.flow_node.flow_element.execution_listeners,
        task_listeners,
        &activity.field_extensions,
        form_properties,
        in_parameters,
        out_parameters,
        activity.failed_job_retry_time_cycle_value.as_deref(),
        &activity.map_exceptions,
    )?;
    if let Some(loop_characteristics) = &activity.loop_characteristics {
        write_multi_instance(writer, loop_characteristics)?;
    }
    // Java writes the associations after the multi-instance block, see
    // BaseBpmnXMLConverter#convertToXML.
    for association in &activity.data_input_associations {
        write_data_association(writer, "dataInputAssociation", association)?;
    }
    for association in &activity.data_output_associations {
        write_data_association(writer, "dataOutputAssociation", association)?;
    }
    Ok(())
}

fn write_data_association(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    value: &DataAssociation,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new(name);
    push_base(&mut node, &value.base_element);
    emit(writer, XmlEvent::Start(node))?;
    if let Some(text) = &value.source_ref {
        text_element(writer, "sourceRef", text)?;
    }
    if let Some(text) = &value.target_ref {
        text_element(writer, "targetRef", text)?;
    }
    if let Some(text) = &value.transformation {
        text_element(writer, "transformation", text)?;
    }
    for assignment in &value.assignments {
        let mut node = BytesStart::new("assignment");
        push_base(&mut node, &assignment.base_element);
        emit(writer, XmlEvent::Start(node))?;
        if let Some(text) = &assignment.from {
            text_element(writer, "from", text)?;
        }
        if let Some(text) = &assignment.to {
            text_element(writer, "to", text)?;
        }
        emit(writer, XmlEvent::End(BytesEnd::new("assignment")))?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new(name)))
}

fn write_flow_body(
    writer: &mut Writer<Vec<u8>>,
    flow: &FlowElement,
) -> Result<(), BpmnXmlWriteError> {
    if let Some(documentation) = &flow.documentation {
        text_element(writer, "documentation", documentation)?;
    }
    write_extensions(
        writer,
        &flow.base_element,
        &flow.execution_listeners,
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        &[],
    )
}

fn write_multi_instance(
    writer: &mut Writer<Vec<u8>>,
    value: &MultiInstanceLoopCharacteristics,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("multiInstanceLoopCharacteristics");
    push_base(&mut node, &value.base_element);
    node.push_attribute(("isSequential", bool_text(value.sequential)));
    if value.handler.is_none() {
        push_opt(
            &mut node,
            "flowable:collection",
            value.collection_string.as_deref(),
        );
    }
    push_opt(
        &mut node,
        "flowable:elementVariable",
        value.element_variable.as_deref(),
    );
    push_opt(
        &mut node,
        "flowable:elementIndexVariable",
        value.element_index_variable.as_deref(),
    );
    emit(writer, XmlEvent::Start(node))?;
    // `overview_aggregations` is a derived view the parser never fills, so only
    // the authored `aggregations` are written back.
    let aggregations = value
        .aggregations
        .as_ref()
        .map(|value| value.aggregations.as_slice())
        .unwrap_or_default();
    if value.handler.is_some() || !aggregations.is_empty() {
        emit(
            writer,
            XmlEvent::Start(BytesStart::new("extensionElements")),
        )?;
        if let Some(handler) = &value.handler {
            let mut collection = BytesStart::new("flowable:collection");
            if let (Some(kind), Some(implementation)) = (
                handler.implementation_type.as_deref(),
                handler.implementation.as_deref(),
            ) {
                let attribute = if kind == "delegateExpression" {
                    "flowable:delegateExpression"
                } else {
                    "flowable:class"
                };
                collection.push_attribute((attribute, implementation));
            }
            emit(writer, XmlEvent::Start(collection))?;
            if let Some(text) = &value.collection_string {
                text_element(writer, "flowable:string", text)?;
            }
            emit(writer, XmlEvent::End(BytesEnd::new("flowable:collection")))?;
        }
        for aggregation in aggregations {
            write_variable_aggregation(writer, aggregation)?;
        }
        emit(writer, XmlEvent::End(BytesEnd::new("extensionElements")))?;
    }
    if let Some(text) = &value.loop_cardinality {
        text_element(writer, "loopCardinality", text)?;
    }
    if let Some(text) = &value.input_data_item {
        text_element(writer, "loopDataInputRef", text)?;
    }
    if let Some(text) = &value.completion_condition {
        text_element(writer, "completionCondition", text)?;
    }
    emit(
        writer,
        XmlEvent::End(BytesEnd::new("multiInstanceLoopCharacteristics")),
    )
}

fn write_variable_aggregation(
    writer: &mut Writer<Vec<u8>>,
    value: &VariableAggregationDefinition,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("flowable:variableAggregation");
    push_opt(&mut node, "target", value.target.as_deref());
    push_opt(
        &mut node,
        "targetExpression",
        value.target_expression.as_deref(),
    );
    push_true(
        &mut node,
        "storeAsTransientVariable",
        value.store_as_transient_variable,
    );
    push_true(
        &mut node,
        "createOverviewVariable",
        value.create_overview_variable,
    );
    // The aggregator is selected by the same class / delegateExpression pair as
    // elsewhere, but written unqualified — that is what the parser reads.
    if let (Some(kind), Some(implementation)) = (
        value.implementation_type.as_deref(),
        value.implementation.as_deref(),
    ) {
        let attribute = if kind == "delegateExpression" {
            "delegateExpression"
        } else {
            "class"
        };
        node.push_attribute((attribute, implementation));
    }
    if value.definitions.is_empty() {
        return emit(writer, XmlEvent::Empty(node));
    }
    emit(writer, XmlEvent::Start(node))?;
    for definition in &value.definitions {
        let mut variable = BytesStart::new("variable");
        push_opt(&mut variable, "source", definition.source.as_deref());
        push_opt(
            &mut variable,
            "sourceExpression",
            definition.source_expression.as_deref(),
        );
        push_opt(&mut variable, "target", definition.target.as_deref());
        push_opt(
            &mut variable,
            "targetExpression",
            definition.target_expression.as_deref(),
        );
        emit(writer, XmlEvent::Empty(variable))?;
    }
    emit(
        writer,
        XmlEvent::End(BytesEnd::new("flowable:variableAggregation")),
    )
}

fn write_extensions(
    writer: &mut Writer<Vec<u8>>,
    base: &BaseElement,
    execution: &[FlowableListener],
    task: &[FlowableListener],
    fields: &[FieldExtension],
    forms: &[FormProperty],
    inputs: &[IOParameter],
    outputs: &[IOParameter],
    retry: Option<&str>,
    maps: &[MapExceptionEntry],
) -> Result<(), BpmnXmlWriteError> {
    if !has_extensions(
        base, execution, task, fields, forms, inputs, outputs, retry, maps,
    ) {
        return Ok(());
    }
    emit(
        writer,
        XmlEvent::Start(BytesStart::new("extensionElements")),
    )?;
    for values in base.extension_elements.values() {
        for extension in values {
            write_extension(writer, extension, &root_namespace_scope())?;
        }
    }
    for listener in execution {
        write_listener(writer, "flowable:executionListener", listener)?;
    }
    for listener in task {
        write_listener(writer, "flowable:taskListener", listener)?;
    }
    for field in fields {
        write_field(writer, field)?;
    }
    for form in forms {
        write_form_property(writer, form)?;
    }
    for input in inputs {
        write_io_parameter(writer, "flowable:in", input)?;
    }
    for output in outputs {
        write_io_parameter(writer, "flowable:out", output)?;
    }
    if let Some(value) = retry {
        text_element(writer, "flowable:failedJobRetryTimeCycle", value)?;
    }
    for map in maps {
        write_map_exception(writer, map)?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new("extensionElements")))
}

fn write_map_exception(
    writer: &mut Writer<Vec<u8>>,
    value: &MapExceptionEntry,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("flowable:mapException");
    push_opt(&mut node, "errorCode", value.error_code.as_deref());
    push_true(&mut node, "includeChildExceptions", value.and_children);
    push_opt(&mut node, "rootCause", value.root_cause.as_deref());
    emit(writer, XmlEvent::Start(node))?;
    emit(
        writer,
        XmlEvent::CData(BytesCData::new(
            value.class_name.as_deref().unwrap_or_default(),
        )),
    )?;
    emit(
        writer,
        XmlEvent::End(BytesEnd::new("flowable:mapException")),
    )
}

fn write_listener(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    value: &FlowableListener,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new(name);
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "event", value.event.as_deref());
    push_opt(&mut node, "onTransaction", value.on_transaction.as_deref());
    if let (Some(kind), Some(implementation)) = (
        value.implementation_type.as_deref(),
        value.implementation.as_deref(),
    ) {
        let attribute = match kind {
            "expression" => "expression",
            "delegateExpression" => "delegateExpression",
            _ => "class",
        };
        node.push_attribute((attribute, implementation));
    }
    if value.field_extensions.is_empty() {
        emit(writer, XmlEvent::Empty(node))
    } else {
        emit(writer, XmlEvent::Start(node))?;
        for field in &value.field_extensions {
            write_field(writer, field)?;
        }
        emit(writer, XmlEvent::End(BytesEnd::new(name)))
    }
}

fn write_field(
    writer: &mut Writer<Vec<u8>>,
    value: &FieldExtension,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("flowable:field");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "name", value.field_name.as_deref());
    if let Some(text) = &value.string_value {
        emit(writer, XmlEvent::Start(node))?;
        text_element(writer, "flowable:string", text)?;
        emit(writer, XmlEvent::End(BytesEnd::new("flowable:field")))
    } else if let Some(text) = &value.expression {
        emit(writer, XmlEvent::Start(node))?;
        text_element(writer, "flowable:expression", text)?;
        emit(writer, XmlEvent::End(BytesEnd::new("flowable:field")))
    } else {
        emit(writer, XmlEvent::Empty(node))
    }
}

fn write_form_property(
    writer: &mut Writer<Vec<u8>>,
    value: &FormProperty,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("flowable:formProperty");
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "name", value.name.as_deref());
    push_opt(&mut node, "type", value.property_type.as_deref());
    push_opt(&mut node, "expression", value.expression.as_deref());
    push_opt(&mut node, "variable", value.variable.as_deref());
    push_opt(&mut node, "default", value.default_expression.as_deref());
    push_opt(&mut node, "datePattern", value.date_pattern.as_deref());
    if !value.readable {
        node.push_attribute(("readable", "false"));
    }
    if !value.writeable {
        node.push_attribute(("writable", "false"));
    }
    push_true(&mut node, "required", value.required);
    if value.form_values.is_empty() {
        emit(writer, XmlEvent::Empty(node))
    } else {
        emit(writer, XmlEvent::Start(node))?;
        for option in &value.form_values {
            let mut child = BytesStart::new("flowable:value");
            push_base(&mut child, &option.base_element);
            push_opt(&mut child, "name", option.name.as_deref());
            emit(writer, XmlEvent::Empty(child))?;
        }
        emit(
            writer,
            XmlEvent::End(BytesEnd::new("flowable:formProperty")),
        )
    }
}

fn write_io_parameter(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    value: &IOParameter,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new(name);
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "source", value.source.as_deref());
    push_opt(
        &mut node,
        "sourceExpression",
        value.source_expression.as_deref(),
    );
    push_opt(&mut node, "target", value.target.as_deref());
    push_opt(
        &mut node,
        "targetExpression",
        value.target_expression.as_deref(),
    );
    push_true(&mut node, "transient", value.transient);
    emit(writer, XmlEvent::Empty(node))
}

fn write_extension(
    writer: &mut Writer<Vec<u8>>,
    value: &ExtensionElement,
    in_scope: &[(String, String)],
) -> Result<(), BpmnXmlWriteError> {
    let local = value.name.as_deref().unwrap_or("extension");
    let name = match value.namespace_prefix.as_deref() {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}:{local}"),
        _ => local.to_string(),
    };
    let mut node = BytesStart::new(name.as_str());
    push_base(&mut node, &value.base_element);
    // Re-declare every prefix this element resolves through that an ancestor has
    // not already declared. A source document is free to bind a prefix on the
    // extension element itself (`<custom:x xmlns:c2=".." c2:id="..">`); such a
    // binding is not in the model's namespace map, so dropping it here would
    // lose the resolved namespace on the next parse.
    let mut scope = in_scope.to_vec();
    let declarations = missing_namespace_declarations(value, &scope);
    for (prefix, namespace) in &declarations {
        node.push_attribute((format!("xmlns:{prefix}").as_str(), namespace.as_str()));
    }
    scope.extend(declarations);
    if value.element_text.is_none() && value.child_elements.is_empty() {
        return emit(writer, XmlEvent::Empty(node));
    }
    emit(writer, XmlEvent::Start(node))?;
    if let Some(text) = &value.element_text {
        emit(writer, XmlEvent::Text(BytesText::new(text)))?;
    }
    for children in value.child_elements.values() {
        for child in children {
            write_extension(writer, child, &scope)?;
        }
    }
    emit(writer, XmlEvent::End(BytesEnd::new(name.as_str())))
}

/// The `(prefix, namespace)` bindings used by `value` — its own and its
/// attributes' — that `scope` does not already provide.
fn missing_namespace_declarations(
    value: &ExtensionElement,
    scope: &[(String, String)],
) -> Vec<(String, String)> {
    let mut missing: Vec<(String, String)> = Vec::new();
    let mut used = Vec::new();
    if let (Some(prefix), Some(namespace)) = (
        value.namespace_prefix.as_deref(),
        value.namespace.as_deref(),
    ) {
        used.push((prefix, namespace));
    }
    for attributes in value.base_element.attributes.values() {
        for attribute in attributes {
            if let (Some(prefix), Some(namespace)) = (
                attribute.namespace_prefix.as_deref(),
                attribute.namespace.as_deref(),
            ) {
                used.push((prefix, namespace));
            }
        }
    }
    for (prefix, namespace) in used {
        if prefix.is_empty() || namespace.is_empty() {
            continue;
        }
        let known = |list: &[(String, String)]| {
            list.iter()
                .any(|(known, bound)| known == prefix && bound == namespace)
        };
        if known(scope) || known(&missing) {
            continue;
        }
        missing.push((prefix.to_string(), namespace.to_string()));
    }
    missing
}

/// The prefixes `write_model` always binds on `<definitions>`, so extension
/// elements under them never need a redundant local declaration.
fn root_namespace_scope() -> Vec<(String, String)> {
    vec![
        ("flowable".to_string(), FLOWABLE_NS.to_string()),
        ("bpmndi".to_string(), BPMNDI_NS.to_string()),
        ("dc".to_string(), DC_NS.to_string()),
        ("di".to_string(), DI_NS.to_string()),
    ]
}

/// An `extensionElements` wrapper holding only the generic extension elements
/// carried on `base`. Callers check for emptiness first when the element may be
/// written as an empty tag.
fn write_generic_extension_elements(
    writer: &mut Writer<Vec<u8>>,
    base: &BaseElement,
) -> Result<(), BpmnXmlWriteError> {
    emit(
        writer,
        XmlEvent::Start(BytesStart::new("extensionElements")),
    )?;
    for values in base.extension_elements.values() {
        for extension in values {
            write_extension(writer, extension, &root_namespace_scope())?;
        }
    }
    emit(writer, XmlEvent::End(BytesEnd::new("extensionElements")))
}

/// The same wrapper, skipped when there is nothing to put in it — for elements
/// already being written with a start tag.
fn write_extension_elements_if_any(
    writer: &mut Writer<Vec<u8>>,
    base: &BaseElement,
) -> Result<(), BpmnXmlWriteError> {
    if base.extension_elements.is_empty() {
        return Ok(());
    }
    write_generic_extension_elements(writer, base)
}

fn write_collaboration(
    writer: &mut Writer<Vec<u8>>,
    model: &BpmnModel,
) -> Result<(), BpmnXmlWriteError> {
    if model.pools.is_empty() && model.message_flows.is_empty() {
        return Ok(());
    }
    let mut collaboration = BytesStart::new("collaboration");
    collaboration.push_attribute(("id", "Collaboration"));
    emit(writer, XmlEvent::Start(collaboration))?;
    for pool in &model.pools {
        let mut node = BytesStart::new("participant");
        push_base(&mut node, &pool.base_element);
        push_opt(&mut node, "name", pool.name.as_deref());
        push_opt(&mut node, "processRef", pool.process_ref.as_deref());
        if pool.base_element.extension_elements.is_empty() {
            emit(writer, XmlEvent::Empty(node))?;
            continue;
        }
        emit(writer, XmlEvent::Start(node))?;
        write_generic_extension_elements(writer, &pool.base_element)?;
        emit(writer, XmlEvent::End(BytesEnd::new("participant")))?;
    }
    for flow in model.message_flows.values() {
        let mut node = BytesStart::new("messageFlow");
        push_base(&mut node, &flow.base_element);
        push_opt(&mut node, "name", flow.name.as_deref());
        push_opt(&mut node, "sourceRef", flow.source_ref.as_deref());
        push_opt(&mut node, "targetRef", flow.target_ref.as_deref());
        push_opt(&mut node, "messageRef", flow.message_ref.as_deref());
        emit(writer, XmlEvent::Empty(node))?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new("collaboration")))
}

fn write_diagram(writer: &mut Writer<Vec<u8>>, model: &BpmnModel) -> Result<(), BpmnXmlWriteError> {
    if model.location_map.is_empty() && model.flow_location_map.is_empty() {
        return Ok(());
    }
    let plane_ref = model
        .pools
        .first()
        .and_then(|p| p.base_element.id.as_deref())
        .or_else(|| {
            model
                .processes
                .first()
                .and_then(|p| p.base_element.id.as_deref())
        })
        .unwrap_or("process");
    let mut diagram = BytesStart::new("bpmndi:BPMNDiagram");
    diagram.push_attribute(("id", "BPMNDiagram_1"));
    emit(writer, XmlEvent::Start(diagram))?;
    let mut plane = BytesStart::new("bpmndi:BPMNPlane");
    plane.push_attribute(("id", "BPMNPlane_1"));
    plane.push_attribute(("bpmnElement", plane_ref));
    emit(writer, XmlEvent::Start(plane))?;
    for (id, bounds) in &model.location_map {
        let mut shape = BytesStart::new("bpmndi:BPMNShape");
        let shape_id = format!("BPMNShape_{id}");
        shape.push_attribute(("id", shape_id.as_str()));
        shape.push_attribute(("bpmnElement", id.as_str()));
        if let Some(expanded) = bounds.expanded {
            shape.push_attribute(("isExpanded", bool_text(expanded)));
        }
        emit(writer, XmlEvent::Start(shape))?;
        write_bounds(writer, bounds)?;
        if let Some(label) = model.label_location_map.get(id) {
            emit(writer, XmlEvent::Start(BytesStart::new("bpmndi:BPMNLabel")))?;
            write_bounds(writer, label)?;
            emit(writer, XmlEvent::End(BytesEnd::new("bpmndi:BPMNLabel")))?;
        }
        emit(writer, XmlEvent::End(BytesEnd::new("bpmndi:BPMNShape")))?;
    }
    for (id, points) in &model.flow_location_map {
        let mut edge = BytesStart::new("bpmndi:BPMNEdge");
        let edge_id = model
            .edge_map
            .get(id)
            .and_then(|e| e.id.as_deref())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("BPMNEdge_{id}"));
        edge.push_attribute(("id", edge_id.as_str()));
        edge.push_attribute(("bpmnElement", id.as_str()));
        emit(writer, XmlEvent::Start(edge))?;
        for point in points {
            let mut waypoint = BytesStart::new("di:waypoint");
            let x = number(point.x);
            let y = number(point.y);
            waypoint.push_attribute(("x", x.as_str()));
            waypoint.push_attribute(("y", y.as_str()));
            emit(writer, XmlEvent::Empty(waypoint))?;
        }
        emit(writer, XmlEvent::End(BytesEnd::new("bpmndi:BPMNEdge")))?;
    }
    emit(writer, XmlEvent::End(BytesEnd::new("bpmndi:BPMNPlane")))?;
    emit(writer, XmlEvent::End(BytesEnd::new("bpmndi:BPMNDiagram")))
}

fn write_bounds(
    writer: &mut Writer<Vec<u8>>,
    value: &GraphicInfo,
) -> Result<(), BpmnXmlWriteError> {
    let mut node = BytesStart::new("dc:Bounds");
    let x = number(value.x);
    let y = number(value.y);
    let width = number(value.width);
    let height = number(value.height);
    node.push_attribute(("x", x.as_str()));
    node.push_attribute(("y", y.as_str()));
    node.push_attribute(("width", width.as_str()));
    node.push_attribute(("height", height.as_str()));
    emit(writer, XmlEvent::Empty(node))
}

fn flow_start(name: &'static str, value: &FlowElement) -> BytesStart<'static> {
    let mut node = BytesStart::new(name);
    push_base(&mut node, &value.base_element);
    push_opt(&mut node, "name", value.name.as_deref());
    node
}
fn flow_node_start(name: &'static str, value: &FlowNode) -> BytesStart<'static> {
    let mut node = flow_start(name, &value.flow_element);
    push_true(&mut node, "flowable:async", value.asynchronous);
    push_true(&mut node, "flowable:asyncLeave", value.asynchronous_leave);
    if value.not_exclusive {
        node.push_attribute(("flowable:exclusive", "false"));
    }
    if value.asynchronous_leave_not_exclusive {
        node.push_attribute(("flowable:asyncLeaveExclusive", "false"));
    }
    node
}
fn activity_start(name: &'static str, value: &Activity) -> BytesStart<'static> {
    let mut node = flow_node_start(name, &value.flow_node);
    push_opt(&mut node, "default", value.default_flow.as_deref());
    push_true(
        &mut node,
        "isForCompensation",
        value.for_compensation || value.is_for_compensation,
    );
    node
}

fn push_base(node: &mut BytesStart<'_>, base: &BaseElement) {
    push_opt(node, "id", base.id.as_deref());
    push_extension_attributes(node, &base.attributes);
}
fn push_extension_attributes(
    node: &mut BytesStart<'_>,
    attributes: &indexmap::IndexMap<String, Vec<ExtensionAttribute>>,
) {
    for (fallback, values) in attributes {
        for attribute in values {
            let local = attribute.name.as_deref().unwrap_or(fallback);
            let name = match attribute.namespace_prefix.as_deref() {
                Some(prefix) if !prefix.is_empty() => format!("{prefix}:{local}"),
                _ => local.to_string(),
            };
            if let Some(value) = &attribute.value {
                node.push_attribute((name.as_str(), value.as_str()));
            }
        }
    }
}
fn has_extensions(
    base: &BaseElement,
    execution: &[FlowableListener],
    task: &[FlowableListener],
    fields: &[FieldExtension],
    forms: &[FormProperty],
    inputs: &[IOParameter],
    outputs: &[IOParameter],
    retry: Option<&str>,
    maps: &[MapExceptionEntry],
) -> bool {
    !base.extension_elements.is_empty()
        || !execution.is_empty()
        || !task.is_empty()
        || !fields.is_empty()
        || !forms.is_empty()
        || !inputs.is_empty()
        || !outputs.is_empty()
        || retry.is_some()
        || !maps.is_empty()
}
fn push_opt(node: &mut BytesStart<'_>, name: &'static str, value: Option<&str>) {
    if let Some(value) = value {
        node.push_attribute((name, value));
    }
}
fn push_true(node: &mut BytesStart<'_>, name: &'static str, value: bool) {
    if value {
        node.push_attribute((name, "true"));
    }
}
fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
fn number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}
fn text_element(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    text: &str,
) -> Result<(), BpmnXmlWriteError> {
    emit(writer, XmlEvent::Start(BytesStart::new(name)))?;
    emit(writer, XmlEvent::CData(BytesCData::new(text)))?;
    emit(writer, XmlEvent::End(BytesEnd::new(name)))
}
fn emit(writer: &mut Writer<Vec<u8>>, event: XmlEvent<'_>) -> Result<(), BpmnXmlWriteError> {
    writer
        .write_event(event)
        .map_err(|error| BpmnXmlWriteError(error.to_string()))
}
