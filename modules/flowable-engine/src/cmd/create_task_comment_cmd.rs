use crate::history::historic_entities::{HistoricComment, HistoricTaskEvent};
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use chrono::Utc;

pub struct CreateTaskCommentCmd {
    task_id: String,
    process_instance_id: Option<String>,
    /// Java `AddCommentCmd.type`; `None` → `CommentEntity.TYPE_COMMENT`.
    comment_type: Option<String>,
    message: String,
    author: Option<String>,
}

impl CreateTaskCommentCmd {
    pub fn new(
        task_id: String,
        process_instance_id: Option<String>,
        message: String,
        author: Option<String>,
    ) -> Self {
        Self::with_type(task_id, process_instance_id, None, message, author)
    }

    pub fn with_type(
        task_id: String,
        process_instance_id: Option<String>,
        comment_type: Option<String>,
        message: String,
        author: Option<String>,
    ) -> Self {
        Self {
            task_id,
            process_instance_id,
            comment_type,
            message,
            author,
        }
    }
}

/// Java `AddCommentCmd` (flowable-engine/.../AddCommentCmd.java:107-110):
/// collapses whitespace with `message.replaceAll("\\s+", " ")`, then if the
/// result is longer than 163 characters truncates to `substring(0, 160) + "..."`.
/// The full original message is stored on the comment; only the event message
/// uses this normalized form.
pub fn normalize_comment_event_message(message: &str) -> String {
    let mut collapsed = String::with_capacity(message.len());
    let mut prev_space = false;
    for ch in message.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
                prev_space = true;
            }
        } else {
            collapsed.push(ch);
            prev_space = false;
        }
    }
    // Java `String.substring` is UTF-16 code-unit based; for BMP text (ASCII /
    // common BMP Unicode) char indices match. Contract tests cover ASCII cases.
    if collapsed.chars().count() > 163 {
        let truncated: String = collapsed.chars().take(160).collect();
        format!("{truncated}...")
    } else {
        collapsed
    }
}

impl Command<HistoricComment> for CreateTaskCommentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HistoricComment, crate::error::FlowableError> {
        // Java `AddCommentCmd` validates both referenced entities before
        // inserting the comment. Keep the validation in this command so direct
        // service callers have the same contract as REST callers.
        // Note: empty / whitespace-only messages are accepted (Java only
        // rejects null at the REST layer; engine has no empty check).
        let (store, session) = command_context.store_and_session();
        let task = store.find_task(&self.task_id, session).ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "Cannot find task with id {}",
                self.task_id
            ))
        })?;
        if task.is_suspended() {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Cannot add a comment to a suspended task '{}'",
                self.task_id
            )));
        }
        if let Some(process_instance_id) = &self.process_instance_id {
            let process_instance = store
                .find_process_instance(process_instance_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "execution {} doesn't exist",
                        process_instance_id
                    ))
                })?;
            if process_instance.is_suspended {
                return Err(crate::error::FlowableError::ExecutionError(format!(
                    "Cannot add a comment to a suspended process instance '{}'",
                    process_instance_id
                )));
            }
        }

        // Comment stores the original full message (Java `setFullMessage`).
        // Type defaults to TYPE_COMMENT when not supplied (Java AddCommentCmd).
        let resolved_type = self
            .comment_type
            .clone()
            .unwrap_or_else(|| HistoricComment::TYPE_COMMENT.to_string());
        let comment = HistoricComment {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: Some(self.task_id.clone()),
            process_instance_id: self.process_instance_id.clone(),
            message: self.message.clone(),
            author: self.author.clone(),
            time: Utc::now(),
            // User comment: Java leaves `action` null on the entity for non-event
            // audit rows; Rust keeps TYPE_EVENT process audit in `action` only.
            action: None,
            comment_type: Some(resolved_type),
        };

        // Event message uses the normalized/truncated form (Java `setMessage`).
        // HistoricTaskEvent stays a separate table (not a TYPE_EVENT comment).
        let event_message = normalize_comment_event_message(&self.message);
        let event = HistoricTaskEvent {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: self.task_id.clone(),
            action: "AddComment".to_string(),
            message: vec![event_message],
            user_id: self.author.clone(),
            time: Utc::now(),
        };

        let (store, session) = command_context.store_and_session();
        store.insert_historic_comment(comment.clone(), session);
        store.insert_historic_task_event(event, session);
        Ok(comment)
    }
}

/// Java `AddCommentCmd` with `taskId == null` (the path used by
/// `HistoricProcessInstanceCommentCollectionResource`): only the runtime
/// execution is validated (missing → `FlowableObjectNotFoundException`
/// "execution {id} doesn't exist", suspended → `FlowableException`), the
/// comment is stored with a `null` task id and no task event is recorded.
pub struct CreateProcessInstanceCommentCmd {
    process_instance_id: String,
    /// Java `AddCommentCmd.type`; `None` → `CommentEntity.TYPE_COMMENT`.
    comment_type: Option<String>,
    message: String,
    author: Option<String>,
}

impl CreateProcessInstanceCommentCmd {
    pub fn new(process_instance_id: String, message: String, author: Option<String>) -> Self {
        Self::with_type(process_instance_id, None, message, author)
    }

    pub fn with_type(
        process_instance_id: String,
        comment_type: Option<String>,
        message: String,
        author: Option<String>,
    ) -> Self {
        Self {
            process_instance_id,
            comment_type,
            message,
            author,
        }
    }
}

