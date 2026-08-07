#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutDirection {
    LeftToRight,
    TopToBottom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BpmnAutoLayoutOptions {
    pub direction: LayoutDirection,
    pub preserve_existing_di: bool,
}

impl Default for BpmnAutoLayoutOptions {
    fn default() -> Self {
        Self {
            direction: LayoutDirection::LeftToRight,
            preserve_existing_di: true,
        }
    }
}
