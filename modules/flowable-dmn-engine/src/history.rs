use crate::error::DmnError;
use crate::models::{HistoricDecisionExecution, PagedResult};
use crate::store::DmnStore;
use flowable_persistence::entity::dmn_execution_history::DmnExecutionHistoryDataManager;
use flowable_persistence::row::DbRow;
use flowable_persistence::statement::RenderedStatement;
use flowable_persistence::value::DbParams;
use std::cmp::Ordering;

#[derive(Clone)]
pub struct DmnHistoryService {
    store: DmnStore,
}

impl DmnHistoryService {
    pub(crate) fn new(store: DmnStore) -> Self {
        Self { store }
    }

    pub fn create_execution_history_query(&self) -> DmnExecutionHistoryQuery {
        DmnExecutionHistoryQuery::new(self.store.clone())
    }

    pub fn delete_historic_decision_execution(&self, id: &str) -> Result<(), DmnError> {
        let mut session = self.store.create_session()?;
        let manager = DmnExecutionHistoryDataManager::new();
        if let Some(entity) = manager.find_by_id(&mut session, id)? {
            manager.delete(&mut session, &entity)?;
            session.commit()?;
            Ok(())
        } else {
            Err(DmnError::not_found(format!(
                "Historic decision execution '{}' was not found",
                id
            )))
        }
    }

    pub fn bulk_delete_historic_decision_executions(&self, ids: &[String]) -> Result<(), DmnError> {
        let mut session = self.store.create_session()?;
        let manager = DmnExecutionHistoryDataManager::new();
        for id in ids {
            if let Some(entity) = manager.find_by_id(&mut session, id)? {
                manager.delete(&mut session, &entity)?;
            }
        }
        session.commit()?;
        Ok(())
    }
}

pub struct DmnExecutionHistoryQuery {
    store: DmnStore,
    execution_id: Option<String>,
    decision_key: Option<String>,
    decision_definition_id: Option<String>,
    deployment_id: Option<String>,
    business_key: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    failed: Option<bool>,
    sort_by: DmnExecutionHistorySort,
    descending: bool,
    start: usize,
    size: Option<usize>,
}

#[derive(Clone, Copy)]
enum DmnExecutionHistorySort {
    ExecutionTime,
    ExecutionId,
    DecisionKey,
    DecisionDefinitionId,
    DeploymentId,
    BusinessKey,
    TenantId,
}

impl DmnExecutionHistoryQuery {
    fn new(store: DmnStore) -> Self {
        Self {
            store,
            execution_id: None,
            decision_key: None,
            decision_definition_id: None,
            deployment_id: None,
            business_key: None,
            tenant_id: None,
            tenant_id_like: None,
            failed: None,
            sort_by: DmnExecutionHistorySort::ExecutionTime,
            descending: false,
            start: 0,
            size: None,
        }
    }

    pub fn execution_id(mut self, execution_id: impl Into<String>) -> Self {
        self.execution_id = Some(execution_id.into());
        self
    }

    pub fn decision_key(mut self, decision_key: impl Into<String>) -> Self {
        self.decision_key = Some(decision_key.into());
        self
    }

    pub fn decision_definition_id(mut self, decision_definition_id: impl Into<String>) -> Self {
        self.decision_definition_id = Some(decision_definition_id.into());
        self
    }

