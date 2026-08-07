// P116: unified CMMN plan item instance storage.
//
// Java reference: `ACT_CMMN_RU_PLAN_ITEM_INST`
// (`flowable.h2.create.cmmn.sql:84-124`) — one row per runtime plan item
// instance (stage / human task / milestone / event listener / timer event
// listener). The Rust engine keeps the type-specific source tables
// (ACT_CMMN_STAGE_INSTANCE, ACT_CMMN_HUMAN_TASK) and mirrors stage / milestone /
// event listener instances here so the unified plan-item-instance query surface
// can read one table. Human-task rows are NOT mirrored: the human-task query
// stays backed by ACT_CMMN_HUMAN_TASK (see `CmmnHumanTaskQuery`).
//
// The DATA_ column follows the Rust storage pattern (full entity as JSON, key
// columns denormalised for filtering), mirroring the existing stage/task tables.
use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnPlanItemInstanceEntity {
    pub id: String,
    pub case_definition_id: String,
    pub case_instance_id: String,
    pub stage_instance_id: Option<String>,
    pub element_id: String,
    pub item_definition_id: String,
    pub item_definition_type: String,
    pub name: String,
    pub state: String,
    pub create_time: String,
    pub ended_time: Option<String>,
    pub occurred_time: Option<String>,
    pub assignee: Option<String>,
    pub tenant_id: Option<String>,
    pub data: String,
}

impl CmmnPlanItemInstanceEntity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        case_definition_id: String,
        case_instance_id: String,
        element_id: String,
        item_definition_id: String,
        item_definition_type: String,
        name: String,
        state: String,
        create_time: String,
        data: String,
    ) -> Self {
        Self {
            id,
            case_definition_id,
            case_instance_id,
            stage_instance_id: None,
            element_id,
            item_definition_id,
            item_definition_type,
            name,
            state,
            create_time,
            ended_time: None,
            occurred_time: None,
            assignee: None,
            tenant_id: None,
            data,
        }
    }

    pub fn set_stage_instance_id(&mut self, stage_instance_id: Option<String>) {
        self.stage_instance_id = stage_instance_id;
    }

    pub fn set_ended_time(&mut self, ended_time: Option<String>) {
        self.ended_time = ended_time;
    }

    pub fn set_occurred_time(&mut self, occurred_time: Option<String>) {
        self.occurred_time = occurred_time;
    }

    pub fn set_assignee(&mut self, assignee: Option<String>) {
        self.assignee = assignee;
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEF_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEF_ID_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            case_instance_id: row.get_text("CASE_INST_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INST_ID_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            stage_instance_id: row.get_text("STAGE_INST_ID_"),
            element_id: row.get_text("ELEMENT_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ELEMENT_ID_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            item_definition_id: row.get_text("ITEM_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ITEM_DEFINITION_ID_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            item_definition_type: row.get_text("ITEM_DEFINITION_TYPE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ITEM_DEFINITION_TYPE_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            name: row.get_text("NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing NAME_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            state: row.get_text("STATE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STATE_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            create_time: row.get_text("CREATE_TIME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CREATE_TIME_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
            ended_time: row.get_text("ENDED_TIME_"),
            occurred_time: row.get_text("OCCURRED_TIME_"),
            assignee: row.get_text("ASSIGNEE_"),
            tenant_id: row.get_text("TENANT_ID_"),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnPlanItemInstanceEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnPlanItemInstanceEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnPlanItemInstance
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnPlanItemInstanceDataManager;

impl CmmnPlanItemInstanceDataManager {
    pub fn new() -> Self {
        Self
    }

    /// Upsert (INSERT OR REPLACE on the primary key) — used for both create and
    /// state-change writes so the row tracks the source instance's lifecycle.
    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnPlanItemInstanceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.case_instance_id.clone());
        params.push(entity.stage_instance_id.clone());
        params.push(entity.element_id.clone());
        params.push(entity.item_definition_id.clone());
        params.push(entity.item_definition_type.clone());
        params.push(entity.name.clone());
        params.push(entity.state.clone());
        params.push(entity.create_time.clone());
        params.push(entity.ended_time.clone());
        params.push(entity.occurred_time.clone());
        params.push(entity.assignee.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.data.clone());

        session.insert(
            entity,
            StatementId::InsertCmmnPlanItemInstance,
            params,
        )
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnPlanItemInstanceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnPlanItemInstance, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnPlanItemInstanceEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnPlanItemInstanceById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnPlanItemInstanceEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_instance_id(
        &self,
        session: &mut DbSession,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnPlanItemInstanceEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_instance_id);

        let rows = session.select_list(
            StatementId::SelectCmmnPlanItemInstancesByCaseInstanceId,
            params,
        )?;
        rows.iter()
            .map(CmmnPlanItemInstanceEntity::from_row)
            .collect()
    }
}

impl Default for CmmnPlanItemInstanceDataManager {
    fn default() -> Self {
        Self::new()
    }
}
