#[derive(Debug, Clone)]
pub struct DeploymentResource {
    pub deployment_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub created_at: i64,
}

impl DeploymentResource {
    pub fn new(
        deployment_id: String,
        resource_name: String,
        bytes: Vec<u8>,
        created_at: i64,
    ) -> Self {
        Self {
            deployment_id,
            resource_type: resource_type_for_name(&resource_name).to_string(),
            content_type: content_type_for_name(&resource_name).to_string(),
            resource_name,
            bytes,
            created_at,
        }
    }

    pub fn from_stored(
        deployment_id: String,
        resource_name: String,
        resource_type: Option<String>,
        content_type: Option<String>,
        bytes: Vec<u8>,
        created_at: Option<i64>,
    ) -> Self {
        let default_resource_type = resource_type_for_name(&resource_name).to_string();
        let default_content_type = content_type_for_name(&resource_name).to_string();
        Self {
            deployment_id,
            resource_name,
            resource_type: resource_type
                .filter(|value| !value.is_empty())
                .unwrap_or(default_resource_type),
            content_type: content_type
                .filter(|value| !value.is_empty())
                .unwrap_or(default_content_type),
            bytes,
            created_at: created_at.unwrap_or_default(),
        }
    }
}

fn resource_type_for_name(resource_name: &str) -> &'static str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".bpmn20.xml") || lower_name.ends_with(".bpmn") {
        "processDefinition"
    } else {
        "resource"
    }
}

fn content_type_for_name(resource_name: &str) -> &'static str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".bpmn20.xml")
        || lower_name.ends_with(".bpmn")
        || lower_name.ends_with(".xml")
    {
        "application/xml"
    } else if lower_name.ends_with(".json") {
        "application/json"
    } else if lower_name.ends_with(".svg") {
        "image/svg+xml"
    } else if lower_name.ends_with(".png") {
        "image/png"
    } else if lower_name.ends_with(".jpg") || lower_name.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower_name.ends_with(".gif") {
        "image/gif"
    } else if lower_name.ends_with(".txt") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}
