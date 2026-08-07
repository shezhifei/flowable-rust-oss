use crate::bpmn::fault::EngineFault;
use crate::bpmn::http_handler::HttpResponseHandlerPlan;
use crate::runtime::execution::Execution;
use flowable_http_service::{HttpExchange, HttpRequest, HttpRuntimeMode};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

/// Canonical Java-visible HTTP contract.
///
/// These fields describe Flowable Java semantics. They are not a second
/// execution mode: both synchronous and asynchronous Rust transports consume
/// the same contract and produce the same [`HttpTaskOutcome`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JavaHttpContract {
    pub ignore_exception: bool,
    pub save_request_variables: bool,
    pub save_response_parameters: bool,
    pub save_response_parameters_transient: bool,
    pub save_response_variable_as_json: bool,
    pub response_variable_name: Option<String>,
    pub result_variable_prefix: String,
    pub fail_status_codes: BTreeSet<String>,
    pub handle_status_codes: BTreeSet<String>,
    pub parallel_in_same_transaction: Option<bool>,
}

impl JavaHttpContract {
    pub fn status_action(&self, status: u16, redirects_disallowed: bool) -> HttpStatusAction {
        if status < 300 || (!redirects_disallowed && status < 400) {
            return HttpStatusAction::Continue;
        }
        let code = status.to_string();
        if matches_status(&self.handle_status_codes, &code) {
            HttpStatusAction::BpmnError(format!("HTTP{code}"))
        } else if matches_status(&self.fail_status_codes, &code) {
            HttpStatusAction::Fail(format!("HTTP{code}"))
        } else {
            HttpStatusAction::Continue
        }
    }
}

/// Existing Rust-visible result projection. This remains additive to the Java
/// contract and keeps the pre-existing structured `httpResult` shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RustHttpProjection {
    pub result_variable_name: Option<String>,
    pub store_result_as_transient: bool,
    pub use_local_scope: bool,
}

