use crate::error::FlowableError;
use crate::scripting::secure_context::SecureScriptContext;
use serde_json::Value;

/// Supported script languages in the M10 entry slice.
const SUPPORTED_LANGUAGES: &[&str] = &["javascript", "groovy"];

/// A bounded, deterministic script execution engine.
///
/// Hardened script execution: constrains the script-visible context and
/// evaluates only the supported languages (`javascript`, `groovy`).
///
/// M9 entry slice: evaluates simple assignment expressions in a controlled
/// context. Does NOT provide full ECMAScript/Groovy runtime — that is
/// intentionally deferred to post-M9. The engine:
/// - accepts only explicitly enabled languages
/// - runs in a sandbox with no host API access
/// - writes results back through the controlled `SecureScriptContext`
pub struct SecureScriptEngine {
    enabled_languages: Vec<String>,
}

impl SecureScriptEngine {
    /// Create a new engine with the given set of enabled languages.
    pub fn new(enabled_languages: Vec<String>) -> Self {
        Self { enabled_languages }
    }

    /// Check whether a language is supported and enabled.
    pub fn is_language_supported(&self, language: &str) -> bool {
        let lang_lower = language.to_lowercase();
        self.enabled_languages
            .iter()
            .any(|l| l.to_lowercase() == lang_lower)
            && SUPPORTED_LANGUAGES
                .iter()
                .any(|s| s.eq_ignore_ascii_case(&lang_lower))
    }

    /// Execute a script in the bounded secure runtime.
    ///
    /// The M9 entry slice supports a deliberately minimal expression language:
    /// - variable assignments: `var result = <expr>;`
    /// - simple arithmetic on literals: `1 + 2`, `10 * 3`
    /// - string literals: `'hello'` or `"hello"`
    /// - reading process variables from the context
    ///
    /// This is intentionally NOT a full JS runtime. It proves the execution
    /// path is no longer pass-through and that scripts execute through the
    /// secure context boundary.
    pub fn execute(
        &self,
        language: &str,
        script: &str,
        context: &mut SecureScriptContext,
    ) -> Result<Option<Value>, FlowableError> {
        if !self.is_language_supported(language) {
            return Err(FlowableError::ExecutionError(format!(
                "Script language '{}' is not supported or not enabled in secure scripting configuration",
                language
            )));
        }

        self.evaluate_script(script, context)
    }

    /// Evaluate the script body in the bounded sandbox using the AST-based pipeline.
    fn evaluate_script(
        &self,
        script: &str,
        context: &mut SecureScriptContext,
    ) -> Result<Option<Value>, FlowableError> {
        let statements = crate::scripting::parser::parse_script(script)?;
        let mut evaluator = crate::scripting::evaluator::Evaluator::new(context);
        evaluator.execute(&statements)
    }
}

/// Validate that a script task definition is acceptable for the current engine configuration.
pub fn validate_script_task(
    language: Option<&str>,
    secure_scripting_enabled: bool,
    supported_languages: &[String],
) -> Result<(), FlowableError> {
    if !secure_scripting_enabled {
        return Err(FlowableError::ExecutionError(
            "Script task execution requires secure scripting to be enabled in engine configuration"
                .to_string(),
        ));
    }

    let lang = language.unwrap_or("javascript");

    let is_supported = SUPPORTED_LANGUAGES
        .iter()
        .any(|s| s.eq_ignore_ascii_case(lang));
    let is_enabled = supported_languages
        .iter()
        .any(|s| s.eq_ignore_ascii_case(lang));

    if !is_supported {
        return Err(FlowableError::ExecutionError(format!(
            "Script language '{}' is not supported by the secure scripting runtime",
            lang
        )));
    }

    if !is_enabled {
        return Err(FlowableError::ExecutionError(format!(
            "Script language '{}' is not enabled in engine configuration",
            lang
        )));
    }

    Ok(())
}
