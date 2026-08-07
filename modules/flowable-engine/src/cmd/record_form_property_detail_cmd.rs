use crate::history::historic_entities::HistoricDetail;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use chrono::Utc;

pub struct RecordFormPropertyDetailCmd {
    process_instance_id: String,
    task_id: Option<String>,
    property_id: String,
    property_value: serde_json::Value,
}

impl RecordFormPropertyDetailCmd {
    pub fn new(
        process_instance_id: String,
        task_id: Option<String>,
        property_id: String,
        property_value: serde_json::Value,
    ) -> Self {
        Self {
            process_instance_id,
            task_id,
            property_id,
            property_value,
        }
    }
}

impl Command<HistoricDetail> for RecordFormPropertyDetailCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HistoricDetail, crate::error::FlowableError> {
        let detail = HistoricDetail {
            id: uuid::Uuid::new_v4().to_string(),
            process_instance_id: self.process_instance_id.clone(),
            execution_id: None,
            activity_instance_id: None,
            task_id: self.task_id.clone(),
            time: Utc::now(),
            detail_type: "formProperty".to_string(),
            revision: None,
            variable_name: None,
            variable_type: None,
            value: None,
            property_id: Some(self.property_id.clone()),
            property_value: Some(self.property_value.clone()),
        };
        let (store, session) = command_context.store_and_session();
        store.insert_historic_detail(detail.clone(), session);
        Ok(detail)
    }
}
