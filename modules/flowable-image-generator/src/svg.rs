use crate::error::ProcessDiagramSvgError;
use crate::options::ProcessDiagramRenderOptions;
use flowable_bpmn_layout::{
    BpmnAutoLayout, DiagramNodeKind, EdgeLayout, NodeLayout, ProcessDiagramLayout,
};
use flowable_bpmn_model::BpmnModel;
use std::collections::HashSet;
use std::fmt::Write;

#[derive(Debug, Clone, Default)]
pub struct DefaultProcessDiagramGenerator {
    options: ProcessDiagramRenderOptions,
}

impl DefaultProcessDiagramGenerator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(options: ProcessDiagramRenderOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &ProcessDiagramRenderOptions {
        &self.options
    }

    pub fn generate_svg(&self, model: &BpmnModel) -> Result<String, ProcessDiagramSvgError> {
        self.validate_options()?;
        let layout = BpmnAutoLayout::with_options(self.options.layout_options.clone())
            .generate(model)?
            .diagram;
        let highlight_activities: HashSet<&str> = self
            .options
            .highlight_activity_ids
            .iter()
            .map(String::as_str)
            .collect();
        let highlight_flows: HashSet<&str> = self
            .options
            .highlight_flow_ids
            .iter()
            .map(String::as_str)
            .collect();
        Ok(render_svg(
            &layout,
            self.options.include_metadata_attributes,
            self.options.draw_sequence_flow_names,
            &highlight_activities,
            &highlight_flows,
        ))
    }

    fn validate_options(&self) -> Result<(), ProcessDiagramSvgError> {
        if self.options.scale_factor != 1 {
            return Err(ProcessDiagramSvgError::UnsupportedOption {
                option: "scale_factor",
                detail: "only a scale factor of 1 is supported in the M18 SVG baseline".to_string(),
            });
        }
        if self.options.draw_sequence_flow_names {
            return Err(ProcessDiagramSvgError::UnsupportedOption {
                option: "draw_sequence_flow_names",
                detail:
                    "sequence-flow labels require advanced label DI and are not part of the baseline"
                        .to_string(),
            });
        }
        Ok(())
    }
}

pub fn generate_process_diagram_svg(model: &BpmnModel) -> Result<String, ProcessDiagramSvgError> {
    DefaultProcessDiagramGenerator::new().generate_svg(model)
}

fn render_svg(
    layout: &ProcessDiagramLayout,
    include_metadata_attributes: bool,
    draw_sequence_flow_names: bool,
    highlight_activities: &HashSet<&str>,
    highlight_flows: &HashSet<&str>,
) -> String {
    let mut svg = String::new();
    let title = format!("Process diagram for {}", layout.process_id);
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" role=\"img\" aria-label=\"{}\">",
        fmt(layout.bounds.x),
        fmt(layout.bounds.y),
        fmt(layout.bounds.width),
        fmt(layout.bounds.height),
        escape_text(&title),
    );
    svg.push_str(
        "<defs><marker id=\"sequence-arrow\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\"><path d=\"M 0 0 L 10 5 L 0 10 z\" fill=\"#4a5568\"/></marker><marker id=\"sequence-arrow-highlighted\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\"><path d=\"M 0 0 L 10 5 L 0 10 z\" fill=\"#dc2626\"/></marker></defs>",
    );
    svg.push_str("<style>.process-diagram{font-family:\"Segoe UI\",sans-serif}.sequence-flow{fill:none;stroke:#4a5568;stroke-width:2}.sequence-flow.highlighted{stroke:#dc2626;stroke-width:3}.activity{fill:#ffffff;stroke:#1f2937;stroke-width:2}.activity.highlighted,.event.highlighted,.gateway.highlighted,.subprocess.highlighted{stroke:#dc2626;stroke-width:3}.highlighted{stroke:#dc2626}.activity-label{fill:#111827;font-size:12px}.event{fill:#ffffff;stroke:#1f2937;stroke-width:2}.gateway{fill:#ffffff;stroke:#1f2937;stroke-width:2}.event-label,.gateway-label{fill:#111827;font-size:12px;text-anchor:middle}.gateway-symbol{fill:#1f2937;font-size:22px;font-weight:700;text-anchor:middle;dominant-baseline:middle}.subprocess{fill:#ffffff;stroke:#1f2937;stroke-width:3;stroke-dasharray:6,3}.subprocess-inner{fill:none;stroke:#1f2937;stroke-width:1;stroke-dasharray:4,2}</style>");
    let _ = write!(
        svg,
        "<g class=\"process-diagram\"{}>",
        metadata_attribute(
            "data-process-id",
            &layout.process_id,
            include_metadata_attributes
        )
    );

    for edge in layout.edges.values() {
        let highlighted = highlight_flows.contains(edge.element_id.as_str());
        render_edge(
            &mut svg,
            edge,
            include_metadata_attributes,
            draw_sequence_flow_names,
            highlighted,
        );
    }
    for node in layout.nodes.values() {
        let highlighted = highlight_activities.contains(node.element_id.as_str());
        render_node(&mut svg, node, include_metadata_attributes, highlighted);
    }

    svg.push_str("</g></svg>");
    svg
}

