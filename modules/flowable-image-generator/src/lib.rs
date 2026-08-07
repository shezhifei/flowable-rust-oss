mod error;
mod options;
mod raster;
mod svg;

use flowable_bpmn_model::BpmnModel;

pub use error::{ProcessDiagramSvgError, SvgRasterizationError};
pub use options::ProcessDiagramRenderOptions;
pub use raster::svg_to_png_bytes;
pub use svg::{DefaultProcessDiagramGenerator, generate_process_diagram_svg};

pub fn generate_process_svg(
    model: &BpmnModel,
    _title: &str,
) -> Result<String, ProcessDiagramSvgError> {
    generate_process_diagram_svg(model)
}
