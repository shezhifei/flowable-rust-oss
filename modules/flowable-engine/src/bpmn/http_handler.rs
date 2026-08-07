use crate::error::FlowableError;
use crate::runtime::execution::Execution;
use crate::scripting::secure_context::SecureScriptContext;
use crate::scripting::secure_engine::SecureScriptEngine;
use flowable_http_service::HttpExchange;
use flowable_http_service::HttpRequest;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub const HTTP_HANDLER_REGISTRY_CACHE_KEY: &str = "flowable.httpHandlerRegistry";

pub struct HttpRequestHandlerContext<'a> {
    pub execution: &'a mut Execution,
    pub request: &'a mut HttpRequest,
    pub fields: &'a BTreeMap<String, Value>,
}

pub struct HttpResponseHandlerContext<'a> {
    pub execution: &'a mut Execution,
    pub exchange: &'a mut HttpExchange,
    pub fields: &'a BTreeMap<String, Value>,
}

pub trait HttpRequestHandler: Send + Sync {
    fn handle_request(
        &self,
        context: &mut HttpRequestHandlerContext<'_>,
    ) -> Result<(), FlowableError>;
}

pub trait HttpResponseHandler: Send + Sync {
    fn handle_response(
        &self,
        context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError>;
}

#[derive(Clone)]
pub(crate) struct HttpResponseHandlerPlan {
    pub(crate) handler: Arc<dyn HttpResponseHandler>,
    pub(crate) fields: BTreeMap<String, Value>,
}

impl std::fmt::Debug for HttpResponseHandlerPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpResponseHandlerPlan")
            .field("fields", &self.fields)
            .finish_non_exhaustive()
    }
}

impl HttpResponseHandlerPlan {
    pub(crate) fn invoke(
        &self,
        execution: &mut Execution,
        exchange: &mut HttpExchange,
    ) -> Result<(), FlowableError> {
        let mut context = HttpResponseHandlerContext {
            execution,
            exchange,
            fields: &self.fields,
        };
        self.handler.handle_response(&mut context)
    }
}

#[derive(Clone, Default)]
pub struct HttpHandlerRegistry {
    request_handlers: BTreeMap<String, Arc<dyn HttpRequestHandler>>,
    response_handlers: BTreeMap<String, Arc<dyn HttpResponseHandler>>,
}

impl std::fmt::Debug for HttpHandlerRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpHandlerRegistry")
            .field(
                "request_handlers",
                &self.request_handlers.keys().collect::<Vec<_>>(),
            )
            .field(
                "response_handlers",
                &self.response_handlers.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl HttpHandlerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_request_handler(
        &mut self,
        name: impl Into<String>,
        handler: Arc<dyn HttpRequestHandler>,
    ) {
        self.request_handlers.insert(name.into(), handler);
    }

    pub fn register_response_handler(
        &mut self,
        name: impl Into<String>,
        handler: Arc<dyn HttpResponseHandler>,
    ) {
        self.response_handlers.insert(name.into(), handler);
    }

    pub fn request_handler(&self, name: &str) -> Option<Arc<dyn HttpRequestHandler>> {
        self.request_handlers.get(name).cloned()
    }

    pub fn response_handler(&self, name: &str) -> Option<Arc<dyn HttpResponseHandler>> {
        self.response_handlers.get(name).cloned()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SecureScriptHttpHandler {
    language: String,
    script: String,
    result_variable: Option<String>,
    enabled_languages: Vec<String>,
}

impl SecureScriptHttpHandler {
    pub(crate) fn new(
        language: String,
        script: String,
        result_variable: Option<String>,
        enabled_languages: Vec<String>,
    ) -> Self {
        Self {
            language,
            script,
            result_variable,
            enabled_languages,
        }
    }

    fn execute(
        &self,
        execution: &mut Execution,
        context_value_name: &str,
        context_value: Value,
    ) -> Result<(), FlowableError> {
        let mut variables = execution.process_variables();
        variables.insert(context_value_name.to_string(), context_value);
        let mut script_context = SecureScriptContext::from_variables(variables);
        let result = SecureScriptEngine::new(self.enabled_languages.clone()).execute(
            &self.language,
            &self.script,
            &mut script_context,
        )?;
        for (name, value) in script_context.into_result_variables() {
            execution.set_process_variable(name, value);
        }
        if let Some(result_variable) = &self.result_variable {
            execution.set_process_variable(result_variable.clone(), result.unwrap_or(Value::Null));
        }
        Ok(())
    }
}

impl HttpRequestHandler for SecureScriptHttpHandler {
    fn handle_request(
        &self,
        context: &mut HttpRequestHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        let request = serde_json::to_value(&*context.request).map_err(|error| {
            FlowableError::ExecutionError(format!(
                "Failed to expose HTTP request to secure script handler: {error}"
            ))
        })?;
        self.execute(context.execution, "httpRequest", request)
    }
}

impl HttpResponseHandler for SecureScriptHttpHandler {
    fn handle_response(
        &self,
        context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        let response = serde_json::to_value(&context.exchange.response).map_err(|error| {
            FlowableError::ExecutionError(format!(
                "Failed to expose HTTP response to secure script handler: {error}"
            ))
        })?;
        self.execute(context.execution, "httpResponse", response)
    }
}
