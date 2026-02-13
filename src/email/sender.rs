use crate::config::Config;
use crate::types::Message;
use anyhow::{Result, Context};
use lettre::{Message as EmailMessage, AsyncSmtpTransport, AsyncTransport};
use lettre::transport::smtp::authentication::Credentials;
use lettre::message::header::ContentType;
use lettre::Tokio1Executor;
use std::sync::Arc;
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct Mailer {
    config: Arc<Config>,
}

impl Mailer {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn send(&self, to: &str, msg: &Message) -> Result<()> {
        if self.config.smtp_sender_addr.is_empty() {
            debug!("SMTP sender address not configured, skipping email");
            return Ok(());
        }

        info!("Sending email to {} via {}", to, self.config.smtp_sender_addr);

        // Parse host and port
        // lettre expects "host" or "host:port" in some contexts, but let's see.
        // AsyncSmtpTransport::relay takes a host string.
        // If smtp_sender_addr contains port, we might need to handle it.
        // But for now let's assume it handles it or we split it.
        
        let host = if let Some((h, _)) = self.config.smtp_sender_addr.split_once(':') {
             h
        } else {
             &self.config.smtp_sender_addr
        };

        // Build email
        let topic_url = format!("{}/{}", self.config.base_url, msg.topic);
        let subject = msg.title.clone().unwrap_or_else(|| "ntfy notification".to_string()); // Default subject
        let body_text = msg.message.clone().unwrap_or_default();
        
        let email = EmailMessage::builder()
            .from(self.config.smtp_sender_from.parse().context("Invalid from address")?)
            .to(to.parse().context("Invalid to address")?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(format!("{}\n\n--\nSent via {}", body_text, topic_url))
            .context("Failed to build email")?;

        // Configure transport
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .context("Failed to create mailer relay")?;

        if !self.config.smtp_sender_user.is_empty() {
            let creds = Credentials::new(
                self.config.smtp_sender_user.clone(),
                self.config.smtp_sender_pass.clone(),
            );
            builder = builder.credentials(creds);
        }

        let mailer = builder.build();

        match mailer.send(email).await {
            Ok(_) => info!("Email sent successfully to {}", to),
            Err(e) => error!("Failed to send email to {}: {}", to, e),
        }
        
        Ok(())
    }
}
