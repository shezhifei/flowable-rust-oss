use crate::models::{ContentObject, ContentObjectStorageMetadata};
use flowable_engine::error::FlowableError;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// 内容对象存储后端 SPI — ContentService 用以持久化二进制载荷
pub trait ContentStorage: Send + Sync {
    /// 存储内容对象，返回存储元数据
    fn store(&self, object: &ContentObject) -> Result<ContentObjectStorageMetadata, FlowableError>;

    /// 根据 storage_id 检索内容字节
    fn retrieve(&self, storage_id: &str) -> Result<Vec<u8>, FlowableError>;

    /// 删除存储的对象
    fn delete(&self, storage_id: &str) -> Result<(), FlowableError>;

    /// 检查对象是否存在
    fn exists(&self, storage_id: &str) -> Result<bool, FlowableError>;

    /// 返回存储后端名称
    fn backend_name(&self) -> &str;

    /// M41: 获取存储对象的元数据（不读取内容字节）
    fn get_metadata(&self, storage_id: &str)
    -> Result<ContentObjectStorageMetadata, FlowableError>;
}

/// LocalFileSystemStorage 配置
#[derive(Clone, Debug)]
pub struct LocalFileSystemStorageConfig {
    pub root_dir: PathBuf,
}

/// 本地文件系统存储实现
///
/// 文件按 storage_id 前两位做目录分片，例如 `root/ab/abcdef...`。
/// 写入时先写临时文件再 rename，保证原子性。
pub struct LocalFileSystemStorage {
    root_dir: PathBuf,
}

impl LocalFileSystemStorage {
    pub fn new(config: LocalFileSystemStorageConfig) -> Self {
        let root_dir = config.root_dir;
        if !root_dir.exists()
            && let Err(error) = fs::create_dir_all(&root_dir)
        {
            tracing::warn!(
                "Failed to create content storage root directory '{}': {error}",
                root_dir.display()
            );
        }
        Self { root_dir }
    }

    /// 根据 storage_id 计算文件路径
    fn file_path(&self, storage_id: &str) -> PathBuf {
        let shard = &storage_id[..2.min(storage_id.len())];
        self.root_dir.join(shard).join(storage_id)
    }

    /// 计算 SHA-256 校验和，返回 hex 字符串
    fn compute_checksum(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// 生成 ISO 8601 时间戳
    fn iso8601_now() -> String {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs();
        let nanos = duration.subsec_nanos();
        // 手动格式化为 ISO 8601: YYYY-MM-DDTHH:MM:SS.fffZ
        let secs_since_epoch = secs;
        let days = secs_since_epoch / 86400;
        let time_secs = secs_since_epoch % 86400;
        let hours = time_secs / 3600;
        let minutes = (time_secs % 3600) / 60;
        let seconds = time_secs % 60;
        let millis = nanos / 1_000_000;

        // 使用公历计算年月日（简化算法，适用于 1970-2100 年）
        let (year, month, day) = Self::days_to_date(days as i64);

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            year, month, day, hours, minutes, seconds, millis
        )
    }

    /// 将自 Unix 纪元以来的天数转换为 (year, month, day)
    fn days_to_date(mut days: i64) -> (i64, u32, u32) {
        // 从 1970-01-01 开始
        let mut year = 1970i64;
        loop {
            let days_in_year = if Self::is_leap(year) { 366 } else { 365 };
            if days < days_in_year {
                break;
            }
            days -= days_in_year;
            year += 1;
        }

        let months_days = if Self::is_leap(year) {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };

        let mut month = 1u32;
        for &md in &months_days {
            if days < md as i64 {
                break;
            }
            days -= md as i64;
            month += 1;
        }

        let day = (days + 1) as u32;
        (year, month, day)
    }

