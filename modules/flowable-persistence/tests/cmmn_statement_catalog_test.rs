use flowable_persistence::{
    DbParams, FlowableStatementCatalog, SqliteDialect, StatementCatalog, StatementId,
};

fn render(statement_id: StatementId) -> String {
    let catalog = FlowableStatementCatalog::new(Box::new(SqliteDialect));
    catalog
        .render(statement_id, &SqliteDialect, &DbParams::new())
        .expect("CMMN statement should render")
        .sql
}

#[test]
fn cmmn_identity_link_cleanup_statements_are_scope_specific() {
    assert_eq!(
        render(StatementId::DeleteCmmnIdentityLinksByScopeDefinitionId),
        "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_ID_ = ? AND SCOPE_TYPE_ = 'definition'"
    );
    assert_eq!(
        render(StatementId::DeleteCmmnIdentityLinksByCaseInstanceId),
        "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_ID_ = ? AND SCOPE_TYPE_ = 'caseInstance'"
    );
    assert_eq!(
        render(StatementId::DeleteCmmnIdentityLinksByTaskId),
        "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_ID_ = ? AND SCOPE_TYPE_ = 'humanTask'"
    );
}

#[test]
fn cmmn_job_cleanup_statements_target_one_scope_column_each() {
    assert_eq!(
        render(StatementId::DeleteCmmnJobsByScopeId),
        "DELETE FROM ACT_CMMN_JOB WHERE SCOPE_ID_ = ?"
    );
    assert_eq!(
        render(StatementId::DeleteCmmnJobsBySubScopeId),
        "DELETE FROM ACT_CMMN_JOB WHERE SUB_SCOPE_ID_ = ?"
    );
    assert_eq!(
        render(StatementId::DeleteCmmnJobsByScopeDefinitionId),
        "DELETE FROM ACT_CMMN_JOB WHERE SCOPE_DEFINITION_ID_ = ?"
    );
}

#[test]
fn cmmn_case_id_lookups_and_subscription_cleanup_use_their_own_owners() {
    assert_eq!(
        render(StatementId::SelectCmmnCaseInstanceIdsByCaseDefinitionId),
        "SELECT ID_ FROM ACT_CMMN_CASE_INSTANCE WHERE CASE_DEFINITION_ID_ = ?"
    );
    assert_eq!(
        render(StatementId::SelectHistoricCmmnCaseInstanceIdsByCaseDefinitionId),
        "SELECT CASE_INSTANCE_ID_ FROM ACT_CMMN_CASE_HISTORY WHERE CASE_DEFINITION_ID_ = ?"
    );
    assert_eq!(
        render(StatementId::DeleteCmmnEventSubscriptionsByCaseDefinitionId),
        "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_DEFINITION_ID_ = ?"
    );
}

#[test]
fn cmmn_stage_instance_schema_has_no_case_definition_id_column() {
    let scripts = flowable_persistence::get_all_scripts();
    for script in scripts
        .iter()
        .filter(|script| script.component == "ACT_CMMN_STAGE_INSTANCE")
    {
        assert!(
            !script.sql.contains("CASE_DEFINITION_ID_"),
            "{} stage schema must not gain an invalid definition column",
            script.database_type
        );
    }
}

#[test]
fn production_statement_catalog_contains_no_panic_macros() {
    let source = include_str!("../src/statement_catalog.rs");
    assert!(!source.contains(&["unreachable!", "("].concat()));
    assert!(!source.contains(&["panic!", "("].concat()));
}