#[derive(Clone, Debug)]
pub struct HttpTaskSpec {
    pub request: HttpRequest,
    pub java: JavaHttpContract,
    pub rust: RustHttpProjection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HttpExecutionMode {
    Inline,
    ParallelInSameTransaction,
}

impl HttpTaskSpec {
    pub(crate) fn execution_mode(&self, runtime_mode: HttpRuntimeMode) -> HttpExecutionMode {
        match self.java.parallel_in_same_transaction {
            Some(true) => HttpExecutionMode::ParallelInSameTransaction,
            Some(false) => HttpExecutionMode::Inline,
            None if runtime_mode == HttpRuntimeMode::Async => {
                // Preserve the pre-existing explicit Rust async runtime extension
                // when Java XML does not choose a mode.
                HttpExecutionMode::ParallelInSameTransaction
            }
            None => HttpExecutionMode::Inline,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PendingHttpCompletion {
    pub(crate) spec: HttpTaskSpec,
    pub(crate) transport_result: Result<HttpExchange, String>,
    pub(crate) response_handler: Option<HttpResponseHandlerPlan>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecutionMutations {
    pub process: HashMap<String, Value>,
    pub local: HashMap<String, Value>,
    pub transient: HashMap<String, Value>,
}

impl ExecutionMutations {
    pub fn apply_to(&self, execution: &mut Execution) {
        for (name, value) in &self.process {
            execution.set_process_variable(name.clone(), value.clone());
        }
        for (name, value) in &self.local {
            execution.set_local_variable(name.clone(), value.clone());
        }
        for (name, value) in &self.transient {
            execution.set_transient_variable(name.clone(), value.clone());
        }
    }
}

#[derive(Clone, Debug)]
pub struct HttpTaskOutcome {
    pub rust_result: Value,
    pub mutations: ExecutionMutations,
    pub status_action: HttpStatusAction,
}

impl HttpTaskOutcome {
    pub fn success(spec: &HttpTaskSpec, exchange: &HttpExchange) -> Self {
        let mut mutations = java_response_mutations(&spec.java, exchange);
        let mut status_action = spec.java.status_action(
            exchange.response.status_code,
            exchange.request.follow_redirects == Some(false),
        );
        if spec.java.ignore_exception {
            if let HttpStatusAction::Fail(message) = &status_action {
                mutations
                    .process
                    .extend(java_error_mutations(&spec.java, message).process);
                status_action = HttpStatusAction::Continue;
            }
        }
        Self {
            rust_result: rust_success_result(exchange),
            mutations,
            status_action,
        }
    }

    pub fn ignored_transport_error(spec: &HttpTaskSpec, error: &str) -> Self {
        Self {
            rust_result: rust_ignored_error_result(&spec.request, error),
            mutations: java_error_mutations(&spec.java, error),
            status_action: HttpStatusAction::Continue,
        }
    }

    pub fn apply_to(&self, execution: &mut Execution) -> Result<Value, EngineFault> {
        self.mutations.apply_to(execution);
        enforce_status_action(self.status_action.clone())?;
        Ok(self.rust_result.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpStatusAction {
    Continue,
    BpmnError(String),
    Fail(String),
}

pub fn parse_status_codes(value: Option<String>) -> BTreeSet<String> {
    value
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .map(str::to_ascii_uppercase)
                .collect::<Vec<_>>()
        })
        .filter(|value| !value.is_empty())
        .collect()
}

pub fn project_java_request_variables(
    execution: &mut Execution,
    contract: &JavaHttpContract,
    request: &HttpRequest,
) {
    java_request_mutations(contract, request).apply_to(execution);
}

pub fn java_request_mutations(
    contract: &JavaHttpContract,
    request: &HttpRequest,
) -> ExecutionMutations {
    let mut mutations = ExecutionMutations::default();
    if !contract.save_request_variables {
        return mutations;
    }
    let prefix = &contract.result_variable_prefix;
    mutations
        .process
        .insert(format!("{prefix}RequestMethod"), json!(request.method));
    mutations
        .process
        .insert(format!("{prefix}RequestUrl"), json!(request.url));
    mutations.process.insert(
        format!("{prefix}RequestHeaders"),
        json!(headers_as_lines(&request.headers)),
    );
    mutations.process.insert(
        format!("{prefix}RequestBody"),
        request
            .body
            .as_ref()
            .map(|body| match body {
                Value::String(raw) => Value::String(raw.clone()),
                structured => Value::String(structured.to_string()),
            })
            .unwrap_or(Value::Null),
    );
    mutations.process.insert(
        format!("{prefix}RequestBodyEncoding"),
        json!(request.body_encoding),
    );
    mutations
        .process
        .insert(format!("{prefix}RequestTimeout"), json!(request.timeout_ms));
    mutations.process.insert(
        format!("{prefix}DisallowRedirects"),
        json!(request.follow_redirects == Some(false)),
    );
    mutations.process.insert(
        format!("{prefix}FailStatusCodes"),
        json!(
            contract
                .fail_status_codes
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        ),
    );
    mutations.process.insert(
        format!("{prefix}HandleStatusCodes"),
        json!(
            contract
                .handle_status_codes
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        ),
    );
    mutations.process.insert(
        format!("{prefix}IgnoreException"),
        json!(contract.ignore_exception),
    );
    mutations.process.insert(
        format!("{prefix}SaveRequestVariables"),
        json!(contract.save_request_variables),
    );
    mutations.process.insert(
        format!("{prefix}SaveResponseParameters"),
        json!(contract.save_response_parameters),
    );
    mutations
}

pub fn java_response_mutations(
    contract: &JavaHttpContract,
    exchange: &HttpExchange,
) -> ExecutionMutations {
    let mut mutations = ExecutionMutations::default();
    let prefix = &contract.result_variable_prefix;
    if contract.save_response_parameters {
        insert_response_mutation(
            &mut mutations,
            contract,
            format!("{prefix}ResponseProtocol"),
            json!("HTTP"),
        );
        insert_response_mutation(
            &mut mutations,
            contract,
            format!("{prefix}ResponseStatusCode"),
            json!(exchange.response.status_code),
        );
        insert_response_mutation(
            &mut mutations,
            contract,
            format!("{prefix}ResponseReason"),
            Value::Null,
        );
        insert_response_mutation(
            &mut mutations,
            contract,
            format!("{prefix}ResponseHeaders"),
            json!(headers_as_lines(&exchange.response.headers)),
        );
    }
    let name = contract
        .response_variable_name
        .clone()
        .unwrap_or_else(|| format!("{prefix}ResponseBody"));
    let body = if contract.save_response_variable_as_json {
        exchange.response.body.clone()
    } else {
        match &exchange.response.body {
            Value::String(value) => Value::String(value.clone()),
            value => Value::String(value.to_string()),
        }
    };
    insert_response_mutation(&mut mutations, contract, name, body);
    mutations
}

pub fn java_error_mutations(contract: &JavaHttpContract, error: &str) -> ExecutionMutations {
    let mut mutations = ExecutionMutations::default();
    mutations.process.insert(
        format!("{}ErrorMessage", contract.result_variable_prefix),
        json!(error),
    );
    mutations
}

pub fn enforce_status_action(action: HttpStatusAction) -> Result<(), EngineFault> {
    match action {
        HttpStatusAction::Continue => Ok(()),
        HttpStatusAction::BpmnError(code) => Err(EngineFault::BpmnError {
            code,
            message: None,
        }),
        HttpStatusAction::Fail(message) => Err(EngineFault::Execution { message }),
    }
}

fn rust_success_result(exchange: &HttpExchange) -> Value {
    let basic_auth_summary = exchange.request.basic_auth.as_ref().map(|auth| {
        json!({
            "hasBasicAuth": true,
            "username": auth.username,
        })
    });
    json!({
        "service": "http",
        "request": {
            "method": exchange.request.method,
            "url": exchange.request.url,
            "headers": exchange.request.headers,
            "body": exchange.request.body,
            "timeoutMs": exchange.request.timeout_ms,
            "connectTimeoutMs": exchange.request.connect_timeout_ms,
            "followRedirects": exchange.request.follow_redirects,
            "bodyEncoding": exchange.request.body_encoding,
            "hasBasicAuth": basic_auth_summary.is_some(),
            "basicAuth": basic_auth_summary,
        },
        "response": {
            "statusCode": exchange.response.status_code,
            "headers": exchange.response.headers,
            "body": exchange.response.body,
        }
    })
}

fn rust_ignored_error_result(request: &HttpRequest, error: &str) -> Value {
    json!({
        "service": "http",
        "request": {
            "method": request.method,
            "url": request.url,
            "headers": request.headers,
            "body": request.body,
        },
        "response": Value::Null,
        "error": { "ignored": true, "message": error }
    })
}

fn insert_response_mutation(
    mutations: &mut ExecutionMutations,
    contract: &JavaHttpContract,
    name: String,
    value: Value,
) {
    if contract.save_response_parameters_transient {
        mutations.transient.insert(name, value);
    } else {
        mutations.process.insert(name, value);
    }
}

fn matches_status(patterns: &BTreeSet<String>, code: &str) -> bool {
    patterns.contains(code)
        || code.starts_with('3') && patterns.contains("3XX")
        || code.starts_with('4') && patterns.contains("4XX")
        || code.starts_with('5') && patterns.contains("5XX")
}

fn headers_as_lines(headers: &std::collections::BTreeMap<String, String>) -> String {
    headers
        .iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_patterns_match_java_exact_and_family_codes() {
        let contract = JavaHttpContract {
            fail_status_codes: parse_status_codes(Some("404,5XX".to_string())),
            handle_status_codes: parse_status_codes(Some("4XX".to_string())),
            ..Default::default()
        };
        assert_eq!(
            contract.status_action(404, false),
            HttpStatusAction::BpmnError("HTTP404".to_string())
        );
        assert_eq!(
            contract.status_action(503, false),
            HttpStatusAction::Fail("HTTP503".to_string())
        );
        assert_eq!(
            contract.status_action(302, true),
            HttpStatusAction::Continue
        );
    }

    #[test]
    fn explicit_java_parallel_flag_overrides_rust_runtime_mode() {
        let mut spec = HttpTaskSpec {
            request: HttpRequest {
                method: "GET".to_string(),
                url: "https://example.flowable.local".to_string(),
                headers: Default::default(),
                body: None,
                timeout_ms: None,
                connect_timeout_ms: None,
                follow_redirects: None,
                basic_auth: None,
                body_encoding: None,
            },
            java: JavaHttpContract::default(),
            rust: RustHttpProjection::default(),
        };
        spec.java.parallel_in_same_transaction = Some(true);
        assert_eq!(
            spec.execution_mode(HttpRuntimeMode::Real),
            HttpExecutionMode::ParallelInSameTransaction
        );
        spec.java.parallel_in_same_transaction = Some(false);
        assert_eq!(
            spec.execution_mode(HttpRuntimeMode::Async),
            HttpExecutionMode::Inline
        );
        spec.java.parallel_in_same_transaction = None;
        assert_eq!(
            spec.execution_mode(HttpRuntimeMode::Async),
            HttpExecutionMode::ParallelInSameTransaction
        );
    }

    #[test]
    fn java_request_body_variable_preserves_the_original_string_contract() {
        let contract = JavaHttpContract {
            save_request_variables: true,
            result_variable_prefix: "contract".to_string(),
            ..Default::default()
        };
        let request = HttpRequest {
            method: "POST".to_string(),
            url: "https://example.flowable.local/orders".to_string(),
            headers: Default::default(),
            body: Some(json!({"orderId": 42})),
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        };

        let mutations = java_request_mutations(&contract, &request);
        assert_eq!(
            mutations.process.get("contractRequestBody"),
            Some(&json!("{\"orderId\":42}")),
            "Flowable Java saves the request body as its original string value"
        );
        assert_eq!(
            rust_ignored_error_result(&request, "ignored")["request"]["body"],
            json!({"orderId": 42}),
            "the existing Rust structured request-body projection remains additive"
        );
    }

    #[test]
    fn ignore_exception_suppresses_fail_status_but_not_bpmn_error() {
        let request = HttpRequest {
            method: "GET".to_string(),
            url: "https://example.flowable.local/orders".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        };
        let exchange = HttpExchange {
            request: request.clone(),
            response: flowable_http_service::HttpResponse {
                status_code: 500,
                headers: Default::default(),
                body: json!({"accepted": false}),
            },
        };
        let mut spec = HttpTaskSpec {
            request,
            java: JavaHttpContract {
                ignore_exception: true,
                fail_status_codes: parse_status_codes(Some("5XX".to_string())),
                result_variable_prefix: "contract".to_string(),
                ..Default::default()
            },
            rust: RustHttpProjection::default(),
        };

        let ignored_failure = HttpTaskOutcome::success(&spec, &exchange);
        assert_eq!(ignored_failure.status_action, HttpStatusAction::Continue);
        assert_eq!(
            ignored_failure
                .mutations
                .process
                .get("contractErrorMessage"),
            Some(&json!("HTTP500"))
        );

        spec.java.handle_status_codes = parse_status_codes(Some("5XX".to_string()));
        let bpmn_error = HttpTaskOutcome::success(&spec, &exchange);
        assert_eq!(
            bpmn_error.status_action,
            HttpStatusAction::BpmnError("HTTP500".to_string())
        );
    }
}
