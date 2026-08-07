use crate::error::FlowableError;
use crate::interceptor::command::Command;
use std::sync::Arc;

/// Hook for wrapping command execution (Flowable Java-style `CommandInterceptor`).
///
/// Call `next(command)` to continue the chain.
/// [`crate::interceptor::command_executor::DefaultCommandExecutor`] is always the terminal executor.
///
/// The method is generic (same shape as Java interceptor chains). For registration on
/// [`crate::service::config::ProcessEngineConfiguration`], use [`CommandInterceptorHandle`]
/// / object-safe [`CommandInterceptorDyn`].
pub trait CommandInterceptor: Send + Sync {
    fn execute<T>(
        &self,
        command: &dyn Command<T>,
        next: &dyn Fn(&dyn Command<T>) -> Result<T, FlowableError>,
    ) -> Result<T, FlowableError>;
}

/// Object-safe interceptor used for configuration lists and the runtime chain.
///
/// Implementations wrap `next()` (the remainder of the chain / terminal executor).
pub trait CommandInterceptorDyn: Send + Sync + std::fmt::Debug {
    fn around(
        &self,
        next: &mut dyn FnMut() -> Result<(), FlowableError>,
    ) -> Result<(), FlowableError>;
}

/// Shared handle for interceptor registration on engine configuration.
pub type CommandInterceptorHandle = Arc<dyn CommandInterceptorDyn>;

/// Optional debug interceptor that logs before/after each command.
#[derive(Debug, Default, Clone, Copy)]
pub struct LoggingCommandInterceptor;

impl LoggingCommandInterceptor {
    pub fn new() -> Self {
        Self
    }

    pub fn handle() -> CommandInterceptorHandle {
        Arc::new(Self)
    }
}

impl CommandInterceptorDyn for LoggingCommandInterceptor {
    fn around(
        &self,
        next: &mut dyn FnMut() -> Result<(), FlowableError>,
    ) -> Result<(), FlowableError> {
        tracing::debug!("command interceptor: before execute");
        let result = next();
        match &result {
            Ok(()) => tracing::debug!("command interceptor: after execute (ok)"),
            Err(error) => {
                tracing::debug!(error = %error, "command interceptor: after execute (err)")
            }
        }
        result
    }
}

/// Adapter that implements the generic [`CommandInterceptor`] API by delegating to
/// an object-safe [`CommandInterceptorDyn`].
pub struct DynCommandInterceptorAdapter {
    inner: CommandInterceptorHandle,
}

impl DynCommandInterceptorAdapter {
    pub fn new(inner: CommandInterceptorHandle) -> Self {
        Self { inner }
    }
}

impl CommandInterceptor for DynCommandInterceptorAdapter {
    fn execute<T>(
        &self,
        command: &dyn Command<T>,
        next: &dyn Fn(&dyn Command<T>) -> Result<T, FlowableError>,
    ) -> Result<T, FlowableError> {
        let mut slot: Option<Result<T, FlowableError>> = None;
        let around_result = self.inner.around(&mut || {
            let result = next(command);
            let status = if result.is_ok() {
                Ok(())
            } else {
                Err(FlowableError::Internal(
                    "__command_interceptor_command_failed__".to_string(),
                ))
            };
            slot = Some(result);
            status
        });
        if let Some(result) = slot {
            return result;
        }
        Err(around_result.err().unwrap_or_else(|| {
            FlowableError::Internal("command interceptor short-circuited execution".to_string())
        }))
    }
}

/// Run a list of object-safe interceptors, then invoke `terminal`.
pub(crate) fn run_with_interceptors<T, F>(
    interceptors: &[CommandInterceptorHandle],
    terminal: F,
) -> Result<T, FlowableError>
where
    F: FnOnce() -> Result<T, FlowableError>,
{
    if interceptors.is_empty() {
        return terminal();
    }

    let slot: std::cell::RefCell<Option<Result<T, FlowableError>>> = std::cell::RefCell::new(None);
    let terminal = std::cell::RefCell::new(Some(terminal));

    let around_result = run_around_chain(interceptors, 0, &mut || {
        let run = terminal
            .borrow_mut()
            .take()
            .expect("terminal command executed more than once");
        let result = run();
        let status = if result.is_ok() {
            Ok(())
        } else {
            Err(FlowableError::Internal(
                "__command_interceptor_command_failed__".to_string(),
            ))
        };
        *slot.borrow_mut() = Some(result);
        status
    });

    if let Some(result) = slot.into_inner() {
        return result;
    }
    Err(around_result.err().unwrap_or_else(|| {
        FlowableError::Internal("command interceptor short-circuited execution".to_string())
    }))
}

fn run_around_chain(
    interceptors: &[CommandInterceptorHandle],
    index: usize,
    terminal: &mut dyn FnMut() -> Result<(), FlowableError>,
) -> Result<(), FlowableError> {
    if index >= interceptors.len() {
        return terminal();
    }
    let current = Arc::clone(&interceptors[index]);
    current.around(&mut || run_around_chain(interceptors, index + 1, terminal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct CountingInterceptor {
        before: Arc<AtomicUsize>,
        after: Arc<AtomicUsize>,
    }

    impl CommandInterceptorDyn for CountingInterceptor {
        fn around(
            &self,
            next: &mut dyn FnMut() -> Result<(), FlowableError>,
        ) -> Result<(), FlowableError> {
            self.before.fetch_add(1, Ordering::SeqCst);
            let result = next();
            self.after.fetch_add(1, Ordering::SeqCst);
            result
        }
    }

    #[test]
    fn empty_interceptor_list_runs_terminal() {
        let result = run_with_interceptors(&[], || Ok(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn interceptors_wrap_terminal_in_order() {
        let before = Arc::new(AtomicUsize::new(0));
        let after = Arc::new(AtomicUsize::new(0));
        let interceptors: Vec<CommandInterceptorHandle> = vec![
            Arc::new(CountingInterceptor {
                before: Arc::clone(&before),
                after: Arc::clone(&after),
            }),
            Arc::new(LoggingCommandInterceptor),
        ];

        let result = run_with_interceptors(&interceptors, || Ok("done"));
        assert_eq!(result.unwrap(), "done");
        assert_eq!(before.load(Ordering::SeqCst), 1);
        assert_eq!(after.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn interceptor_sees_terminal_errors() {
        let before = Arc::new(AtomicUsize::new(0));
        let after = Arc::new(AtomicUsize::new(0));
        let interceptors: Vec<CommandInterceptorHandle> = vec![Arc::new(CountingInterceptor {
            before: Arc::clone(&before),
            after: Arc::clone(&after),
        })];

        let result: Result<(), FlowableError> = run_with_interceptors(&interceptors, || {
            Err(FlowableError::Internal("boom".to_string()))
        });
        assert!(result.is_err());
        assert_eq!(before.load(Ordering::SeqCst), 1);
        assert_eq!(after.load(Ordering::SeqCst), 1);
        assert_eq!(result.unwrap_err().to_string(), "Internal error: boom");
    }
}
