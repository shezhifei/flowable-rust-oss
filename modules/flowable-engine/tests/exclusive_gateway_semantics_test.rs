use flowable_engine::engine::process_engine::ProcessEngine;

fn deploy_exclusive_gateway_process(
    repository_service: &flowable_engine::engine::repository_service::RepositoryService,
) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="exclusiveGatewayProcess" name="Exclusive Gateway Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow_start_gateway" sourceRef="startEvent1" targetRef="exclusiveGateway1" />

            <exclusiveGateway id="exclusiveGateway1" default="flow_default" />
            <sequenceFlow id="flow_first_match" sourceRef="exclusiveGateway1" targetRef="firstEnd">
                <conditionExpression><![CDATA[${approved == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow_second_match" sourceRef="exclusiveGateway1" targetRef="secondEnd">
                <conditionExpression><![CDATA[${secondary == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow_default" sourceRef="exclusiveGateway1" targetRef="defaultEnd" />

            <endEvent id="firstEnd" />
            <endEvent id="secondEnd" />
            <endEvent id="defaultEnd" />
        </process>
    </definitions>"#;

    let deployment = repository_service
        .create_deployment()
        .name("Exclusive Gateway Deployment".to_string())
        .add_string(
            "exclusiveGatewayProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(deployment).unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

fn start_and_get_activity(
    engine: &ProcessEngine,
    process_definition_id: String,
    approved: bool,
    secondary: bool,
) -> String {
    let runtime_service = engine.get_runtime_service();
    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("exclusive-gateway-instance".to_string())
        .variable("approved".to_string(), serde_json::Value::Bool(approved))
        .variable("secondary".to_string(), serde_json::Value::Bool(secondary));

    let process_instance = runtime_service.start_process_instance(builder).unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    let execution = executions
        .get(&process_instance.id)
        .expect("expected root execution to remain in runtime store");

    execution
        .activity_id
        .clone()
        .expect("expected execution to have a current activity id")
}

#[test]
fn exclusive_gateway_takes_only_the_first_matching_branch() {
    let engine = ProcessEngine::new("default".to_string());
    let repository_service = engine.get_repository_service();
    let process_definition_id = deploy_exclusive_gateway_process(&repository_service);

    let activity = start_and_get_activity(&engine, process_definition_id, true, true);

    assert_eq!(activity, "firstEnd");
}

#[test]
fn exclusive_gateway_falls_back_to_default_flow_when_no_condition_matches() {
    let engine = ProcessEngine::new("default".to_string());
    let repository_service = engine.get_repository_service();
    let process_definition_id = deploy_exclusive_gateway_process(&repository_service);

    let activity = start_and_get_activity(&engine, process_definition_id, false, false);

    assert_eq!(activity, "defaultEnd");
}

/// Java `ExclusiveGatewayActivityBehavior.java:104-115` throws a
/// `FlowableException` when no outgoing sequence flow can be selected and
/// no default flow is configured. Verifies the Rust implementation now
/// surfaces the error instead of silently deleting the execution.
#[test]
fn exclusive_gateway_throws_when_no_outgoing_flow_matches_and_no_default() {
    let engine = ProcessEngine::new("default".to_string());
    let repository_service = engine.get_repository_service();

    // Exclusive gateway with two conditional flows and no default. None of
    // the conditions will match when variables are unset, so the engine
    // must raise a FlowableException rather than swallowing the path.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="exclusiveNoDefaultProcess" name="Exclusive No-Default Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow_start_gateway" sourceRef="startEvent1" targetRef="exclusiveGateway1" />

            <exclusiveGateway id="exclusiveGateway1" />
            <sequenceFlow id="flow_first" sourceRef="exclusiveGateway1" targetRef="firstEnd">
                <conditionExpression><![CDATA[${false}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow_second" sourceRef="exclusiveGateway1" targetRef="secondEnd">
                <conditionExpression><![CDATA[${false}]]></conditionExpression>
            </sequenceFlow>

            <endEvent id="firstEnd" />
            <endEvent id="secondEnd" />
        </process>
    </definitions>"#;

    let deployment = repository_service
        .create_deployment()
        .name("Exclusive No-Default Deployment".to_string())
        .add_string(
            "exclusiveNoDefaultProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(deployment).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let runtime_service = engine.get_runtime_service();
    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("exclusive-no-default".to_string());
    let process_instance = runtime_service
        .start_process_instance(builder)
        .expect_err("exclusive gateway with no matching flow and no default must error");

    let message = process_instance.to_string();
    assert!(
        message.contains("exclusive gateway 'exclusiveGateway1'")
            && message.contains("could be selected"),
        "unexpected error message: {}",
        message
    );
}
