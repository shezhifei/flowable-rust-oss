use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_layout::{
    BpmnAutoLayout, BpmnAutoLayoutOptions, BpmnLayoutError, DiagramNodeKind, LayoutDirection,
};

fn parse_model(xml: &str) -> flowable_bpmn_model::BpmnModel {
    BpmnXMLConverter::new().convert_to_bpmn_model(xml)
}

#[test]
fn auto_layout_generates_deterministic_di_for_supported_processes() {
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
    let auto_layout = BpmnAutoLayout::new();

    let first = auto_layout
        .generate(&model)
        .expect("supported process should auto-layout");
    let second = auto_layout
        .generate(&model)
        .expect("auto-layout should be deterministic");

    assert_eq!(first.diagram, second.diagram);
    assert_eq!(
        first.diagram.nodes["start"].kind,
        DiagramNodeKind::StartEvent
    );
    assert_eq!(
        first.diagram.nodes["gateway"].kind,
        DiagramNodeKind::ExclusiveGateway
    );
    assert_eq!(
        first.diagram.nodes["approve"].kind,
        DiagramNodeKind::UserTask
    );
    assert_eq!(
        first.diagram.nodes["publish"].kind,
        DiagramNodeKind::ServiceTask
    );

    let start = &first.diagram.nodes["start"].bounds;
    let gateway = &first.diagram.nodes["gateway"].bounds;
    let approve = &first.diagram.nodes["approve"].bounds;
    let publish = &first.diagram.nodes["publish"].bounds;
    let end = &first.diagram.nodes["end"].bounds;

    assert!(start.x < gateway.x);
    assert!(gateway.x < approve.x);
    assert!(gateway.x < publish.x);
    assert_eq!(approve.x, publish.x);
    assert_ne!(approve.y, publish.y);
    assert!(approve.x < end.x);
    assert!(publish.x < end.x);

    let di_model = first.into_model();
    for element_id in ["start", "gateway", "approve", "publish", "end"] {
        assert!(
            di_model.location_map.contains_key(element_id),
            "missing generated DI for {element_id}"
        );
    }

    for flow_id in [
        "flow_start_gateway",
        "flow_gateway_approve",
        "flow_gateway_publish",
        "flow_approve_end",
        "flow_publish_end",
    ] {
        let waypoints = di_model
            .flow_location_map
            .get(flow_id)
            .unwrap_or_else(|| panic!("missing generated waypoints for {flow_id}"));
        assert!(
            waypoints.len() >= 2,
            "expected at least a start and end waypoint for {flow_id}"
        );
    }
}

#[test]
fn auto_layout_rejects_unsupported_advanced_features() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process" isExecutable="true">
    <laneSet id="laneSet">
      <lane id="laneA" name="Owners">
        <flowNodeRef>task1</flowNodeRef>
      </lane>
    </laneSet>
    <startEvent id="start" />
    <task id="task1" />
    <endEvent id="end" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
  </process>
</definitions>"#;
    let model = parse_model(xml);

    let error = BpmnAutoLayout::with_options(BpmnAutoLayoutOptions {
        direction: LayoutDirection::TopToBottom,
        ..BpmnAutoLayoutOptions::default()
    })
    .generate(&model)
    .expect_err("unsupported advanced layout option should fail structurally");

    assert!(matches!(
        error,
        BpmnLayoutError::UnsupportedOption { option, .. } if option == "direction"
    ));

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("lane layout should now succeed");

    assert!(
        result.bpmn_model.location_map.contains_key("laneA"),
        "lane should have a diagram location"
    );
}

#[test]
fn auto_layout_supports_message_flows() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process1" isExecutable="true">
    <startEvent id="start1" />
    <task id="task1" />
    <task id="task2" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="task2" />
  </process>
  <collaboration id="collab1">
    <messageFlow id="msg1" name="notify" sourceRef="task1" targetRef="task2" />
  </collaboration>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("message-flow layout should succeed");

    assert!(
        result.bpmn_model.flow_location_map.contains_key("msg1"),
        "message flow should have generated waypoints"
    );
    let waypoints = &result.bpmn_model.flow_location_map["msg1"];
    assert!(
        waypoints.len() >= 2,
        "message flow should have start and end waypoints"
    );
}

#[test]
fn auto_layout_supports_data_objects() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="process1" isExecutable="true">
    <startEvent id="start1" />
    <task id="task1" />
    <endEvent id="end1" />
    <dataObject id="do1" name="order" />
    <dataObjectReference id="dref1" name="order-ref" dataObjectRef="do1" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end1" />
  </process>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("data-object layout should succeed");

    assert!(
        result.bpmn_model.location_map.contains_key("do1"),
        "data object should have a diagram location"
    );
    let info = &result.bpmn_model.location_map["do1"];
    assert!(
        info.width > 0.0 && info.height > 0.0,
        "data object should have non-zero size"
    );
    assert!(
        result.bpmn_model.location_map.contains_key("dref1"),
        "data object reference should also have a diagram location"
    );
}

