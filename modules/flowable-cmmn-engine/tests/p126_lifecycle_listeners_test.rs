//! P126 — end-to-end: CMMN lifecycle listeners declared in XML fire on state transitions.
//!
//! Java references:
//! - CmmnXmlConstants.java:64-65 — the two element names, `caseLifecycleListener` (on the case)
//!   and `planItemLifecycleListener` (on any plan item definition).
//! - CaseInstanceLifeCycleListenerUtil.java:35-85 — case instance notification: same-state early
//!   return (:36-38), listeners off the case model (:41-42), state filter (:48, :76-78).
//! - CmmnListenerNotificationHelper.java:103-160 — plan item twin, plus the class /
//!   delegateExpression resolution at :162-169.
//! - AbstractChangeCaseInstanceStateOperation.java:45,47 — fire, then assign the new state.
//!
//! 勘误 (correction to the P126 brief): Java has TWO lifecycle listener elements, not three.
//! `stageLifecycleListener` and `TaskLifecycleListener` do not exist anywhere in the Java source
//! tree — a stage takes `planItemLifecycleListener` because `Stage extends PlanItemDefinition`
//! (AbstractPlanItemDefinitionExport.java:113 writes that element for every plan item
//! definition). The "三型" this file exercises are the three *implementation* attributes:
//! class / expression / delegateExpression.
//!
//! Deviation exercised here: Rust's `SimpleExpression` is read-only (stated verbatim in the BPMN
//! precedent, execution_listener_util.rs), so an `expression` listener cannot write a case
//! variable the way Java's UEL can. Its side-effect channel is a bean method registered on the
//! engine (`${auditBean.record(...)}`), asserted below.

use flowable_cmmn_engine::{
    CmmnCaseInstanceStartRequest, CmmnEngine, CmmnError, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnLifecycleListenerContext, CmmnLifecycleListenerHandler,
    CmmnLifecycleScope,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// One recorded `stateChanged` call, flattened for assertion.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Call {
    scope: &'static str,
    definition_id: Option<String>,
    old_state: String,
    new_state: String,
}

/// Test stand-in for a Java `CaseInstanceLifecycleListener` / `PlanItemInstanceLifecycleListener`
/// implementation class; records every notification it receives.
#[derive(Clone, Default)]
struct RecordingListener {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl RecordingListener {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls").clone()
    }
}

impl CmmnLifecycleListenerHandler for RecordingListener {
    fn state_changed(&self, context: &CmmnLifecycleListenerContext) -> Result<(), CmmnError> {
        self.calls.lock().expect("calls").push(Call {
            scope: match context.scope {
                CmmnLifecycleScope::CaseInstance => "case",
                CmmnLifecycleScope::PlanItem => "planItem",
            },
            definition_id: context.plan_item_definition_id.clone(),
            old_state: context.old_state.clone(),
            new_state: context.new_state.clone(),
        });
        Ok(())
    }
}

/// Side-effect sink for `expression` listeners: `${auditBean.record('...')}` appends its argument.
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

fn active_task_id(engine: &CmmnEngine, case_instance_id: &str, name: &str) -> String {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|task| task.name == name)
        .unwrap_or_else(|| panic!("no active task named {name}"))
        .id
}

// ── class ────────────────────────────────────────────────────────────────────

const CLASS_LISTENER_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="classListenerCase" name="Class listener case">
    <extensionElements>
      <flowable:caseLifecycleListener class="com.example.CaseAudit" />
    </extensionElements>
    <casePlanModel id="planModel">
      <planItem id="planItemWork" definitionRef="taskWork" />
      <humanTask id="taskWork" name="Work">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.TaskAudit" />
        </extensionElements>
      </humanTask>
    </casePlanModel>
  </case>
</definitions>
"#;

/// A `class` listener resolves through the engine's name → handler registry — Rust's minimal
/// stand-in for Java instantiating the class (CmmnListenerNotificationHelper.java:162-169).
#[test]
fn class_listener_is_invoked_on_case_and_plan_item_transitions() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_listener = RecordingListener::default();
    let task_listener = RecordingListener::default();
    engine.register_lifecycle_listener("com.example.CaseAudit", Arc::new(case_listener.clone()));
    engine.register_lifecycle_listener("com.example.TaskAudit", Arc::new(task_listener.clone()));

    deploy(&engine, "p126-class", CLASS_LISTENER_XML);
    let case_instance = engine
        .start_case_instance_by_key("classListenerCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case");

    // The only task completing drives the case to completed, so both listeners fire.
    let task_id = active_task_id(&engine, &case_instance.id, "Work");
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task");

    assert_eq!(
        task_listener.calls(),
        vec![Call {
            scope: "planItem",
            definition_id: Some("taskWork".to_string()),
            old_state: "active".to_string(),
            new_state: "completed".to_string(),
        }],
        "plan item listener sees the active → completed transition"
    );
    assert_eq!(
        case_listener.calls(),
        vec![Call {
            scope: "case",
            definition_id: None,
            old_state: "active".to_string(),
            new_state: "completed".to_string(),
        }],
        "case listener sees the case instance completing"
    );
}

