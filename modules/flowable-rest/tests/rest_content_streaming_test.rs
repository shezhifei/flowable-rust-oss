use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
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

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

#[tokio::test]
async fn test_rest_content_range_streaming() {
    let (_engine, base_url, client) = spawn_server("rest-content-range-streaming").await;

    // 1. 创建 10 字节长度的样本内容项: "0123456789"
    let create = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "digits.txt",
            "mimeType": "text/plain",
            "content": "0123456789"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body: Value = create.json().await.unwrap();
    let content_item_id = create_body["id"].as_str().unwrap();

    // 2. 测试不带 Range 的普通请求 (返回 200 OK)
    let normal_res = client
        .get(format!(
            "{}/content-service/content-items/{}/data",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(normal_res.status(), reqwest::StatusCode::OK);
    assert_eq!(
        normal_res
            .headers()
            .get(reqwest::header::ACCEPT_RANGES)
            .unwrap(),
        "bytes"
    );
    assert_eq!(normal_res.text().await.unwrap(), "0123456789");

    // 3. 测试 Range: bytes=0-4 (返回 206 Partial Content, digits "01234")
    let range1_res = client
        .get(format!(
            "{}/content-service/content-items/{}/data",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::RANGE, "bytes=0-4")
        .send()
        .await
        .unwrap();

    assert_eq!(range1_res.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        range1_res
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .unwrap(),
        "bytes 0-4/10"
    );
    assert_eq!(
        range1_res
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .unwrap(),
        "5"
    );
    assert_eq!(range1_res.text().await.unwrap(), "01234");

    // 4. 测试 Range: bytes=5- (返回 206, digits "56789")
    let range2_res = client
        .get(format!(
            "{}/content-service/content-items/{}/data",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::RANGE, "bytes=5-")
        .send()
        .await
        .unwrap();

    assert_eq!(range2_res.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        range2_res
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .unwrap(),
        "bytes 5-9/10"
    );
    assert_eq!(range2_res.text().await.unwrap(), "56789");

    // 5. 测试 Range: bytes=-3 (返回 206, digits "789")
    let range3_res = client
        .get(format!(
            "{}/content-service/content-items/{}/data",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::RANGE, "bytes=-3")
        .send()
        .await
        .unwrap();

    assert_eq!(range3_res.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        range3_res
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .unwrap(),
        "bytes 7-9/10"
    );
    assert_eq!(range3_res.text().await.unwrap(), "789");

    // 6. 测试越界不可满足 Range: bytes=12-15 (返回 416 Range Not Satisfiable)
    let range_err_res = client
        .get(format!(
            "{}/content-service/content-items/{}/data",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::RANGE, "bytes=12-15")
        .send()
        .await
        .unwrap();

    assert_eq!(
        range_err_res.status(),
        reqwest::StatusCode::RANGE_NOT_SATISFIABLE
    );
    assert_eq!(
        range_err_res
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .unwrap(),
        "bytes */10"
    );
}
