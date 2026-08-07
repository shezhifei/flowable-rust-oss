use axum::http::StatusCode;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const DRD_TEST_DMN: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             xmlns:dmndi="https://www.omg.org/spec/DMN/20191111/DMNDI/"
             xmlns:dc="http://www.omg.org/spec/DMN/20180521/DC/"
             id="definitions_drd_test"
             name="DRD Test"
             namespace="http://www.flowable.org/dmn">
    
    <decisionService id="decisionService1" name="My Decision Service">
        <variable id="serviceVar1" name="Service Result" typeRef="string" />
        <outputDecision href="#decisionB" />
        <outputDecision href="#decisionC" />
        <encapsulatedDecision href="#decisionA" />
    </decisionService>

    <decision id="decisionA" name="Decision A">
        <variable id="varA" name="varA" typeRef="string" />
        <decisionTable id="decisionTableA" hitPolicy="UNIQUE">
            <input id="inputA1" label="Input A1">
                <inputExpression id="inputExpressionA1" typeRef="integer">
                    <text>inputVal</text>
                </inputExpression>
            </input>
            <output id="outputA1" label="Output A1" name="varA" typeRef="string" />
            <rule id="ruleA1">
                <inputEntry id="inputEntryA1">
                    <text>&gt;= 100</text>
                </inputEntry>
                <outputEntry id="outputEntryA1">
                    <text>'High'</text>
                </outputEntry>
            </rule>
            <rule id="ruleA2">
                <inputEntry id="inputEntryA2">
                    <text>&lt; 100</text>
                </inputEntry>
                <outputEntry id="outputEntryA2">
                    <text>'Low'</text>
                </outputEntry>
            </rule>
        </decisionTable>
    </decision>

    <decision id="decisionB" name="Decision B">
        <variable id="varB" name="varB" typeRef="string" />
        <informationRequirement id="infoReq1">
            <requiredDecision href="#decisionA" />
        </informationRequirement>
        <authorityRequirement id="authReq1">
            <requiredAuthority href="#knowledgeSource1" />
        </authorityRequirement>
        <decisionTable id="decisionTableB" hitPolicy="UNIQUE">
            <input id="inputB1" label="Input B1">
                <inputExpression id="inputExpressionB1" typeRef="string">
                    <text>varA</text>
                </inputExpression>
            </input>
            <output id="outputB1" label="Output B1" name="varB" typeRef="string" />
            <rule id="ruleB1">
                <inputEntry id="inputEntryB1">
                    <text>'High'</text>
                </inputEntry>
                <outputEntry id="outputEntryB1">
                    <text>'Approved'</text>
                </outputEntry>
            </rule>
            <rule id="ruleB2">
                <inputEntry id="inputEntryB2">
                    <text>'Low'</text>
                </inputEntry>
                <outputEntry id="outputEntryB2">
                    <text>'Rejected'</text>
                </outputEntry>
            </rule>
        </decisionTable>
    </decision>

    <knowledgeSource id="knowledgeSource1" name="Regulatory Authority">
        <authorityRequirement id="authReq2">
            <requiredDecision href="#decisionA" />
        </authorityRequirement>
    </knowledgeSource>

    <decision id="decisionC" name="Decision C">
        <variable id="varC" name="varC" typeRef="string" />
        <decisionTable id="decisionTableC" hitPolicy="UNIQUE">
            <input id="inputC1" label="Input C1">
                <inputExpression id="inputExpressionC1" typeRef="integer">
                    <text>inputVal</text>
                </inputExpression>
            </input>
            <output id="outputC1" label="Output C1" name="varC" typeRef="string" />
            <rule id="ruleC1">
                <inputEntry id="inputEntryC1">
                    <text>&gt;= 100</text>
                </inputEntry>
                <outputEntry id="outputEntryC1">
                    <text>'ManualReview'</text>
                </outputEntry>
            </rule>
            <rule id="ruleC2">
                <inputEntry id="inputEntryC2">
                    <text>&lt; 100</text>
                </inputEntry>
                <outputEntry id="outputEntryC2">
                    <text>'AutoReject'</text>
                </outputEntry>
            </rule>
        </decisionTable>
    </decision>

