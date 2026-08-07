use crate::error::CmmnError;
use crate::job::TYPE_TRIGGER_TIMER;
use crate::models::{CMMN_SCOPE_TYPE, CmmnJob, CmmnJobFamily, CmmnPlanItemInstance, PagedResult};
use crate::parent_state_resolver::ensure_cmmn_job_parent_allows_activation;
use crate::store::CmmnStore;
use crate::timer_util::{next_cron_after, parse_date_time, prepare_repeat, resolve_timer_due};
use chrono::{DateTime, Utc};
use flowable_persistence::db_session::DbSession;
use flowable_persistence::entity::cmmn_job::{CmmnJobDataManager, CmmnJobEntity};
use flowable_persistence::statement::{RenderedStatement, StatementId};
use flowable_persistence::value::DbParams;
use uuid::Uuid;

const DEFAULT_ASYNC_EXECUTOR_RETRIES: i32 = 3;
const TIMER_JOB_CONFIG_REPEAT_KEY: &str = "repeat";

#[derive(Clone)]
pub struct CmmnManagementService {
    store: CmmnStore,
}

impl CmmnManagementService {
    pub(crate) fn new(store: CmmnStore) -> Self {
        Self { store }
    }

    pub fn create_job_query(&self) -> CmmnManagementJobQuery {
        CmmnManagementJobQuery::new(self.store.clone())
    }

    pub fn get_job(&self, job_id: &str) -> Result<CmmnJob, CmmnError> {
        let mut session = self.store.create_session()?;
        load_job(&mut session, job_id)
    }

