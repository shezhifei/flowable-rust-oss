use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use std::collections::VecDeque;

pub trait AgendaOperation {
    fn run(&self, command_context: &mut CommandContext) -> Result<(), FlowableError>;
}

pub trait FlowableEngineAgenda {
    fn plan_operation(&mut self, operation: Box<dyn AgendaOperation>);
    fn plan_continue_process_operation(&mut self, execution: Execution);
    fn plan_take_outgoing_sequence_flows_operation(&mut self, execution: Execution);
    fn plan_wait_for_future_operation(
        &mut self,
        future_id: String,
        execution: Execution,
        continuation: crate::agenda::future_operations::WaitForFutureContinuation,
    );

    fn pop_operation(&mut self) -> Option<Box<dyn AgendaOperation>>;
    fn is_empty(&self) -> bool;
    fn clear(&mut self);
}

pub struct DefaultFlowableEngineAgenda {
    operations: VecDeque<Box<dyn AgendaOperation>>,
}

impl Default for DefaultFlowableEngineAgenda {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultFlowableEngineAgenda {
    pub fn new() -> Self {
        Self {
            operations: VecDeque::new(),
        }
    }
}

impl FlowableEngineAgenda for DefaultFlowableEngineAgenda {
    fn plan_operation(&mut self, operation: Box<dyn AgendaOperation>) {
        self.operations.push_back(operation);
    }

    fn plan_continue_process_operation(&mut self, execution: Execution) {
        use crate::agenda::continue_process_operation::ContinueProcessOperation;
        let op = ContinueProcessOperation::new(execution);
        self.plan_operation(Box::new(op));
    }

    fn plan_take_outgoing_sequence_flows_operation(&mut self, execution: Execution) {
        use crate::agenda::take_outgoing_sequence_flows_operation::TakeOutgoingSequenceFlowsOperation;
        let op = TakeOutgoingSequenceFlowsOperation::new(execution);
        self.plan_operation(Box::new(op));
    }

    fn plan_wait_for_future_operation(
        &mut self,
        future_id: String,
        execution: Execution,
        continuation: crate::agenda::future_operations::WaitForFutureContinuation,
    ) {
        use crate::agenda::future_operations::WaitForFutureOperation;
        let op = WaitForFutureOperation::new(future_id, execution).with_continuation(continuation);
        self.plan_operation(Box::new(op));
    }

    fn pop_operation(&mut self) -> Option<Box<dyn AgendaOperation>> {
        self.operations.pop_front()
    }

    fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    fn clear(&mut self) {
        self.operations.clear();
    }
}