impl Command<HistoricComment> for CreateProcessInstanceCommentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HistoricComment, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        let process_instance = store
            .find_process_instance(&self.process_instance_id, session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "execution {} doesn't exist",
                    self.process_instance_id
                ))
            })?;
        if process_instance.is_suspended {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Cannot add a comment to a suspended process instance '{}'",
                self.process_instance_id
            )));
        }

        let resolved_type = self
            .comment_type
            .clone()
            .unwrap_or_else(|| HistoricComment::TYPE_COMMENT.to_string());
        let comment = HistoricComment {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: None,
            process_instance_id: Some(self.process_instance_id.clone()),
            message: self.message.clone(),
            author: self.author.clone(),
            time: Utc::now(),
            action: None,
            comment_type: Some(resolved_type),
        };
        store.insert_historic_comment(comment.clone(), session);
        Ok(comment)
    }
}

/// Java `SaveCommentCmd`: updates an existing comment in place. Preserves id
/// and association fields supplied on the entity; type and full message may
/// change. Does not rewrite task events.
pub struct SaveCommentCmd {
    comment: HistoricComment,
}

impl SaveCommentCmd {
    pub fn new(comment: HistoricComment) -> Self {
        Self { comment }
    }
}

impl Command<()> for SaveCommentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        if self.comment.id.is_empty() {
            return Err(crate::error::FlowableError::BadRequest(
                "comment id is null".to_string(),
            ));
        }
        let (store, session) = command_context.store_and_session();
        if store
            .find_historic_comment(&self.comment.id, session)
            .is_none()
        {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Comment '{}' was not found",
                self.comment.id
            )));
        }
        // Upsert preserves id; projected columns (including type) are refreshed.
        store.insert_historic_comment(self.comment.clone(), session);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::process_engine::ProcessEngine;
    use crate::runtime::process_instance::ProcessInstance;
    use crate::task::Task;

    #[test]
    fn missing_task_is_rejected_before_comment_insert() {
        let engine = ProcessEngine::new("comment-missing-task".to_string());

        let error = engine
            .get_history_service()
            .create_task_comment("missing", None, "comment", None)
            .expect_err("Java AddCommentCmd requires the task to exist");

        assert!(matches!(error, crate::error::FlowableError::NotFound(_)));
        let mut session = engine.get_runtime_store().create_session().unwrap();
        assert!(
            engine
                .get_history_service()
                .get_task_comments("missing", &mut session)
                .is_empty()
        );
        session.rollback().unwrap();
    }

    #[test]
    fn suspended_task_is_rejected_before_comment_insert() {
        let engine = ProcessEngine::new("comment-suspended-task".to_string());
        let store = engine.get_runtime_store();
        let mut task = task("task-1", "process-1");
        task.set_suspension_state(true);
        let mut session = store.create_session().unwrap();
        store.insert_task(&task, &mut session);
        session.flush_and_commit().unwrap();

        let error = engine
            .get_history_service()
            .create_task_comment("task-1", None, "comment", None)
            .expect_err("Java AddCommentCmd rejects suspended tasks");

        assert!(error.to_string().contains("suspended task"));
    }

    #[test]
    fn suspended_process_instance_is_validated_independently() {
        let engine = ProcessEngine::new("comment-suspended-process".to_string());
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_task(&task("task-1", "process-1"), &mut session);
        store.insert_process_instance(&process_instance("process-1", true), &mut session);
        session.flush_and_commit().unwrap();

        let error = engine
            .get_history_service()
            .create_task_comment("task-1", Some("process-1"), "comment", None)
            .expect_err("Java AddCommentCmd validates the supplied execution independently");

        assert!(error.to_string().contains("suspended process instance"));
    }

    #[test]
    fn process_instance_comment_requires_runtime_execution() {
        let engine = ProcessEngine::new("comment-pi-missing-execution".to_string());

        let error = engine
            .get_history_service()
            .create_process_instance_comment("missing-pi", "comment", None)
            .expect_err("Java AddCommentCmd requires the runtime execution to exist");

        assert!(matches!(error, crate::error::FlowableError::NotFound(_)));
        assert!(
            error
                .to_string()
                .contains("execution missing-pi doesn't exist")
        );
    }

    #[test]
    fn process_instance_comment_rejects_suspended_instance() {
        let engine = ProcessEngine::new("comment-pi-suspended".to_string());
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", true), &mut session);
        session.flush_and_commit().unwrap();

        let error = engine
            .get_history_service()
            .create_process_instance_comment("process-1", "comment", None)
            .expect_err("Java AddCommentCmd rejects suspended process instances");

        assert!(error.to_string().contains("suspended process instance"));
    }

    fn task(id: &str, process_instance_id: &str) -> Task {
        Task::new(
            id.to_string(),
            process_instance_id.to_string(),
            process_instance_id.to_string(),
            "task-definition".to_string(),
            "Review".to_string(),
        )
    }

    fn process_instance(id: &str, is_suspended: bool) -> ProcessInstance {
        ProcessInstance {
            id: id.to_string(),
            name: None,
            process_definition_id: "definition-1".to_string(),
            process_definition_key: "definition".to_string(),
            process_definition_name: None,
            process_definition_version: 1,
            business_key: None,
            business_status: None,
            is_suspended,
            tenant_id: None,
            start_time: None,
            start_user_id: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            is_ended: false,
            super_execution_id: None,
            root_process_instance_id: Some(id.to_string()),
        }
    }
}
