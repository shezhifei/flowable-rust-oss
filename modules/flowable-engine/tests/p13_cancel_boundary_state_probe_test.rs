use flowable_engine::engine::process_engine::ProcessEngine;

/// P13 probe: cancel end → cancel boundary does not go through
/// `execute_boundary_trigger`; assert whether `boundary_event_state` remains.
#[test]
fn p13_probe_cancel_boundary_state_after_cancel_end() {
    let engine = ProcessEngine::new("p13-cancel-state-probe".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="cancelStateProbe" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="transaction" />
        <transaction id="transaction">
            <startEvent id="txStart" />
            <sequenceFlow id="txF1" sourceRef="txStart" targetRef="txTask" />
            <userTask id="txTask" />
            <sequenceFlow id="txF2" sourceRef="txTask" targetRef="throwCancel" />
            <endEvent id="throwCancel">
                <cancelEventDefinition />
            </endEvent>
        </transaction>
        <boundaryEvent id="catchCancel" attachedToRef="transaction">
            <cancelEventDefinition />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="catchCancel" targetRef="cancelTask" />
        <userTask id="cancelTask" />
        <sequenceFlow id="f3" sourceRef="cancelTask" targetRef="end" />
        <sequenceFlow id="f4" sourceRef="transaction" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("cancel_probe.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .name("cancelStateProbe".to_string()),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&instance.id, &mut session);
    assert_eq!(
        states_before.len(),
        1,
        "cancel boundary should register on transaction enter"
    );
    assert_eq!(states_before[0].boundary_event_id, "catchCancel");
    drop(session);

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks[0].task_definition_key.as_str(), "txTask");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks[0].task_definition_key.as_str(), "cancelTask");

    let mut session = runtime_store.create_session().unwrap();
    let states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&instance.id, &mut session);
    // Pre-fix probe residue was ["catchCancel"] (cancel path bypasses
    // execute_boundary_trigger). Cleanup is now applied in
    // CancelEndEventActivityBehavior::trigger_cancel_boundary.
    assert!(
        states_after.is_empty(),
        "cancel boundary state should be deleted after cancel path fires; residue={:?}",
        states_after
            .iter()
            .map(|s| s.boundary_event_id.clone())
            .collect::<Vec<_>>()
    );
}