fn highlight_class(highlighted: bool) -> &'static str {
    if highlighted {
        " highlighted"
    } else {
        ""
    }
}

fn render_edge(
    svg: &mut String,
    edge: &EdgeLayout,
    include_metadata_attributes: bool,
    _draw_sequence_flow_names: bool,
    highlighted: bool,
) {
    let points = edge
        .waypoints
        .iter()
        .map(|waypoint| format!("{},{}", fmt(waypoint.x), fmt(waypoint.y)))
        .collect::<Vec<_>>()
        .join(" ");
    let _ = write!(
        svg,
        "<polyline class=\"sequence-flow{}\" points=\"{}\" marker-end=\"{}\"{}{}{} />",
        highlight_class(highlighted),
        points,
        if highlighted { "url(#sequence-arrow-highlighted)" } else { "url(#sequence-arrow)" },
        metadata_attribute(
            "data-element-id",
            &edge.element_id,
            include_metadata_attributes
        ),
        metadata_attribute(
            "data-source-ref",
            &edge.source_ref,
            include_metadata_attributes
        ),
        metadata_attribute(
            "data-target-ref",
            &edge.target_ref,
            include_metadata_attributes
        ),
    );
}

fn render_node(svg: &mut String, node: &NodeLayout, include_metadata_attributes: bool,
    highlighted: bool,
) {
    match node.kind {
        DiagramNodeKind::StartEvent
        | DiagramNodeKind::EndEvent
        | DiagramNodeKind::IntermediateCatchEvent
        | DiagramNodeKind::IntermediateThrowEvent => {
            render_event(svg, node, include_metadata_attributes, highlighted)
        }
        DiagramNodeKind::ExclusiveGateway
        | DiagramNodeKind::ParallelGateway
        | DiagramNodeKind::InclusiveGateway
        | DiagramNodeKind::EventBasedGateway
        | DiagramNodeKind::ComplexGateway => {
            render_gateway(svg, node, include_metadata_attributes, highlighted)
        }
        DiagramNodeKind::Task
        | DiagramNodeKind::UserTask
        | DiagramNodeKind::ServiceTask
        | DiagramNodeKind::ScriptTask
        | DiagramNodeKind::ManualTask
        | DiagramNodeKind::ReceiveTask
        | DiagramNodeKind::BusinessRuleTask
        | DiagramNodeKind::CallActivity => render_activity(svg, node, include_metadata_attributes, highlighted),
        DiagramNodeKind::SubProcess
        | DiagramNodeKind::Transaction
        | DiagramNodeKind::EventSubProcess
        | DiagramNodeKind::AdhocSubProcess => {
            render_subprocess(svg, node, include_metadata_attributes, highlighted)
        }
        DiagramNodeKind::BoundaryEvent => render_event(svg, node, include_metadata_attributes, highlighted),
        DiagramNodeKind::DataObject => render_data_object(svg, node, include_metadata_attributes, highlighted),
        DiagramNodeKind::Pool => render_pool(svg, node, include_metadata_attributes, highlighted),
        DiagramNodeKind::Lane => render_lane(svg, node, include_metadata_attributes, highlighted),
    }
}

fn render_data_object(svg: &mut String, node: &NodeLayout, include_metadata_attributes: bool,
    highlighted: bool,
) {
    let classes = format!("data-object{}", highlight_class(highlighted));
    let label = node.name.clone().unwrap_or_default();
    let _ = write!(
        svg,
        "<g class=\"{}\"{}><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" class=\"data-object\" /><text x=\"{}\" y=\"{}\" class=\"data-object-label\">{}</text></g>",
        classes,
        metadata_attribute(
            "data-element-id",
            &node.element_id,
            include_metadata_attributes
        ),
        node.bounds.x,
        node.bounds.y,
        node.bounds.width,
        node.bounds.height,
        node.bounds.x + node.bounds.width / 2.0,
        node.bounds.y + node.bounds.height + 14.0,
        escape_xml_text(&label),
    );
}

