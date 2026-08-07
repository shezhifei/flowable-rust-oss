use crate::error::SvgRasterizationError;
use resvg::{tiny_skia, usvg};

pub fn svg_to_png_bytes(svg: &str) -> Result<Vec<u8>, SvgRasterizationError> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();

    let tree = usvg::Tree::from_data(svg.as_bytes(), &options)
        .map_err(|error| SvgRasterizationError::Parse(error.to_string()))?;
    let pixmap_size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(pixmap_size.width(), pixmap_size.height())
        .ok_or(SvgRasterizationError::EmptyCanvas)?;

    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|error| SvgRasterizationError::PngEncode(error.to_string()))
}