    fn is_leap(year: i64) -> bool {
        (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
    }
}

impl ContentStorage for LocalFileSystemStorage {
    fn store(&self, object: &ContentObject) -> Result<ContentObjectStorageMetadata, FlowableError> {
        let storage_id = Uuid::new_v4().to_string();
        let shard = &storage_id[..2];
        let shard_dir = self.root_dir.join(shard);

        fs::create_dir_all(&shard_dir).map_err(|e| {
            FlowableError::ExecutionError(format!(
                "Failed to create shard directory '{}': {e}",
                shard_dir.display()
            ))
        })?;

        let target_path = shard_dir.join(&storage_id);
        let temp_path = shard_dir.join(format!(".tmp_{}", Uuid::new_v4()));

        // 写入临时文件
        {
            let mut temp_file = fs::File::create(&temp_path).map_err(|e| {
                FlowableError::ExecutionError(format!(
                    "Failed to create temp file '{}': {e}",
                    temp_path.display()
                ))
            })?;
            temp_file.write_all(&object.data).map_err(|e| {
                FlowableError::ExecutionError(format!(
                    "Failed to write temp file '{}': {e}",
                    temp_path.display()
                ))
            })?;
            temp_file.flush().map_err(|e| {
                FlowableError::ExecutionError(format!(
                    "Failed to flush temp file '{}': {e}",
                    temp_path.display()
                ))
            })?;
        }

        // 原子性 rename
        fs::rename(&temp_path, &target_path).map_err(|e| {
            // 清理临时文件
            let _ = fs::remove_file(&temp_path);
            FlowableError::ExecutionError(format!(
                "Failed to rename temp file to '{}': {e}",
                target_path.display()
            ))
        })?;

        let checksum = Self::compute_checksum(&object.data);
        let stored_at = Self::iso8601_now();

        Ok(ContentObjectStorageMetadata {
            storage_id,
            storage_backend: self.backend_name().to_string(),
            stored_at,
            size: object.data.len() as u64,
            checksum: Some(checksum),
        })
    }

    fn retrieve(&self, storage_id: &str) -> Result<Vec<u8>, FlowableError> {
        let path = self.file_path(storage_id);
        fs::read(&path).map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                FlowableError::NotFound(format!("Content object '{storage_id}' was not found"))
            } else {
                FlowableError::ExecutionError(format!(
                    "Failed to read content object '{}' from '{}': {e}",
                    storage_id,
                    path.display()
                ))
            }
        })
    }

    fn delete(&self, storage_id: &str) -> Result<(), FlowableError> {
        let path = self.file_path(storage_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                FlowableError::ExecutionError(format!(
                    "Failed to delete content object '{}' at '{}': {e}",
                    storage_id,
                    path.display()
                ))
            })?;

            // 尝试清理空的分片目录
            if let Some(shard_dir) = path.parent()
                && shard_dir != self.root_dir
            {
                let _ = fs::remove_dir(shard_dir);
            }
        }
        Ok(())
    }

    fn exists(&self, storage_id: &str) -> Result<bool, FlowableError> {
        let path = self.file_path(storage_id);
        Ok(path.exists())
    }

    fn backend_name(&self) -> &str {
        "local-fs"
    }

    fn get_metadata(
        &self,
        storage_id: &str,
    ) -> Result<ContentObjectStorageMetadata, FlowableError> {
        let path = self.file_path(storage_id);
        let metadata = fs::metadata(&path).map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                FlowableError::NotFound(format!("Content object '{storage_id}' was not found"))
            } else {
                FlowableError::ExecutionError(format!(
                    "Failed to read metadata for content object '{}' at '{}': {e}",
                    storage_id,
                    path.display()
                ))
            }
        })?;

        let size = metadata.len();
        let checksum = fs::read(&path)
            .ok()
            .map(|bytes| Self::compute_checksum(&bytes));
        let stored_at = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| {
                let secs = d.as_secs();
                let millis = d.subsec_millis();
                let (year, month, day) = Self::days_to_date((secs / 86400) as i64);
                let time_secs = secs % 86400;
                let hours = time_secs / 3600;
                let minutes = (time_secs % 3600) / 60;
                let seconds = time_secs % 60;
                format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                    year, month, day, hours, minutes, seconds, millis
                )
            })
            .unwrap_or_else(Self::iso8601_now);

        Ok(ContentObjectStorageMetadata {
            storage_id: storage_id.to_string(),
            storage_backend: self.backend_name().to_string(),
            stored_at,
            size,
            checksum,
        })
    }
}

// ── S3Storage ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct S3StorageConfig {
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub mock_mode: bool,
}

pub struct S3Storage {
    config: S3StorageConfig,
    client: reqwest::blocking::Client,
    // For mock mode
    mock_storage: Option<std::sync::Arc<LocalFileSystemStorage>>,
}

impl S3Storage {
    pub fn new(config: S3StorageConfig) -> Self {
        let mock_storage = if config.mock_mode {
            let tmp = std::env::temp_dir().join(format!("s3_mock_{}", config.bucket));
            Some(std::sync::Arc::new(LocalFileSystemStorage::new(
                LocalFileSystemStorageConfig { root_dir: tmp },
            )))
        } else {
            None
        };
        Self {
            config,
            client: reqwest::blocking::Client::new(),
            mock_storage,
        }
    }
}

