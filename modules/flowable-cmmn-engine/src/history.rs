use crate::error::CmmnError;
use crate::models::{
    CmmnCaseDefinition, CmmnCaseInstanceState, CmmnHistoricCaseInstance,
    CmmnHistoricHumanTaskInstance, CmmnHistoricMilestoneInstance, CmmnHumanTaskInstance,
    CmmnHumanTaskState, CmmnIdentityLink, CmmnMigrationDocument, CmmnPlanItemInstance,
    CmmnStageInstance, CmmnStageInstanceState, CmmnStageOverview, PagedResult,
};
use crate::repository::CmmnRepositoryService;
use crate::runtime::CmmnUserGroupResolver;
use crate::store::CmmnStore;
use chrono::{DateTime, Utc};
use flowable_persistence::db_session::DbSession;
use flowable_persistence::entity::cmmn_case_definition::CmmnCaseDefinitionDataManager;
use flowable_persistence::entity::cmmn_case_history::{
    CmmnCaseHistoryDataManager, CmmnCaseHistoryEntity,
};
use flowable_persistence::entity::cmmn_human_task_history::CmmnHumanTaskHistoryDataManager;
use flowable_persistence::entity::cmmn_milestone_history::CmmnMilestoneHistoryDataManager;
use flowable_persistence::entity::cmmn_stage_history::CmmnStageHistoryDataManager;
use flowable_persistence::statement::{RenderedStatement, StatementId};
use flowable_persistence::value::DbParams;

#[derive(Clone)]
pub struct CmmnHistoryService {
    store: CmmnStore,
    repository_service: CmmnRepositoryService,
}

impl CmmnHistoryService {
    pub(crate) fn new(store: CmmnStore, repository_service: CmmnRepositoryService) -> Self {
        Self {
            store,
            repository_service,
        }
    }

    pub fn create_historic_case_instance_query(&self) -> CmmnHistoricCaseInstanceQuery {
        CmmnHistoricCaseInstanceQuery::new(self.store.clone())
    }

    pub fn get_historic_case_instance(
        &self,
        case_instance_id: &str,
    ) -> Result<CmmnHistoricCaseInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        let manager = CmmnCaseHistoryDataManager::new();
        let entity = manager
            .find_by_id(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "Historic CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        serde_json::from_str(&entity.data).map_err(Into::into)
    }

    pub fn delete_historic_case_instance(&self, case_instance_id: &str) -> Result<(), CmmnError> {
        self.bulk_delete_historic_case_instances(&[case_instance_id.to_string()])
    }

    pub fn bulk_delete_historic_case_instances(
        &self,
        case_instance_ids: &[String],
    ) -> Result<(), CmmnError> {
        if case_instance_ids.is_empty() {
            return Err(CmmnError::execution("historic case instanceIds are empty"));
        }

        let mut session = self.store.create_session()?;

        let mut unique_case_instance_ids = Vec::new();
        for case_instance_id in case_instance_ids {
            if !unique_case_instance_ids.contains(case_instance_id) {
                unique_case_instance_ids.push(case_instance_id.clone());
            }
        }

        for case_instance_id in &unique_case_instance_ids {
            ensure_historic_case_instance_exists_tx(&mut session, case_instance_id)?;
        }

        for case_instance_id in &unique_case_instance_ids {
            delete_historic_case_instance_tx(&mut session, case_instance_id)?;
        }

        session.commit()?;
        Ok(())
    }

    pub fn create_historic_human_task_query(&self) -> CmmnHistoricHumanTaskQuery {
        CmmnHistoricHumanTaskQuery::new(self.store.clone())
    }

    pub fn get_historic_human_task(
        &self,
        task_id: &str,
    ) -> Result<CmmnHistoricHumanTaskInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        let manager = CmmnHumanTaskHistoryDataManager::new();
        let entity = manager.find_by_id(&mut session, task_id)?.ok_or_else(|| {
            CmmnError::not_found(format!(
                "Historic CMMN human task '{task_id}' was not found"
            ))
        })?;
        serde_json::from_str(&entity.data).map_err(Into::into)
    }

    pub fn get_stage_overview(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnStageOverview>, CmmnError> {
        self.get_historic_case_instance(case_instance_id)?;
        list_stage_overview(&self.store, case_instance_id)
    }

    pub fn migrate_historic_case_instance(
        &self,
        case_instance_id: &str,
        document: CmmnMigrationDocument,
    ) -> Result<(), CmmnError> {
        let target_definition = self
            .repository_service
            .get_case_definition(&document.target_case_definition_id)?;
        let mut session = self.store.create_session()?;
        let mut case_instance = load_historic_case_instance_tx(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "Historic CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        if case_instance.case_definition_id != target_definition.id {
            apply_case_definition_to_historic_case(&mut case_instance, &target_definition);
            persist_historic_case_tx(&mut session, &case_instance)?;
        }
        session.commit()?;
        Ok(())
    }

    pub fn migrate_historic_case_instances_of_case_definition(
        &self,
        case_definition_id: &str,
        document: CmmnMigrationDocument,
    ) -> Result<(), CmmnError> {
        self.repository_service
            .get_case_definition(case_definition_id)?;
        self.repository_service
            .get_case_definition(&document.target_case_definition_id)?;

        let historic_cases = self
            .create_historic_case_instance_query()
            .case_definition_id(case_definition_id)
            .list()?;
        for historic_case in historic_cases {
            self.migrate_historic_case_instance(&historic_case.case_instance_id, document.clone())?;
        }
        Ok(())
    }

    pub fn create_historic_milestone_query(&self) -> CmmnHistoricMilestoneQuery {
        CmmnHistoricMilestoneQuery::new(self.store.clone())
    }

    pub fn get_historic_milestone(
        &self,
        milestone_instance_id: &str,
    ) -> Result<CmmnHistoricMilestoneInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        let manager = CmmnMilestoneHistoryDataManager::new();
        let entity = manager
            .find_by_id(&mut session, milestone_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "Historic CMMN milestone instance '{milestone_instance_id}' was not found"
                ))
            })?;
        serde_json::from_str(&entity.data).map_err(Into::into)
    }
}

