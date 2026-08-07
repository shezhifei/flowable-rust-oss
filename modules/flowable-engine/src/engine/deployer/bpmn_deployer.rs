use crate::engine::deployment_manager::DeploymentManager;
use crate::error::FlowableError;
use crate::persistence::db_session::DbSession;
use crate::repository::deployment::Deployment;
use crate::repository::process_definition::ProcessDefinition;
use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::BpmnModel;

pub struct BpmnDeployer;

impl Default for BpmnDeployer {
    fn default() -> Self {
        Self::new()
    }
}

impl BpmnDeployer {
    pub fn new() -> Self {
        Self
    }

    pub fn deploy(
        &self,
        deployment: &Deployment,
        deployment_manager: &DeploymentManager,
        session: &mut DbSession,
    ) -> Result<Vec<(ProcessDefinition, BpmnModel)>, FlowableError> {
        let mut results = Vec::new();
        let converter = BpmnXMLConverter::new();

        for (resource_name, resource_bytes) in &deployment.resources {
            if resource_name.ends_with(".bpmn") || resource_name.ends_with(".bpmn20.xml") {
                if let Ok(xml_str) = std::str::from_utf8(resource_bytes) {
                    let bpmn_model = converter.try_convert_to_bpmn_model(xml_str)?;
                    for process in &bpmn_model.processes {
                        let process_key = process.base_element.id.clone().unwrap_or_default();
                        let version = deployment_manager.next_process_definition_version(
                            deployment.tenant_id.as_deref(),
                            &process_key,
                            session,
                        );
                        // Java DefaultHistoryConfigurationSettings
                        // .getProcessDefinitionHistoryLevel:68-73 reads
                        // process extensionElements["historyLevel"] text.
                        let history_level = process
                            .base_element
                            .extension_elements
                            .get("historyLevel")
                            .and_then(|elems| elems.first())
                            .and_then(|elem| elem.element_text.as_ref())
                            .map(|text| text.trim().to_string())
                            .filter(|text| !text.is_empty());
                        let process_definition = ProcessDefinition {
                            id: format!("{}:{}:{}", process_key, version, uuid::Uuid::new_v4()),
                            // Java BpmnParse: the definition category is the
                            // model's targetNamespace, not the deployment's.
                            category: bpmn_model
                                .target_namespace
                                .clone()
                                .or_else(|| deployment.category.clone()),
                            name: process.name.clone(),
                            key: process_key,
                            description: process.documentation.clone(),
                            version,
                            resource_name: Some(resource_name.clone()),
                            deployment_id: Some(deployment.id.clone()),
                            diagram_resource_name: None,
                            has_start_form_key: false,
                            has_graphical_notation: !bpmn_model.location_map.is_empty(),
                            is_suspended: false,
                            tenant_id: deployment.tenant_id.clone(),
                            engine_version: deployment.engine_version.clone(),
                            app_version: None,
                            history_level,
                        };
                        let mut process_model = bpmn_model.clone();
                        process_model.main_process = Some(process.clone());
                        results.push((process_definition, process_model));
                    }
                } else {
                    return Err(FlowableError::InvalidBpmnXml {
                        position: 0,
                        message: format!("resource {resource_name} is not valid UTF-8"),
                    });
                }
            }
        }

        Ok(results)
    }
}
