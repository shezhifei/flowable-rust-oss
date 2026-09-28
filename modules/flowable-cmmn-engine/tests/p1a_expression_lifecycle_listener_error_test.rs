//! P1-A (CMMN) — an `expression` plan-item / case lifecycle listener must
//! propagate expression evaluation failures instead of discarding them.
//!
//! Java evidence: `ExpressionPlanItemLifecycleListener.java:46-48` —
//! `stateChanged` calls `expression.getValue(planItemInstance)` with no
//! try/catch; the return value is ignored but an evaluation exception escapes
//! through `CmmnListenerNotificationHelper.executeLifecycleListeners` (which
//! does not catch) and rolls the transition command back.
//!
//! Undefined variables stay lenient null and must not fail the transition;
//! only a registered expression method that returns an error propagates as
//! `CmmnError::Execution`.

use flowable_cmmn_engine::{
    CmmnCaseInstanceStartRequest, CmmnEngine, CmmnHumanTaskCompletionRequest, CmmnHumanTaskState,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct AuditBean {
    entries: Arc<Mutex<Vec<String>>>,
}

impl AuditBean {
    fn install(&self, engine: &CmmnEngine) {
        let entries = Arc::clone(&self.entries);
        engine.register_lifecycle_listener_expression_method("auditBean", "record", move |args| {
            let entry = args
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            entries.lock().expect("entries").push(entry);
            Ok(Value::Bool(true))
        });
        engine.register_lifecycle_listener_expression_method(
            "auditBean",
            "failUnconditionally",
            |_| Err("cmmn bean deliberately rejected this listener".to_string()),
        );
    }

    fn entries(&self) -> Vec<String> {
        self.entries.lock().expect("entries").clone()
    }
}

fn deploy(engine: &CmmnEngine, name: &str, xml: &str) {
    engine
        .repository_service()
        .new_deployment()
        .name(name)
        .add_string("lifecycle.cmmn", xml)
        .expect("add cmmn")
        .deploy()
        .expect("deploy");
}

fn only_active_task_id(engine: &CmmnEngine, case_instance_id: &str) -> String {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .next()
        .expect("exactly one active task")
        .id
}

const FAILING_EXPRESSION_LISTENER_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="failingExpressionListenerCase" name="Failing expression listener case">
    <extensionElements>
      <flowable:caseLifecycleListener
          expression="${auditBean.failUnconditionally()}" />
    </extensionElements>
    <casePlanModel id="planModel">
      <planItem id="planItemWork" definitionRef="taskWork" />
      <humanTask id="taskWork" name="Work" />
    </casePlanModel>
  </case>
</definitions>
"#;

/// A registered bean method that fails inside the case-level `expression`
/// listener must make the completion transition fail. Before P1-A the error
/// was dropped with `let _` and the transition reported success.
#[test]
fn failing_bean_method_in_expression_lifecycle_listener_fails_transition() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let audit = AuditBean::default();
    audit.install(&engine);

    deploy(&engine, "p1a-cmmn-failing", FAILING_EXPRESSION_LISTENER_XML);
    let case_instance = engine
        .start_case_instance_by_key(
            "failingExpressionListenerCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("start case (listener fires on completion, not on start)");
    let task_id = only_active_task_id(&engine, &case_instance.id);

    let error = engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect_err("case completion must fail when the listener expression errors");

    let message = error.to_string();
    assert!(
        message.contains("auditBean.failUnconditionally"),
        "error should identify the listener expression, got: {message}"
    );
    assert!(
        message.contains("cmmn bean deliberately rejected this listener"),
        "error should carry the bean method message, got: {message}"
    );
}

/// A successful expression listener still runs for its side effect; the
/// expression value itself is discarded (mirrors the p126 success case
/// through the new strict entry).
#[test]
fn successful_bean_method_expression_lifecycle_listener_runs() {
    const XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="successfulExpressionListenerCase" name="Successful expression listener case">
    <extensionElements>
      <flowable:caseLifecycleListener expression="${auditBean.record('case-done')}" />
    </extensionElements>
    <casePlanModel id="planModel">
      <planItem id="planItemWork" definitionRef="taskWork" />
      <humanTask id="taskWork" name="Work">
        <extensionElements>
          <flowable:planItemLifecycleListener
              expression="${auditBean.record('task-done')}" />
        </extensionElements>
      </humanTask>
    </casePlanModel>
  </case>
</definitions>
"#;
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let audit = AuditBean::default();
    audit.install(&engine);

    deploy(&engine, "p1a-cmmn-ok", XML);
    let case_instance = engine
        .start_case_instance_by_key(
            "successfulExpressionListenerCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("start case");
    let task_id = only_active_task_id(&engine, &case_instance.id);
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("successful listener expressions must not fail the transition");

    assert_eq!(
        audit.entries(),
        vec!["task-done".to_string(), "case-done".to_string()]
    );
}

/// An undefined variable in an expression listener resolves to lenient null
/// and the transition succeeds — the aligned Java behaviour for an
/// unresolved variable, as opposed to a method failure.
#[test]
fn undefined_variable_in_expression_lifecycle_listener_is_lenient_null() {
    const XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="lenientUndefinedExpressionListenerCase" name="Lenient undefined case">
    <extensionElements>
      <flowable:caseLifecycleListener expression="${thisVariableIsNeverDefined}" />
    </extensionElements>
    <casePlanModel id="planModel">
      <planItem id="planItemWork" definitionRef="taskWork" />
      <humanTask id="taskWork" name="Work" />
    </casePlanModel>
  </case>
</definitions>
"#;
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let audit = AuditBean::default();
    audit.install(&engine);

    deploy(&engine, "p1a-cmmn-undefined", XML);
    let case_instance = engine
        .start_case_instance_by_key(
            "lenientUndefinedExpressionListenerCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("start case");
    let task_id = only_active_task_id(&engine, &case_instance.id);
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("undefined variable must stay lenient null, not fail the transition");
    assert!(audit.entries().is_empty());
}