    pub fn delete_job(&self, job_id: &str) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        delete_job_entity(&mut session, job_id)?;
        session.commit()?;
        Ok(())
    }

    /// Java `CmmnManagementService.moveSuspendedJobToExecutableJob`.
    ///
    /// Accepts only the suspended family. Retries (including 0/negative) and all
    /// job fields are preserved; source suspended row is deleted and an executable
    /// row is inserted in the same transaction. Does not use BPMN process paths.
    pub fn move_suspended_job_to_executable_job(&self, job_id: &str) -> Result<CmmnJob, CmmnError> {
        self.in_transaction(|session| move_suspended_job_to_executable_job_session(session, job_id))
    }

    /// Java `CmmnManagementService.deleteSuspendedJob`.
    ///
    /// Family-typed: only suspended jobs are deleted. Other families / missing id → NotFound.
    pub fn delete_suspended_job(&self, job_id: &str) -> Result<(), CmmnError> {
        self.in_transaction(|session| delete_suspended_job_session(session, job_id))
    }

    /// Java `CmmnManagementService.rescheduleTimeDateJob` delegates a typed date to
    /// `RescheduleTimerJobCmd` (`CmmnManagementServiceImpl.java:203-210`).
    pub fn reschedule_time_date_job(
        &self,
        job_id: &str,
        due_date: DateTime<Utc>,
    ) -> Result<CmmnJob, CmmnError> {
        self.in_transaction(|session| {
            reschedule_timer_job_session(session, job_id, TimerRescheduleValue::DueDate(due_date))
        })
    }

    /// Java `CmmnManagementService.rescheduleTimeDateValueJob` delegates a date,
    /// duration, repetition or cron string (`CmmnManagementServiceImpl.java:208-210`).
    pub fn reschedule_time_date_value_job(
        &self,
        job_id: &str,
        date_value: &str,
    ) -> Result<CmmnJob, CmmnError> {
        self.in_transaction(|session| {
            reschedule_timer_job_session(
                session,
                job_id,
                TimerRescheduleValue::DateValue(date_value),
            )
        })
    }

    /// Java REST `move` dispatches by persisted jobType: history-origin deadletters
    /// return to history, every other type becomes executable (JobResource.java:306-323).
    pub fn move_deadletter_job_by_type(
        &self,
        job_id: &str,
        retries: i32,
    ) -> Result<CmmnJob, CmmnError> {
        self.in_transaction(|session| {
            move_deadletter_job_session(session, job_id, retries, DeadletterDestination::ByType)
        })
    }

    /// Java `CmmnManagementService.moveDeadLetterJobToExecutableJob`.
    pub fn move_deadletter_job_to_executable_job(
        &self,
        job_id: &str,
        retries: i32,
    ) -> Result<CmmnJob, CmmnError> {
        self.in_transaction(|session| {
            move_deadletter_job_session(
                session,
                job_id,
                retries,
                DeadletterDestination::Executable,
            )
        })
    }

    /// Java `CmmnManagementService.moveDeadLetterJobToHistoryJob`.
    pub fn move_deadletter_job_to_history_job(
        &self,
        job_id: &str,
        retries: i32,
    ) -> Result<CmmnJob, CmmnError> {
        self.in_transaction(|session| {
            move_deadletter_job_session(session, job_id, retries, DeadletterDestination::History)
        })
    }

    /// Run work in a single management DB transaction (commit on Ok, rollback on Err).
    /// Used by activation/delete and by contract tests that force outer rollback.
    pub fn in_transaction<T, F>(&self, op: F) -> Result<T, CmmnError>
    where
        F: FnOnce(&mut DbSession) -> Result<T, CmmnError>,
    {
        let mut session = self.store.create_session()?;
        match op(&mut session) {
            Ok(value) => {
                session.commit()?;
                Ok(value)
            }
            Err(error) => {
                let _ = session.rollback();
                Err(error)
            }
        }
    }

    /// Session-scoped activation for composing multi-step transactions.
    pub fn move_suspended_job_to_executable_job_in_session(
        &self,
        session: &mut DbSession,
        job_id: &str,
    ) -> Result<CmmnJob, CmmnError> {
        let _ = self;
        move_suspended_job_to_executable_job_session(session, job_id)
    }

    /// Session-scoped suspended delete for composing multi-step transactions.
    pub fn delete_suspended_job_in_session(
        &self,
        session: &mut DbSession,
        job_id: &str,
    ) -> Result<(), CmmnError> {
        let _ = self;
        delete_suspended_job_session(session, job_id)
    }

    pub fn insert_job(&self, mut job: CmmnJob) -> Result<CmmnJob, CmmnError> {
        if job.id.trim().is_empty() {
            job.id = format!("cmmn-job:{}", Uuid::new_v4());
        }
        if job.created_at.timestamp_millis() == 0 {
            job.created_at = Utc::now();
        }

        let mut session = self.store.create_session()?;
        let manager = CmmnJobDataManager::new();
        let mut entity = CmmnJobEntity::new(
            job.id.clone(),
            job.family.as_str().to_string(),
            job.state.clone(),
            job.scope_id.clone().unwrap_or_default(),
            job.scope_definition_id.clone().unwrap_or_default(),
            job.element_id.clone().unwrap_or_default(),
            job.retries,
            job.created_at.timestamp_millis(),
            serde_json::to_string(&job)?,
        );
        entity.set_sub_scope_id(job.sub_scope_id.clone());
        entity.set_tenant_id(job.tenant_id.clone());
        entity.set_due_date(job.due_date.map(|value| value.to_rfc3339()));
        entity.set_lock_owner(job.lock_owner.clone());
        entity.set_exception_message(job.exception_message.clone());
        entity.set_exception_stacktrace(job.exception_stacktrace.clone());
        manager.insert(&mut session, entity)?;
        session.commit()?;
        Ok(job)
    }

    /// Persist updates to an existing CMMN job (DATA_ blob + denormalized columns).
    pub fn update_job(&self, job: &CmmnJob) -> Result<CmmnJob, CmmnError> {
        let mut session = self.store.create_session()?;
        let manager = CmmnJobDataManager::new();
        let _existing = manager
            .find_by_id(&mut session, &job.id)?
            .ok_or_else(|| CmmnError::not_found(format!("CMMN job '{}' was not found", job.id)))?;
        let mut params = DbParams::new();
        params.push(job.family.as_str());
        params.push(job.state.as_str());
        params.push(job.scope_id.clone().unwrap_or_default());
        params.push(job.sub_scope_id.clone());
        params.push(job.scope_definition_id.clone().unwrap_or_default());
        params.push(job.element_id.clone().unwrap_or_default());
        params.push(job.tenant_id.clone());
        params.push(job.due_date.map(|value| value.to_rfc3339()));
        params.push(job.lock_owner.clone());
        params.push(job.retries as i64);
        params.push(job.exception_message.clone());
        params.push(job.exception_stacktrace.clone());
        params.push(job.created_at.timestamp_millis());
        params.push(serde_json::to_string(job)?);
        params.push(job.id.as_str());
        let result = session.execute_raw(RenderedStatement::new(
            "UPDATE ACT_CMMN_JOB SET FAMILY_ = ?, STATE_ = ?, SCOPE_ID_ = ?, SUB_SCOPE_ID_ = ?, \
             SCOPE_DEFINITION_ID_ = ?, ELEMENT_ID_ = ?, TENANT_ID_ = ?, DUE_DATE_ = ?, \
             LOCK_OWNER_ = ?, RETRIES_ = ?, EXCEPTION_MESSAGE_ = ?, EXCEPTION_STACKTRACE_ = ?, \
             CREATED_AT_ = ?, DATA_ = ? WHERE ID_ = ?"
                .to_string(),
            params,
        ))?;
        if result.rows_affected == 0 {
            return Err(CmmnError::not_found(format!(
                "CMMN job '{}' was not found",
                job.id
            )));
        }
        session.commit()?;
        Ok(job.clone())
    }
}

