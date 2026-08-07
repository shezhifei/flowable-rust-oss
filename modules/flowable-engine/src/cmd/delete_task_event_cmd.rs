use crate::history::historic_entities::HistoricTaskEvent;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;

pub struct DeleteTaskEventCmd {
    task_id: String,
    event_id: String,
}

impl DeleteTaskEventCmd {
    pub fn new(task_id: String, event_id: String) -> Self {
        Self { task_id, event_id }
    }
}

impl Command<Option<HistoricTaskEvent>> for DeleteTaskEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<HistoricTaskEvent>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();

        let event = match store.find_historic_task_event(&self.event_id, session) {
            Some(event) => event,
            None => return Ok(None),
        };

        if event.task_id != self.task_id {
            return Ok(None);
        }

        store.delete_historic_task_event(&self.event_id, session);
        Ok(Some(event))
    }
}