</definitions>"##;

async fn spawn_real_server(test_name: &str) -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "admin".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn test_dmn_drd_and_decision_service_flow() {
    let (base_url, client) = spawn_real_server("test_dmn_drd_and_decision_service").await;

    // 1. 部署 DMN 资源
    let deploy_res = client
        .post(format!("{}/dmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "DRD Test Deploy",
            "resourceName": "drd-test.dmn",
            "resource": DRD_TEST_DMN
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(deploy_res.status(), StatusCode::CREATED);
    let deploy_body: Value = deploy_res.json().await.unwrap();
    assert_eq!(deploy_body["name"], "DRD Test Deploy");
    let deployment_id = deploy_body["id"].as_str().unwrap().to_string();

    let service_list_res = client
        .get(format!(
            "{}/dmn-repository/decision-services?key=decisionService1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(service_list_res.status(), StatusCode::OK);
    let service_list_body: Value = service_list_res.json().await.unwrap();
    assert_eq!(service_list_body["total"], 1);
    let service = &service_list_body["data"][0];
    let service_id = service["id"].as_str().unwrap().to_string();
    assert_eq!(service["key"], "decisionService1");
    assert_eq!(service["name"], "My Decision Service");
    assert_eq!(service["deploymentId"], deployment_id);
    assert_eq!(service["resourceName"], "drd-test.dmn");
    assert_eq!(service["requiredDecisionKeys"], json!(["decisionA"]));
    assert_eq!(
        service["outputDecisionKeys"],
        json!(["decisionB", "decisionC"])
    );

    let service_get_res = client
        .get(format!(
            "{}/dmn-repository/decision-services/{}",
            base_url, service_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(service_get_res.status(), StatusCode::OK);
    let service_get_body: Value = service_get_res.json().await.unwrap();
    assert_eq!(service_get_body["id"], service_id);
    assert_eq!(service_get_body["key"], "decisionService1");

    // 2. 触发执行决策 (decisionB)
    // decisionB 依赖 decisionA 的输出 (varA)
    // 当 inputVal=150 时，decisionA 输出 varA='High'
    // decisionB 根据 varA='High' 输出 varB='Approved'
    let exec_res = client
        .post(format!("{}/dmn-runtime/decision-executions", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "decisionB",
            "variables": {
                "varA": "High"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(exec_res.status(), StatusCode::CREATED);
    let exec_body: Value = exec_res.json().await.unwrap();
    // P85: each row is a list of EngineRestVariable {name, type, value}
    let result_vars = exec_body["resultVariables"].as_array().unwrap()[0]
        .as_array()
        .unwrap();
    assert_eq!(
        result_vars,
        &vec![json!({"name": "varB", "type": "string", "value": "Approved"})]
    );

    // 3. 获取 DRD 列表
    let drds_res = client
        .get(format!(
            "{}/dmn-repository/decision-requirements-diagrams",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(drds_res.status(), StatusCode::OK);
    let drds_body: Value = drds_res.json().await.unwrap();
    let drds = drds_body["data"].as_array().unwrap();
    assert!(!drds.is_empty(), "DRD list should not be empty");
    let drd = &drds[0];
    let drd_id = drd["id"].as_str().unwrap().to_string();

    // 4. 获取特定 DRD
    let drd_res = client
        .get(format!(
            "{}/dmn-repository/decision-requirements-diagrams/{}",
            base_url, drd_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(drd_res.status(), StatusCode::OK);
    let drd_body: Value = drd_res.json().await.unwrap();
    assert_eq!(drd_body["id"].as_str().unwrap(), drd_id);

    // 验证包含的决策、决策服务、知识源
    let decisions = drd_body["decisions"].as_array().unwrap();
    assert!(
        decisions
            .iter()
            .any(|d| d["id"].as_str() == Some("decisionA"))
    );
    assert!(
        decisions
            .iter()
            .any(|d| d["id"].as_str() == Some("decisionB"))
    );

    let services = drd_body["decisionServices"].as_array().unwrap();
    assert!(
        services
            .iter()
            .any(|s| s["id"].as_str() == Some("decisionService1"))
    );

    let sources = drd_body["knowledgeSources"].as_array().unwrap();
    assert!(
        sources
            .iter()
            .any(|s| s["id"].as_str() == Some("knowledgeSource1"))
    );

    // 5. 获取 DRD 的原始 XML 字节数据
    let resource_res = client
        .get(format!(
            "{}/dmn-repository/decision-requirements-diagrams/{}/resourcedata",
            base_url, drd_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(resource_res.status(), StatusCode::OK);
    let resource_bytes = resource_res.bytes().await.unwrap();
    let resource_str = String::from_utf8(resource_bytes.to_vec()).unwrap();
    assert!(resource_str.contains("definitions_drd_test"));

    // 6. 获取决策执行的历史记录
    let history_res = client
        .get(format!(
            "{}/dmn-history/historic-decision-executions",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history_res.status(), StatusCode::OK);
    let history_body: Value = history_res.json().await.unwrap();
    let history_records = history_body["data"].as_array().unwrap();
    assert!(!history_records.is_empty());

    let exec_id_1 = history_records[0]["id"].as_str().unwrap().to_string();

    // 7. 测试删除单个决策执行历史记录
    let delete_res = client
        .delete(format!(
            "{}/dmn-history/historic-decision-executions/{}",
            base_url, exec_id_1
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_res.status(), StatusCode::NO_CONTENT);

    // 验证已被删除
    let check_res = client
        .get(format!(
            "{}/dmn-history/historic-decision-executions",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let check_body: Value = check_res.json().await.unwrap();
    let check_records = check_body["data"].as_array().unwrap();
    assert!(
        !check_records
            .iter()
            .any(|r| r["id"].as_str() == Some(&exec_id_1))
    );

    // 8. 触发另一次执行决策服务，为批量删除做准备
    let exec_res_2 = client
        .post(format!("{}/dmn-runtime/decision-executions", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "decisionService1",
            "variables": {
                "inputVal": 50
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(exec_res_2.status(), StatusCode::CREATED);
    let exec_body_2: Value = exec_res_2.json().await.unwrap();
    assert_eq!(exec_body_2["decisionKey"], "decisionService1");
    // P79: decision-service flattens each output decision's rows
    // (Java DmnRestResponseFactory.java:168-188); P85 wraps each variable.
    let service_rows = exec_body_2["resultVariables"].as_array().unwrap();
    let named = |row: &Value, name: &str| -> Option<Value> {
        row.as_array()?
            .iter()
            .find(|var| var["name"] == name)
            .cloned()
    };
    assert!(
        service_rows.iter().any(|row| named(row, "varB")
            == Some(json!({"name": "varB", "type": "string", "value": "Rejected"}))),
        "expected varB=Rejected in service rows: {service_rows:?}"
    );
    assert!(
        service_rows.iter().any(|row| named(row, "varC")
            == Some(json!({"name": "varC", "type": "string", "value": "AutoReject"}))),
        "expected varC=AutoReject in service rows: {service_rows:?}"
    );
    assert!(service_rows.iter().all(|row| named(row, "varA").is_none()));
    let exec_id_2 = exec_body_2["id"].as_str().unwrap().to_string();

    // 9. 测试批量删除历史记录
    let bulk_delete_res = client
        .post(format!(
            "{}/dmn-history/historic-decision-executions/delete",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionExecutionIds": vec![exec_id_2.clone()]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bulk_delete_res.status(), StatusCode::NO_CONTENT);

    // 验证已被批量删除
    let final_check_res = client
        .get(format!(
            "{}/dmn-history/historic-decision-executions",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let final_check_body: Value = final_check_res.json().await.unwrap();
    let final_records = final_check_body["data"].as_array().unwrap();
    assert!(
        !final_records
            .iter()
            .any(|r| r["id"].as_str() == Some(&exec_id_2))
    );
}