fn escape_xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn render_pool(svg: &mut String, node: &NodeLayout, include_metadata_attributes: bool,
    highlighted: bool,
) {
    let label = node.name.clone().unwrap_or_default();
    let _ = write!(
        svg,
        "<g class=\"pool{}\"{}><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" class=\"pool\" /><text x=\"{}\" y=\"{}\" class=\"pool-label\">{}</text></g>",
        highlight_class(highlighted),
        metadata_attribute(
            "data-element-id",
            &node.element_id,
            include_metadata_attributes
        ),
        node.bounds.x,
        node.bounds.y,
        node.bounds.width,
        node.bounds.height,
        node.bounds.x + 12.0,
        node.bounds.y + 24.0,
        escape_xml_text(&label),
    );
}

fn render_lane(svg: &mut String, node: &NodeLayout, include_metadata_attributes: bool,
    highlighted: bool,
) {
    let label = node.name.clone().unwrap_or_default();
    let _ = write!(
        svg,
        "<g class=\"lane{}\"{}><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" class=\"lane\" /><text x=\"{}\" y=\"{}\" class=\"lane-label\" writing-mode=\"tb\">{}</text></g>",
        highlight_class(highlighted),
        metadata_attribute(
            "data-element-id",
            &node.element_id,
            include_metadata_attributes
        ),
        node.bounds.x,
        node.bounds.y,
        node.bounds.width,
        node.bounds.height,
        node.bounds.x + 14.0,
        node.bounds.y + node.bounds.height / 2.0,
        escape_xml_text(&label),
    );
}

fn render_activity(svg: &mut String, node: &NodeLayout, include_metadata_attributes: bool,
    highlighted: bool,
) {
    let classes = format!("activity {}{}", activity_class_name(&node.kind), highlight_class(highlighted));
    let _ = write!(
        svg,
        "<g class=\"{}\"{}><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"12\" ry=\"12\" class=\"activity{}\" /></g>",
        classes,
        metadata_attribute(
            "data-element-id",
            &node.element_id,
            include_metadata_attributes
        ),
        fmt(node.bounds.x),
        fmt(node.bounds.y),
        fmt(node.bounds.width),
        fmt(node.bounds.height),
        highlight_class(highlighted),
    );
    if let Some(name) = &node.name {
        let _ = write!(
            svg,
            "<text class=\"activity-label\" x=\"{}\" y=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\">{}</text>",
            fmt(node.bounds.x + node.bounds.width / 2.0),
            fmt(node.bounds.y + node.bounds.height / 2.0),
            escape_text(name),
        );
    }
}

fn render_subprocess(svg: &mut String, node: &NodeLayout, include_metadata_attributes: bool,
    highlighted: bool,
) {
    let _ = write!(
        svg,
        "<g class=\"subprocess{}\"{}><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"12\" ry=\"12\" class=\"subprocess{}\" />",
        highlight_class(highlighted),
        metadata_attribute(
            "data-element-id",
            &node.element_id,
            include_metadata_attributes
        ),
        fmt(node.bounds.x),
        fmt(node.bounds.y),
        fmt(node.bounds.width),
        fmt(node.bounds.height),
        highlight_class(highlighted),
    );
    // inner lighter rectangle to suggest nesting
    let _ = write!(
        svg,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" ry=\"10\" class=\"subprocess-inner\" /></g>",
        fmt(node.bounds.x + 3.0),
        fmt(node.bounds.y + 3.0),
        fmt(node.bounds.width - 6.0),
        fmt(node.bounds.height - 6.0),
    );
    if let Some(name) = &node.name {
        let _ = write!(
            svg,
            "<text class=\"activity-label\" x=\"{}\" y=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\">{}</text>",
            fmt(node.bounds.x + node.bounds.width / 2.0),
            fmt(node.bounds.y + node.bounds.height / 2.0),
            escape_text(name),
        );
    }
}

fn render_event(svg: &mut String, node: &NodeLayout, include_metadata_attributes: bool,
    highlighted: bool,
) {
    let bounds = &node.bounds;
    let center_x = bounds.x + bounds.width / 2.0;
    let center_y = bounds.y + bounds.height / 2.0;
    let radius = bounds.width.min(bounds.height) / 2.0;
    let _ = write!(
        svg,
        "<g class=\"event {}{}\"{}><circle class=\"event{}\" cx=\"{}\" cy=\"{}\" r=\"{}\"{} /></g>",
        event_class_name(&node.kind),
        highlight_class(highlighted),
        metadata_attribute(
            "data-element-id",
            &node.element_id,
            include_metadata_attributes
        ),
        highlight_class(highlighted),
        fmt(center_x),
        fmt(center_y),
        fmt(radius),
        if matches!(
            node.kind,
            DiagramNodeKind::EndEvent
                | DiagramNodeKind::IntermediateCatchEvent
                | DiagramNodeKind::IntermediateThrowEvent
        ) {
            " stroke-width=\"3\""
        } else {
            ""
        }
    );
    if matches!(
        node.kind,
        DiagramNodeKind::IntermediateCatchEvent | DiagramNodeKind::IntermediateThrowEvent
    ) {
        let _ = write!(
            svg,
            "<circle class=\"event\" cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" />",
            fmt(center_x),
            fmt(center_y),
            fmt(radius - 5.0),
        );
    }
    if let Some(name) = &node.name {
        let _ = write!(
            svg,
            "<text class=\"event-label\" x=\"{}\" y=\"{}\">{}</text>",
            fmt(center_x),
            fmt(bounds.y + bounds.height + 18.0),
            escape_text(name),
        );
    }
}