pub struct CmmnManagementJobQuery {
    store: CmmnStore,
    family: Option<CmmnJobFamily>,
    id: Option<String>,
    state: Option<String>,
    scope_id: Option<String>,
    sub_scope_id: Option<String>,
    scope_definition_id: Option<String>,
    scope_type: Option<String>,
    element_id: Option<String>,
    handler_type: Option<String>,
    without_scope_id: bool,
    timers_only: bool,
    messages_only: bool,
    with_exception: bool,
    exception_message: Option<String>,
    due_before: Option<DateTime<Utc>>,
    due_after: Option<DateTime<Utc>>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    start: usize,
    size: Option<usize>,
}

impl CmmnManagementJobQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            family: None,
            id: None,
            state: None,
            scope_id: None,
            sub_scope_id: None,
            scope_definition_id: None,
            scope_type: None,
            element_id: None,
            handler_type: None,
            without_scope_id: false,
            timers_only: false,
            messages_only: false,
            with_exception: false,
            exception_message: None,
            due_before: None,
            due_after: None,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
            start: 0,
            size: None,
        }
    }

    pub fn family(mut self, family: CmmnJobFamily) -> Self {
        self.family = Some(family);
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn state(mut self, state: impl Into<String>) -> Self {
        self.state = Some(state.into());
        self
    }

    /// Java `JobQuery.scopeId` — CMMN REST passes `caseInstanceId` here
    /// (JobCollectionResource.java:115-118); SQL `RES.SCOPE_ID_ = ?` (Job.xml:182-184).
    pub fn scope_id(mut self, scope_id: impl Into<String>) -> Self {
        self.scope_id = Some(scope_id.into());
        self
    }

    /// Java `JobQuery.subScopeId` — CMMN REST passes `planItemInstanceId`
    /// (JobCollectionResource.java:122-125); SQL `RES.SUB_SCOPE_ID_ = ?` (Job.xml:188-190).
    pub fn sub_scope_id(mut self, sub_scope_id: impl Into<String>) -> Self {
        self.sub_scope_id = Some(sub_scope_id.into());
        self
    }

    /// Java `JobQuery.scopeDefinitionId`; `caseDefinitionId` is an alias that delegates
    /// to the same field (JobQueryImpl.java:231-235). SQL `RES.SCOPE_DEFINITION_ID_ = ?`
    /// (Job.xml:194-196).
    pub fn scope_definition_id(mut self, scope_definition_id: impl Into<String>) -> Self {
        self.scope_definition_id = Some(scope_definition_id.into());
        self
    }

    /// Java `JobQuery.scopeType` — SQL `RES.SCOPE_TYPE_ = ?` (Job.xml:191-193).
    pub fn scope_type(mut self, scope_type: impl Into<String>) -> Self {
        self.scope_type = Some(scope_type.into());
        self
    }

    /// Java `JobQuery.handlerType` — filter by job handler TYPE string.
    pub fn handler_type(mut self, handler_type: impl Into<String>) -> Self {
        self.handler_type = Some(handler_type.into());
        self
    }

    /// Java `JobQuery.elementId` — SQL `RES.ELEMENT_ID_ = ?` (Job.xml:176-178).
    pub fn element_id(mut self, element_id: impl Into<String>) -> Self {
        self.element_id = Some(element_id.into());
        self
    }

    /// Java `JobQuery.withoutScopeId` — SQL `RES.SCOPE_ID_ IS NULL` (Job.xml:185-187).
    pub fn without_scope_id(mut self) -> Self {
        self.without_scope_id = true;
        self
    }

    /// Java `JobQuery.timers()` — SQL `RES.TYPE_ = 'timer'` (Job.xml:203-205).
    pub fn timers(mut self) -> Self {
        self.timers_only = true;
        self
    }

    /// Java `JobQuery.messages()` — SQL `RES.TYPE_ = 'message'` (Job.xml:206-208).
    pub fn messages(mut self) -> Self {
        self.messages_only = true;
        self
    }

    /// Java `JobQuery.withException` — SQL `EXCEPTION_MSG_ is not null or
    /// EXCEPTION_STACK_ID_ is not null` (Job.xml:221-223).
    pub fn with_exception(mut self) -> Self {
        self.with_exception = true;
        self
    }

    /// Java `JobQuery.exceptionMessage` — SQL `RES.EXCEPTION_MSG_ = ?` (Job.xml:224-226).
    pub fn exception_message(mut self, message: impl Into<String>) -> Self {
        self.exception_message = Some(message.into());
        self
    }

    /// Java `JobQuery.duedateLowerThan` — CMMN REST's `dueBefore`
    /// (JobCollectionResource.java:149-151); SQL `RES.DUEDATE_ < ?` (Job.xml:212-214).
    pub fn due_before(mut self, due_before: DateTime<Utc>) -> Self {
        self.due_before = Some(due_before);
        self
    }

    /// Java `JobQuery.duedateHigherThan` — CMMN REST's `dueAfter`
    /// (JobCollectionResource.java:152-154); SQL `RES.DUEDATE_ > ?` (Job.xml:209-211).
    pub fn due_after(mut self, due_after: DateTime<Utc>) -> Self {
        self.due_after = Some(due_after);
        self
    }

    /// Java `JobQuery.jobTenantId` — SQL `RES.TENANT_ID_ = ?` (Job.xml:236-238).
    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Java `JobQuery.jobTenantIdLike` — SQL `RES.TENANT_ID_ like ?` (Job.xml:239-241).
    pub fn tenant_id_like(mut self, tenant_id_like: impl Into<String>) -> Self {
        self.tenant_id_like = Some(tenant_id_like.into());
        self
    }

    /// Java `JobQuery.jobWithoutTenantId` — SQL `TENANT_ID_ = '' or TENANT_ID_ is null`
    /// (Job.xml:242-244).
    pub fn without_tenant_id(mut self) -> Self {
        self.without_tenant_id = true;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnJob>, CmmnError> {
        let mut session = self.store.create_session()?;
        let sql = String::from("SELECT DATA_ FROM ACT_CMMN_JOB ORDER BY CREATED_AT_ ASC, ID_ ASC");
        let rendered = RenderedStatement::new(sql, DbParams::new());
        let rows = session.select_raw(rendered)?;
        let mut jobs: Vec<CmmnJob> = rows
            .into_iter()
            .map(|row| {
                let data = row
                    .get_text("DATA_")
                    .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN job query result"))?;
                serde_json::from_str(&data).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        jobs.retain(|job| {
            self.family
                .as_ref()
                .is_none_or(|family| job.family == *family)
        });
        jobs.retain(|job| self.id.as_ref().is_none_or(|id| job.id == *id));
        jobs.retain(|job| self.state.as_ref().is_none_or(|state| job.state == *state));
        // Java Job.xml:182-196 — scopeId / subScopeId / scopeDefinitionId / scopeType are
        // plain equality filters; scopeType defaults to CMMN for rows written by this
        // engine (parent_state_resolver.rs:34 makes the same assumption for older blobs).
        jobs.retain(|job| {
            self.scope_id
                .as_ref()
                .is_none_or(|scope_id| job.scope_id.as_deref() == Some(scope_id.as_str()))
        });
        jobs.retain(|job| {
            self.sub_scope_id
                .as_ref()
                .is_none_or(|sub_scope_id| job.sub_scope_id.as_deref() == Some(sub_scope_id.as_str()))
        });
        jobs.retain(|job| {
            self.scope_definition_id.as_ref().is_none_or(|definition| {
                job.scope_definition_id.as_deref() == Some(definition.as_str())
            })
        });
        jobs.retain(|job| {
            self.scope_type.as_ref().is_none_or(|scope_type| {
                job.scope_type.as_deref().unwrap_or(CMMN_SCOPE_TYPE) == scope_type.as_str()
            })
        });
        jobs.retain(|job| {
            self.element_id
                .as_ref()
                .is_none_or(|element_id| job.element_id.as_deref() == Some(element_id.as_str()))
        });
        jobs.retain(|job| {
            self.handler_type.as_ref().is_none_or(|handler_type| {
                job.handler_type.as_deref() == Some(handler_type.as_str())
            })
        });
        // Java Job.xml:185-187 `RES.SCOPE_ID_ IS NULL`. Rust persists an empty string when
        // scope_id is None (insert_job_entity above), so both readings count as "without".
        if self.without_scope_id {
            jobs.retain(|job| job.scope_id.as_deref().unwrap_or_default().is_empty());
        }
        // Java splits timer/message via the TYPE_ column (Job.xml:203-208). Rust has no
        // TYPE_ column; the timer family and the `cmmn-trigger-timer` handler carry the
        // same distinction, everything else is a message-style async job.
        if self.timers_only {
            jobs.retain(is_timer_job);
        }
        if self.messages_only {
            jobs.retain(|job| !is_timer_job(job));
        }
        // Java Job.xml:221-223 checks EXCEPTION_MSG_ or EXCEPTION_STACK_ID_.
        if self.with_exception {
            jobs.retain(|job| {
                job.exception_message
                    .as_deref()
                    .is_some_and(|message| !message.is_empty())
                    || job
                        .exception_stacktrace
                        .as_deref()
                        .is_some_and(|stacktrace| !stacktrace.is_empty())
            });
        }
        jobs.retain(|job| {
            self.exception_message
                .as_ref()
                .is_none_or(|message| job.exception_message.as_deref() == Some(message.as_str()))
        });
        // Java Job.xml:209-214 — strict `<` / `>` on DUEDATE_; a job without a due date
        // never matches a due-date filter (SQL NULL comparison).
        jobs.retain(|job| {
            self.due_before
                .is_none_or(|due_before| job.due_date.is_some_and(|due| due < due_before))
        });
        jobs.retain(|job| {
            self.due_after
                .is_none_or(|due_after| job.due_date.is_some_and(|due| due > due_after))
        });
        jobs.retain(|job| {
            self.tenant_id
                .as_ref()
                .is_none_or(|tenant_id| job.tenant_id.as_deref() == Some(tenant_id.as_str()))
        });
        // Java Job.xml:239-241 uses SQL `like`, where `%` is the multi-char wildcard.
        jobs.retain(|job| {
            self.tenant_id_like.as_ref().is_none_or(|pattern| {
                sql_like_matches(job.tenant_id.as_deref().unwrap_or_default(), pattern)
            })
        });
        // Java Job.xml:242-244 treats '' and NULL alike.
        if self.without_tenant_id {
            jobs.retain(|job| job.tenant_id.as_deref().unwrap_or_default().is_empty());
        }

        Ok(jobs)
    }

    pub fn single_result(&self) -> Result<Option<CmmnJob>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnJob>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

/// Java distinguishes timer jobs from message jobs through the `TYPE_` column
/// (Job.xml:203-208); Rust's `ACT_CMMN_JOB` has no such column. The timer family and the
/// `cmmn-trigger-timer` handler carry the same information: a job is a timer when it lives
/// in the timer family (TimerJobEntityManagerImpl.java:222 sets TYPE_='timer' on exactly
/// those rows) or when it was scheduled by the timer event listener path
/// (runtime.rs:6670-6674). Everything else is a message-style async job
/// (DefaultJobManager.java:695 sets TYPE_='message' for those).
fn is_timer_job(job: &CmmnJob) -> bool {
    job.family == CmmnJobFamily::Timer || job.handler_type.as_deref() == Some(TYPE_TRIGGER_TIMER)
}

/// Minimal SQL `LIKE` for the `tenantIdLike` filter (Job.xml:239-241): `%` matches any
/// run of characters, `_` matches exactly one. No escape clause is supported because the
/// CMMN REST layer never passes one.
/// Local signature is `(value, pattern)`; shared impl is `(pattern, value)`.
fn sql_like_matches(value: &str, pattern: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}

fn load_job(session: &mut DbSession, job_id: &str) -> Result<CmmnJob, CmmnError> {
    let manager = CmmnJobDataManager::new();
    let entity = manager
        .find_by_id(session, job_id)?
        .ok_or_else(|| CmmnError::not_found(format!("CMMN job '{job_id}' was not found")))?;
    serde_json::from_str(&entity.data).map_err(Into::into)
}

pub(crate) fn delete_job_entity(session: &mut DbSession, job_id: &str) -> Result<(), CmmnError> {
    let manager = CmmnJobDataManager::new();
    let entity = manager
        .find_by_id(session, job_id)?
        .ok_or_else(|| CmmnError::not_found(format!("CMMN job '{job_id}' was not found")))?;
    manager.delete(session, &entity)?;
    Ok(())
}

pub(crate) fn insert_job_entity(session: &mut DbSession, job: &CmmnJob) -> Result<(), CmmnError> {
    let manager = CmmnJobDataManager::new();
    let mut entity = CmmnJobEntity::new(
        job.id.clone(),
        job.family.as_str().to_string(),
        job.state.clone(),
        job.scope_id.clone().unwrap_or_default(),
        job.scope_definition_id.clone().unwrap_or_default(),
        job.element_id.clone().unwrap_or_default(),
        job.retries,
        job.created_at.timestamp_millis(),
        serde_json::to_string(job)?,
    );
    entity.set_sub_scope_id(job.sub_scope_id.clone());
    entity.set_tenant_id(job.tenant_id.clone());
    entity.set_due_date(job.due_date.map(|value| value.to_rfc3339()));
    entity.set_lock_owner(job.lock_owner.clone());
    entity.set_exception_message(job.exception_message.clone());
    entity.set_exception_stacktrace(job.exception_stacktrace.clone());
    manager.insert(session, entity)?;
    Ok(())
}

fn move_suspended_job_to_executable_job_session(
    session: &mut DbSession,
    job_id: &str,
) -> Result<CmmnJob, CmmnError> {
    let suspended = load_job(session, job_id)?;
    if suspended.family != CmmnJobFamily::Suspended {
        return Err(CmmnError::not_found(format!(
            "No suspended job found with id '{job_id}'"
        )));
    }

    ensure_cmmn_job_parent_allows_activation(session, &suspended)?;

    let mut executable = suspended;
    executable.family = CmmnJobFamily::Executable;
    executable.state = CmmnJobFamily::Executable.as_str().to_string();
    // Locks are cleared on activation (parity with BPMN suspended activation).
    executable.lock_owner = None;

    // Java deletes the suspended-table row then inserts an executable-table row
    // (often same id). Rust uses one ACT_CMMN_JOB table; session flush runs
    // inserts before deletes, so a pending delete+insert of the same id collides.
    // Perform an immediate DELETE then stage the executable INSERT in this txn.
    let mut params = DbParams::new();
    params.push(job_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_JOB WHERE ID_ = ?".to_string(),
        params,
    ))?;
    insert_job_entity(session, &executable)?;
    Ok(executable)
}

fn delete_suspended_job_session(session: &mut DbSession, job_id: &str) -> Result<(), CmmnError> {
    let job = load_job(session, job_id)
        .map_err(|_| CmmnError::not_found(format!("No suspended job found with id '{job_id}'")))?;
    if job.family != CmmnJobFamily::Suspended {
        return Err(CmmnError::not_found(format!(
            "No suspended job found with id '{job_id}'"
        )));
    }
    delete_job_entity(session, job_id)
}

enum TimerRescheduleValue<'a> {
    DueDate(DateTime<Utc>),
    DateValue(&'a str),
}

fn reschedule_timer_job_session(
    session: &mut DbSession,
    job_id: &str,
    value: TimerRescheduleValue<'_>,
) -> Result<CmmnJob, CmmnError> {
    let timer = load_job(session, job_id)
        .map_err(|_| CmmnError::not_found(format!("CMMN timer job '{job_id}' was not found")))?;
    if timer.family != CmmnJobFamily::Timer {
        return Err(CmmnError::not_found(format!(
            "CMMN timer job '{job_id}' was not found"
        )));
    }

    // Java RescheduleTimerJobCmd.java:78-88 resolves the timer first and then requires
    // its sub-scope plan-item instance. Rust timer rows currently store the definition's
    // plan-item id in SUB_SCOPE_ID_, so accept either the mirror id or its plan_item_id.
    ensure_live_timer_plan_item_instance(session, &timer)?;

    let now = Utc::now();
    let (due_date, repeat) = resolve_reschedule_value(value, now, &timer)?;

    // Java RescheduleTimerJobCmd.java:127-147 creates a fresh timer entity, copies the
    // scheduling identity fields, deletes the old row and schedules the new row. Keep
    // that new-id contract even though all Rust CMMN job families share one table.
    let mut rebuilt = timer.clone();
    rebuilt.id = format!("cmmn-job:{}", Uuid::new_v4());
    rebuilt.family = CmmnJobFamily::Timer;
    rebuilt.state = CmmnJobFamily::Timer.as_str().to_string();
    rebuilt.due_date = Some(due_date);
    rebuilt.retries = DEFAULT_ASYNC_EXECUTOR_RETRIES;
    rebuilt.created_at = now;
    rebuilt.lock_owner = None;
    rebuilt.exception_message = None;
    rebuilt.exception_stacktrace = None;
    rebuilt.configuration =
        repeat.map(|repeat| serde_json::json!({ TIMER_JOB_CONFIG_REPEAT_KEY: repeat }).to_string());

    delete_job_entity(session, job_id)?;
    insert_job_entity(session, &rebuilt)?;
    Ok(rebuilt)
}

fn ensure_live_timer_plan_item_instance(
    session: &mut DbSession,
    timer: &CmmnJob,
) -> Result<(), CmmnError> {
    let plan_item_instance_id = timer
        .sub_scope_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| CmmnError::execution("Plan item instance id is missing from timer job"))?;
    let rows = session.select_list(StatementId::SelectAllCmmnPlanItemInstances, DbParams::new())?;
    let found = rows.iter().try_fold(false, |found, row| {
        if found {
            return Ok::<bool, CmmnError>(true);
        }
        let data = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN plan item instance row"))?;
        let instance: CmmnPlanItemInstance = serde_json::from_str(&data)?;
        Ok(instance.ended_at.is_none()
            && (instance.id == plan_item_instance_id
                || instance.plan_item_id == plan_item_instance_id))
    })?;
    if !found {
        return Err(CmmnError::execution(format!(
            "Plan item instance not found for id {plan_item_instance_id}"
        )));
    }
    Ok(())
}

fn resolve_reschedule_value(
    value: TimerRescheduleValue<'_>,
    now: DateTime<Utc>,
    timer: &CmmnJob,
) -> Result<(DateTime<Utc>, Option<String>), CmmnError> {
    if let TimerRescheduleValue::DueDate(due_date) = value {
        // Java RescheduleTimerJobCmd.java:92-94 uses a typed date verbatim.
        return Ok((due_date, None));
    }
    let TimerRescheduleValue::DateValue(raw) = value else {
        unreachable!("typed date returned above")
    };
    let expression = raw.trim();

    // Java RescheduleTimerJobCmd.java:97-102 dispatches durations to the due-date
    // calendar and R expressions to the cycle calendar, marking only the latter repeatable.
    if expression.starts_with('P') {
        return resolve_timer_due(expression, now)
            .map(|due| (due, None))
            .ok_or_else(|| unresolved_timer_expression(expression, timer));
    }
    if expression.starts_with('R') {
        return resolve_timer_due(expression, now)
            .map(|due| (due, Some(prepare_repeat(expression, now))))
            .ok_or_else(|| unresolved_timer_expression(expression, timer));
    }

    // Java RescheduleTimerJobCmd.java:104-116 first tries ISO-8601 parsing, then the
    // cycle business calendar for Quartz cron and marks a successful cron as repeating.
    if let Some(due) = parse_date_time(expression) {
        return Ok((due, None));
    }
    next_cron_after(expression, now)
        .map(|due| (due, Some(expression.to_string())))
        .ok_or_else(|| unresolved_timer_expression(expression, timer))
}

fn unresolved_timer_expression(expression: &str, timer: &CmmnJob) -> CmmnError {
    // Java RescheduleTimerJobCmd.java:119-121 rejects any value that neither calendar
    // nor the ISO parser resolves; leaving this as an error keeps the old row transactional.
    CmmnError::validation(format!(
        "Timer expression '{expression}' did not resolve to a date for CMMN timer job '{}'",
        timer.id
    ))
}

#[derive(Clone, Copy)]
enum DeadletterDestination {
    ByType,
    Executable,
    History,
}

fn move_deadletter_job_session(
    session: &mut DbSession,
    job_id: &str,
    retries: i32,
    destination: DeadletterDestination,
) -> Result<CmmnJob, CmmnError> {
    let mut job = load_job(session, job_id).map_err(|_| {
        CmmnError::not_found(format!("CMMN deadletter job '{job_id}' was not found"))
    })?;
    if job.family != CmmnJobFamily::Deadletter {
        return Err(CmmnError::not_found(format!(
            "CMMN deadletter job '{job_id}' was not found"
        )));
    }

    let is_history = job.job_type.as_deref() == Some("history");
    // Java JobResource.java:311-323 sends ordinary `move` to history only when jobType
    // is `history`; DefaultJobManager.java:268-301 enforces both forced destinations.
    let target_family = match destination {
        DeadletterDestination::ByType if is_history => CmmnJobFamily::History,
        DeadletterDestination::ByType => CmmnJobFamily::Executable,
        DeadletterDestination::Executable if is_history => {
            return Err(CmmnError::validation(
                "Cannot move a history job to an executable job",
            ));
        }
        DeadletterDestination::Executable => CmmnJobFamily::Executable,
        DeadletterDestination::History if !is_history => {
            return Err(CmmnError::validation(
                "Can only move a history job to a history job",
            ));
        }
        DeadletterDestination::History => CmmnJobFamily::History,
    };

    job.family = target_family.clone();
    job.state = target_family.as_str().to_string();
    job.retries = retries;
    job.lock_owner = None;

    // Java DefaultJobManager.java:281-318 copies the row with the same id into another
    // family table and deletes the deadletter row. Rust uses one ACT_CMMN_JOB table, so
    // perform the equivalent immediate delete + same-id insert inside this transaction.
    let mut params = DbParams::new();
    params.push(job_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_JOB WHERE ID_ = ?".to_string(),
        params,
    ))?;
    insert_job_entity(session, &job)?;
    Ok(job)
}

fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let start = start.min(total);
    let size = size.unwrap_or(total.saturating_sub(start));
    let data = items.into_iter().skip(start).take(size).collect::<Vec<_>>();
    PagedResult {
        start,
        size: data.len(),
        total,
        data,
    }
}
