mod auto_layout;
mod error;
mod options;
mod types;

use flowable_bpmn_model::BpmnModel;

pub use auto_layout::BpmnAutoLayout;
pub use error::BpmnLayoutError;
pub use options::{BpmnAutoLayoutOptions, LayoutDirection};
pub use types::{
    BpmnLayoutResult, DiagramNodeKind, EdgeLayout, LayoutBounds, LayoutWaypoint, NodeLayout,
    ProcessDiagramLayout,
};

pub fn ensure_layout(model: &mut BpmnModel) -> Result<(), BpmnLayoutError> {
    let laid_out = BpmnAutoLayout::new().generate(model)?;
    *model = laid_out.into_model();
    Ok(())
}
