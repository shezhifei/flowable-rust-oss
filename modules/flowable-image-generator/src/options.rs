use flowable_bpmn_layout::BpmnAutoLayoutOptions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessDiagramRenderOptions {
    pub layout_options: BpmnAutoLayoutOptions,
    pub scale_factor: i32,
    pub highlight_activity_ids: Vec<String>,
    pub highlight_flow_ids: Vec<String>,
    pub draw_sequence_flow_names: bool,
    pub include_metadata_attributes: bool,
}

impl Default for ProcessDiagramRenderOptions {
    fn default() -> Self {
        Self {
            layout_options: BpmnAutoLayoutOptions::default(),
            scale_factor: 1,
            highlight_activity_ids: Vec::new(),
            highlight_flow_ids: Vec::new(),
            draw_sequence_flow_names: false,
            include_metadata_attributes: true,
        }
    }
}
