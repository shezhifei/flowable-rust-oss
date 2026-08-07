mod test_support;

use flowable_content_service::{
    ContentObject, ContentStorage, CreateContentItemRequest, FlowableContentService,
    LocalFileSystemStorage, LocalFileSystemStorageConfig,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

fn temp_storage_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "flowable-content-storage-{}-{}",
        label,
        Uuid::new_v4()
    ))
}

fn service_with_temp_storage(label: &str) -> (FlowableContentService, PathBuf) {
    let storage_dir = temp_storage_dir(label);
    let storage = Arc::new(LocalFileSystemStorage::new(LocalFileSystemStorageConfig {
        root_dir: storage_dir.clone(),
    }));
    let engine = Arc::new(ProcessEngine::new(label.to_string()));
    let service = FlowableContentService::with_storage(engine, storage);
    (service, storage_dir)
}

fn make_content_object(content_item_id: &str, data: &[u8], mime_type: &str) -> ContentObject {
    ContentObject {
        id: Uuid::new_v4().to_string(),
        content_item_id: content_item_id.to_string(),
        data: data.to_vec(),
        mime_type: mime_type.to_string(),
        file_name: Some("test.bin".to_string()),
        size: data.len() as u64,
    }
}

// ---------------------------------------------------------------------------
// LocalFileSystemStorage 单元测试
// ---------------------------------------------------------------------------

