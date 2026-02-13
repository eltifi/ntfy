use crate::state::AppState;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::Result;
use tracing::{info, debug, error};
use crate::types::{Message, EVENT_MESSAGE};
use uuid::Uuid;
use chrono::Utc;

pub struct SmtpServer {
    state: AppState,
}

impl SmtpServer {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    pub async fn run(&self) -> Result<()> {
        let addr = &self.state.config.smtp_server_listen;
        if addr.is_empty() {
            debug!("SMTP server not configured, skipping");
            return Ok(());
        }

        info!("SMTP server listening on {}", addr);
        let listener = TcpListener::bind(addr).await?;

        loop {
            let (socket, _) = listener.accept().await?;
            let state = self.state.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_smtp_connection(socket, state).await {
                    error!("SMTP connection error: {}", e);
                }
            });
        }
    }
}

async fn handle_smtp_connection(mut socket: TcpStream, state: AppState) -> Result<()> {
    let domain = &state.config.smtp_server_domain;
    
    // Initial greeting
    socket.write_all(format!("220 {} ESMTP ntfy\r\n", domain).as_bytes()).await?;

    let mut buf = [0; 1024];
    let mut buffer = String::new();
    let mut topic_id = String::new();
    let mut data_mode = false;
    let mut data_buffer = Vec::new();

    loop {
        if !data_mode {
            let n = socket.read(&mut buf).await?;
            if n == 0 {
                return Ok(());
            }
            buffer.push_str(&String::from_utf8_lossy(&buf[..n]));

            while let Some(line_end) = buffer.find("\r\n") {
                let line = buffer[..line_end].to_string();
                buffer = buffer[line_end + 2..].to_string();
                
                let upper_line = line.to_uppercase();
                if upper_line.starts_with("HELO") || upper_line.starts_with("EHLO") {
                    socket.write_all(b"250 Hello\r\n").await?;
                } else if upper_line.starts_with("MAIL FROM:") {
                    socket.write_all(b"250 OK\r\n").await?;
                } else if upper_line.starts_with("RCPT TO:") {
                    // Extract email address: RCPT TO:<topic@domain>
                    let start = line.find('<').unwrap_or(0) + 1;
                    let end = line.find('>').unwrap_or(line.len());
                    let email = line[start..end].trim();
                    
                    if let Some((t, d)) = email.split_once('@') {
                        if d == domain {
                            topic_id = t.to_string(); // Handles topic+token logic later?
                            socket.write_all(b"250 OK\r\n").await?;
                        } else {
                            socket.write_all(b"550 Invalid domain\r\n").await?;
                            return Ok(()); // Close connection? Or just reject recipient
                        }
                    } else {
                         socket.write_all(b"550 Invalid address\r\n").await?;
                    }
                } else if upper_line.starts_with("DATA") {
                    data_mode = true;
                    socket.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n").await?;
                    // Remaining buffer might contain data?
                    // Yes, we should process `buffer` for data now.
                    if !buffer.is_empty() {
                         data_buffer.extend_from_slice(buffer.as_bytes());
                         buffer.clear();
                    }
                    break; // Break inner loop to go to outer loop for data reading
                } else if upper_line.starts_with("QUIT") {
                    socket.write_all(b"221 Bye\r\n").await?;
                    return Ok(());
                } else {
                    socket.write_all(b"500 Command not recognized\r\n").await?;
                }
            }
        } else {
            // Data mode
            let n = socket.read(&mut buf).await?;
            if n == 0 {
                return Ok(());
            }
            data_buffer.extend_from_slice(&buf[..n]);

            // Check for end of data sequence: "\r\n.\r\n"
            // We need to be careful about buffer splitting
            // Simplistic check for now: convert last 5 chars? 
            if data_buffer.ends_with(b"\r\n.\r\n") {
                let email_data = &data_buffer[..data_buffer.len() - 5];
                
                // Parse email
                if let Some(email) = mail_parser::MessageParser::default().parse(email_data) {
                    let subject = email.subject().unwrap_or("ntfy notification").to_string();
                    let body = email.body_text(0).unwrap_or(std::borrow::Cow::Borrowed("")).to_string();

                    let id = Uuid::new_v4().to_string().chars().take(12).collect::<String>();
                    let msg = Message {
                        id,
                        sequence_id: None,
                        time: Utc::now().timestamp(),
                        expires: None,
                        event: EVENT_MESSAGE.to_string(),
                        topic: topic_id.clone(),
                        title: Some(subject),
                        message: Some(body),
                        priority: None,
                        tags: None,
                        click: None,
                        icon: None,
                        actions: None,
                        attachment: None,
                        poll_id: None,
                        content_type: Some("text/plain".to_string()),
                        encoding: None,
                        sender: None,
                        user: None,
                        published: true,
                    };
        
                    if let Err(e) = state.process_publish(msg, None).await {
                        error!("Failed to process email message: {}", e);
                        socket.write_all(b"451 Local error\r\n").await?;
                    } else {
                        socket.write_all(b"250 OK\r\n").await?;
                    }
                } else {
                    error!("Failed to parse email content");
                    socket.write_all(b"550 Invalid content\r\n").await?;
                }
                data_buffer.clear();
                data_mode = false;
            }
        }
    }
}
