//! lettre-backed SMTP transport for the onboarding invitation adapter.
//!
//! This module is the only place that speaks SMTP. It uses the maintained
//! [`lettre`] crate rather than a hand-written protocol implementation, so the
//! connection, AUTH, STARTTLS/implicit TLS, DATA framing, and reply handling are
//! owned by a maintained client.
//!
//! Behaviour required by the plan:
//!
//! * The transport security mode comes from configuration and is never lowered
//!   after a connection or TLS failure.
//! * A bounded timeout covers the connection and the SMTP exchange.
//! * A message is reported as accepted only when the server accepted it. That
//!   is SMTP acceptance, never a claim of delivery to a real inbox.
//! * Failures return a short safe code. The code never carries the host, a
//!   credential, an invitation token, or the message body.

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::transport::smtp::SmtpTransport;
use lettre::{Message, Transport};

use super::delivery::{SmtpSettings, SmtpTlsMode};

/// Sends one plain-text message and returns `Ok(())` only on SMTP acceptance.
///
/// The configured TLS mode is applied exactly: `none` stays plaintext,
/// `implicit` wraps the connection in TLS, and `starttls` requires STARTTLS.
/// No branch falls back to a lower mode after a failure.
pub fn send(
    settings: &SmtpSettings,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<(), &'static str> {
    let from: Mailbox = settings.from.parse().map_err(|_| "smtp_invalid_from")?;
    let to: Mailbox = to.parse().map_err(|_| "smtp_invalid_recipient")?;

    let email = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .body(body.to_string())
        .map_err(|_| "smtp_invalid_message")?;

    let mut builder = SmtpTransport::builder_dangerous(settings.host.as_str())
        .port(settings.port)
        .timeout(Some(settings.timeout));

    builder = match settings.tls {
        SmtpTlsMode::None => builder.tls(Tls::None),
        SmtpTlsMode::Implicit => builder.tls(Tls::Wrapper(tls_parameters(settings)?)),
        SmtpTlsMode::StartTls => builder.tls(Tls::Required(tls_parameters(settings)?)),
    };

    if let (Some(username), Some(password)) = (&settings.username, &settings.password) {
        builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
    }

    builder
        .build()
        .send(&email)
        .map(|_| ())
        .map_err(safe_send_error)
}

/// Builds TLS parameters for the configured host.
///
/// The domain is used for certificate verification. A failure here is reported
/// as configuration, not retried in plaintext.
fn tls_parameters(settings: &SmtpSettings) -> Result<TlsParameters, &'static str> {
    TlsParameters::new(settings.host.clone()).map_err(|_| "smtp_tls_config")
}

/// Maps a lettre error to a short, safe code.
///
/// The mapping preserves the useful distinction between a transient rejection,
/// a permanent rejection, a timeout, a TLS failure, and a client/configuration
/// error, without exposing provider text, addresses, credentials, or bodies.
fn safe_send_error(error: lettre::transport::smtp::Error) -> &'static str {
    if socket_timeout(&error) {
        "smtp_timeout"
    } else if error.is_tls() {
        "smtp_tls_failed"
    } else if error.is_transient() {
        "smtp_transient_rejected"
    } else if error.is_permanent() {
        "smtp_rejected"
    } else if error.is_client() {
        "smtp_client_error"
    } else {
        "smtp_send_failed"
    }
}

/// Returns true when the failure is a socket deadline.
///
/// `lettre::Error::is_timeout` only recognizes `ErrorKind::TimedOut`. On macOS a
/// `SO_RCVTIMEO` expiry surfaces as `WouldBlock`, so both kinds count here.
fn socket_timeout(error: &lettre::transport::smtp::Error) -> bool {
    if error.is_timeout() {
        return true;
    }
    let mut source = std::error::Error::source(error);
    while let Some(error) = source {
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            if matches!(
                io.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                return true;
            }
        }
        source = std::error::Error::source(error);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn settings() -> SmtpSettings {
        SmtpSettings {
            host: "127.0.0.1".to_string(),
            port: 1025,
            tls: SmtpTlsMode::None,
            from: "tran@example.test".to_string(),
            username: None,
            password: None,
            timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn malformed_sender_is_refused_before_connecting() {
        let mut settings = settings();
        settings.from = "not-a-mailbox".to_string();
        assert_eq!(
            send(&settings, "person@example.test", "subject", "body"),
            Err("smtp_invalid_from")
        );
    }

    #[test]
    fn malformed_recipient_is_refused_before_connecting() {
        assert_eq!(
            send(&settings(), "not-a-mailbox", "subject", "body"),
            Err("smtp_invalid_recipient")
        );
    }

    #[test]
    fn tls_modes_are_explicit_and_parsed_strictly() {
        assert_eq!(SmtpTlsMode::parse("none"), Some(SmtpTlsMode::None));
        assert_eq!(SmtpTlsMode::parse("implicit"), Some(SmtpTlsMode::Implicit));
        assert_eq!(SmtpTlsMode::parse("tls"), Some(SmtpTlsMode::Implicit));
        assert_eq!(SmtpTlsMode::parse("starttls"), Some(SmtpTlsMode::StartTls));
        assert_eq!(SmtpTlsMode::parse("start_tls"), Some(SmtpTlsMode::StartTls));
        assert_eq!(SmtpTlsMode::parse("opportunistic"), None);
        assert_eq!(SmtpTlsMode::parse(""), None);
    }
}