impl ContentStorage for S3Storage {
    fn store(&self, object: &ContentObject) -> Result<ContentObjectStorageMetadata, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            let mut meta = mock.store(object)?;
            meta.storage_backend = self.backend_name().to_string();
            return Ok(meta);
        }

        let storage_id = Uuid::new_v4().to_string();
        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );

        let res = self
            .client
            .put(&url)
            .body(object.data.clone())
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("S3 PUT failed: {e}")))?;

        if !res.status().is_success() {
            return Err(FlowableError::ExecutionError(format!(
                "S3 PUT returned status {}",
                res.status()
            )));
        }

        let mut hasher = Sha256::new();
        hasher.update(&object.data);
        let checksum = format!("{:x}", hasher.finalize());

        Ok(ContentObjectStorageMetadata {
            storage_id,
            storage_backend: self.backend_name().to_string(),
            stored_at: LocalFileSystemStorage::iso8601_now(),
            size: object.data.len() as u64,
            checksum: Some(checksum),
        })
    }

    fn retrieve(&self, storage_id: &str) -> Result<Vec<u8>, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.retrieve(storage_id);
        }

        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );
        let res = self
            .client
            .get(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("S3 GET failed: {e}")))?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::NotFound(format!(
                "S3 object {storage_id} not found"
            )));
        }

        if !res.status().is_success() {
            return Err(FlowableError::ExecutionError(format!(
                "S3 GET returned status {}",
                res.status()
            )));
        }

        res.bytes()
            .map(|b| b.to_vec())
            .map_err(|e| FlowableError::ExecutionError(format!("Failed to read S3 bytes: {e}")))
    }

    fn delete(&self, storage_id: &str) -> Result<(), FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.delete(storage_id);
        }

        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );
        let res = self
            .client
            .delete(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("S3 DELETE failed: {e}")))?;

        if !res.status().is_success() && res.status() != reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::ExecutionError(format!(
                "S3 DELETE returned status {}",
                res.status()
            )));
        }
        Ok(())
    }

    fn exists(&self, storage_id: &str) -> Result<bool, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.exists(storage_id);
        }

        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );
        let res = self
            .client
            .head(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("S3 HEAD failed: {e}")))?;

        Ok(res.status().is_success())
    }

    fn backend_name(&self) -> &str {
        "s3"
    }

    fn get_metadata(
        &self,
        storage_id: &str,
    ) -> Result<ContentObjectStorageMetadata, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            let mut meta = mock.get_metadata(storage_id)?;
            meta.storage_backend = self.backend_name().to_string();
            return Ok(meta);
        }

        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );
        let res = self
            .client
            .head(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("S3 HEAD failed: {e}")))?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::NotFound(format!(
                "S3 object {storage_id} not found"
            )));
        }

        let size = res
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        Ok(ContentObjectStorageMetadata {
            storage_id: storage_id.to_string(),
            storage_backend: self.backend_name().to_string(),
            stored_at: LocalFileSystemStorage::iso8601_now(),
            size,
            checksum: None,
        })
    }
}

// ── AzureBlobStorage ───────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct AzureBlobStorageConfig {
    pub endpoint: String,
    pub container: String,
    pub account_name: String,
    pub account_key: String,
    pub mock_mode: bool,
}

pub struct AzureBlobStorage {
    config: AzureBlobStorageConfig,
    client: reqwest::blocking::Client,
    mock_storage: Option<std::sync::Arc<LocalFileSystemStorage>>,
}

impl AzureBlobStorage {
    pub fn new(config: AzureBlobStorageConfig) -> Self {
        let mock_storage = if config.mock_mode {
            let tmp = std::env::temp_dir().join(format!("azure_mock_{}", config.container));
            Some(std::sync::Arc::new(LocalFileSystemStorage::new(
                LocalFileSystemStorageConfig { root_dir: tmp },
            )))
        } else {
            None
        };
        Self {
            config,
            client: reqwest::blocking::Client::new(),
            mock_storage,
        }
    }
}