// ── delegateExpression ───────────────────────────────────────────────────────

const DELEGATE_LISTENER_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="delegateListenerCase" name="Delegate listener case">
    <extensionElements>
      <flowable:caseLifecycleListener delegateExpression="${caseAuditBean}" />
    </extensionElements>
    <casePlanModel id="planModel">
      <planItem id="planItemWork" definitionRef="taskWork" />
      <humanTask id="taskWork" name="Work" />
    </casePlanModel>
  </case>
</definitions>
"#;

/// A `delegateExpression` resolves the bean name inside `${…}` — Java looks it up in the Spring
/// context, Rust in the same name → handler registry.
#[test]
fn delegate_expression_listener_resolves_the_bean_name_inside_the_wrapper() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let listener = RecordingListener::default();
    engine.register_lifecycle_listener("caseAuditBean", Arc::new(listener.clone()));

    deploy(&engine, "p126-delegate", DELEGATE_LISTENER_XML);
    let case_instance = engine
        .start_case_instance_by_key("delegateListenerCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case");
    let task_id = active_task_id(&engine, &case_instance.id, "Work");
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task");

    assert_eq!(
        listener.calls(),
        vec![Call {
            scope: "case",
            definition_id: None,
            old_state: "active".to_string(),
            new_state: "completed".to_string(),
        }]
    );
}

/// Java would fail to resolve an unknown bean and let the exception roll the command back
/// (CmmnListenerNotificationHelper.java:145-152 does not catch), so the transition fails here too.
#[test]
fn unregistered_listener_name_fails_the_transition() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p126-unregistered", DELEGATE_LISTENER_XML);
    let case_instance = engine
        .start_case_instance_by_key("delegateListenerCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case");
    let task_id = active_task_id(&engine, &case_instance.id, "Work");

    let error = engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect_err("no handler registered for caseAuditBean");
    assert!(
        error.to_string().contains("caseAuditBean"),
        "unexpected error: {error}"
    );
}

// ── expression ───────────────────────────────────────────────────────────────

const EXPRESSION_LISTENER_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="expressionListenerCase" name="Expression listener case">
    <extensionElements>
      <flowable:caseLifecycleListener expression="${auditBean.record('case-completed')}" />
    </extensionElements>
    <casePlanModel id="planModel">
      <planItem id="planItemWork" definitionRef="taskWork" />
      <humanTask id="taskWork" name="Work">
        <extensionElements>
          <flowable:planItemLifecycleListener
              expression="${auditBean.record('task-completed')}" />
        </extensionElements>
      </humanTask>
    </casePlanModel>
  </case>
</definitions>
"#;

/// Java's `ExpressionPlanItemLifecycleListener.stateChanged` evaluates the expression and drops
/// the value; only the side effect matters. Rust's `SimpleExpression` is read-only, so the side
/// effect comes from a registered bean method rather than a variable write (documented deviation).
#[test]
fn expression_listener_side_effect_runs_on_both_scopes() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let audit = AuditBean::default();
    audit.install(&engine);

    deploy(&engine, "p126-expression", EXPRESSION_LISTENER_XML);
    let case_instance = engine
        .start_case_instance_by_key("expressionListenerCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case");
    let task_id = active_task_id(&engine, &case_instance.id, "Work");
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task");

    // Plan item first: the task transition is notified before the case completes.
    assert_eq!(audit.entries(), vec!["task-completed", "case-completed"]);
}

// ── state filters ────────────────────────────────────────────────────────────

const STATE_FILTER_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="stateFilterCase" name="State filter case">
    <casePlanModel id="planModel">
      <planItem id="planItemWork" definitionRef="taskWork" />
      <humanTask id="taskWork" name="Work">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.Hit"
              sourceState="active" targetState="completed" />
          <flowable:planItemLifecycleListener class="com.example.MissSource"
              sourceState="available" targetState="completed" />
          <flowable:planItemLifecycleListener class="com.example.MissTarget"
              sourceState="active" targetState="terminated" />
        </extensionElements>
      </humanTask>
    </casePlanModel>
  </case>
</definitions>
"#;

/// `sourceState`/`targetState` are compared with Java's `stateMatches`
/// (CaseInstanceLifeCycleListenerUtil.java:76-78): an empty expected value matches anything,
/// otherwise it must equal the actual state. Only the listener whose *both* filters match runs.
#[test]
fn state_filters_select_only_the_matching_listener() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let hit = RecordingListener::default();
    let miss_source = RecordingListener::default();
    let miss_target = RecordingListener::default();
    engine.register_lifecycle_listener("com.example.Hit", Arc::new(hit.clone()));
    engine.register_lifecycle_listener("com.example.MissSource", Arc::new(miss_source.clone()));
    engine.register_lifecycle_listener("com.example.MissTarget", Arc::new(miss_target.clone()));

    deploy(&engine, "p126-state-filter", STATE_FILTER_XML);
    let case_instance = engine
        .start_case_instance_by_key("stateFilterCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case");
    let task_id = active_task_id(&engine, &case_instance.id, "Work");
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task");

    assert_eq!(
        hit.calls(),
        vec![Call {
            scope: "planItem",
            definition_id: Some("taskWork".to_string()),
            old_state: "active".to_string(),
            new_state: "completed".to_string(),
        }],
        "active → completed matches both filters"
    );
    assert!(
        miss_source.calls().is_empty(),
        "sourceState=available does not match the actual source state active"
    );
    assert!(
        miss_target.calls().is_empty(),
        "targetState=terminated does not match the actual target state completed"
    );
}