    pub fn deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
    }

    pub fn business_key(mut self, business_key: impl Into<String>) -> Self {
        self.business_key = Some(business_key.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn tenant_id_like(mut self, tenant_id_like: impl Into<String>) -> Self {
        self.tenant_id_like = Some(tenant_id_like.into());
        self
    }

    pub fn failed(mut self, failed: bool) -> Self {
        self.failed = Some(failed);
        self
    }

    pub fn order_by_execution_time(mut self) -> Self {
        self.sort_by = DmnExecutionHistorySort::ExecutionTime;
        self
    }

    pub fn order_by_execution_id(mut self) -> Self {
        self.sort_by = DmnExecutionHistorySort::ExecutionId;
        self
    }

    pub fn order_by_decision_key(mut self) -> Self {
        self.sort_by = DmnExecutionHistorySort::DecisionKey;
        self
    }

    pub fn order_by_decision_definition_id(mut self) -> Self {
        self.sort_by = DmnExecutionHistorySort::DecisionDefinitionId;
        self
    }

    pub fn order_by_deployment_id(mut self) -> Self {
        self.sort_by = DmnExecutionHistorySort::DeploymentId;
        self
    }

    pub fn order_by_business_key(mut self) -> Self {
        self.sort_by = DmnExecutionHistorySort::BusinessKey;
        self
    }

    pub fn order_by_tenant_id(mut self) -> Self {
        self.sort_by = DmnExecutionHistorySort::TenantId;
        self
    }

    pub fn asc(mut self) -> Self {
        self.descending = false;
        self
    }

    pub fn desc(mut self) -> Self {
        self.descending = true;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<HistoricDecisionExecution>, DmnError> {
        let mut session = self.store.create_session()?;
        let params = DbParams::new();
        let rendered = RenderedStatement::new(
            "SELECT DATA_ FROM ACT_DMN_HI_EXECUTION ORDER BY EXECUTED_AT_ ASC, EXECUTION_ID_ ASC"
                .to_string(),
            params,
        );
        let rows = session.select_raw(rendered)?;
        let mut history: Vec<HistoricDecisionExecution> = rows
            .into_iter()
            .map(|row| history_from_row(&row))
            .collect::<Result<Vec<_>, _>>()?;

        history.retain(|item| matches_optional(&self.execution_id, &item.execution_id));
        history.retain(|item| matches_optional(&self.decision_key, &item.decision_key));
        history.retain(|item| {
            matches_optional(&self.decision_definition_id, &item.decision_definition_id)
        });
        history.retain(|item| matches_optional(&self.deployment_id, &item.deployment_id));
        history.retain(|item| {
            matches_optional_option(&self.business_key, item.business_key.as_deref())
        });
        history.retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        if let Some(ref pattern) = self.tenant_id_like {
            history.retain(|item| {
                item.tenant_id
                    .as_deref()
                    .map(|tid| sql_like_matches(tid, pattern))
                    .unwrap_or(false)
            });
        }
        // P83: filter on the explicit `failed` flag written by
        // `PersistHistoricDecisionExecutionCmd` parity (`FAILED_`), not the
        // pre-P83 heuristic of `matched_rule_count == 0` (which also matched
        // successful no-hit executions).
        if let Some(failed) = self.failed {
            history.retain(|item| item.failed == failed);
        }
        sort_history(&mut history, self.sort_by, self.descending);

        Ok(history)
    }

    pub fn single_result(&self) -> Result<Option<HistoricDecisionExecution>, DmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<HistoricDecisionExecution>, DmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

fn history_from_row(row: &DbRow) -> Result<HistoricDecisionExecution, DmnError> {
    let data = row
        .get_text("DATA_")
        .ok_or_else(|| DmnError::storage("Missing DATA_ in DMN execution history row"))?;
    serde_json::from_str(&data).map_err(DmnError::from)
}

fn sort_history(
    history: &mut [HistoricDecisionExecution],
    sort_by: DmnExecutionHistorySort,
    descending: bool,
) {
    history.sort_by(|left, right| {
        let ordering = match sort_by {
            DmnExecutionHistorySort::ExecutionTime => left.executed_at.cmp(&right.executed_at),
            DmnExecutionHistorySort::ExecutionId => left.execution_id.cmp(&right.execution_id),
            DmnExecutionHistorySort::DecisionKey => left.decision_key.cmp(&right.decision_key),
            DmnExecutionHistorySort::DecisionDefinitionId => left
                .decision_definition_id
                .cmp(&right.decision_definition_id),
            DmnExecutionHistorySort::DeploymentId => left.deployment_id.cmp(&right.deployment_id),
            DmnExecutionHistorySort::BusinessKey => left.business_key.cmp(&right.business_key),
            DmnExecutionHistorySort::TenantId => left.tenant_id.cmp(&right.tenant_id),
        };

        apply_order(ordering, descending).then_with(|| left.execution_id.cmp(&right.execution_id))
    });
}

fn apply_order(ordering: Ordering, descending: bool) -> Ordering {
    if descending {
        ordering.reverse()
    } else {
        ordering
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

fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let start = start.min(total);
    let page_size = size.unwrap_or(total.saturating_sub(start));
    let data: Vec<T> = items.into_iter().skip(start).take(page_size).collect();

    PagedResult {
        start,
        size: data.len(),
        total,
        data,
    }
}

/// Local signature is `(candidate, pattern)`; shared impl is `(pattern, value)`.
fn sql_like_matches(candidate: &str, pattern: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, candidate)
}