fn ensure_historic_case_instance_exists_tx(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<(), CmmnError> {
    load_historic_case_instance_tx(session, case_instance_id)?
        .map(|_| ())
        .ok_or_else(|| {
            CmmnError::not_found(format!(
                "Historic CMMN case instance '{case_instance_id}' was not found"
            ))
        })
}

fn load_historic_case_instance_tx(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<Option<CmmnHistoricCaseInstance>, CmmnError> {
    let manager = CmmnCaseHistoryDataManager::new();
    match manager.find_by_id(session, case_instance_id)? {
        Some(entity) => Ok(Some(serde_json::from_str(&entity.data)?)),
        None => Ok(None),
    }
}

fn apply_case_definition_to_historic_case(
    historic_case: &mut CmmnHistoricCaseInstance,
    target_definition: &CmmnCaseDefinition,
) {
    historic_case.case_definition_id = target_definition.id.clone();
    historic_case.deployment_id = target_definition.deployment_id.clone();
    historic_case.case_definition_key = target_definition.key.clone();
    historic_case.case_definition_name = target_definition.name.clone();
    historic_case.case_definition_version = target_definition.version;
    historic_case.tenant_id = historic_case
        .tenant_id
        .clone()
        .or_else(|| target_definition.tenant_id.clone());
}

pub(crate) fn persist_historic_case_tx(
    session: &mut DbSession,
    historic_case: &CmmnHistoricCaseInstance,
) -> Result<(), CmmnError> {
    let mut entity = CmmnCaseHistoryEntity::new(
        historic_case.case_instance_id.clone(),
        historic_case.case_definition_id.clone(),
        historic_case.case_definition_key.clone(),
        historic_case.state.as_str().to_string(),
        historic_case.started_at.to_rfc3339(),
        serde_json::to_string(historic_case)?,
    );
    entity.set_tenant_id(historic_case.tenant_id.clone());
    entity.set_business_key(historic_case.business_key.clone());
    entity.set_completed_at(historic_case.completed_at.map(|value| value.to_rfc3339()));
    CmmnCaseHistoryDataManager::new().insert(session, entity)?;
    Ok(())
}

fn delete_historic_case_instance_tx(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<(), CmmnError> {
    let task_manager = CmmnHumanTaskHistoryDataManager::new();
    let tasks = task_manager.find_by_case_instance_id(session, case_instance_id)?;

    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute(StatementId::DeleteCmmnIdentityLinksByCaseInstanceId, params)?;

    for task in tasks {
        let mut params = DbParams::new();
        params.push(task.task_id);
        session.execute(StatementId::DeleteCmmnIdentityLinksByTaskId, params)?;
    }

    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute(
        StatementId::DeleteCmmnMilestoneHistoryByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(
        StatementId::DeleteCmmnStageHistoryByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(
        StatementId::DeleteCmmnHumanTaskHistoryByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(StatementId::DeleteCmmnCaseHistory, params)?;
    Ok(())
}

fn list_stage_overview(
    store: &CmmnStore,
    case_instance_id: &str,
) -> Result<Vec<CmmnStageOverview>, CmmnError> {
    let mut session = store.create_session()?;
    let manager = CmmnStageHistoryDataManager::new();
    let entities = manager.find_by_case_instance_id(&mut session, case_instance_id)?;
    entities
        .into_iter()
        .map(|entity| serde_json::from_str::<CmmnStageInstance>(&entity.data).map_err(Into::into))
        .map(|stage| stage.map(stage_overview_from_stage_instance))
        .collect()
}

fn stage_overview_from_stage_instance(stage: CmmnStageInstance) -> CmmnStageOverview {
    let current = stage.state == CmmnStageInstanceState::Active && stage.ended_at.is_none();
    let ended = stage.ended_at.is_some()
        || matches!(
            stage.state,
            CmmnStageInstanceState::Completed | CmmnStageInstanceState::Terminated
        );

    CmmnStageOverview {
        id: stage.stage_definition_id,
        name: stage.name,
        current,
        ended,
        end_time: stage.ended_at,
    }
}

/// Java `HistoricCaseInstanceQueryImpl` (cmmn-engine) rendered by
/// `HistoricCaseInstance.xml`. The Rust store keeps each historic case as a JSON
/// `DATA_` blob, so the SQL predicates are evaluated in memory over the decoded
/// rows; each filter below mirrors one `<if>` block of that mapper.
///
/// P120: extends the previous 5-filter surface to the high-frequency parameter
/// set that `HistoricCaseInstanceCollectionResource` exposes.
#[derive(Default)]
pub struct CmmnHistoricCaseInstanceQuery {
    store: Option<CmmnStore>,
    case_instance_id: Option<String>,
    case_instance_ids: Option<Vec<String>>,
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    case_definition_key_like: Option<String>,
    case_definition_key_like_ignore_case: Option<String>,
    case_definition_category: Option<String>,
    case_definition_category_like: Option<String>,
    case_definition_category_like_ignore_case: Option<String>,
    case_definition_name: Option<String>,
    case_definition_name_like: Option<String>,
    case_definition_name_like_ignore_case: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    business_key: Option<String>,
    business_key_like: Option<String>,
    business_key_like_ignore_case: Option<String>,
    business_status: Option<String>,
    business_status_like: Option<String>,
    business_status_like_ignore_case: Option<String>,
    started_by: Option<String>,
    /// Java exact historic predicates (`HistoricCaseInstance.xml:789-793`).
    reference_id: Option<String>,
    reference_type: Option<String>,
    started_before: Option<DateTime<Utc>>,
    started_after: Option<DateTime<Utc>>,
    finished_before: Option<DateTime<Utc>>,
    finished_after: Option<DateTime<Utc>>,
    /// Java `finishedBy` exact `END_USER_ID_` predicate
    /// (`HistoricCaseInstance.xml:845-846`).
    finished_by: Option<String>,
    /// Java `finished()` / `unfinished()` — `END_TIME_ is (not) null`
    /// (HistoricCaseInstance.xml:734-739).
    finished: Option<bool>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    tenant_id_like_ignore_case: Option<String>,
    without_tenant_id: bool,
    callback_id: Option<String>,
    callback_ids: Option<Vec<String>>,
    callback_type: Option<String>,
    without_callback_id: bool,
    involved_user: Option<String>,
    active_plan_item_definition_id: Option<String>,
    state: Option<CmmnCaseInstanceState>,
    start: usize,
    size: Option<usize>,
}

impl CmmnHistoricCaseInstanceQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store: Some(store),
            ..Default::default()
        }
    }

    /// Java `caseDefinitionCategory` — `CASE_DEF_CATEGORY_` on the joined case
    /// definition (HistoricCaseInstance.xml:359-381).
    pub fn case_definition_category(mut self, category: impl Into<String>) -> Self {
        self.case_definition_category = Some(category.into());
        self
    }

    /// Java `caseDefinitionCategoryLike` (HistoricCaseInstance.xml:359-381).
    pub fn case_definition_category_like(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_category_like = Some(pattern.into());
        self
    }

    /// Java `caseDefinitionCategoryLikeIgnoreCase` (HistoricCaseInstance.xml:359-381).
    pub fn case_definition_category_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_category_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn case_instance_id(mut self, case_instance_id: impl Into<String>) -> Self {
        self.case_instance_id = Some(case_instance_id.into());
        self
    }

    /// Java `caseInstanceIds` (HistoricCaseInstanceBaseResource.java:74-76).
    pub fn case_instance_ids(mut self, case_instance_ids: Vec<String>) -> Self {
        self.case_instance_ids = Some(case_instance_ids);
        self
    }

    pub fn case_definition_id(mut self, case_definition_id: impl Into<String>) -> Self {
        self.case_definition_id = Some(case_definition_id.into());
        self
    }

    pub fn case_definition_key(mut self, case_definition_key: impl Into<String>) -> Self {
        self.case_definition_key = Some(case_definition_key.into());
        self
    }

    /// Java `caseDefinitionKeyLike` (HistoricCaseInstance.xml:362-364).
    pub fn case_definition_key_like(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_key_like = Some(pattern.into());
        self
    }

    /// Java `caseDefinitionKeyLikeIgnoreCase` (HistoricCaseInstance.xml:365-367).
    pub fn case_definition_key_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_key_like_ignore_case = Some(pattern.into());
        self
    }

    /// Java `caseDefinitionName` (HistoricCaseInstanceBaseResource.java:104-106).
    pub fn case_definition_name(mut self, case_definition_name: impl Into<String>) -> Self {
        self.case_definition_name = Some(case_definition_name.into());
        self
    }

    /// Java `caseDefinitionNameLike` (HistoricCaseInstanceBaseResource.java:107-109).
    pub fn case_definition_name_like(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_name_like = Some(pattern.into());
        self
    }

    /// Java `caseDefinitionNameLikeIgnoreCase`
    /// (HistoricCaseInstanceCollectionResource.java:152-154).
    pub fn case_definition_name_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_name_like_ignore_case = Some(pattern.into());
        self
    }

    /// Java `caseInstanceName` (HistoricCaseInstance.xml:694-696).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Java `caseInstanceNameLike` (HistoricCaseInstance.xml:697-699).
    pub fn name_like(mut self, pattern: impl Into<String>) -> Self {
        self.name_like = Some(pattern.into());
        self
    }

    /// Java `caseInstanceNameLikeIgnoreCase` (HistoricCaseInstance.xml:700-702).
    pub fn name_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.name_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn business_key(mut self, business_key: impl Into<String>) -> Self {
        self.business_key = Some(business_key.into());
        self
    }

    /// Java `caseInstanceBusinessKeyLike` (HistoricCaseInstance.xml:719-721).
    pub fn business_key_like(mut self, pattern: impl Into<String>) -> Self {
        self.business_key_like = Some(pattern.into());
        self
    }

    /// Java `caseInstanceBusinessKeyLikeIgnoreCase` (HistoricCaseInstance.xml:722-724).
    pub fn business_key_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.business_key_like_ignore_case = Some(pattern.into());
        self
    }

    /// Java `caseInstanceBusinessStatus` (HistoricCaseInstance.xml:725-727).
    pub fn business_status(mut self, business_status: impl Into<String>) -> Self {
        self.business_status = Some(business_status.into());
        self
    }

    /// Java `caseInstanceBusinessStatusLike` (HistoricCaseInstance.xml:728-730).
    pub fn business_status_like(mut self, pattern: impl Into<String>) -> Self {
        self.business_status_like = Some(pattern.into());
        self
    }

    /// Java `caseInstanceBusinessStatusLikeIgnoreCase`
    /// (HistoricCaseInstanceCollectionResource.java:188-190).
    pub fn business_status_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.business_status_like_ignore_case = Some(pattern.into());
        self
    }

    /// Java `startedBy` — `START_USER_ID_` (HistoricCaseInstance.xml:752-754).
    pub fn started_by(mut self, started_by: impl Into<String>) -> Self {
        self.started_by = Some(started_by.into());
        self
    }

    /// Java `HistoricCaseInstanceQueryImpl.caseInstanceReferenceId`
    /// (`HistoricCaseInstanceQueryImpl.java:761-771`).
    pub fn reference_id(mut self, reference_id: impl Into<String>) -> Self {
        self.reference_id = Some(reference_id.into());
        self
    }

    /// Java `HistoricCaseInstanceQueryImpl.caseInstanceReferenceType`
    /// (`HistoricCaseInstanceQueryImpl.java:773-784`).
    pub fn reference_type(mut self, reference_type: impl Into<String>) -> Self {
        self.reference_type = Some(reference_type.into());
        self
    }

    /// Java `startedBefore` — `START_TIME_ <=` (HistoricCaseInstance.xml:740-742).
    pub fn started_before(mut self, started_before: DateTime<Utc>) -> Self {
        self.started_before = Some(started_before);
        self
    }

    /// Java `startedAfter` — `START_TIME_ >=` (HistoricCaseInstance.xml:743-745).
    pub fn started_after(mut self, started_after: DateTime<Utc>) -> Self {
        self.started_after = Some(started_after);
        self
    }

    /// Java `finishedBefore` — `END_TIME_ <=` (HistoricCaseInstance.xml:746-748).
    pub fn finished_before(mut self, finished_before: DateTime<Utc>) -> Self {
        self.finished_before = Some(finished_before);
        self
    }

    /// Java `finishedAfter` — `END_TIME_ >=` (HistoricCaseInstance.xml:749-751).
    pub fn finished_after(mut self, finished_after: DateTime<Utc>) -> Self {
        self.finished_after = Some(finished_after);
        self
    }

    /// Java `HistoricCaseInstanceQueryImpl.finishedBy`
    /// (`HistoricCaseInstanceQueryImpl.java:630-640`).
    pub fn finished_by(mut self, finished_by: impl Into<String>) -> Self {
        self.finished_by = Some(finished_by.into());
        self
    }

    /// Java `finished()` / `unfinished()` (HistoricCaseInstance.xml:734-739).
    pub fn finished(mut self, finished: bool) -> Self {
        self.finished = Some(finished);
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Java `caseInstanceTenantIdLike` (HistoricCaseInstanceBaseResource.java:227-229).
    pub fn tenant_id_like(mut self, pattern: impl Into<String>) -> Self {
        self.tenant_id_like = Some(pattern.into());
        self
    }

    /// Java `caseInstanceTenantIdLikeIgnoreCase`
    /// (HistoricCaseInstanceCollectionResource.java:288-290).
    pub fn tenant_id_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.tenant_id_like_ignore_case = Some(pattern.into());
        self
    }

    /// Java `caseInstanceWithoutTenantId` (HistoricCaseInstanceBaseResource.java:234-236).
    pub fn without_tenant_id(mut self) -> Self {
        self.without_tenant_id = true;
        self
    }

    /// Java `caseInstanceCallbackId` — `CALLBACK_ID_ = ?`
    /// (`HistoricCaseInstance.xml:767-769`).
    pub fn callback_id(mut self, callback_id: impl Into<String>) -> Self {
        self.callback_id = Some(callback_id.into());
        self
    }

    /// Java `caseInstanceCallbackIds` — `CALLBACK_ID_ in (...)`
    /// (`HistoricCaseInstance.xml:770-775`).
    pub fn callback_ids(mut self, callback_ids: Vec<String>) -> Self {
        self.callback_ids = Some(callback_ids);
        self
    }

    /// Java `caseInstanceCallbackType` — `CALLBACK_TYPE_ = ?`
    /// (`HistoricCaseInstance.xml:776-778`).
    pub fn callback_type(mut self, callback_type: impl Into<String>) -> Self {
        self.callback_type = Some(callback_type.into());
        self
    }

    /// Java `withoutCaseInstanceCallbackId` — `CALLBACK_ID_ is null`
    /// (`HistoricCaseInstance.xml:786-788`).
    pub fn without_callback_id(mut self) -> Self {
        self.without_callback_id = true;
        self
    }

    /// Java `involvedUser` checks a CMMN case-scoped historic identity link
    /// (`HistoricCaseInstance.xml:817-819`). Rust retains the same logical link
    /// in `ACT_CMMN_IDENTITY_LINK` for runtime and historic reads.
    pub fn involved_user(mut self, involved_user: impl Into<String>) -> Self {
        self.involved_user = Some(involved_user.into());
        self
    }

    /// Java checks for a plan item whose historic state is `active`
    /// (`HistoricCaseInstance.xml:807-809`). Active Rust plan items are held by
    /// the runtime unified plan-item mirror plus the human-task table.
    pub fn active_plan_item_definition_id(
        mut self,
        plan_item_definition_id: impl Into<String>,
    ) -> Self {
        self.active_plan_item_definition_id = Some(plan_item_definition_id.into());
        self
    }

    pub fn state(mut self, state: CmmnCaseInstanceState) -> Self {
        self.state = Some(state);
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnHistoricCaseInstance>, CmmnError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| CmmnError::storage("Historic case instance query has no store"))?;
        let mut session = store.create_session()?;
        let sql = String::from(
            "SELECT DATA_ FROM ACT_CMMN_CASE_HISTORY ORDER BY STARTED_AT_ ASC, CASE_INSTANCE_ID_ ASC",
        );
        let rendered = RenderedStatement::new(sql, DbParams::new());
        let rows = session.select_raw(rendered)?;
        let mut items: Vec<CmmnHistoricCaseInstance> = rows
            .into_iter()
            .map(|row| {
                let data = row.get_text("DATA_").ok_or_else(|| {
                    CmmnError::storage("Missing DATA_ in historic case instance query result")
                })?;
                serde_json::from_str(&data).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        items.retain(|item| matches_optional(&self.case_instance_id, &item.case_instance_id));
        // Java `caseInstanceIds` renders as `ID_ in (...)`
        // (HistoricCaseInstance.xml:688-693).
        items.retain(|item| {
            self.case_instance_ids
                .as_ref()
                .is_none_or(|ids| ids.iter().any(|id| id == &item.case_instance_id))
        });
        items.retain(|item| matches_optional(&self.case_definition_id, &item.case_definition_id));
        items.retain(|item| matches_optional(&self.case_definition_key, &item.case_definition_key));
        items.retain(|item| {
            matches_like_optional(&self.case_definition_key_like, &item.case_definition_key)
        });
        items.retain(|item| {
            matches_like_ignore_case_optional(
                &self.case_definition_key_like_ignore_case,
                &item.case_definition_key,
            )
        });
        items
            .retain(|item| matches_optional(&self.case_definition_name, &item.case_definition_name));
        items.retain(|item| {
            matches_like_optional(&self.case_definition_name_like, &item.case_definition_name)
        });
        items.retain(|item| {
            matches_like_ignore_case_optional(
                &self.case_definition_name_like_ignore_case,
                &item.case_definition_name,
            )
        });
        self.retain_case_definition_category_matches(&mut session, &mut items)?;
        items.retain(|item| matches_optional(&self.name, &item.name));
        items.retain(|item| matches_like_optional(&self.name_like, &item.name));
        items.retain(|item| {
            matches_like_ignore_case_optional(&self.name_like_ignore_case, &item.name)
        });
        items.retain(|item| {
            matches_optional_option(&self.business_key, item.business_key.as_deref())
        });
        items.retain(|item| {
            matches_like_optional_option(&self.business_key_like, item.business_key.as_deref())
        });
        items.retain(|item| {
            matches_like_ignore_case_optional_option(
                &self.business_key_like_ignore_case,
                item.business_key.as_deref(),
            )
        });
        items.retain(|item| {
            matches_optional_option(&self.business_status, item.business_status.as_deref())
        });
        items.retain(|item| {
            matches_like_optional_option(&self.business_status_like, item.business_status.as_deref())
        });
        items.retain(|item| {
            matches_like_ignore_case_optional_option(
                &self.business_status_like_ignore_case,
                item.business_status.as_deref(),
            )
        });
        items.retain(|item| matches_optional_option(&self.started_by, item.started_by.as_deref()));
        // Java `HistoricCaseInstance.xml:789-793` uses exact equality for both fields.
        items.retain(|item| {
            matches_optional_option(&self.reference_id, item.reference_id.as_deref())
        });
        items.retain(|item| {
            matches_optional_option(&self.reference_type, item.reference_type.as_deref())
        });
        // Java renders startedBefore/After as inclusive bounds on START_TIME_
        // (HistoricCaseInstance.xml:740-745).
        items.retain(|item| {
            self.started_before
                .is_none_or(|bound| item.started_at <= bound)
        });
        items.retain(|item| self.started_after.is_none_or(|bound| item.started_at >= bound));
        // Java finishedBefore/After compare END_TIME_, so unfinished cases (null
        // END_TIME_) never satisfy either bound (HistoricCaseInstance.xml:746-751).
        items.retain(|item| {
            self.finished_before
                .is_none_or(|bound| item.completed_at.is_some_and(|value| value <= bound))
        });
        items.retain(|item| {
            self.finished_after
                .is_none_or(|bound| item.completed_at.is_some_and(|value| value >= bound))
        });
        // Java mapper uses exact equality on END_USER_ID_
        // (`HistoricCaseInstance.xml:845-846`).
        items.retain(|item| {
            matches_optional_option(&self.finished_by, item.finished_by.as_deref())
        });
        items.retain(|item| {
            self.finished
                .is_none_or(|finished| item.completed_at.is_some() == finished)
        });
        items.retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        items.retain(|item| {
            matches_like_optional_option(&self.tenant_id_like, item.tenant_id.as_deref())
        });
        items.retain(|item| {
            matches_like_ignore_case_optional_option(
                &self.tenant_id_like_ignore_case,
                item.tenant_id.as_deref(),
            )
        });
        // Java `caseInstanceWithoutTenantId` renders `TENANT_ID_ is null or = ''`
        // (HistoricCaseInstance.xml withoutTenantId block).
        items.retain(|item| {
            !self.without_tenant_id
                || item
                    .tenant_id
                    .as_deref()
                    .is_none_or(|tenant_id| tenant_id.is_empty())
        });
        items.retain(|item| {
            matches_optional_option(&self.callback_id, item.callback_id.as_deref())
        });
        items.retain(|item| {
            self.callback_ids.as_ref().is_none_or(|callback_ids| {
                item.callback_id
                    .as_ref()
                    .is_some_and(|callback_id| callback_ids.contains(callback_id))
            })
        });
        items.retain(|item| {
            matches_optional_option(&self.callback_type, item.callback_type.as_deref())
        });
        items.retain(|item| !self.without_callback_id || item.callback_id.is_none());
        self.retain_involved_user_matches(&mut session, &mut items)?;
        self.retain_active_plan_item_definition_matches(&mut session, &mut items)?;
        items.retain(|item| self.state.as_ref().is_none_or(|value| item.state == *value));

        Ok(items)
    }

    /// Java joins `ACT_CMMN_CASEDEF` and filters on `CASE_DEF_CATEGORY_`
    /// (HistoricCaseInstance.xml:359-381). The Rust historic case row keeps no
    /// category, so the definition's `CATEGORY_` is read per distinct case
    /// definition id — on the session already open for this query, since the
    /// store serialises sessions and a nested one would deadlock.
    fn retain_case_definition_category_matches(
        &self,
        session: &mut DbSession,
        items: &mut Vec<CmmnHistoricCaseInstance>,
    ) -> Result<(), CmmnError> {
        if self.case_definition_category.is_none()
            && self.case_definition_category_like.is_none()
            && self.case_definition_category_like_ignore_case.is_none()
        {
            return Ok(());
        }

        let manager = CmmnCaseDefinitionDataManager::new();
        let mut categories: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        for item in items.iter() {
            if categories.contains_key(&item.case_definition_id) {
                continue;
            }
            // A definition removed by a cascade delete leaves the historic case
            // behind; Java's inner join drops such rows, so a missing definition
            // yields `None` and fails every category predicate below.
            let category = manager
                .find_by_id(session, &item.case_definition_id)?
                .and_then(|entity| entity.category);
            categories.insert(item.case_definition_id.clone(), category);
        }

        items.retain(|item| {
            let category = categories
                .get(&item.case_definition_id)
                .and_then(Option::as_deref);
            let Some(category) = category else {
                return false;
            };
            self.case_definition_category
                .as_deref()
                .is_none_or(|expected| expected == category)
                && self
                    .case_definition_category_like
                    .as_deref()
                    .is_none_or(|pattern| sql_like_matches(category, pattern))
                && self
                    .case_definition_category_like_ignore_case
                    .as_deref()
                    .is_none_or(|pattern| {
                        sql_like_matches(&category.to_lowercase(), &pattern.to_lowercase())
                    })
        });
        Ok(())
    }

    fn retain_involved_user_matches(
        &self,
        session: &mut DbSession,
        items: &mut Vec<CmmnHistoricCaseInstance>,
    ) -> Result<(), CmmnError> {
        let Some(involved_user) = self.involved_user.as_deref() else {
            return Ok(());
        };

        let rows = session.select_raw(RenderedStatement::new(
            "SELECT DATA_ FROM ACT_CMMN_IDENTITY_LINK".to_string(),
            DbParams::new(),
        ))?;
        let matching_case_ids: std::collections::HashSet<String> = rows
            .into_iter()
            .filter_map(|row| row.get_text("DATA_"))
            .filter_map(|json| serde_json::from_str::<CmmnIdentityLink>(&json).ok())
            .filter(|link| {
                link.scope_type == "caseInstance"
                    && link.user_id.as_deref() == Some(involved_user)
            })
            .map(|link| link.scope_id)
            .collect();
        items.retain(|item| matching_case_ids.contains(&item.case_instance_id));
        Ok(())
    }

    fn retain_active_plan_item_definition_matches(
        &self,
        session: &mut DbSession,
        items: &mut Vec<CmmnHistoricCaseInstance>,
    ) -> Result<(), CmmnError> {
        let Some(definition_id) = self.active_plan_item_definition_id.as_deref() else {
            return Ok(());
        };

        let plan_item_rows = session.select_raw(RenderedStatement::new(
            "SELECT DATA_ FROM ACT_CMMN_RU_PLAN_ITEM_INST WHERE STATE_ = 'ACTIVE'".to_string(),
            DbParams::new(),
        ))?;
        let mut matching_case_ids: std::collections::HashSet<String> = plan_item_rows
            .into_iter()
            .filter_map(|row| row.get_text("DATA_"))
            .filter_map(|json| serde_json::from_str::<CmmnPlanItemInstance>(&json).ok())
            .filter(|plan_item| plan_item.plan_item_definition_id == definition_id)
            .map(|plan_item| plan_item.case_instance_id)
            .collect();

        // Human tasks are deliberately not mirrored into the unified runtime
        // table (runtime.rs:10430-10434), so include that source explicitly.
        let human_task_rows = session.select_raw(RenderedStatement::new(
            "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE STATE_ = 'ACTIVE'".to_string(),
            DbParams::new(),
        ))?;
        matching_case_ids.extend(
            human_task_rows
                .into_iter()
                .filter_map(|row| row.get_text("DATA_"))
                .filter_map(|json| serde_json::from_str::<CmmnHumanTaskInstance>(&json).ok())
                .filter(|task| task.task_definition_id == definition_id)
                .map(|task| task.case_instance_id),
        );

        items.retain(|item| matching_case_ids.contains(&item.case_instance_id));
        Ok(())
    }

    pub fn single_result(&self) -> Result<Option<CmmnHistoricCaseInstance>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnHistoricCaseInstance>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

/// Java `HistoricTaskInstanceQueryImpl` (flowable-task-service) rendered by
/// `HistoricTaskInstance.xml`. Rust keeps historic human tasks as JSON `DATA_`
/// blobs, so each `<if>` predicate is evaluated in memory over decoded rows.
///
/// P120: extends the previous 7-filter surface with the high-frequency task
/// parameters plus the candidate/involved identity-link filters.
#[derive(Default)]
pub struct CmmnHistoricHumanTaskQuery {
    store: Option<CmmnStore>,
    task_id: Option<String>,
    case_instance_id: Option<String>,
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    stage_instance_id: Option<String>,
    completed_by: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    task_definition_key: Option<String>,
    task_definition_key_like: Option<String>,
    plan_item_instance_id: Option<String>,
    assignee: Option<String>,
    assignee_like: Option<String>,
    owner: Option<String>,
    owner_like: Option<String>,
    category: Option<String>,
    /// Java `taskDeleteReason` — `DELETE_REASON_ = ?`. The Rust CMMN engine never
    /// records a delete reason, so the column is always null and any non-null
    /// filter selects nothing (HistoricTaskInstance.xml deleteReason block).
    delete_reason: Option<String>,
    created_before: Option<DateTime<Utc>>,
    created_after: Option<DateTime<Utc>>,
    completed_before: Option<DateTime<Utc>>,
    completed_after: Option<DateTime<Utc>>,
    /// Java `finished()` / `unfinished()` — `END_TIME_ is (not) null`.
    finished: Option<bool>,
    candidate_user: Option<String>,
    candidate_group: Option<String>,
    candidate_group_in: Option<Vec<String>>,
    involved_user: Option<String>,
    involved_groups: Option<Vec<String>>,
    ignore_assignee: bool,
    user_group_resolver: Option<CmmnUserGroupResolver>,
    state: Option<CmmnHumanTaskState>,
    start: usize,
    size: Option<usize>,
}

impl CmmnHistoricHumanTaskQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store: Some(store),
            ..Default::default()
        }
    }

    pub fn task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn case_instance_id(mut self, case_instance_id: impl Into<String>) -> Self {
        self.case_instance_id = Some(case_instance_id.into());
        self
    }

    pub fn case_definition_id(mut self, case_definition_id: impl Into<String>) -> Self {
        self.case_definition_id = Some(case_definition_id.into());
        self
    }

    pub fn case_definition_key(mut self, case_definition_key: impl Into<String>) -> Self {
        self.case_definition_key = Some(case_definition_key.into());
        self
    }

    pub fn stage_instance_id(mut self, stage_instance_id: impl Into<String>) -> Self {
        self.stage_instance_id = Some(stage_instance_id.into());
        self
    }

    pub fn completed_by(mut self, completed_by: impl Into<String>) -> Self {
        self.completed_by = Some(completed_by.into());
        self
    }

    /// Java `taskName` (HistoricTaskInstanceBaseResource.java:117-119).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Java `taskNameLike` (HistoricTaskInstanceBaseResource.java:120-122).
    pub fn name_like(mut self, pattern: impl Into<String>) -> Self {
        self.name_like = Some(pattern.into());
        self
    }

    /// Java `taskNameLikeIgnoreCase` (HistoricTaskInstanceBaseResource.java:123-125).
    pub fn name_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.name_like_ignore_case = Some(pattern.into());
        self
    }

    /// Java `taskDefinitionKey` (HistoricTaskInstanceBaseResource.java:132-134).
    /// Maps onto the Rust historic task's `task_definition_id`, the same field the
    /// runtime task query binds `taskDefinitionKey` to.
    pub fn task_definition_key(mut self, task_definition_key: impl Into<String>) -> Self {
        self.task_definition_key = Some(task_definition_key.into());
        self
    }

    /// Java `taskDefinitionKeyLike` (HistoricTaskInstanceBaseResource.java:135-137).
    pub fn task_definition_key_like(mut self, pattern: impl Into<String>) -> Self {
        self.task_definition_key_like = Some(pattern.into());
        self
    }

    /// Java `planItemInstanceId` (HistoricTaskInstanceBaseResource.java:81-83). The
    /// Rust historic human task carries its plan item in `plan_item_id`.
    pub fn plan_item_instance_id(mut self, plan_item_instance_id: impl Into<String>) -> Self {
        self.plan_item_instance_id = Some(plan_item_instance_id.into());
        self
    }

    /// Java `taskAssignee` (HistoricTaskInstanceBaseResource.java:156-158).
    pub fn assignee(mut self, assignee: impl Into<String>) -> Self {
        self.assignee = Some(assignee.into());
        self
    }

    /// Java `taskAssigneeLike` (HistoricTaskInstanceBaseResource.java:159-161).
    pub fn assignee_like(mut self, pattern: impl Into<String>) -> Self {
        self.assignee_like = Some(pattern.into());
        self
    }

    /// Java `taskOwner` (HistoricTaskInstanceBaseResource.java:162-164).
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    /// Java `taskOwnerLike` (HistoricTaskInstanceBaseResource.java:165-167).
    pub fn owner_like(mut self, pattern: impl Into<String>) -> Self {
        self.owner_like = Some(pattern.into());
        self
    }

    /// Java `taskCategory` (HistoricTaskInstanceBaseResource.java:138-140).
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Java `taskDeleteReason` (HistoricTaskInstanceBaseResource.java:150-152).
    pub fn delete_reason(mut self, delete_reason: impl Into<String>) -> Self {
        self.delete_reason = Some(delete_reason.into());
        self
    }

    /// Java `taskCreatedBefore` — `START_TIME_ <` on the historic task
    /// (HistoricTaskInstanceBaseResource.java:215-217). Rust records task creation
    /// as `activated_at`.
    pub fn created_before(mut self, created_before: DateTime<Utc>) -> Self {
        self.created_before = Some(created_before);
        self
    }

    /// Java `taskCreatedAfter` (HistoricTaskInstanceBaseResource.java:218-220).
    pub fn created_after(mut self, created_after: DateTime<Utc>) -> Self {
        self.created_after = Some(created_after);
        self
    }

    /// Java `taskCompletedBefore` — `END_TIME_ <`
    /// (HistoricTaskInstanceBaseResource.java:233-235).
    pub fn completed_before(mut self, completed_before: DateTime<Utc>) -> Self {
        self.completed_before = Some(completed_before);
        self
    }

    /// Java `taskCompletedAfter` (HistoricTaskInstanceBaseResource.java:236-238).
    pub fn completed_after(mut self, completed_after: DateTime<Utc>) -> Self {
        self.completed_after = Some(completed_after);
        self
    }

    /// Java `finished()` / `unfinished()` (HistoricTaskInstanceBaseResource.java:183-189).
    pub fn finished(mut self, finished: bool) -> Self {
        self.finished = Some(finished);
        self
    }

    /// Java `taskCandidateUser` (HistoricTaskInstanceQueryImpl.java:1888-1899).
    pub fn candidate_user(mut self, candidate_user: impl Into<String>) -> Self {
        self.candidate_user = Some(candidate_user.into());
        self
    }

    /// Java `taskCandidateGroup` (HistoricTaskInstanceQueryImpl.java:1902-1917).
    pub fn candidate_group(mut self, candidate_group: impl Into<String>) -> Self {
        self.candidate_group = Some(candidate_group.into());
        self
    }

    /// Java `taskCandidateGroupIn` (HistoricTaskInstanceQueryImpl.java:1920-1939).
    pub fn candidate_group_in(mut self, candidate_group_in: Vec<String>) -> Self {
        self.candidate_group_in = Some(candidate_group_in);
        self
    }

    /// Java `taskInvolvedUser` (HistoricTaskInstanceQueryImpl.java:1942-1953).
    pub fn involved_user(mut self, involved_user: impl Into<String>) -> Self {
        self.involved_user = Some(involved_user.into());
        self
    }

    /// Java `taskInvolvedGroups` (HistoricTaskInstanceQueryImpl.java:1956-1969).
    pub fn involved_groups(mut self, involved_groups: Vec<String>) -> Self {
        self.involved_groups = Some(involved_groups);
        self
    }

    /// Java `ignoreAssigneeValue` (HistoricTaskInstanceQueryImpl.java:1972-1979):
    /// drops the implicit `ASSIGNEE_ is null` gate on the candidate block.
    pub fn ignore_assignee_value(mut self) -> Self {
        self.ignore_assignee = true;
        self
    }

    /// Group expansion for `candidateUser`, mirroring Java
    /// `getGroupsForCandidateUser` (HistoricTaskInstanceQueryImpl.java:2235-2245),
    /// which resolves the user's groups through the IDM identity service.
    pub fn user_group_resolver(mut self, resolver: CmmnUserGroupResolver) -> Self {
        self.user_group_resolver = Some(resolver);
        self
    }

    pub fn state(mut self, state: CmmnHumanTaskState) -> Self {
        self.state = Some(state);
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnHistoricHumanTaskInstance>, CmmnError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| CmmnError::storage("Historic human task query has no store"))?;
        let mut session = store.create_session()?;
        let sql = String::from(
            "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK_HISTORY ORDER BY ACTIVATED_AT_ ASC, TASK_ID_ ASC",
        );
        let rendered = RenderedStatement::new(sql, DbParams::new());
        let rows = session.select_raw(rendered)?;
        let mut items: Vec<CmmnHistoricHumanTaskInstance> = rows
            .into_iter()
            .map(|row| {
                let data = row.get_text("DATA_").ok_or_else(|| {
                    CmmnError::storage("Missing DATA_ in historic human task query result")
                })?;
                serde_json::from_str(&data).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        items.retain(|item| matches_optional(&self.task_id, &item.task_id));
        items.retain(|item| matches_optional(&self.case_instance_id, &item.case_instance_id));
        items.retain(|item| matches_optional(&self.case_definition_id, &item.case_definition_id));
        items.retain(|item| matches_optional(&self.case_definition_key, &item.case_definition_key));
        items.retain(|item| {
            matches_optional_option(&self.stage_instance_id, item.stage_instance_id.as_deref())
        });
        items.retain(|item| {
            matches_optional_option(&self.completed_by, item.completed_by.as_deref())
        });
        items.retain(|item| matches_optional(&self.name, &item.name));
        items.retain(|item| matches_like_optional(&self.name_like, &item.name));
        items.retain(|item| {
            matches_like_ignore_case_optional(&self.name_like_ignore_case, &item.name)
        });
        items.retain(|item| {
            matches_optional(&self.task_definition_key, &item.task_definition_id)
        });
        items.retain(|item| {
            matches_like_optional(&self.task_definition_key_like, &item.task_definition_id)
        });
        items.retain(|item| matches_optional(&self.plan_item_instance_id, &item.plan_item_id));
        items.retain(|item| matches_optional_option(&self.assignee, item.assignee.as_deref()));
        items.retain(|item| {
            matches_like_optional_option(&self.assignee_like, item.assignee.as_deref())
        });
        items.retain(|item| matches_optional_option(&self.owner, item.owner.as_deref()));
        items.retain(|item| matches_like_optional_option(&self.owner_like, item.owner.as_deref()));
        items.retain(|item| matches_optional_option(&self.category, item.category.as_deref()));
        // Java compares `DELETE_REASON_ = ?`; the Rust CMMN engine never records a
        // delete reason on historic human tasks, so the column is uniformly null and
        // an equality filter selects no rows.
        if self.delete_reason.is_some() {
            items.clear();
        }
        items.retain(|item| {
            self.created_before
                .is_none_or(|bound| item.activated_at <= bound)
        });
        items.retain(|item| {
            self.created_after
                .is_none_or(|bound| item.activated_at >= bound)
        });
        items.retain(|item| {
            self.completed_before
                .is_none_or(|bound| item.completed_at.is_some_and(|value| value <= bound))
        });
        items.retain(|item| {
            self.completed_after
                .is_none_or(|bound| item.completed_at.is_some_and(|value| value >= bound))
        });
        items.retain(|item| {
            self.finished
                .is_none_or(|finished| item.completed_at.is_some() == finished)
        });
        items.retain(|item| self.state.as_ref().is_none_or(|value| item.state == *value));

        self.retain_identity_link_matches(&mut session, &mut items)?;

        Ok(items)
    }

    /// Java renders the candidate block as a correlated `exists` over
    /// `ACT_HI_IDENTITYLINK` with `TYPE_ = 'candidate'`, preceded by an implicit
    /// `ASSIGNEE_ is null` unless `ignoreAssigneeValue`
    /// (HistoricTaskInstance.xml:1484-1512). `involvedUser` matches any link type
    /// on the task, or the assignee, or the owner (:1514-1520); `involvedGroups`
    /// matches any link carrying one of the group ids (:1521-1535).
    ///
    /// Rust keeps one identity-link table (`ACT_CMMN_IDENTITY_LINK`, scope
    /// `humanTask`) shared by runtime and history: links are written on task
    /// creation (C10) and are only removed when the case history itself is deleted
    /// (`delete_historic_case_instance_tx`), so the historic query reads the same
    /// rows the runtime query does.
    fn retain_identity_link_matches(
        &self,
        session: &mut DbSession,
        items: &mut Vec<CmmnHistoricHumanTaskInstance>,
    ) -> Result<(), CmmnError> {
        let needs_candidate = self.candidate_user.is_some()
            || self.candidate_group.is_some()
            || self.candidate_group_in.is_some();
        if !needs_candidate && self.involved_user.is_none() && self.involved_groups.is_none() {
            return Ok(());
        }

        let link_rows = session.select_raw(RenderedStatement::new(
            "SELECT DATA_ FROM ACT_CMMN_IDENTITY_LINK".to_string(),
            DbParams::new(),
        ))?;
        let mut links_by_task: std::collections::HashMap<String, Vec<CmmnIdentityLink>> =
            std::collections::HashMap::new();
        for row in link_rows {
            let Some(json) = row.get_text("DATA_") else {
                continue;
            };
            let Ok(link) = serde_json::from_str::<CmmnIdentityLink>(&json) else {
                continue;
            };
            if link.scope_type == "humanTask" {
                links_by_task
                    .entry(link.scope_id.clone())
                    .or_default()
                    .push(link);
            }
        }

        // Applied once for the whole candidate block, not per condition
        // (HistoricTaskInstance.xml:1485-1487).
        if needs_candidate && !self.ignore_assignee {
            items.retain(|task| task.assignee.is_none());
        }

        if let Some(candidate_user) = &self.candidate_user {
            let user_group_ids: std::collections::HashSet<String> = self
                .user_group_resolver
                .as_ref()
                .map(|resolver| resolver(candidate_user).into_iter().collect())
                .unwrap_or_default();
            items.retain(|task| {
                links_by_task.get(task.task_id.as_str()).is_some_and(|links| {
                    links.iter().any(|link| {
                        link.link_type == "candidate"
                            && (link.user_id.as_deref() == Some(candidate_user.as_str())
                                || link
                                    .group_id
                                    .as_ref()
                                    .is_some_and(|gid| user_group_ids.contains(gid)))
                    })
                })
            });
        }

        if let Some(candidate_group) = &self.candidate_group {
            items.retain(|task| {
                links_by_task.get(task.task_id.as_str()).is_some_and(|links| {
                    links.iter().any(|link| {
                        link.link_type == "candidate"
                            && link.group_id.as_deref() == Some(candidate_group.as_str())
                    })
                })
            });
        }

        if let Some(candidate_group_in) = &self.candidate_group_in {
            let groups: std::collections::HashSet<&str> =
                candidate_group_in.iter().map(String::as_str).collect();
            items.retain(|task| {
                links_by_task.get(task.task_id.as_str()).is_some_and(|links| {
                    links.iter().any(|link| {
                        link.link_type == "candidate"
                            && link
                                .group_id
                                .as_deref()
                                .is_some_and(|gid| groups.contains(gid))
                    })
                })
            });
        }

        if let Some(involved_user) = &self.involved_user {
            items.retain(|task| {
                if task.assignee.as_deref() == Some(involved_user.as_str())
                    || task.owner.as_deref() == Some(involved_user.as_str())
                {
                    return true;
                }
                links_by_task.get(task.task_id.as_str()).is_some_and(|links| {
                    links
                        .iter()
                        .any(|link| link.user_id.as_deref() == Some(involved_user.as_str()))
                })
            });
        }

        if let Some(involved_groups) = &self.involved_groups {
            let groups: std::collections::HashSet<&str> =
                involved_groups.iter().map(String::as_str).collect();
            items.retain(|task| {
                links_by_task.get(task.task_id.as_str()).is_some_and(|links| {
                    links.iter().any(|link| {
                        link.group_id
                            .as_deref()
                            .is_some_and(|gid| groups.contains(gid))
                    })
                })
            });
        }

        Ok(())
    }

    pub fn single_result(&self) -> Result<Option<CmmnHistoricHumanTaskInstance>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnHistoricHumanTaskInstance>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

pub struct CmmnHistoricMilestoneQuery {
    store: CmmnStore,
    id: Option<String>,
    case_instance_id: Option<String>,
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    milestone_id: Option<String>,
    start: usize,
    size: Option<usize>,
}

impl CmmnHistoricMilestoneQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            id: None,
            case_instance_id: None,
            case_definition_id: None,
            case_definition_key: None,
            milestone_id: None,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn case_instance_id(mut self, case_instance_id: impl Into<String>) -> Self {
        self.case_instance_id = Some(case_instance_id.into());
        self
    }

    pub fn case_definition_id(mut self, case_definition_id: impl Into<String>) -> Self {
        self.case_definition_id = Some(case_definition_id.into());
        self
    }

    pub fn case_definition_key(mut self, case_definition_key: impl Into<String>) -> Self {
        self.case_definition_key = Some(case_definition_key.into());
        self
    }

    pub fn milestone_id(mut self, milestone_id: impl Into<String>) -> Self {
        self.milestone_id = Some(milestone_id.into());
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnHistoricMilestoneInstance>, CmmnError> {
        let mut session = self.store.create_session()?;
        let sql = String::from(
            "SELECT DATA_ FROM ACT_CMMN_MILESTONE_HISTORY ORDER BY TIME_ ASC, ID_ ASC",
        );
        let rendered = RenderedStatement::new(sql, DbParams::new());
        let rows = session.select_raw(rendered)?;
        let mut items: Vec<CmmnHistoricMilestoneInstance> = rows
            .into_iter()
            .map(|row| {
                let data = row.get_text("DATA_").ok_or_else(|| {
                    CmmnError::storage("Missing DATA_ in historic milestone query result")
                })?;
                serde_json::from_str(&data).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        items.retain(|item| matches_optional(&self.id, &item.id));
        items.retain(|item| matches_optional(&self.case_instance_id, &item.case_instance_id));
        items.retain(|item| matches_optional(&self.case_definition_id, &item.case_definition_id));
        items.retain(|item| matches_optional(&self.case_definition_key, &item.case_definition_key));
        items.retain(|item| matches_optional(&self.milestone_id, &item.milestone_id));

        Ok(items)
    }

    pub fn single_result(&self) -> Result<Option<CmmnHistoricMilestoneInstance>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnHistoricMilestoneInstance>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

fn matches_optional(filter: &Option<String>, actual: &str) -> bool {
    filter.as_ref().is_none_or(|value| value == actual)
}

fn matches_optional_option(filter: &Option<String>, actual: Option<&str>) -> bool {
    filter
        .as_ref()
        .is_none_or(|value| actual == Some(value.as_str()))
}

fn matches_like_optional(pattern: &Option<String>, actual: &str) -> bool {
    pattern
        .as_ref()
        .is_none_or(|pattern| sql_like_matches(actual, pattern))
}

fn matches_like_optional_option(pattern: &Option<String>, actual: Option<&str>) -> bool {
    pattern.as_ref().is_none_or(|pattern| {
        actual.is_some_and(|actual| sql_like_matches(actual, pattern))
    })
}

fn matches_like_ignore_case_optional(pattern: &Option<String>, actual: &str) -> bool {
    pattern.as_ref().is_none_or(|pattern| {
        sql_like_matches(&actual.to_lowercase(), &pattern.to_lowercase())
    })
}

fn matches_like_ignore_case_optional_option(
    pattern: &Option<String>,
    actual: Option<&str>,
) -> bool {
    pattern.as_ref().is_none_or(|pattern| {
        actual.is_some_and(|actual| {
            sql_like_matches(&actual.to_lowercase(), &pattern.to_lowercase())
        })
    })
}

/// SQL `LIKE` with `%` (any run) and `_` (single char) wildcards, matching the
/// `like` predicates the Java mappers render for the `*Like` query parameters.
/// Local signature is `(value, pattern)`; shared impl is `(pattern, value)`.
fn sql_like_matches(value: &str, pattern: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}

fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let start = start.min(total);
    let page_size = size.unwrap_or(total.saturating_sub(start));
    let data = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    PagedResult {
        start,
        size: data.len(),
        total,
        data,
    }
}
