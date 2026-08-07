use crate::history::historic_entities::HistoricComment;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;

pub struct DeleteTaskCommentCmd {
    task_id: String,
    comment_id: String,
}

impl DeleteTaskCommentCmd {
    pub fn new(task_id: String, comment_id: String) -> Self {
        Self {
            task_id,
            comment_id,
        }
    }
}

impl Command<Option<HistoricComment>> for DeleteTaskCommentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<HistoricComment>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();

        let comment = match store.find_historic_comment(&self.comment_id, session) {
            Some(comment) => comment,
            None => return Ok(None),
        };

        if comment.task_id.as_deref() != Some(&self.task_id) {
            return Ok(None);
        }

        store.delete_historic_comment(&self.comment_id, session);
        Ok(Some(comment))
    }
}
