use flowable_mail_service::{
    DeterministicMailRuntime, MailMessage, MailRuntime, MailRuntimeMode, SmtpMailConfig,
    SmtpMailRuntime,
};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[test]
fn deterministic_mail_runtime_records_sent_mail_in_outbox_shape() {
    let runtime = DeterministicMailRuntime::default();
    let send_record = runtime
        .send(MailMessage {
            to: "ops@example.flowable.local; audit@example.flowable.local".to_string(),
            recipients: Vec::new(),
            from: Some("noreply@example.flowable.local".to_string()),
            subject: "Deployment finished".to_string(),
            text: "Deployment finished successfully.".to_string(),
            html: Some("<p>Deployment finished successfully.</p>".to_string()),
            headers: Default::default(),
            attachments: Vec::new(),
        })
        .expect("owned Mail subset should execute");

    assert_eq!(send_record.status, "SENT");
    assert_eq!(
        send_record.message.to,
        "ops@example.flowable.local,audit@example.flowable.local"
    );
    assert_eq!(send_record.message.recipients.len(), 2);
    assert!(send_record.message.html.is_some());
    assert_eq!(send_record.message.subject, "Deployment finished");
    assert_eq!(send_record.transport, "deterministic-outbox");
    assert_eq!(MailRuntime::mode(&runtime), MailRuntimeMode::Deterministic);
}

#[test]
fn deterministic_mail_runtime_requires_recipient() {
    let runtime = DeterministicMailRuntime::default();
    let error = runtime
        .send(MailMessage {
            to: String::new(),
            recipients: Vec::new(),
            from: Some("noreply@example.flowable.local".to_string()),
            subject: "Deployment finished".to_string(),
            text: "Deployment finished successfully.".to_string(),
            html: None,
            headers: Default::default(),
            attachments: Vec::new(),
        })
        .expect_err("recipient is required");

    assert!(error.to_string().contains("recipient"));
}

#[test]
fn smtp_mail_runtime_sends_via_local_tcp_listener_mock() {
    let transcript = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock smtp");
    let port = listener.local_addr().expect("addr").port();
    let transcript_for_server = transcript.clone();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        stream
            .write_all(b"220 mock.smtp ESMTP ready\r\n")
            .expect("greeting");

        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut in_data = false;

        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).ok().unwrap_or(0) == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
            if in_data {
                if trimmed == "." {
                    in_data = false;
                    stream.write_all(b"250 OK\r\n").expect("data ok");
                }
                continue;
            }
            transcript_for_server
                .lock()
                .expect("lock")
                .push(trimmed.clone());
            let upper = trimmed.to_ascii_uppercase();
            if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                stream
                    .write_all(b"250-mock.smtp Hello\r\n250 OK\r\n")
                    .expect("ehlo");
            } else if upper == "DATA" {
                in_data = true;
                stream
                    .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                    .expect("data");
            } else if upper == "QUIT" {
                stream.write_all(b"221 Bye\r\n").expect("quit");
                break;
            } else {
                stream.write_all(b"250 OK\r\n").expect("ok");
            }
        }
    });

    let runtime = SmtpMailRuntime::new(SmtpMailConfig {
        host: "127.0.0.1".to_string(),
        port,
        username: None,
        password: None,
        starttls: false,
        default_from: "noreply@flowable.local".to_string(),
        timeout: Duration::from_secs(5),
    });

    let record = MailRuntime::send(
        &runtime,
        MailMessage {
            to: "ops@example.flowable.local; audit@example.flowable.local".to_string(),
            recipients: Vec::new(),
            from: Some("noreply@example.flowable.local".to_string()),
            subject: "SMTP outbox test".to_string(),
            text: "Sent via mock SMTP listener.".to_string(),
            html: Some("<p>Sent via mock SMTP listener.</p>".to_string()),
            headers: Default::default(),
            attachments: Vec::new(),
        },
    )
    .expect("SMTP send against local mock should succeed");

    assert_eq!(record.status, "SENT");
    assert_eq!(record.transport, "smtp");
    assert_eq!(record.message.recipients.len(), 2);
    assert_eq!(MailRuntime::mode(&runtime), MailRuntimeMode::Smtp);

    server.join().expect("mock server");
    let lines = transcript.lock().expect("lock");
    assert!(
        lines
            .iter()
            .any(|l| l.to_ascii_uppercase().starts_with("MAIL FROM:")),
        "MAIL FROM missing: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .filter(|l| l.to_ascii_uppercase().starts_with("RCPT TO:"))
            .count()
            >= 2,
        "expected two RCPT TO lines: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.to_ascii_uppercase() == "DATA"),
        "DATA missing: {lines:?}"
    );
}

#[test]
fn smtp_mail_runtime_starttls_fails_closed() {
    let runtime = SmtpMailRuntime::new(SmtpMailConfig {
        host: "127.0.0.1".to_string(),
        port: 25,
        starttls: true,
        ..SmtpMailConfig::default()
    });
    let error = MailRuntime::send(
        &runtime,
        MailMessage {
            to: "ops@example.flowable.local".to_string(),
            recipients: Vec::new(),
            from: None,
            subject: "x".to_string(),
            text: "body".to_string(),
            html: None,
            headers: Default::default(),
            attachments: Vec::new(),
        },
    )
    .expect_err("starttls must be rejected by thin client");
    assert!(error.to_string().contains("STARTTLS"));
}
