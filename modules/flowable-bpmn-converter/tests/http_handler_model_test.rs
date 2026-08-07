use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::FlowElementEnum;

#[test]
fn java_http_handlers_are_available_as_typed_model_and_raw_extensions() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="httpHandlers">
    <serviceTask id="httpTask" flowable:type="http">
      <extensionElements>
        <flowable:httpRequestHandler class="com.example.RequestHandler">
          <flowable:field name="marker"><flowable:string>request</flowable:string></flowable:field>
        </flowable:httpRequestHandler>
        <flowable:httpResponseHandler type="script">
          <flowable:script language="javascript" resultVariable="handlerResult">
            return "response";
          </flowable:script>
        </flowable:httpResponseHandler>
      </extensionElements>
    </serviceTask>
  </process>
</definitions>"#;
    let model = BpmnXMLConverter::new().convert_to_bpmn_model(xml);
    let process = model.main_process.unwrap();
    let FlowElementEnum::ServiceTask(task) = process.flow_element_map.get("httpTask").unwrap()
    else {
        panic!("httpTask should be a service task");
    };
    let request = task.http_request_handler.as_ref().unwrap();
    assert_eq!(request.implementation_type.as_deref(), Some("class"));
    assert_eq!(
        request.implementation.as_deref(),
        Some("com.example.RequestHandler")
    );
    assert_eq!(request.field_extensions.len(), 1);
    assert_eq!(
        request.field_extensions[0].string_value.as_deref(),
        Some("request")
    );
    let response = task.http_response_handler.as_ref().unwrap();
    assert_eq!(response.implementation_type.as_deref(), Some("script"));
    let script = response.script_info.as_ref().unwrap();
    assert_eq!(script.language.as_deref(), Some("javascript"));
    assert_eq!(script.result_variable.as_deref(), Some("handlerResult"));
    assert!(script.script.as_deref().unwrap().contains("response"));
    let raw = &task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .extension_elements;
    assert!(raw.contains_key("httpRequestHandler"));
    assert!(raw.contains_key("httpResponseHandler"));
}
