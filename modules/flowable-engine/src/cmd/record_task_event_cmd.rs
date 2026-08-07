use crate::history::historic_entities::HistoricTaskEvent;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use chrono::Utc;

pub struct RecordTaskEventCmd {
    task_id: String,
    action: String,
    message: Vec<String>,
    user_id: Option<String>,
}

impl RecordTaskEventCmd {
    pub fn new(
        task_id: String,
        action: String,
        message: Vec<String>,
        user_id: Option<String>,
    ) -> Self {
        Self {
            task_id,
            action,
            message,
            user_id,
        }
    }
}

impl Command<HistoricTaskEvent> for RecordTaskEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HistoricTaskEvent, crate::error::FlowableError> {
        let event = HistoricTaskEvent {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: self.task_id.clone(),
            action: self.action.clone(),
            message: self.message.clone(),
            user_id: self.user_id.clone(),
            time: Utc::now(),
        };
        let (store, session) = command_context.store_and_session();
        store.insert_historic_task_event(event.clone(), session);
        Ok(event)
    }
}
