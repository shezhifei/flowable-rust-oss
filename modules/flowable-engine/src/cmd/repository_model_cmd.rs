use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::repository::model::{RepositoryModel, RepositoryModelBytes};

pub struct CreateRepositoryModelCmd {
    model: RepositoryModel,
}

impl CreateRepositoryModelCmd {
    pub fn new(mut model: RepositoryModel) -> Self {
        if model.id.is_empty() {
            model.id = uuid::Uuid::new_v4().to_string();
        }
        Self { model }
    }
}

impl Command<RepositoryModel> for CreateRepositoryModelCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RepositoryModel, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let now = store.time_source().now().timestamp_millis();
        let mut model = self.model.clone();
        model.create_time = now;
        model.last_update_time = now;
        dm.insert_repository_model(model.clone(), Vec::new(), Vec::new(), session);
        Ok(model)
    }
}

pub struct UpdateRepositoryModelCmd {
    model: RepositoryModel,
}

impl UpdateRepositoryModelCmd {
    pub fn new(model: RepositoryModel) -> Self {
        Self { model }
    }
}

impl Command<RepositoryModel> for UpdateRepositoryModelCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RepositoryModel, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let current = dm
            .get_repository_model(&self.model.id, session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Model '{}' was not found",
                    self.model.id
                ))
            })?;
        let mut model = self.model.clone();
        let now = store.time_source().now().timestamp_millis();
        model.last_update_time = now.max(current.last_update_time + 1);
        if dm.update_repository_model(model.clone(), session).is_none() {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Model '{}' was not found",
                model.id
            )));
        }
        Ok(model)
    }
}

pub struct DeleteRepositoryModelCmd {
    model_id: String,
}

impl DeleteRepositoryModelCmd {
    pub fn new(model_id: String) -> Self {
        Self { model_id }
    }
}

impl Command<()> for DeleteRepositoryModelCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let _store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        if dm.delete_repository_model(&self.model_id, session) {
            Ok(())
        } else {
            Err(crate::error::FlowableError::NotFound(format!(
                "Model '{}' was not found",
                self.model_id
            )))
        }
    }
}

pub struct GetRepositoryModelsCmd;

impl Command<Vec<RepositoryModel>> for GetRepositoryModelsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<RepositoryModel>, crate::error::FlowableError> {
        let _store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        Ok(dm.get_repository_models(session))
    }
}

pub struct GetRepositoryModelCmd {
    model_id: String,
}

impl GetRepositoryModelCmd {
    pub fn new(model_id: String) -> Self {
        Self { model_id }
    }
}

impl Command<RepositoryModel> for GetRepositoryModelCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RepositoryModel, crate::error::FlowableError> {
        let _store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        dm.get_repository_model(&self.model_id, session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Model '{}' was not found",
                    self.model_id
                ))
            })
    }
}

pub struct GetRepositoryModelSourceCmd {
    model_id: String,
}

impl GetRepositoryModelSourceCmd {
    pub fn new(model_id: String) -> Self {
        Self { model_id }
    }
}

impl Command<RepositoryModelBytes> for GetRepositoryModelSourceCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RepositoryModelBytes, crate::error::FlowableError> {
        let _store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        dm.get_repository_model_source(&self.model_id, session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Source for model '{}' was not found",
                    self.model_id
                ))
            })
    }
}

pub struct UpdateRepositoryModelSourceCmd {
    model_id: String,
    content_type: String,
    bytes: Vec<u8>,
}

impl UpdateRepositoryModelSourceCmd {
    pub fn new(model_id: String, content_type: String, bytes: Vec<u8>) -> Self {
        Self {
            model_id,
            content_type,
            bytes,
        }
    }
}

impl Command<()> for UpdateRepositoryModelSourceCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let mut model = dm
            .get_repository_model(&self.model_id, session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Model '{}' was not found",
                    self.model_id
                ))
            })?;
        let now = store.time_source().now().timestamp_millis();
        model.last_update_time = now.max(model.last_update_time + 1);
        model.source_content_type = self.content_type.clone();
        dm.update_repository_model_source(model, self.bytes.clone(), session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Model '{}' was not found",
                    self.model_id
                ))
            })
    }
}

pub struct GetRepositoryModelSourceExtraCmd {
    model_id: String,
}

impl GetRepositoryModelSourceExtraCmd {
    pub fn new(model_id: String) -> Self {
        Self { model_id }
    }
}

impl Command<RepositoryModelBytes> for GetRepositoryModelSourceExtraCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RepositoryModelBytes, crate::error::FlowableError> {
        let _store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        dm.get_repository_model_source_extra(&self.model_id, session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Source extra for model '{}' was not found",
                    self.model_id
                ))
            })
    }
}

pub struct UpdateRepositoryModelSourceExtraCmd {
    model_id: String,
    content_type: String,
    bytes: Vec<u8>,
}

impl UpdateRepositoryModelSourceExtraCmd {
    pub fn new(model_id: String, content_type: String, bytes: Vec<u8>) -> Self {
        Self {
            model_id,
            content_type,
            bytes,
        }
    }
}

impl Command<()> for UpdateRepositoryModelSourceExtraCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let mut model = dm
            .get_repository_model(&self.model_id, session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Model '{}' was not found",
                    self.model_id
                ))
            })?;
        let now = store.time_source().now().timestamp_millis();
        model.last_update_time = now.max(model.last_update_time + 1);
        model.source_extra_content_type = self.content_type.clone();
        dm.update_repository_model_source_extra(model, self.bytes.clone(), session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Model '{}' was not found",
                    self.model_id
                ))
            })
    }
}
