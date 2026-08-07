mod test_support;

use flowable_content_service::{
    AzureBlobStorage, AzureBlobStorageConfig, ContentObject, ContentStorage,
    CreateContentItemRequest, GcsStorage, GcsStorageConfig, S3Storage, S3StorageConfig,
};

#[test]
fn test_s3_mock_storage() {
    let config = S3StorageConfig {
        endpoint: "http://localhost:9000".to_string(),
        bucket: "test-bucket-s3".to_string(),
        access_key: "key".to_string(),
        secret_key: "secret".to_string(),
        region: "us-east-1".to_string(),
        mock_mode: true,
    };
    let storage = S3Storage::new(config);
    assert_eq!(storage.backend_name(), "s3");

    let object = ContentObject {
        id: "test-obj-s3".to_string(),
        content_item_id: "item-s3".to_string(),
        data: b"hello-s3-mock".to_vec(),
        mime_type: "text/plain".to_string(),
        file_name: Some("test.txt".to_string()),
        size: 13,
    };

    let meta = storage.store(&object).unwrap();
    assert_eq!(meta.storage_backend, "s3");
    assert_eq!(meta.size, 13);

    let retrieved = storage.retrieve(&meta.storage_id).unwrap();
    assert_eq!(retrieved, b"hello-s3-mock");

    assert!(storage.exists(&meta.storage_id).unwrap());

    let object_meta = storage.get_metadata(&meta.storage_id).unwrap();
    assert_eq!(object_meta.size, 13);

    storage.delete(&meta.storage_id).unwrap();
    assert!(!storage.exists(&meta.storage_id).unwrap());
}

#[test]
fn test_azure_mock_storage() {
    let config = AzureBlobStorageConfig {
        endpoint: "http://localhost:10000".to_string(),
        container: "test-container-azure".to_string(),
        account_name: "devstoreaccount1".to_string(),
        account_key: "key".to_string(),
        mock_mode: true,
    };
    let storage = AzureBlobStorage::new(config);
    assert_eq!(storage.backend_name(), "azure");

    let object = ContentObject {
        id: "test-obj-azure".to_string(),
        content_item_id: "item-azure".to_string(),
        data: b"hello-azure-mock".to_vec(),
        mime_type: "text/plain".to_string(),
        file_name: Some("test.txt".to_string()),
        size: 16,
    };

    let meta = storage.store(&object).unwrap();
    assert_eq!(meta.storage_backend, "azure");
    assert_eq!(meta.size, 16);

    let retrieved = storage.retrieve(&meta.storage_id).unwrap();
    assert_eq!(retrieved, b"hello-azure-mock");

    assert!(storage.exists(&meta.storage_id).unwrap());

    let object_meta = storage.get_metadata(&meta.storage_id).unwrap();
    assert_eq!(object_meta.size, 16);

    storage.delete(&meta.storage_id).unwrap();
    assert!(!storage.exists(&meta.storage_id).unwrap());
}

#[test]
fn test_gcs_mock_storage() {
    let config = GcsStorageConfig {
        endpoint: "http://localhost:4443".to_string(),
        bucket: "test-bucket-gcs".to_string(),
        service_account_key: "key".to_string(),
        mock_mode: true,
    };
    let storage = GcsStorage::new(config);
    assert_eq!(storage.backend_name(), "gcs");

    let object = ContentObject {
        id: "test-obj-gcs".to_string(),
        content_item_id: "item-gcs".to_string(),
        data: b"hello-gcs-mock".to_vec(),
        mime_type: "text/plain".to_string(),
        file_name: Some("test.txt".to_string()),
        size: 14,
    };

    let meta = storage.store(&object).unwrap();
    assert_eq!(meta.storage_backend, "gcs");
    assert_eq!(meta.size, 14);

    let retrieved = storage.retrieve(&meta.storage_id).unwrap();
    assert_eq!(retrieved, b"hello-gcs-mock");

    assert!(storage.exists(&meta.storage_id).unwrap());

    let object_meta = storage.get_metadata(&meta.storage_id).unwrap();
    assert_eq!(object_meta.size, 14);

    storage.delete(&meta.storage_id).unwrap();
    assert!(!storage.exists(&meta.storage_id).unwrap());
}

#[test]
fn test_content_versioning() {
    let service = test_support::service("content-item-versioning-test");

    let req1 = CreateContentItemRequest {
        name: "document.txt".to_string(),
        mime_type: Some("text/plain".to_string()),
        description: None,
        attachment_type: None,
        external_url: None,
        content: Some("version 1 data".to_string()),
        task_id: Some("task-version-01".to_string()),
        process_instance_id: None,
        scope_type: None,
        scope_id: None,
        created_by: Some("test-user".to_string()),
        expires_in_seconds: None,
    };
    let item1 = service.create_content_item(req1.clone()).unwrap();
    assert_eq!(item1.version, Some(1));

    let req2 = CreateContentItemRequest {
        name: "document.txt".to_string(),
        mime_type: Some("text/plain".to_string()),
        description: None,
        attachment_type: None,
        external_url: None,
        content: Some("version 2 data".to_string()),
        task_id: Some("task-version-01".to_string()),
        process_instance_id: None,
        scope_type: None,
        scope_id: None,
        created_by: Some("test-user".to_string()),
        expires_in_seconds: None,
    };
    let item2 = service.create_content_item(req2).unwrap();
    assert_eq!(item2.version, Some(2));
}

#[test]
fn test_content_ttl_and_cleanup() {
    let service = test_support::service("content-item-ttl-test");

    let req = CreateContentItemRequest {
        name: "temp.txt".to_string(),
        mime_type: Some("text/plain".to_string()),
        description: None,
        attachment_type: None,
        external_url: None,
        content: Some("temporary data".to_string()),
        task_id: None,
        process_instance_id: None,
        scope_type: None,
        scope_id: None,
        created_by: Some("test-user".to_string()),
        expires_in_seconds: Some(1),
    };
    let item = service.create_content_item(req).unwrap();

    let cleaned = service.cleanup_expired_items().unwrap();
    assert_eq!(cleaned, 0);

    std::thread::sleep(std::time::Duration::from_millis(1100));

    let cleaned = service.cleanup_expired_items().unwrap();
    assert_eq!(cleaned, 1);

    assert!(service.get_content_item(&item.id).is_err());
}
