use flowable_engine::el::expression::{Expression, SimpleExpression};
use flowable_engine::el::method_registry::ExpressionMethodRegistry;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::{Value, json};

#[test]
fn java_math_static_type_calls_are_available_by_default() {
    let execution = Execution::default();

    assert_eq!(
        SimpleExpression::new("${T(java.lang.Math).max(4, 9)}".to_string())
            .get_value(&execution),
        Some(json!(9))
    );
    assert_eq!(
        SimpleExpression::new(
            "${T(java.lang.Math).max(4, T(java.lang.Math).abs(-12))}".to_string()
        )
        .get_value(&execution),
        Some(json!(12.0))
    );
}

#[test]
fn registered_bean_method_is_isolated_to_its_engine_command_context() {
    let registry = ExpressionMethodRegistry::default();
    registry.register_bean_method("routing", "isPriority", |arguments| {
        let tier = arguments
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| "routing.isPriority expects a string tier".to_string())?;
        Ok(Value::Bool(tier == "gold"))
    });
    let config = ProcessEngineConfiguration {
        expression_method_registry: registry,
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("p61".to_string(), config);
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p61BeanGateway">
    <startEvent id="start" />
    <sequenceFlow id="toGateway" sourceRef="start" targetRef="gateway" />
    <exclusiveGateway id="gateway" />
    <sequenceFlow id="toPriority" sourceRef="gateway" targetRef="priorityEnd">
      <conditionExpression><![CDATA[${routing.isPriority(customerTier)}]]></conditionExpression>
    </sequenceFlow>
    <sequenceFlow id="toStandard" sourceRef="gateway" targetRef="standardEnd">
      <conditionExpression><![CDATA[${!routing.isPriority(customerTier)}]]></conditionExpression>
    </sequenceFlow>
    <endEvent id="priorityEnd" />
    <endEvent id="standardEnd" />
  </process>
</definitions>"#;
    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("P61 bean gateway".to_string())
                .add_string("p61-bean.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let definition_id = repository.get_process_definition_ids().unwrap()[0].clone();

    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id)
                .variable("customerTier".to_string(), json!("gold")),
        )
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);

    assert_eq!(
        executions
            .get(&instance.id)
            .and_then(|execution| execution.activity_id.as_deref()),
        Some("priorityEnd")
    );

    let outside_engine_context = SimpleExpression::new(
        "${routing.isPriority(customerTier)}".to_string(),
    )
    .get_value(&Execution {
        variables: [("customerTier".to_string(), json!("gold"))].into(),
        ..Default::default()
    });
    assert_eq!(outside_engine_context, None);
}

#[test]
fn custom_static_type_methods_can_be_registered_without_reflection() {
    let registry = ExpressionMethodRegistry::new();
    registry.register_static_method("com.acme.Score", "double", |arguments| {
        let score = arguments
            .first()
            .and_then(Value::as_i64)
            .ok_or_else(|| "Score.double expects an integer".to_string())?;
        Ok(json!(score * 2))
    });

    assert_eq!(
        registry.evaluate(
            "${T(com.acme.Score).double(score)}",
            &Execution {
                variables: [("score".to_string(), json!(7))].into(),
                ..Default::default()
            }
        ),
        Some(json!(14))
    );
}