#[test]
fn auto_layout_supports_pools() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="proc1" isExecutable="true">
    <startEvent id="start1" />
    <task id="task1" />
    <endEvent id="end1" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end1" />
  </process>
  <collaboration id="collab1">
    <participant id="pool1" name="Order Processing" processRef="proc1" />
  </collaboration>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("pool-aware layout should succeed");

    assert!(
        result.bpmn_model.location_map.contains_key("pool1"),
        "pool should have a diagram location"
    );
    let info = &result.bpmn_model.location_map["pool1"];
    assert!(
        info.width > 0.0 && info.height > 0.0,
        "pool should have non-zero size"
    );
}

#[test]
fn auto_layout_supports_pool_with_lanes() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="proc1" isExecutable="true">
    <laneSet id="laneSet">
      <lane id="lane1" name="Dev">
        <flowNodeRef>start1</flowNodeRef>
        <flowNodeRef>task1</flowNodeRef>
      </lane>
      <lane id="lane2" name="QA">
        <flowNodeRef>task2</flowNodeRef>
        <flowNodeRef>end1</flowNodeRef>
      </lane>
    </laneSet>
    <startEvent id="start1" />
    <task id="task1" />
    <task id="task2" />
    <endEvent id="end1" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="task2" />
    <sequenceFlow id="flow3" sourceRef="task2" targetRef="end1" />
  </process>
  <collaboration id="collab1">
    <participant id="pool1" name="Pipeline" processRef="proc1" />
  </collaboration>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("pool + lane layout should succeed");

    assert!(
        result.bpmn_model.location_map.contains_key("pool1"),
        "pool should be laid out"
    );
    assert!(
        result.bpmn_model.location_map.contains_key("lane1"),
        "lane1 should be laid out"
    );
    assert!(
        result.bpmn_model.location_map.contains_key("lane2"),
        "lane2 should be laid out"
    );
    let pool = &result.bpmn_model.location_map["pool1"];
    let lane2 = &result.bpmn_model.location_map["lane2"];
    assert!(
        pool.height >= lane2.y + lane2.height - pool.y + 1.0,
        "pool should wrap all lanes (pool.y={}, pool.h={}, lane2.y={}, lane2.h={})",
        pool.y,
        pool.height,
        lane2.y,
        lane2.height
    );
}

#[test]
fn auto_layout_supports_associations() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="proc1" isExecutable="true">
    <startEvent id="start1" />
    <task id="task1" />
    <endEvent id="end1" />
    <dataObject id="do1" name="order" />
    <association id="assoc1" sourceRef="task1" targetRef="do1" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end1" />
  </process>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("association layout should succeed");

    assert!(
        result.bpmn_model.flow_location_map.contains_key("assoc1"),
        "association should have generated waypoints"
    );
    let waypoints = &result.bpmn_model.flow_location_map["assoc1"];
    assert!(
        waypoints.len() >= 2,
        "association should have start and end waypoints"
    );
}

#[test]
fn auto_layout_supports_lanes() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="proc1" isExecutable="true">
    <laneSet id="laneSet">
      <lane id="lane1" name="Development">
        <flowNodeRef>start1</flowNodeRef>
        <flowNodeRef>task1</flowNodeRef>
      </lane>
      <lane id="lane2" name="Testing">
        <flowNodeRef>task2</flowNodeRef>
        <flowNodeRef>end1</flowNodeRef>
      </lane>
    </laneSet>
    <startEvent id="start1" />
    <task id="task1" name="Develop" />
    <task id="task2" name="Test" />
    <endEvent id="end1" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="task2" />
    <sequenceFlow id="flow3" sourceRef="task2" targetRef="end1" />
  </process>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("lane-aware layout should succeed");

    assert!(
        result.bpmn_model.location_map.contains_key("lane1"),
        "lane1 should have a diagram location"
    );
    assert!(
        result.bpmn_model.location_map.contains_key("lane2"),
        "lane2 should have a diagram location"
    );
    let lane1 = &result.bpmn_model.location_map["lane1"];
    let lane2 = &result.bpmn_model.location_map["lane2"];
    assert!(
        lane1.width > 0.0 && lane1.height > 0.0,
        "lane1 should have non-zero size"
    );
    assert!(
        lane2.width > 0.0 && lane2.height > 0.0,
        "lane2 should have non-zero size"
    );
    assert!(
        lane2.y >= lane1.y + lane1.height - 1.0,
        "lane2 should be below lane1"
    );
    let task1 = &result.bpmn_model.location_map["task1"];
    let task2 = &result.bpmn_model.location_map["task2"];
    assert!(
        (task2.y - task1.y).abs() > 1.0,
        "nodes in different lanes should be in different rows"
    );
}

