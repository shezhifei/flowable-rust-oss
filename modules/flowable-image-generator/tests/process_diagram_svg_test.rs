use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_image_generator::{
    DefaultProcessDiagramGenerator, ProcessDiagramRenderOptions, ProcessDiagramSvgError,
};

fn parse_model(xml: &str) -> flowable_bpmn_model::BpmnModel {
    BpmnXMLConverter::new().convert_to_bpmn_model(xml)
}

#[test]
fn generates_deterministic_svg_for_process_without_di() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process" name="Publishing" isExecutable="true">
    <startEvent id="start" name="Start" />
    <exclusiveGateway id="gateway" name="Review" />
    <userTask id="approve" name="Approve" />
    <serviceTask id="publish" name="Publish" />
    <endEvent id="end" name="Done" />
    <sequenceFlow id="flow_start_gateway" sourceRef="start" targetRef="gateway" />
    <sequenceFlow id="flow_gateway_approve" sourceRef="gateway" targetRef="approve" />
    <sequenceFlow id="flow_gateway_publish" sourceRef="gateway" targetRef="publish" />
    <sequenceFlow id="flow_approve_end" sourceRef="approve" targetRef="end" />
    <sequenceFlow id="flow_publish_end" sourceRef="publish" targetRef="end" />
  </process>
</definitions>"#;
    let model = parse_model(xml);
    let generator = DefaultProcessDiagramGenerator::new();

    let first = generator
        .generate_svg(&model)
        .expect("generator should auto-layout when DI is missing");
    let second = generator
        .generate_svg(&model)
        .expect("svg generation should be deterministic");

    assert_eq!(first, second);
    assert!(first.contains("<svg"));
    assert!(first.contains("data-process-id=\"process\""));
    assert!(first.contains("data-element-id=\"start\""));
    assert!(first.contains("data-element-id=\"gateway\""));
    assert!(first.contains("data-element-id=\"approve\""));
    assert!(first.contains("data-element-id=\"publish\""));
    assert!(first.contains("data-element-id=\"end\""));
    assert!(first.contains("Approve"));
    assert!(first.contains("Publish"));
    assert!(first.contains("marker-end"));
}

#[test]
fn rejects_unsupported_advanced_render_options() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process" isExecutable="true">
    <startEvent id="start" />
    <task id="task1" />
    <endEvent id="end" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
  </process>
</definitions>"#;
    let model = parse_model(xml);
    let generator = DefaultProcessDiagramGenerator::with_options(ProcessDiagramRenderOptions {
        draw_sequence_flow_names: true,
        ..ProcessDiagramRenderOptions::default()
    });

    let error = generator
        .generate_svg(&model)
        .expect_err("unsupported rendering option should fail structurally");

    assert!(matches!(
        error,
        ProcessDiagramSvgError::UnsupportedOption { option, .. }
            if option == "draw_sequence_flow_names"
    ));
}

#[test]
fn renders_highlighted_activity_and_flow_classes() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process" isExecutable="true">
    <startEvent id="start" />
    <userTask id="approve" name="Approve" />
    <serviceTask id="publish" name="Publish" />
    <endEvent id="end" />
    <sequenceFlow id="flow_start_approve" sourceRef="start" targetRef="approve" />
    <sequenceFlow id="flow_approve_publish" sourceRef="approve" targetRef="publish" />
    <sequenceFlow id="flow_publish_end" sourceRef="publish" targetRef="end" />
  </process>
</definitions>"#;
    let model = parse_model(xml);
    let generator = DefaultProcessDiagramGenerator::with_options(ProcessDiagramRenderOptions {
        highlight_activity_ids: vec!["approve".to_string(), "end".to_string()],
        highlight_flow_ids: vec!["flow_approve_publish".to_string()],
        ..ProcessDiagramRenderOptions::default()
    });

    let svg = generator
        .generate_svg(&model)
        .expect("highlight options should be supported");

    assert!(
        svg.contains("class=\"activity user-task highlighted\""),
        "highlighted activity group should include the highlighted class: {svg}"
    );
    assert!(
        svg.contains("class=\"sequence-flow highlighted\""),
        "highlighted sequence flow should include the highlighted class: {svg}"
    );
    assert!(
        svg.contains("data-element-id=\"approve\""),
        "highlighted activity id should still be present as metadata"
    );
    assert!(
        svg.contains("data-element-id=\"flow_approve_publish\""),
        "highlighted flow id should still be present as metadata"
    );
    assert!(
        !svg.contains("class=\"activity service-task highlighted\""),
        "publish task must not be highlighted"
    );
}