#[test]
fn fs_store_retrieve_round_trip() {
    let dir = temp_storage_dir("fs-round-trip");
    let storage = LocalFileSystemStorage::new(LocalFileSystemStorageConfig {
        root_dir: dir.clone(),
    });

    let object = make_content_object("ci-1", b"hello world", "text/plain");
    let metadata = storage.store(&object).unwrap();

    assert_eq!(metadata.storage_backend, "local-fs");
    assert_eq!(metadata.size, 11);
    assert!(metadata.checksum.is_some());
    assert!(!metadata.storage_id.is_empty());
    assert!(metadata.stored_at.contains('T'));

    // 验证文件确实存在
    assert!(storage.exists(&metadata.storage_id).unwrap());

    // 读取回来
    let retrieved = storage.retrieve(&metadata.storage_id).unwrap();
    assert_eq!(retrieved, b"hello world");

    // 清理
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fs_delete_removes_file_and_retrieve_fails() {
    let dir = temp_storage_dir("fs-delete");
    let storage = LocalFileSystemStorage::new(LocalFileSystemStorageConfig {
        root_dir: dir.clone(),
    });

    let object = make_content_object("ci-2", b"to be deleted", "text/plain");
    let metadata = storage.store(&object).unwrap();
    let storage_id = metadata.storage_id.clone();

    assert!(storage.exists(&storage_id).unwrap());

    storage.delete(&storage_id).unwrap();

    assert!(!storage.exists(&storage_id).unwrap());

    let result = storage.retrieve(&storage_id);
    assert!(result.is_err());

    // 删除不存在的对象不应报错
    assert!(storage.delete(&storage_id).is_ok());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fs_exists_check() {
    let dir = temp_storage_dir("fs-exists");
    let storage = LocalFileSystemStorage::new(LocalFileSystemStorageConfig {
        root_dir: dir.clone(),
    });

    // 不存在的 ID
    assert!(!storage.exists("nonexistent-id").unwrap());

    let object = make_content_object("ci-3", b"exists test", "text/plain");
    let metadata = storage.store(&object).unwrap();

    assert!(storage.exists(&metadata.storage_id).unwrap());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fs_checksum_is_sha256_hex() {
    let dir = temp_storage_dir("fs-checksum");
    let storage = LocalFileSystemStorage::new(LocalFileSystemStorageConfig {
        root_dir: dir.clone(),
    });

    let object = make_content_object("ci-4", b"checksum test data", "application/octet-stream");
    let metadata = storage.store(&object).unwrap();

    let checksum = metadata.checksum.expect("checksum should be present");
    // SHA-256 hex 应为 64 个十六进制字符
    assert_eq!(checksum.len(), 64);
    assert!(checksum.chars().all(|c| c.is_ascii_hexdigit()));

    // 相同内容应产生相同 checksum
    let object2 = make_content_object("ci-4b", b"checksum test data", "application/octet-stream");
    let metadata2 = storage.store(&object2).unwrap();
    assert_eq!(metadata2.checksum.unwrap(), checksum);

    // 不同内容应产生不同 checksum
    let object3 = make_content_object("ci-4c", b"different data", "application/octet-stream");
    let metadata3 = storage.store(&object3).unwrap();
    assert_ne!(metadata3.checksum.unwrap(), checksum);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fs_directory_sharding() {
    let dir = temp_storage_dir("fs-shard");
    let storage = LocalFileSystemStorage::new(LocalFileSystemStorageConfig {
        root_dir: dir.clone(),
    });

    let object = make_content_object("ci-5", b"shard test", "text/plain");
    let metadata = storage.store(&object).unwrap();

    // storage_id 是 UUID，前两位应作为分片目录
    let shard = &metadata.storage_id[..2];
    let expected_path = dir.join(shard).join(&metadata.storage_id);
    assert!(
        expected_path.exists(),
        "Expected file at {:?}",
        expected_path
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fs_backend_name() {
    let dir = temp_storage_dir("fs-name");
    let storage = LocalFileSystemStorage::new(LocalFileSystemStorageConfig {
        root_dir: dir.clone(),
    });

    assert_eq!(storage.backend_name(), "local-fs");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// FlowableContentService 端到端测试（集成 storage provider）
// ---------------------------------------------------------------------------

#[test]
fn service_create_get_data_delete_with_storage() {
    let (service, storage_dir) = service_with_temp_storage("e2e-storage");

    let created = service
        .create_content_item(CreateContentItemRequest {
            name: "storage-test.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("storage-backed-content".to_string()),
            task_id: Some("task-e2e".to_string()),
            process_instance_id: Some("process-e2e".to_string()),
            scope_type: Some("task".to_string()),
            scope_id: Some("task-e2e".to_string()),
            created_by: Some("tester".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();

    // 验证 ContentItem 包含 storage 信息
    assert!(created.storage_id.is_some(), "should have storage_id");
    assert_eq!(
        created.storage_backend.as_deref(),
        Some("local-fs"),
        "should have storage_backend = local-fs"
    );
    assert_eq!(created.content_size, "storage-backed-content".len());

    // 获取元数据
    let metadata = service.get_content_item(&created.id).unwrap();
    assert_eq!(metadata.storage_id, created.storage_id);
    assert_eq!(metadata.storage_backend, created.storage_backend);

    // 获取数据
    let data = service.get_content_item_data(&created.id).unwrap();
    assert_eq!(data.content_item_id, created.id);
    assert_eq!(data.content, b"storage-backed-content");
    assert_eq!(data.mime_type.as_deref(), Some("text/plain"));

    // 删除
    service.delete_content_item(&created.id).unwrap();

    // 删除后元数据不可访问
    let meta_err = service.get_content_item(&created.id).unwrap_err();
    assert!(matches!(meta_err, FlowableError::NotFound(_)));

    // 删除后数据不可访问
    let data_err = service.get_content_item_data(&created.id).unwrap_err();
    assert!(matches!(data_err, FlowableError::NotFound(_)));

    let _ = fs::remove_dir_all(&storage_dir);
}

#[test]
fn service_create_without_content_has_no_storage_id() {
    let (service, storage_dir) = service_with_temp_storage("e2e-no-content");

    let created = service
        .create_content_item(CreateContentItemRequest {
            name: "empty.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: None,
            task_id: None,
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: None,
            expires_in_seconds: None,
        })
        .unwrap();

    // 无内容时不应有 storage_id
    assert!(created.storage_id.is_none());
    assert!(created.storage_backend.is_none());
    assert_eq!(created.content_size, 0);

    let _ = fs::remove_dir_all(&storage_dir);
}