#[test]
fn auto_layout_supports_multi_process() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="proc1" isExecutable="true">
    <startEvent id="start1" />
    <task id="task1" />
    <endEvent id="end1" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end1" />
  </process>
  <process id="proc2" isExecutable="true">
    <startEvent id="start2" />
    <task id="task2" />
    <endEvent id="end2" />
    <sequenceFlow id="flow3" sourceRef="start2" targetRef="task2" />
    <sequenceFlow id="flow4" sourceRef="task2" targetRef="end2" />
  </process>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("multi-process layout should succeed");

    assert!(
        result.bpmn_model.location_map.contains_key("start1"),
        "proc1 elements should be laid out"
    );
    assert!(
        result.bpmn_model.location_map.contains_key("start2"),
        "proc2 elements should be laid out"
    );
    let task1 = &result.bpmn_model.location_map["task1"];
    let task2 = &result.bpmn_model.location_map["task2"];
    assert!(
        task2.x >= task1.x + task1.width - 1.0,
        "proc2 should be to the right of proc1"
    );
}

#[test]
fn auto_layout_supports_multi_process_with_message_flow() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="proc1" isExecutable="true">
    <startEvent id="start1" />
    <task id="task1" />
    <endEvent id="end1" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end1" />
  </process>
  <process id="proc2" isExecutable="true">
    <startEvent id="start2" />
    <task id="task2" />
    <endEvent id="end2" />
    <sequenceFlow id="flow3" sourceRef="start2" targetRef="task2" />
    <sequenceFlow id="flow4" sourceRef="task2" targetRef="end2" />
  </process>
  <collaboration id="collab1">
    <messageFlow id="msg1" name="handoff" sourceRef="task1" targetRef="task2" />
  </collaboration>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("multi-process with message flow should succeed");

    assert!(
        result.bpmn_model.location_map.contains_key("start1"),
        "proc1 should be laid out"
    );
    assert!(
        result.bpmn_model.location_map.contains_key("start2"),
        "proc2 should be laid out"
    );
    assert!(
        result.bpmn_model.flow_location_map.contains_key("msg1"),
        "message flow should have waypoints"
    );
    assert!(
        result.bpmn_model.flow_location_map.contains_key("flow1"),
        "proc1 sequence flow should have waypoints"
    );
    assert!(
        result.bpmn_model.flow_location_map.contains_key("flow3"),
        "proc2 sequence flow should have waypoints"
    );
}

#[test]
fn auto_layout_supports_lane_with_data_object() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="proc1" isExecutable="true">
    <laneSet id="laneSet">
      <lane id="lane1" name="Development">
        <flowNodeRef>start1</flowNodeRef>
        <flowNodeRef>task1</flowNodeRef>
      </lane>
    </laneSet>
    <startEvent id="start1" />
    <task id="task1" />
    <endEvent id="end1" />
    <dataObject id="do1" name="spec" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="task1" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end1" />
  </process>
</definitions>"#;
    let model = parse_model(xml);

    let result = BpmnAutoLayout::new()
        .generate(&model)
        .expect("lane + data object layout should succeed");

    assert!(
        result.bpmn_model.location_map.contains_key("lane1"),
        "lane should be laid out"
    );
    assert!(
        result.bpmn_model.location_map.contains_key("do1"),
        "data object should be laid out"
    );
}

#[test]
fn auto_layout_is_deterministic() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <process id="proc1" isExecutable="true">
    <startEvent id="start1" />
    <parallelGateway id="gw1" />
    <task id="task1" name="Branch A" />
    <task id="task2" name="Branch B" />
    <parallelGateway id="gw2" />
    <endEvent id="end1" />
    <sequenceFlow id="flow1" sourceRef="start1" targetRef="gw1" />
    <sequenceFlow id="flow2" sourceRef="gw1" targetRef="task1" />
    <sequenceFlow id="flow3" sourceRef="gw1" targetRef="task2" />
    <sequenceFlow id="flow4" sourceRef="task1" targetRef="gw2" />
    <sequenceFlow id="flow5" sourceRef="task2" targetRef="gw2" />
    <sequenceFlow id="flow6" sourceRef="gw2" targetRef="end1" />
  </process>
</definitions>"#;
    let model = parse_model(xml);

    let result1 = BpmnAutoLayout::new()
        .generate(&model)
        .expect("first layout should succeed");
    let result2 = BpmnAutoLayout::new()
        .generate(&model)
        .expect("second layout should succeed");

    let keys: Vec<&String> = result1.bpmn_model.location_map.keys().collect();
    for key in &keys {
        let a = &result1.bpmn_model.location_map[*key];
        let b = &result2.bpmn_model.location_map[*key];
        assert!(
            (a.x - b.x).abs() < 0.001 && (a.y - b.y).abs() < 0.001,
            "element {} should have identical position across runs: {:?} vs {:?}",
            key,
            (a.x, a.y),
            (b.x, b.y)
        );
    }
}
