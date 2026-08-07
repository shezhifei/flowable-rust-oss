//! Minimal SMTP transport over plain TCP for local / AUTH-less relays.
//!
//! Supports EHLO, optional AUTH LOGIN, MAIL FROM, RCPT TO, DATA, QUIT.
//! STARTTLS fails closed with a structured error.

use crate::{
    prepare_message, MailMessage, MailRuntime, MailRuntimeMode, MailSendRecord, MailServiceError,
};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct SmtpMailConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub starttls: bool,
    pub default_from: String,
    pub timeout: Duration,
}

impl Default for SmtpMailConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 25,
            username: None,
            password: None,
            starttls: false,
            default_from: "noreply@flowable.local".to_string(),
            timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SmtpMailRuntime {
    config: SmtpMailConfig,
}

impl SmtpMailRuntime {
    pub fn new(config: SmtpMailConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &SmtpMailConfig {
        &self.config
    }

    pub fn send(&self, message: MailMessage) -> Result<MailSendRecord, MailServiceError> {
        if self.config.starttls {
            return Err(MailServiceError::new(
                "SMTP STARTTLS is not supported by the built-in thin SMTP client; \
                 use a plain SMTP endpoint (starttls=false) or a TLS-terminating proxy",
            ));
        }

        let host = self.config.host.trim();
        if host.is_empty() {
            return Err(MailServiceError::new(
                "Mail SMTP host is required when runtime mode is smtp",
            ));
        }

        let username = self
            .config
            .username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let password = self
            .config
            .password
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if username.is_some() != password.is_some() {
            return Err(MailServiceError::new(
                "Mail SMTP username and password must both be set when using AUTH LOGIN",
            ));
        }

        let prepared = prepare_message(message, &self.config.default_from)?;
        let from = prepared
            .from
            .as_deref()
            .unwrap_or(self.config.default_from.as_str());

        let addr = format!("{}:{}", host, self.config.port);
        let stream = TcpStream::connect(&addr).map_err(|error| {
            MailServiceError::new(format!(
                "Failed to connect to SMTP server {addr}: {error}"
            ))
        })?;
        stream
            .set_read_timeout(Some(self.config.timeout))
            .map_err(|error| {
                MailServiceError::new(format!("Failed to set SMTP read timeout: {error}"))
            })?;
        stream
            .set_write_timeout(Some(self.config.timeout))
            .map_err(|error| {
                MailServiceError::new(format!("Failed to set SMTP write timeout: {error}"))
            })?;

        let mut session = SmtpSession::new(stream)?;
        session.expect_code(220, "greeting")?;
        session.command(&format!("EHLO {}", ehlo_hostname()), &[250])?;

        if let (Some(user), Some(pass)) = (username, password) {
            session.command("AUTH LOGIN", &[334])?;
            session.command(&base64_encode(user.as_bytes()), &[334])?;
            session.command(&base64_encode(pass.as_bytes()), &[235])?;
        }

        session.command(&format!("MAIL FROM:<{from}>"), &[250])?;
        for recipient in &prepared.recipients {
            session.command(&format!("RCPT TO:<{recipient}>"), &[250, 251])?;
        }
        session.command("DATA", &[354])?;
        session.send_data(&build_rfc822(&prepared, from))?;
        session.expect_code(250, "DATA result")?;
        let _ = session.command("QUIT", &[221]);

        Ok(MailSendRecord {
            status: "SENT".to_string(),
            transport: "smtp".to_string(),
            message: prepared,
        })
    }
}

impl MailRuntime for SmtpMailRuntime {
    fn send(&self, message: MailMessage) -> Result<MailSendRecord, MailServiceError> {
        SmtpMailRuntime::send(self, message)
    }

    fn mode(&self) -> MailRuntimeMode {
        MailRuntimeMode::Smtp
    }
}

struct SmtpSession {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl SmtpSession {
    fn new(stream: TcpStream) -> Result<Self, MailServiceError> {
        let writer = stream.try_clone().map_err(|error| {
            MailServiceError::new(format!("Failed to clone SMTP TcpStream: {error}"))
        })?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    fn command(&mut self, line: &str, accepted: &[u16]) -> Result<String, MailServiceError> {
        write!(self.writer, "{line}\r\n").map_err(|error| {
            MailServiceError::new(format!("Failed to write SMTP command '{line}': {error}"))
        })?;
        self.writer.flush().map_err(|error| {
            MailServiceError::new(format!("Failed to flush SMTP command '{line}': {error}"))
        })?;
        let response = self.read_response()?;
        let code = parse_reply_code(&response).ok_or_else(|| {
            MailServiceError::new(format!(
                "SMTP command '{line}' returned unparseable response: {response}"
            ))
        })?;
        if !accepted.contains(&code) {
            let safe_line = if looks_like_base64_credential(line) {
                "<credential>"
            } else {
                line
            };
            return Err(MailServiceError::new(format!(
                "SMTP command '{safe_line}' failed with reply {code}: {response}"
            )));
        }
        Ok(response)
    }

    fn send_data(&mut self, body: &str) -> Result<(), MailServiceError> {
        for line in body.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.starts_with('.') {
                write!(self.writer, ".{line}\r\n")
            } else {
                write!(self.writer, "{line}\r\n")
            }
            .map_err(|error| MailServiceError::new(format!("Failed to write SMTP DATA: {error}")))?;
        }
        write!(self.writer, ".\r\n").map_err(|error| {
            MailServiceError::new(format!("Failed to terminate SMTP DATA: {error}"))
        })?;
        self.writer.flush().map_err(|error| {
            MailServiceError::new(format!("Failed to flush SMTP DATA: {error}"))
        })?;
        Ok(())
    }

