use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod smtp;

pub use smtp::{SmtpMailConfig, SmtpMailRuntime};

// ── MailRuntime trait ──────────────────────────────────────────────

/// Mail runtime abstraction — supports deterministic outbox and real SMTP.
pub trait MailRuntime: Send + Sync {
    fn send(&self, message: MailMessage) -> Result<MailSendRecord, MailServiceError>;
    fn mode(&self) -> MailRuntimeMode;
}

// ── MailRuntimeMode ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailRuntimeMode {
    Deterministic,
    Smtp,
}

// ── MailServiceError ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailServiceError {
    message: String,
}

impl MailServiceError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for MailServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MailServiceError {}

// ── Message / send record ──────────────────────────────────────────

/// Attachment reference recorded for deterministic outbox / SMTP passthrough.
///
/// Java `BaseMailActivityDelegate.addExpressionValueAttachment` accepts File,
/// path strings, DataSource, ContentItem (etc.). The owned Rust subset records
/// name/contentType for field passthrough without opening real files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailAttachment {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MailMessage {
    pub to: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipients: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub subject: String,
    /// Plain-text body. May be empty when `html` is set
    /// (Java `BaseMailActivityDelegate.createMessage`: either html or text required).
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// Custom headers — Java `BaseMailActivityDelegate.addHeader` (colon-separated lines).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Attachment refs — Java `BaseMailActivityDelegate.addAttachments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MailAttachment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MailSendRecord {
    pub status: String,
    pub transport: String,
    pub message: MailMessage,
}

// ── DeterministicMailRuntime ───────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DeterministicMailRuntime {
    default_from: String,
}

impl Default for DeterministicMailRuntime {
    fn default() -> Self {
        Self::new("noreply@flowable.local")
    }
}

impl DeterministicMailRuntime {
    pub fn new(default_from: impl Into<String>) -> Self {
        Self {
            default_from: default_from.into(),
        }
    }

    pub fn send(&self, message: MailMessage) -> Result<MailSendRecord, MailServiceError> {
        let prepared = prepare_message(message, &self.default_from)?;
        Ok(MailSendRecord {
            status: "SENT".to_string(),
            transport: "deterministic-outbox".to_string(),
            message: prepared,
        })
    }
}

impl MailRuntime for DeterministicMailRuntime {
    fn send(&self, message: MailMessage) -> Result<MailSendRecord, MailServiceError> {
        DeterministicMailRuntime::send(self, message)
    }

    fn mode(&self) -> MailRuntimeMode {
        MailRuntimeMode::Deterministic
    }
}

// ── Shared validation / normalization ──────────────────────────────

/// Validate and normalize a mail message for the owned M14 subset.
pub(crate) fn prepare_message(
    message: MailMessage,
    default_from: &str,
) -> Result<MailMessage, MailServiceError> {
    let recipients = normalize_recipients(&message);
    if recipients.is_empty() {
        return Err(MailServiceError::new(
            "Mail recipient is required for the owned M14 subset",
        ));
    }
    if message.subject.trim().is_empty() {
        return Err(MailServiceError::new(
            "Mail subject is required for the owned M14 subset",
        ));
    }
    // Java BaseMailActivityDelegate.createMessage:112-114 —
    // "'html' or 'text' is required to be defined when using the mail activity".
    let text_empty = message.text.trim().is_empty();
    let html_empty = message
        .html
        .as_deref()
        .map(str::trim)
        .is_none_or(|h| h.is_empty());
    if text_empty && html_empty {
        return Err(MailServiceError::new(
            "'html' or 'text' is required to be defined when using the mail activity",
        ));
    }

    let effective_from = message
        .from
        .clone()
        .filter(|from| !from.trim().is_empty())
        .unwrap_or_else(|| default_from.to_string());

    Ok(MailMessage {
        to: recipients.join(","),
        recipients,
        from: Some(effective_from),
        subject: message.subject,
        text: message.text,
        html: message.html,
        headers: message.headers,
        attachments: message.attachments,
    })
}

fn normalize_recipients(message: &MailMessage) -> Vec<String> {
    if !message.recipients.is_empty() {
        return message
            .recipients
            .iter()
            .map(|recipient| recipient.trim())
            .filter(|recipient| !recipient.is_empty())
            .map(str::to_string)
            .collect();
    }

    message
        .to
        .split([',', ';'])
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
        .map(str::to_string)
        .collect()
}
