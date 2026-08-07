#[path = "task.rs"]
mod task_model;

pub use crate::persistence::task_entity_manager::{DefaultTaskEntityManager, TaskEntityManager};
pub use task_model::Task;