// ── stage ────────────────────────────────────────────────────────────────────

const STAGE_LISTENER_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="stageListenerCase" name="Stage listener case">
    <casePlanModel id="planModel">
      <planItem id="planItemStage" definitionRef="stageA" />
      <stage id="stageA" name="Stage A">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.StageAudit" />
        </extensionElements>
        <planItem id="planItemInner" definitionRef="taskInner" />
        <humanTask id="taskInner" name="Inner" />
      </stage>
    </casePlanModel>
  </case>
</definitions>
"#;

/// A stage carries `planItemLifecycleListener` like any other plan item definition — there is no
/// separate stage element in Java. Its completion transition is notified.
#[test]
fn stage_receives_plan_item_lifecycle_listener_notifications() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let stage_listener = RecordingListener::default();
    engine.register_lifecycle_listener("com.example.StageAudit", Arc::new(stage_listener.clone()));

    deploy(&engine, "p126-stage", STAGE_LISTENER_XML);
    let case_instance = engine
        .start_case_instance_by_key("stageListenerCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case");
    let task_id = active_task_id(&engine, &case_instance.id, "Inner");
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete inner task");

    let states: Vec<(String, String)> = stage_listener
        .calls()
        .into_iter()
        .map(|call| (call.old_state, call.new_state))
        .collect();
    assert!(
        states.contains(&("active".to_string(), "completed".to_string())),
        "stage completion was not notified: {states:?}"
    );
    assert!(
        stage_listener
            .calls()
            .iter()
            .all(|call| call.definition_id.as_deref() == Some("stageA")),
        "stage notifications must carry the stage definition id"
    );
}

const MILESTONE_LISTENER_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="milestoneListenerCase" name="Milestone listener case">
    <casePlanModel id="planModel">
      <planItem id="planItemTrigger" definitionRef="taskTrigger" />
      <planItem id="planItemMilestone" definitionRef="milestoneReached">
        <entryCriterion id="entryMilestone" sentryRef="sentryTriggerComplete" />
      </planItem>
      <humanTask id="taskTrigger" name="Trigger" />
      <milestone id="milestoneReached" name="Reached">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.MilestoneAudit"
              sourceState="available" targetState="completed" />
        </extensionElements>
      </milestone>
      <sentry id="sentryTriggerComplete">
        <planItemOnPart id="onTriggerComplete" sourceRef="planItemTrigger">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

#[test]
fn milestone_listener_observes_the_materialized_available_source_state() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let listener = RecordingListener::default();
    engine.register_lifecycle_listener(
        "com.example.MilestoneAudit",
        Arc::new(listener.clone()),
    );

    deploy(&engine, "p132-milestone-listener", MILESTONE_LISTENER_XML);
    let case_instance = engine
        .start_case_instance_by_key(
            "milestoneListenerCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("start case");

    let available = engine
        .runtime_service()
        .create_plan_item_instance_query()
        .case_instance_id(&case_instance.id)
        .plan_item_definition_type("milestone")
        .single_result()
        .expect("milestone query")
        .expect("pre-materialized milestone");
    assert_eq!(available.state, "AVAILABLE");

    let trigger_id = active_task_id(&engine, &case_instance.id, "Trigger");
    engine
        .complete_human_task(&trigger_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete trigger");

    assert_eq!(
        listener.calls(),
        vec![Call {
            scope: "planItem",
            definition_id: Some("milestoneReached".to_string()),
            old_state: "available".to_string(),
            new_state: "completed".to_string(),
        }],
        "OccurPlanItemInstanceOperation.java:34-63 transitions the materialized row"
    );
}

/// A case without any listener element must not reach the registry at all — the absence of a
/// registered handler is not an error when nothing declares one.
#[test]
fn case_without_listeners_completes_untouched() {
    const PLAIN_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="plainCase" name="Plain case">
    <casePlanModel id="planModel">
      <planItem id="planItemWork" definitionRef="taskWork" />
      <humanTask id="taskWork" name="Work" />
    </casePlanModel>
  </case>
</definitions>
"#;
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p126-plain", PLAIN_XML);
    let case_instance = engine
        .start_case_instance_by_key("plainCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case");
    let task_id = active_task_id(&engine, &case_instance.id, "Work");
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task");
}
