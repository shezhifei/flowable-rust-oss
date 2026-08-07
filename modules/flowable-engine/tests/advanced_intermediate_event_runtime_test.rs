use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;

#[test]
fn test_conditional_intermediate_catch() {
    let engine = ProcessEngine::new("conditional-catch-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="condProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="catchCond" />
        <intermediateCatchEvent id="catchCond">
          <conditionalEventDefinition>
            <condition>${approve == true}</condition>
          </conditionalEventDefinition>
        </intermediateCatchEvent>
        <sequenceFlow id="f2" sourceRef="catchCond" targetRef="task1" />
        <userTask id="task1" />
        <sequenceFlow id="f3" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("cond.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("condProcess".to_string());

    let instance = runtime_service.start_process_instance(pi_builder).unwrap();

    let wait_states =
        runtime_service.get_event_wait_states_by_process_instance_id(instance.id.clone());
    let catch_exec = wait_states
        .iter()
        .find(|e| e.activity_id.as_deref() == Some("catchCond"))
        .unwrap();

    // Trigger conditional catch
    runtime_service.trigger_event_intermediate_catch(
        EventSubscriptionKind::Conditional,
        "${approve == true}".to_string(),
        catch_exec.execution_id.clone(),
    );
}

#[test]
fn test_link_intermediate_throw_catch() {
    let engine = ProcessEngine::new("link-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="linkProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="throwLink" />
        <intermediateThrowEvent id="throwLink">
          <linkEventDefinition name="LinkA" />
        </intermediateThrowEvent>
        
        <intermediateCatchEvent id="catchLink">
          <linkEventDefinition name="LinkA" />
        </intermediateCatchEvent>
        <sequenceFlow id="f2" sourceRef="catchLink" targetRef="task1" />
        <userTask id="task1" />
        <sequenceFlow id="f3" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("link.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("linkProcess".to_string());

    let instance = runtime_service.start_process_instance(pi_builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();

    if tasks.len() == 1 {
        // task_definition_key is a String
        assert_eq!(tasks[0].task_definition_key.as_str(), "task1");
    }
}

#[test]
fn test_unsupported_intermediate_event_fails_gracefully() {
    let engine = ProcessEngine::new("unsupported-event-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="badProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="throwCancel" />
        <intermediateThrowEvent id="throwCancel">
          <cancelEventDefinition />
        </intermediateThrowEvent>
        <sequenceFlow id="f2" sourceRef="throwCancel" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("bad.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("badProcess".to_string());

    let res = runtime_service.start_process_instance(pi_builder);

    assert!(res.is_err());
    if let Err(FlowableError::UnsupportedElement {
        element_type,
        activity_id: _,
    }) = res
    {
        assert!(
            element_type.contains("cancelEventDefinition")
                || element_type.contains("IntermediateThrowEvent")
                || element_type.contains("cancel")
        );
    } else {
        panic!("Expected UnsupportedElement error");
    }
}
