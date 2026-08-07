use flowable_bpmn_layout::BpmnLayoutError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessDiagramSvgError {
    Layout(BpmnLayoutError),
    UnsupportedOption {
        option: &'static str,
        detail: String,
    },
}

impl std::fmt::Display for ProcessDiagramSvgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "{error}"),
            Self::UnsupportedOption { option, detail } => {
                write!(f, "unsupported process diagram option '{option}': {detail}")
            }
        }
    }
}

impl std::error::Error for ProcessDiagramSvgError {}

impl From<BpmnLayoutError> for ProcessDiagramSvgError {
    fn from(value: BpmnLayoutError) -> Self {
        Self::Layout(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvgRasterizationError {
    Parse(String),
    EmptyCanvas,
    PngEncode(String),
}

impl std::fmt::Display for SvgRasterizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "failed to parse SVG for PNG rendering: {error}"),
            Self::EmptyCanvas => write!(f, "SVG has an empty canvas and cannot be rendered to PNG"),
            Self::PngEncode(error) => write!(f, "failed to encode PNG rendering: {error}"),
        }
    }
}

impl std::error::Error for SvgRasterizationError {}
