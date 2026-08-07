use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use base64::Engine;
use flowable_rest::{
    error::ApiError,
    routes::content::{
        self, ContentItemCreateCommand, ContentItemQuery, ContentItemRecord, DynContentService,
    },
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockContentService {
    items: Mutex<HashMap<String, ContentItemRecord>>,
}

impl MockContentService {
    fn with_seed() -> Self {
        let service = Self::default();
        service.items.lock().unwrap().insert(
            "content-1".to_string(),
            ContentItemRecord {
                id: "content-1".to_string(),
                name: "invoice.pdf".to_string(),
                mime_type: Some("application/pdf".to_string()),
                description: None,
                attachment_type: None,
                external_url: None,
                task_id: Some("task-1".to_string()),
                process_instance_id: Some("process-1".to_string()),
                scope_type: Some("bpmn".to_string()),
                scope_id: Some("process-1".to_string()),
                created: 1_713_674_400_000,
                modified: 1_713_674_400_000,
                content_size: 256,
            },
        );
        service
    }
}

impl content::ContentServiceApi for MockContentService {
    fn create_content_item(
        &self,
        command: ContentItemCreateCommand,
        _authenticated_user_id: Option<&str>,
    ) -> Result<ContentItemRecord, ApiError> {
        let id = {
            let items = self.items.lock().unwrap();
            format!("content-{}", items.len() + 1)
        };
        let record = ContentItemRecord {
            id: id.clone(),
            name: command.name,
            mime_type: command.mime_type,
            description: command.description,
            attachment_type: command.attachment_type,
            external_url: command.external_url,
            task_id: command.task_id,
            process_instance_id: command.process_instance_id,
            scope_type: command.scope_type,
            scope_id: command.scope_id,
            created: 1_713_674_500_000,
            modified: 1_713_674_500_000,
            content_size: command.content.as_deref().map(str::len).unwrap_or_default(),
        };
        self.items.lock().unwrap().insert(id, record.clone());
        Ok(record)
    }

    fn list_content_items(
        &self,
        query: ContentItemQuery,
    ) -> Result<flowable_rest::common::PagedResponse<ContentItemRecord>, ApiError> {
        let mut items: Vec<ContentItemRecord> =
            self.items.lock().unwrap().values().cloned().collect();
        items.sort_by(|left, right| left.id.cmp(&right.id));
        let filtered: Vec<ContentItemRecord> = items
            .into_iter()
            .filter(|item| {
                query.name.as_ref().is_none_or(|value| item.name == *value)
                    && query
                        .mime_type
                        .as_ref()
                        .is_none_or(|value| item.mime_type.as_deref() == Some(value.as_str()))
                    && query
                        .task_id
                        .as_ref()
                        .is_none_or(|value| item.task_id.as_deref() == Some(value.as_str()))
                    && query.process_instance_id.as_ref().is_none_or(|value| {
                        item.process_instance_id.as_deref() == Some(value.as_str())
                    })
                    && query
                        .scope_type
                        .as_ref()
                        .is_none_or(|value| item.scope_type.as_deref() == Some(value.as_str()))
                    && query
                        .scope_id
                        .as_ref()
                        .is_none_or(|value| item.scope_id.as_deref() == Some(value.as_str()))
            })
            .collect();

        Ok(query.paging.paginate(filtered))
    }

    fn get_content_item(&self, content_item_id: &str) -> Result<ContentItemRecord, ApiError> {
        self.items
            .lock()
            .unwrap()
            .get(content_item_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!("Content item '{content_item_id}' was not found"))
            })
    }

    fn delete_content_item(&self, content_item_id: &str) -> Result<(), ApiError> {
        self.items
            .lock()
            .unwrap()
            .remove(content_item_id)
            .map(|_| ())
            .ok_or_else(|| {
                ApiError::NotFound(format!("Content item '{content_item_id}' was not found"))
            })
    }

    fn get_content_item_object_metadata(
        &self,
        content_item_id: &str,
    ) -> Result<flowable_content_service::ContentObjectStorageMetadata, ApiError> {
        self.get_content_item(content_item_id)?;
        Err(ApiError::NotFound(format!(
            "No storage object associated with content item '{content_item_id}'"
        )))
    }

    fn get_storage_status(&self) -> Result<Value, ApiError> {
        Ok(json!({
            "backend": "in-memory",
            "status": "ok"
        }))
    }
}

async fn auth_middleware(req: Request, next: Next) -> Result<Response, ApiError> {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .unwrap_or("");

    if !auth_header.starts_with("Basic ") {
        return Err(ApiError::Unauthorized);
    }

    let encoded = auth_header.trim_start_matches("Basic ");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::Unauthorized)?;
    let decoded = String::from_utf8(decoded).map_err(|_| ApiError::Unauthorized)?;

    if decoded != "admin:test" {
        return Err(ApiError::Unauthorized);
    }

    Ok(next.run(req).await)
}

async fn spawn_server(service: DynContentService) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let app = Router::new()
        .merge(content::router(service))
        .layer(middleware::from_fn(auth_middleware));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn content_item_routes_follow_common_rest_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockContentService::with_seed())).await;

    let create_response = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "travel-request.txt",
            "mimeType": "text/plain",
            "taskId": "task-2",
            "processInstanceId": "process-2",
            "scopeType": "bpmn",
            "scopeId": "process-2",
            "content": "approved"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create_response.status(), StatusCode::CREATED);
    let create_body: Value = create_response.json().await.unwrap();
    assert_eq!(create_body["name"], "travel-request.txt");
    assert_eq!(create_body["mimeType"], "text/plain");
    assert_eq!(create_body["contentSize"], 8);

    let list_response = client
        .get(format!(
            "{}/content-service/content-items?taskId=task-1&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["start"], 0);
    assert_eq!(list_body["size"], 1);
    assert_eq!(list_body["total"], 1);
    let item = &list_body["data"][0];
    assert_eq!(item["id"], "content-1");
    assert_eq!(item["taskId"], "task-1");
    assert_eq!(item["processInstanceId"], "process-1");

    let get_response = client
        .get(format!(
            "{}/content-service/content-items/content-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], "content-1");
    assert_eq!(get_body["name"], "invoice.pdf");

    let delete_response = client
        .delete(format!(
            "{}/content-service/content-items/content-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let missing_after_delete = client
        .get(format!(
            "{}/content-service/content-items/content-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_after_delete.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn content_item_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockContentService::with_seed())).await;

    let unauthorized = client
        .get(format!("{}/content-service/content-items", base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let bad_query = client
        .get(format!(
            "{}/content-service/content-items?tenantId=tenant-a",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_query.status(), StatusCode::BAD_REQUEST);
    let bad_query_body: Value = bad_query.json().await.unwrap();
    assert_eq!(bad_query_body["code"], "BAD_REQUEST");
    assert!(
        bad_query_body["details"]
            .as_str()
            .unwrap()
            .contains("tenantId")
    );

    let missing = client
        .delete(format!(
            "{}/content-service/content-items/missing-content",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing_body: Value = missing.json().await.unwrap();
    assert_eq!(missing_body["code"], "NOT_FOUND");
}