    fn expect_code(&mut self, code: u16, context: &str) -> Result<String, MailServiceError> {
        let response = self.read_response()?;
        let actual = parse_reply_code(&response).ok_or_else(|| {
            MailServiceError::new(format!(
                "SMTP {context} returned unparseable response: {response}"
            ))
        })?;
        if actual != code {
            return Err(MailServiceError::new(format!(
                "SMTP {context} expected {code}, got {actual}: {response}"
            )));
        }
        Ok(response)
    }

    fn read_response(&mut self) -> Result<String, MailServiceError> {
        let mut full = String::new();
        loop {
            let mut line = String::new();
            let bytes = self.reader.read_line(&mut line).map_err(|error| {
                MailServiceError::new(format!("Failed to read SMTP response: {error}"))
            })?;
            if bytes == 0 {
                return Err(MailServiceError::new(
                    "SMTP connection closed while reading response",
                ));
            }
            full.push_str(&line);
            if line.len() >= 4 {
                let bytes = line.as_bytes();
                if bytes[3] == b' ' {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(full.trim_end().to_string())
    }
}

fn looks_like_base64_credential(line: &str) -> bool {
    !line.is_empty()
        && !line.contains(' ')
        && line
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

fn parse_reply_code(response: &str) -> Option<u16> {
    response
        .lines()
        .last()
        .and_then(|line| line.get(0..3))
        .and_then(|code| code.parse().ok())
}

fn build_rfc822(message: &MailMessage, from: &str) -> String {
    let mut headers = String::new();
    headers.push_str(&format!("From: {from}\r\n"));
    headers.push_str(&format!("To: {}\r\n", message.to));
    headers.push_str(&format!("Subject: {}\r\n", message.subject));
    // Java BaseMailActivityDelegate.addHeader → MailMessage headers on the wire.
    for (name, value) in &message.headers {
        headers.push_str(&format!("{name}: {value}\r\n"));
    }
    headers.push_str("MIME-Version: 1.0\r\n");

    if let Some(html) = message.html.as_deref().filter(|value| !value.is_empty()) {
        let boundary = "----=_Flowable_Mail_Boundary_7a3f";
        headers.push_str(&format!(
            "Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n"
        ));
        headers.push_str("\r\n");
        headers.push_str(&format!("--{boundary}\r\n"));
        headers.push_str("Content-Type: text/plain; charset=utf-8\r\n\r\n");
        headers.push_str(&message.text);
        headers.push_str("\r\n");
        headers.push_str(&format!("--{boundary}\r\n"));
        headers.push_str("Content-Type: text/html; charset=utf-8\r\n\r\n");
        headers.push_str(html);
        headers.push_str("\r\n");
        headers.push_str(&format!("--{boundary}--\r\n"));
    } else {
        headers.push_str("Content-Type: text/plain; charset=utf-8\r\n");
        headers.push_str("\r\n");
        headers.push_str(&message.text);
        headers.push_str("\r\n");
    }
    headers
}

fn ehlo_hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "localhost".to_string())
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < input.len() {
        let remaining = input.len() - i;
        let b0 = input[i];
        let b1 = if remaining > 1 { input[i + 1] } else { 0 };
        let b2 = if remaining > 2 { input[i + 2] } else { 0 };
        let triple = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if remaining > 1 {
            out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if remaining > 2 {
            out.push(TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn spawn_mock_smtp(transcript: Arc<Mutex<Vec<String>>>) -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock smtp");
        let port = listener.local_addr().expect("local addr").port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept smtp client");
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            stream
                .write_all(b"220 mock.smtp ESMTP ready\r\n")
                .expect("greeting");

            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut in_data = false;
            let mut data_lines: Vec<String> = Vec::new();

            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
                let trimmed = line.trim_end_matches(['\r', '\n']).to_string();

                if in_data {
                    if trimmed == "." {
                        transcript
                            .lock()
                            .expect("lock")
                            .push(format!("DATA_BODY:{}", data_lines.join("\n")));
                        in_data = false;
                        stream.write_all(b"250 OK\r\n").expect("data ok");
                    } else {
                        data_lines.push(trimmed);
                    }
                    continue;
                }

                transcript.lock().expect("lock").push(trimmed.clone());
                let upper = trimmed.to_ascii_uppercase();

                if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                    stream
                        .write_all(b"250-mock.smtp Hello\r\n250 AUTH LOGIN\r\n")
                        .expect("ehlo");
                } else if upper == "AUTH LOGIN" {
                    stream
                        .write_all(b"334 VXNlcm5hbWU6\r\n")
                        .expect("auth user prompt");
                } else if upper == "DATA" {
                    in_data = true;
                    data_lines.clear();
                    stream
                        .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                        .expect("data go-ahead");
                } else if upper.starts_with("MAIL FROM:") || upper.starts_with("RCPT TO:") {
                    stream.write_all(b"250 OK\r\n").expect("ok");
                } else if upper == "QUIT" {
                    stream.write_all(b"221 Bye\r\n").expect("quit");
                    break;
                } else if looks_like_base64_credential(&trimmed) {
                    let auth_creds = transcript
                        .lock()
                        .expect("lock")
                        .iter()
                        .filter(|l| {
                            looks_like_base64_credential(l) && l.to_ascii_uppercase() != "DATA"
                        })
                        .count();
                    if auth_creds <= 1 {
                        stream
                            .write_all(b"334 UGFzc3dvcmQ6\r\n")
                            .expect("auth pass prompt");
                    } else {
                        stream
                            .write_all(b"235 Authentication successful\r\n")
                            .expect("auth ok");
                    }
                } else {
                    stream.write_all(b"250 OK\r\n").expect("ok");
                }
            }
        });
        (port, handle)
    }

    #[test]
    fn smtp_runtime_sends_authless_message_to_mock_server() {
        let transcript = Arc::new(Mutex::new(Vec::new()));
        let (port, handle) = spawn_mock_smtp(transcript.clone());

        let runtime = SmtpMailRuntime::new(SmtpMailConfig {
            host: "127.0.0.1".to_string(),
            port,
            username: None,
            password: None,
            starttls: false,
            default_from: "noreply@flowable.local".to_string(),
            timeout: Duration::from_secs(5),
        });

        let record = runtime
            .send(MailMessage {
                to: "ops@example.flowable.local".to_string(),
                recipients: Vec::new(),
                from: Some("sender@example.flowable.local".to_string()),
                subject: "SMTP smoke".to_string(),
                text: "Hello from thin SMTP client.".to_string(),
                html: None,
                headers: Default::default(),
                attachments: Vec::new(),
            })
            .expect("smtp send should succeed against mock");

        assert_eq!(record.status, "SENT");
        assert_eq!(record.transport, "smtp");
        assert_eq!(
            record.message.recipients,
            vec!["ops@example.flowable.local"]
        );
        assert_eq!(MailRuntime::mode(&runtime), MailRuntimeMode::Smtp);

        handle.join().expect("mock server thread");
        let lines = transcript.lock().expect("lock");
        assert!(
            lines
                .iter()
                .any(|l| l.to_ascii_uppercase().starts_with("EHLO")),
            "expected EHLO in transcript: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.to_ascii_uppercase().starts_with("MAIL FROM:")),
            "expected MAIL FROM in transcript: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.to_ascii_uppercase().starts_with("RCPT TO:")),
            "expected RCPT TO in transcript: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.to_ascii_uppercase() == "DATA"),
            "expected DATA in transcript: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("DATA_BODY:") && l.contains("SMTP smoke")),
            "expected subject in DATA body: {lines:?}"
        );
    }

    #[test]
    fn smtp_runtime_rejects_starttls() {
        let runtime = SmtpMailRuntime::new(SmtpMailConfig {
            starttls: true,
            ..SmtpMailConfig::default()
        });
        let error = runtime
            .send(MailMessage {
                to: "ops@example.flowable.local".to_string(),
                recipients: Vec::new(),
                from: None,
                subject: "x".to_string(),
                text: "body".to_string(),
                html: None,
                headers: Default::default(),
                attachments: Vec::new(),
            })
            .expect_err("starttls must fail closed");
        assert!(
            error.to_string().contains("STARTTLS"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn smtp_runtime_requires_recipient() {
        let runtime = SmtpMailRuntime::new(SmtpMailConfig::default());
        let error = runtime
            .send(MailMessage {
                to: String::new(),
                recipients: Vec::new(),
                from: None,
                subject: "x".to_string(),
                text: "body".to_string(),
                html: None,
                headers: Default::default(),
                attachments: Vec::new(),
            })
            .expect_err("recipient required");
        assert!(error.to_string().contains("recipient"));
    }

    #[test]
    fn base64_encode_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"username"), "dXNlcm5hbWU=");
    }
}
