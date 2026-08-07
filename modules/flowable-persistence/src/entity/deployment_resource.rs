use crate::db_session::DbSession;
use crate::entity::byte_array::ByteArrayEntity;
use crate::error::PersistenceError;
use crate::statement::StatementId;
use crate::value::DbParams;

pub struct DeploymentResourceDataManager;

impl DeploymentResourceDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: ByteArrayEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.name.clone());
        params.push(entity.deployment_id.clone());
        params.push(entity.bytes.clone());

        session.insert(entity, StatementId::InsertDeploymentResource, params)
    }

    pub fn delete_by_deployment_id_and_name(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
        name: &str,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);
        params.push(name);

        session.execute(StatementId::DeleteDeploymentResource, params)?;
        Ok(())
    }

    pub fn delete_by_deployment_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);

        session.execute(StatementId::DeleteDeploymentResourcesByDeploymentId, params)?;
        Ok(())
    }

    pub fn find_by_deployment_id_and_name(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
        name: &str,
    ) -> Result<Option<ByteArrayEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);
        params.push(name);

        let row = session.select_one(StatementId::SelectDeploymentResourceById, params)?;
        match row {
            Some(row) => Ok(Some(ByteArrayEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_deployment_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
    ) -> Result<Vec<ByteArrayEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);

        let rows =
            session.select_list(StatementId::SelectDeploymentResourcesByDeploymentId, params)?;
        rows.iter().map(ByteArrayEntity::from_row).collect()
    }
}

impl Default for DeploymentResourceDataManager {
    fn default() -> Self {
        Self::new()
    }
}
