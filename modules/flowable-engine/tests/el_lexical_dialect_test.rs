use flowable_engine::engine::process_engine::ProcessEngine;

/// P104: lexical EL operator dialect — sequence-flow conditions written with
/// JUEL keyword operators (`${a eq 'x'}`, `and`, `empty`) must drive
/// exclusive-gateway routing exactly like their symbolic counterparts. Java
/// resolves these through the de.odysseus EL parser in flowable-engine-common
/// (Scanner.java keyword map + Parser.java grammar).
fn deploy_dialect_gateway_process(
    repository_service: &flowable_engine::engine::repository_service::RepositoryService,
) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="elDialectProcess" name="EL Dialect Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow_start_gateway" sourceRef="startEvent1" targetRef="exclusiveGateway1" />

            <exclusiveGateway id="exclusiveGateway1" default="flow_default" />
            <sequenceFlow id="flow_approved" sourceRef="exclusiveGateway1" targetRef="approvedEnd">
                <conditionExpression><![CDATA[${status eq 'approve' and not empty approver}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow_rejected" sourceRef="exclusiveGateway1" targetRef="rejectedEnd">
                <conditionExpression><![CDATA[${status eq 'reject'}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow_default" sourceRef="exclusiveGateway1" targetRef="defaultEnd" />

            <endEvent id="approvedEnd" />
            <endEvent id="rejectedEnd" />
            <endEvent id="defaultEnd" />
        </process>
    </definitions>"#;

    let deployment = repository_service
        .create_deployment()
        .name("EL Dialect Deployment".to_string())
        .add_string("elDialectProcess.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(deployment).unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

fn start_and_get_activity(
    engine: &ProcessEngine,
    process_definition_id: String,
    variables: &[(String, serde_json::Value)],
) -> String {
    let runtime_service = engine.get_runtime_service();
    let mut builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("el-dialect-instance".to_string());
    for (name, value) in variables {
        builder = builder.variable(name.clone(), value.clone());
    }
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
fn lexical_condition_expression_routes_exclusive_gateway() {
    let engine = ProcessEngine::new("default".to_string());
    let repository_service = engine.get_repository_service();
    let process_definition_id = deploy_dialect_gateway_process(&repository_service);

    // `status eq 'approve' and not empty approver` matches the approve branch.
    let activity = start_and_get_activity(
        &engine,
        process_definition_id.clone(),
        &[
            (
                "status".to_string(),
                serde_json::Value::String("approve".to_string()),
            ),
            (
                "approver".to_string(),
                serde_json::Value::String("alice".to_string()),
            ),
        ],
    );
    assert_eq!(activity, "approvedEnd");

    // `status eq 'approve'` but `approver` null → `not empty approver` is
    // false, so the approve branch drops out and the default flow is taken.
    let activity = start_and_get_activity(
        &engine,
        process_definition_id.clone(),
        &[
            (
                "status".to_string(),
                serde_json::Value::String("approve".to_string()),
            ),
            ("approver".to_string(), serde_json::Value::Null),
        ],
    );
    assert_eq!(activity, "defaultEnd");

    // `status eq 'reject'` routes to the reject branch. The approve branch
    // short-circuits on `status eq 'approve'` (false), so the unset `approver`
    // variable is never evaluated.
    let activity = start_and_get_activity(
        &engine,
        process_definition_id,
        &[(
            "status".to_string(),
            serde_json::Value::String("reject".to_string()),
        )],
    );
    assert_eq!(activity, "rejectedEnd");
}

/// P104: bracket indexing in a condition — `approvers[0] eq 'alice'` routes an
/// exclusive gateway. Exercises the `[expr]` AST/instruction path end-to-end.
#[test]
fn lexical_bracket_index_condition_routes_exclusive_gateway() {
    let engine = ProcessEngine::new("default".to_string());
    let repository_service = engine.get_repository_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="elBracketProcess" name="EL Bracket Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow_start_gateway" sourceRef="startEvent1" targetRef="exclusiveGateway1" />

            <exclusiveGateway id="exclusiveGateway1" default="flow_default" />
            <sequenceFlow id="flow_alice" sourceRef="exclusiveGateway1" targetRef="aliceEnd">
                <conditionExpression><![CDATA[${approvers[0] eq 'alice'}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow_default" sourceRef="exclusiveGateway1" targetRef="defaultEnd" />

            <endEvent id="aliceEnd" />
            <endEvent id="defaultEnd" />
        </process>
    </definitions>"#;

    let deployment = repository_service
        .create_deployment()
        .name("EL Bracket Deployment".to_string())
        .add_string("elBracketProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(deployment).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // approvers[0] == "alice" → alice branch.
    let activity = start_and_get_activity(
        &engine,
        process_definition_id.clone(),
        &[(
            "approvers".to_string(),
            serde_json::Value::Array(vec![
                serde_json::Value::String("alice".to_string()),
                serde_json::Value::String("bob".to_string()),
            ]),
        )],
    );
    assert_eq!(activity, "aliceEnd");

    // approvers[0] != "alice" → condition false, default flow taken.
    let activity = start_and_get_activity(
        &engine,
        process_definition_id,
        &[(
            "approvers".to_string(),
            serde_json::Value::Array(vec![serde_json::Value::String("carol".to_string())]),
        )],
    );
    assert_eq!(activity, "defaultEnd");
}