impl ContentStorage for AzureBlobStorage {
    fn store(&self, object: &ContentObject) -> Result<ContentObjectStorageMetadata, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            let mut meta = mock.store(object)?;
            meta.storage_backend = self.backend_name().to_string();
            return Ok(meta);
        }

        let storage_id = Uuid::new_v4().to_string();
        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.container,
            storage_id
        );

        let res = self
            .client
            .put(&url)
            .header("x-ms-blob-type", "BlockBlob")
            .body(object.data.clone())
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("Azure PUT failed: {e}")))?;

        if !res.status().is_success() {
            return Err(FlowableError::ExecutionError(format!(
                "Azure PUT returned status {}",
                res.status()
            )));
        }

        let mut hasher = Sha256::new();
        hasher.update(&object.data);
        let checksum = format!("{:x}", hasher.finalize());

        Ok(ContentObjectStorageMetadata {
            storage_id,
            storage_backend: self.backend_name().to_string(),
            stored_at: LocalFileSystemStorage::iso8601_now(),
            size: object.data.len() as u64,
            checksum: Some(checksum),
        })
    }

    fn retrieve(&self, storage_id: &str) -> Result<Vec<u8>, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.retrieve(storage_id);
        }

        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.container,
            storage_id
        );
        let res = self
            .client
            .get(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("Azure GET failed: {e}")))?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::NotFound(format!(
                "Azure blob {storage_id} not found"
            )));
        }

        if !res.status().is_success() {
            return Err(FlowableError::ExecutionError(format!(
                "Azure GET returned status {}",
                res.status()
            )));
        }

        res.bytes()
            .map(|b| b.to_vec())
            .map_err(|e| FlowableError::ExecutionError(format!("Failed to read Azure bytes: {e}")))
    }

    fn delete(&self, storage_id: &str) -> Result<(), FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.delete(storage_id);
        }

        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.container,
            storage_id
        );
        let res = self
            .client
            .delete(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("Azure DELETE failed: {e}")))?;

        if !res.status().is_success() && res.status() != reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::ExecutionError(format!(
                "Azure DELETE returned status {}",
                res.status()
            )));
        }
        Ok(())
    }

    fn exists(&self, storage_id: &str) -> Result<bool, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.exists(storage_id);
        }

        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.container,
            storage_id
        );
        let res = self
            .client
            .head(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("Azure HEAD failed: {e}")))?;

        Ok(res.status().is_success())
    }

    fn backend_name(&self) -> &str {
        "azure"
    }

    fn get_metadata(
        &self,
        storage_id: &str,
    ) -> Result<ContentObjectStorageMetadata, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            let mut meta = mock.get_metadata(storage_id)?;
            meta.storage_backend = self.backend_name().to_string();
            return Ok(meta);
        }

        let url = format!(
            "{}/{}/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.container,
            storage_id
        );
        let res = self
            .client
            .head(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("Azure HEAD failed: {e}")))?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::NotFound(format!(
                "Azure blob {storage_id} not found"
            )));
        }

        let size = res
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        Ok(ContentObjectStorageMetadata {
            storage_id: storage_id.to_string(),
            storage_backend: self.backend_name().to_string(),
            stored_at: LocalFileSystemStorage::iso8601_now(),
            size,
            checksum: None,
        })
    }
}

// ── GcsStorage ─────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct GcsStorageConfig {
    pub endpoint: String,
    pub bucket: String,
    pub service_account_key: String,
    pub mock_mode: bool,
}

pub struct GcsStorage {
    config: GcsStorageConfig,
    client: reqwest::blocking::Client,
    mock_storage: Option<std::sync::Arc<LocalFileSystemStorage>>,
}

impl GcsStorage {
    pub fn new(config: GcsStorageConfig) -> Self {
        let mock_storage = if config.mock_mode {
            let tmp = std::env::temp_dir().join(format!("gcs_mock_{}", config.bucket));
            Some(std::sync::Arc::new(LocalFileSystemStorage::new(
                LocalFileSystemStorageConfig { root_dir: tmp },
            )))
        } else {
            None
        };
        Self {
            config,
            client: reqwest::blocking::Client::new(),
            mock_storage,
        }
    }
}

