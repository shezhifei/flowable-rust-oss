#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BpmnLayoutError {
    MissingMainProcess,
    UnsupportedModelFeature {
        feature: &'static str,
        element_id: Option<String>,
        detail: String,
    },
    UnsupportedOption {
        option: &'static str,
        detail: String,
    },
    InvalidModel {
        detail: String,
    },
}

impl std::fmt::Display for BpmnLayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingMainProcess => write!(f, "BPMN model does not contain a main process"),
            Self::UnsupportedModelFeature {
                feature,
                element_id,
                detail,
            } => write!(
                f,
                "unsupported BPMN layout feature '{feature}' for element {:?}: {detail}",
                element_id
            ),
            Self::UnsupportedOption { option, detail } => {
                write!(f, "unsupported BPMN layout option '{option}': {detail}")
            }
            Self::InvalidModel { detail } => write!(f, "invalid BPMN model for layout: {detail}"),
        }
    }
}

impl std::error::Error for BpmnLayoutError {}
