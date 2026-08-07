use crate::error::FlowableError;
use crate::interceptor::command_executor::DefaultCommandExecutor;
use std::marker::PhantomData;
use std::sync::Arc;

pub trait Query<T, Q>: Send + Sync {
    fn list(&self) -> Result<Vec<T>, FlowableError>;
    fn single_result(&self) -> Result<Option<T>, FlowableError>;
    fn count(&self) -> Result<i64, FlowableError>;
}

#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Asc,
    Desc,
}

pub struct QueryState<T> {
    pub(crate) command_executor: Arc<DefaultCommandExecutor>,
    pub(crate) phantom: PhantomData<T>,
    pub(crate) order_by: Option<String>,
    pub(crate) direction: Direction,
}

impl<T> QueryState<T> {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            command_executor,
            phantom: PhantomData,
            order_by: None,
            direction: Direction::Asc,
        }
    }
}