fn render_gateway(svg: &mut String, node: &NodeLayout, include_metadata_attributes: bool,
    highlighted: bool,
) {
    let bounds = &node.bounds;
    let left = bounds.x;
    let right = bounds.x + bounds.width;
    let top = bounds.y;
    let bottom = bounds.y + bounds.height;
    let center_x = bounds.x + bounds.width / 2.0;
    let center_y = bounds.y + bounds.height / 2.0;
    let _ = write!(
        svg,
        "<g class=\"gateway {}{}\"{}><polygon class=\"gateway{}\" points=\"{},{} {},{} {},{} {},{}\" /></g>",
        gateway_class_name(&node.kind),
        highlight_class(highlighted),
        metadata_attribute(
            "data-element-id",
            &node.element_id,
            include_metadata_attributes
        ),
        highlight_class(highlighted),
        fmt(center_x),
        fmt(top),
        fmt(right),
        fmt(center_y),
        fmt(center_x),
        fmt(bottom),
        fmt(left),
        fmt(center_y),
    );
    let _ = write!(
        svg,
        "<text class=\"gateway-symbol\" x=\"{}\" y=\"{}\">{}</text>",
        fmt(center_x),
        fmt(center_y + 1.0),
        gateway_symbol(&node.kind),
    );
    if let Some(name) = &node.name {
        let _ = write!(
            svg,
            "<text class=\"gateway-label\" x=\"{}\" y=\"{}\">{}</text>",
            fmt(center_x),
            fmt(bounds.y + bounds.height + 18.0),
            escape_text(name),
        );
    }
}

fn activity_class_name(kind: &DiagramNodeKind) -> &'static str {
    match kind {
        DiagramNodeKind::Task => "task",
        DiagramNodeKind::UserTask => "user-task",
        DiagramNodeKind::ServiceTask => "service-task",
        DiagramNodeKind::ScriptTask => "script-task",
        DiagramNodeKind::ManualTask => "manual-task",
        DiagramNodeKind::ReceiveTask => "receive-task",
        DiagramNodeKind::BusinessRuleTask => "business-rule-task",
        DiagramNodeKind::CallActivity => "call-activity",
        _ => "activity",
    }
}

fn event_class_name(kind: &DiagramNodeKind) -> &'static str {
    match kind {
        DiagramNodeKind::StartEvent => "start-event",
        DiagramNodeKind::EndEvent => "end-event",
        DiagramNodeKind::IntermediateCatchEvent => "intermediate-catch-event",
        DiagramNodeKind::IntermediateThrowEvent => "intermediate-throw-event",
        DiagramNodeKind::BoundaryEvent => "boundary-event",
        _ => "event",
    }
}

fn gateway_class_name(kind: &DiagramNodeKind) -> &'static str {
    match kind {
        DiagramNodeKind::ExclusiveGateway => "exclusive-gateway",
        DiagramNodeKind::ParallelGateway => "parallel-gateway",
        DiagramNodeKind::InclusiveGateway => "inclusive-gateway",
        DiagramNodeKind::EventBasedGateway => "event-based-gateway",
        DiagramNodeKind::ComplexGateway => "complex-gateway",
        _ => "gateway",
    }
}

fn gateway_symbol(kind: &DiagramNodeKind) -> &'static str {
    match kind {
        DiagramNodeKind::ExclusiveGateway => "X",
        DiagramNodeKind::ParallelGateway => "+",
        DiagramNodeKind::InclusiveGateway => "O",
        DiagramNodeKind::EventBasedGateway => "E",
        DiagramNodeKind::ComplexGateway => "*",
        _ => "",
    }
}

fn metadata_attribute(name: &str, value: &str, enabled: bool) -> String {
    if enabled {
        format!(" {}=\"{}\"", name, escape_attribute(value))
    } else {
        String::new()
    }
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
}

fn fmt(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}
