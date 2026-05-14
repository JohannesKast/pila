//! Email delivery via SMTP (lettre).
//!
//! SMTP configuration is global (same mail server for all leagues).
//! If the required env vars are missing, `SmtpConfig::from_env()` returns
//! `None` and all email operations become no-ops.

use lettre::message::{header::ContentType, Mailbox};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

pub type MailError = Box<dyn std::error::Error + Send + Sync>;

/// Global SMTP configuration, read once at startup.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub from: String,
}

impl SmtpConfig {
    /// Read from env vars. Returns `None` if any required var is missing/empty.
    pub fn from_env() -> Option<Self> {
        let host = std::env::var("SMTP_HOST").ok().filter(|s| !s.is_empty())?;
        let port: u16 = std::env::var("SMTP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(587);
        let user = std::env::var("SMTP_USER").ok().filter(|s| !s.is_empty())?;
        let pass = std::env::var("SMTP_PASS").ok().filter(|s| !s.is_empty())?;
        let from = std::env::var("SMTP_FROM")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| user.clone());

        Some(Self {
            host,
            port,
            user,
            pass,
            from,
        })
    }
}

/// Build an async SMTP transport from the config.
fn build_transport(cfg: &SmtpConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>, MailError> {
    let creds = Credentials::new(cfg.user.clone(), cfg.pass.clone());
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)?
        .port(cfg.port)
        .credentials(creds)
        .build();
    Ok(mailer)
}

/// Send a plain-text email.
pub async fn send_email(
    cfg: &SmtpConfig,
    to_address: &str,
    subject: &str,
    body: &str,
) -> Result<(), MailError> {
    let from_mailbox: Mailbox = cfg.from.parse().map_err(|e| format!("Invalid SMTP_FROM: {e}"))?;
    let to_mailbox: Mailbox = to_address.parse().map_err(|e| format!("Invalid recipient: {e}"))?;

    let email = Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(subject)
        .header(ContentType::parse("text/plain; charset=utf-8").unwrap())
        .body(body.to_string())?;

    let mailer = build_transport(cfg)?;
    mailer.send(email).await?;
    Ok(())
}

/// Send a magic-link invitation email to a new user.
pub async fn send_invite_email(
    cfg: &SmtpConfig,
    name: &str,
    email: &str,
    magic_link: &str,
) -> Result<(), MailError> {
    let subject = "Pila — Dein Login-Link";
    let body = format!(
        "Hallo {name}!\n\n\
         Du bist beim Pila WM-Tippspiel dabei.\n\
         Dein Login-Link: {magic_link}\n\n\
         Bookmarke diesen Link — er ist dein persönlicher Schlüssel. \
         Wer ihn kennt, ist als {name} eingeloggt.\n\n\
         Viel Erfolg beim Tippen!\n\
         — Pila"
    );
    send_email(cfg, email, subject, &body).await
}

/// Send a reminder email to a user who hasn't tipped a match yet.
pub async fn send_reminder_email(
    cfg: &SmtpConfig,
    name: &str,
    email: &str,
    home: &str,
    away: &str,
    stage_label: &str,
    magic_link: &str,
) -> Result<(), MailError> {
    let subject = format!("⚽ Tipp-Erinnerung: {home} – {away}");
    let body = format!(
        "Hallo {name}!\n\n\
         {home} – {away} ({stage_label}) wird in weniger als 24 Stunden angepfiffen \
         und du hast noch keinen Tipp abgegeben.\n\n\
         Jetzt tippen: {magic_link}\n\n\
         \u{2014} Pila"
    );
    send_email(cfg, email, &subject, &body).await
}

/// Send a reminder that the World Champion pick is about to lock.
pub async fn send_champion_reminder_email(
    cfg: &SmtpConfig,
    name: &str,
    email: &str,
    magic_link: &str,
) -> Result<(), MailError> {
    let subject = "⏰ Weltmeister-Tipp wird bald gesperrt";
    let body = format!(
        "Hallo {name}!\n\n\
         Der Weltmeister-Tipp wird in weniger als 24 Stunden gesperrt \
         (Anpfiff Eröffnungsspiel). Gib jetzt deinen Tipp ab!\n\n\
         {magic_link}\n\n\
         — Pila"
    );
    send_email(cfg, email, subject, &body).await
}
