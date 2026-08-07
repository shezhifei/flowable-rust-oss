use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use std::sync::Arc;

pub struct UnlockOwnedJobsCmd {
    owner_id: Arc<str>,
    tenant_scope: OwnedJobsTenantScope,
}

enum OwnedJobsTenantScope {
    All,
    Included(Vec<String>),
}

impl UnlockOwnedJobsCmd {
    pub fn new(owner_id: Arc<str>) -> Self {
        Self {
            owner_id,
            tenant_scope: OwnedJobsTenantScope::All,
        }
    }

    pub fn with_tenant_ids(mut self, tenant_ids: Vec<String>) -> Self {
        self.tenant_scope = if tenant_ids.is_empty() {
            OwnedJobsTenantScope::All
        } else {
            OwnedJobsTenantScope::Included(tenant_ids)
        };
        self
    }
}

impl Command<usize> for UnlockOwnedJobsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<usize, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _deployment_manager = command_context.deployment_manager_handle();
        let session = command_context.session();
        let tenant_filter = match &self.tenant_scope {
            OwnedJobsTenantScope::All => None,
            OwnedJobsTenantScope::Included(tenant_ids) => Some(tenant_ids.as_slice()),
        };

        Ok(store.unlock_owned_executable_jobs(self.owner_id.as_ref(), tenant_filter, session)?)
    }
}
