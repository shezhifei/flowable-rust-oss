use crate::error::CmmnError;
use crate::models::{CmmnDeployment, CmmnDeploymentRequest, CmmnDeploymentResource, CmmnModel};
use crate::repository::CmmnRepositoryService;
use flowable_cmmn_converter::parse_cmmn_definitions;
use std::io::{Cursor, Read};
use zip::ZipArchive;

pub struct CmmnDeploymentBuilder {
    repository: CmmnRepositoryService,
    request: CmmnDeploymentRequest,
}

impl CmmnDeploymentBuilder {
    pub fn new(repository: CmmnRepositoryService) -> Self {
        Self {
            repository,
            request: CmmnDeploymentRequest::new(""),
        }
    }

    pub fn name(mut self, value: impl Into<String>) -> Self {
        self.request.name = Some(value.into());
        self
    }
    pub fn category(mut self, value: impl Into<String>) -> Self {
        self.request.category = Some(value.into());
        self
    }
    pub fn key(mut self, value: impl Into<String>) -> Self {
        self.request.key = Some(value.into());
        self
    }
    pub fn tenant_id(mut self, value: impl Into<String>) -> Self {
        self.request.tenant_id = Some(value.into());
        self
    }
    pub fn parent_deployment_id(mut self, value: impl Into<String>) -> Self {
        self.request.parent_deployment_id = Some(value.into());
        self
    }
    pub fn disable_schema_validation(mut self) -> Self {
        self.request.validate_schema = false;
        self
    }
    pub fn enable_duplicate_filtering(mut self) -> Self {
        self.request.enable_duplicate_filtering = true;
        self
    }

    pub fn add_string(
        self,
        name: impl Into<String>,
        xml: impl AsRef<str>,
    ) -> Result<Self, CmmnError> {
        self.add_bytes(name, xml.as_ref().as_bytes())
    }

    pub fn add_bytes(
        mut self,
        name: impl Into<String>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<Self, CmmnError> {
        let resource_name = name.into();
        let bytes = bytes.as_ref().to_vec();
        let model = if is_cmmn_resource(&resource_name) {
            let xml =
                std::str::from_utf8(&bytes).map_err(|e| CmmnError::validation(e.to_string()))?;
            parse_cmmn_definitions(xml)
                .map(CmmnModel::from)
                .map_err(|e| CmmnError::validation(e.to_string()))?
        } else {
            CmmnModel::new(Vec::new())
        };
        self.request.resources.push(CmmnDeploymentResource {
            resource_name,
            model,
            resource_bytes: bytes,
        });
        Ok(self)
    }

    pub fn add_zip(mut self, bytes: impl AsRef<[u8]>) -> Result<Self, CmmnError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_ref()))
            .map_err(|e| CmmnError::validation(e.to_string()))?;
        if archive.len() > 1_000 {
            return Err(CmmnError::validation("Deployment ZIP exceeds 1000 entries"));
        }
        let mut total_size = 0_u64;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|e| CmmnError::validation(e.to_string()))?;
            let name = entry.name().to_string();
            if name.starts_with('/') || name.contains("..") || name.contains('\\') {
                return Err(CmmnError::validation(format!(
                    "Unsafe deployment ZIP entry '{name}'"
                )));
            }
            total_size += entry.size();
            if total_size > 50 * 1024 * 1024 {
                return Err(CmmnError::validation(
                    "Deployment ZIP exceeds 50 MiB uncompressed",
                ));
            }
            if name.ends_with('/') {
                continue;
            }
            let mut contents = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut contents)
                .map_err(|e| CmmnError::validation(e.to_string()))?;
            self = self.add_bytes(name, contents)?;
        }
        Ok(self)
    }

    pub fn deploy(self) -> Result<CmmnDeployment, CmmnError> {
        self.repository.deploy(self.request)
    }
}

pub(crate) fn is_cmmn_resource(resource_name: &str) -> bool {
    let name = resource_name.to_ascii_lowercase();
    name.ends_with(".cmmn") || name.ends_with(".cmmn.xml")
}