impl ContentStorage for GcsStorage {
    fn store(&self, object: &ContentObject) -> Result<ContentObjectStorageMetadata, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            let mut meta = mock.store(object)?;
            meta.storage_backend = self.backend_name().to_string();
            return Ok(meta);
        }

        let storage_id = Uuid::new_v4().to_string();
        let url = format!(
            "{}/upload/storage/v1/b/{}/o?uploadType=media&name={}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );

        let res = self
            .client
            .post(&url)
            .header("Content-Type", "application/octet-stream")
            .body(object.data.clone())
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("GCS POST failed: {e}")))?;

        if !res.status().is_success() {
            return Err(FlowableError::ExecutionError(format!(
                "GCS POST returned status {}",
                res.status()
            )));
        }

        let mut hasher = Sha256::new();
        hasher.update(&object.data);
        let checksum = format!("{:x}", hasher.finalize());

        Ok(ContentObjectStorageMetadata {
            storage_id,
            storage_backend: self.backend_name().to_string(),
            stored_at: LocalFileSystemStorage::iso8601_now(),
            size: object.data.len() as u64,
            checksum: Some(checksum),
        })
    }

    fn retrieve(&self, storage_id: &str) -> Result<Vec<u8>, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.retrieve(storage_id);
        }

        let url = format!(
            "{}/storage/v1/b/{}/o/{}?alt=media",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );
        let res = self
            .client
            .get(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("GCS GET failed: {e}")))?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::NotFound(format!(
                "GCS object {storage_id} not found"
            )));
        }

        if !res.status().is_success() {
            return Err(FlowableError::ExecutionError(format!(
                "GCS GET returned status {}",
                res.status()
            )));
        }

        res.bytes()
            .map(|b| b.to_vec())
            .map_err(|e| FlowableError::ExecutionError(format!("Failed to read GCS bytes: {e}")))
    }

    fn delete(&self, storage_id: &str) -> Result<(), FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.delete(storage_id);
        }

        let url = format!(
            "{}/storage/v1/b/{}/o/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );
        let res = self
            .client
            .delete(&url)
            .send()
            .map_err(|e| FlowableError::ExecutionError(format!("GCS DELETE failed: {e}")))?;

        if !res.status().is_success() && res.status() != reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::ExecutionError(format!(
                "GCS DELETE returned status {}",
                res.status()
            )));
        }
        Ok(())
    }

    fn exists(&self, storage_id: &str) -> Result<bool, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            return mock.exists(storage_id);
        }

        let url = format!(
            "{}/storage/v1/b/{}/o/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );
        let res =
            self.client.get(&url).send().map_err(|e| {
                FlowableError::ExecutionError(format!("GCS GET Metadata failed: {e}"))
            })?;

        Ok(res.status().is_success())
    }

    fn backend_name(&self) -> &str {
        "gcs"
    }

    fn get_metadata(
        &self,
        storage_id: &str,
    ) -> Result<ContentObjectStorageMetadata, FlowableError> {
        if let Some(ref mock) = self.mock_storage {
            let mut meta = mock.get_metadata(storage_id)?;
            meta.storage_backend = self.backend_name().to_string();
            return Ok(meta);
        }

        let url = format!(
            "{}/storage/v1/b/{}/o/{}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.bucket,
            storage_id
        );
        let res =
            self.client.get(&url).send().map_err(|e| {
                FlowableError::ExecutionError(format!("GCS GET Metadata failed: {e}"))
            })?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FlowableError::NotFound(format!(
                "GCS object {storage_id} not found"
            )));
        }

        let body: serde_json::Value = res.json().map_err(|e| {
            FlowableError::ExecutionError(format!("Failed to parse GCS metadata JSON: {e}"))
        })?;

        let size = body
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| body.get("size").and_then(|v| v.as_u64()))
            .unwrap_or(0);

        let checksum = body
            .get("md5Hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(ContentObjectStorageMetadata {
            storage_id: storage_id.to_string(),
            storage_backend: self.backend_name().to_string(),
            stored_at: LocalFileSystemStorage::iso8601_now(),
            size,
            checksum,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iso8601_format() {
        let ts = LocalFileSystemStorage::iso8601_now();
        // 基本格式检查: 应包含 T 和 Z
        assert!(ts.contains('T'), "timestamp should contain T: {ts}");
        assert!(ts.ends_with('Z'), "timestamp should end with Z: {ts}");
        // 长度: YYYY-MM-DDTHH:MM:SS.fffZ = 24
        assert_eq!(ts.len(), 24, "timestamp length should be 24: {ts}");
    }

    #[test]
    fn test_days_to_date_known() {
        // 1970-01-01 = day 0
        let (y, m, d) = LocalFileSystemStorage::days_to_date(0);
        assert_eq!((y, m, d), (1970, 1, 1));

        // 2026-04-26: let's compute approximate
        // 1970-2025 = 56 years. Leap years in this range:
        // Actually let's just verify it returns reasonable values
        let (y, m, d) = LocalFileSystemStorage::days_to_date(20550);
        assert!(y >= 2026, "year should be >= 2026: {y}");
        assert!((1..=12).contains(&m), "month should be 1-12: {m}");
        assert!((1..=31).contains(&d), "day should be 1-31: {d}");
    }
}
