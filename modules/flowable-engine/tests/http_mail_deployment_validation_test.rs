use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;

fn deploy_xml(xml: &str, config: ProcessEngineConfiguration) -> Result<(), FlowableError> {
    let process_engine = ProcessEngine::new_with_config("default".to_string(), config);
    let repository_service = process_engine.get_repository_service();

    let builder = repository_service
        .create_deployment()
        .name("HTTP Mail Validation Deployment".to_string())
        .add_string(
            "http_mail_validation.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).map(|_| ())
}

#[test]
fn owned_http_and_mail_subset_deploy_successfully() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="ownedSubsetProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
            <serviceTask id="httpTask1" flowable:type="http" flowable:resultVariableName="httpResult">
                <extensionElements>
                    <flowable:requestMethod>GET</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/health</flowable:requestUrl>
                    <flowable:basicAuthenticationUsername>health-user</flowable:basicAuthenticationUsername>
                    <flowable:basicAuthenticationPassword>health-password</flowable:basicAuthenticationPassword>
                    <flowable:bodyEncoding>json</flowable:bodyEncoding>
                    <flowable:requestTimeout>5000</flowable:requestTimeout>
                    <flowable:connectTimeout>1000</flowable:connectTimeout>
                    <flowable:followRedirects>false</flowable:followRedirects>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="mailTask1" />
            <serviceTask id="mailTask1" flowable:type="mail" flowable:resultVariableName="mailResult">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local; audit@example.flowable.local</flowable:to>
                    <flowable:subject>Done</flowable:subject>
                    <flowable:text>Owned subset execution.</flowable:text>
                    <flowable:html>&lt;b&gt;Owned subset execution.&lt;/b&gt;</flowable:html>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow3" sourceRef="mailTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(
        result.is_ok(),
        "Owned M14 HTTP/Mail subset should deploy successfully"
    );
}

#[test]
fn http_task_with_non_integer_timeout_extension_fails_structurally() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="unsupportedHttpProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
            <serviceTask id="httpTask1" flowable:type="http">
                <extensionElements>
                    <flowable:requestMethod>GET</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/health</flowable:requestUrl>
                    <flowable:requestTimeout>PT5S</flowable:requestTimeout>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("HTTP service task"));
    assert!(err.contains("requestTimeout"));
}

#[test]
fn http_task_with_incomplete_basic_auth_fails_structurally() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="incompleteBasicAuthProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
            <serviceTask id="httpTask1" flowable:type="http">
                <extensionElements>
                    <flowable:requestMethod>GET</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/health</flowable:requestUrl>
                    <flowable:basicAuthenticationUsername>health-user</flowable:basicAuthenticationUsername>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("basicAuthenticationUsername"));
    assert!(err.contains("basicAuthenticationPassword"));
}

#[test]
fn http_task_with_unsupported_body_encoding_fails_structurally() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="unsupportedBodyEncodingProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
            <serviceTask id="httpTask1" flowable:type="http">
                <extensionElements>
                    <flowable:requestMethod>POST</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/orders</flowable:requestUrl>
                    <flowable:requestBody>{"orderId":42}</flowable:requestBody>
                    <flowable:bodyEncoding>xml</flowable:bodyEncoding>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("bodyEncoding"));
    assert!(err.contains("xml"));
    assert!(err.contains("json, form, or text"));
}

#[test]
fn mail_task_with_html_content_and_multiple_recipients_deploys() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="unsupportedMailProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="mailTask1" />
            <serviceTask id="mailTask1" flowable:type="mail">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local, audit@example.flowable.local</flowable:to>
                    <flowable:subject>Done</flowable:subject>
                    <flowable:text>Owned subset execution.</flowable:text>
                    <flowable:html>&lt;b&gt;supported&lt;/b&gt;</flowable:html>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="mailTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_ok());
}

fn http_handler_xml(handler: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="httpHandlerValidation">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="httpTask" />
    <serviceTask id="httpTask" flowable:type="http">
      <extensionElements>
        <flowable:requestMethod>GET</flowable:requestMethod>
        <flowable:requestUrl>https://example.flowable.local/handler</flowable:requestUrl>
        {handler}
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="f2" sourceRef="httpTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#
    )
}

#[test]
fn http_script_handler_requires_secure_scripting_at_deployment() {
    let xml = http_handler_xml(
        r#"<flowable:httpRequestHandler type="script">
          <flowable:script language="javascript">var handled = true;</flowable:script>
        </flowable:httpRequestHandler>"#,
    );
    let error = deploy_xml(&xml, ProcessEngineConfiguration::default()).unwrap_err();
    assert!(error.to_string().contains("secure scripting"));
}

#[test]
fn http_handler_rejects_multiple_implementation_types() {
    let xml = http_handler_xml(
        r#"<flowable:httpResponseHandler class="com.example.Handler" delegateExpression="${handler}" />"#,
    );
    let error = deploy_xml(&xml, ProcessEngineConfiguration::default()).unwrap_err();
    assert!(error.to_string().contains("exactly one"));
}

#[test]
fn http_script_handler_deploys_when_secure_language_is_enabled() {
    let xml = http_handler_xml(
        r#"<flowable:httpRequestHandler type="script">
          <flowable:script language="javascript" resultVariable="handlerResult">
            var handled = true;
            return handled;
          </flowable:script>
        </flowable:httpRequestHandler>"#,
    );
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };
    assert!(deploy_xml(&xml, config).is_ok());
}
