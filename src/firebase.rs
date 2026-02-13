use crate::config::Config;
use crate::types::{Message, EVENT_MESSAGE, EVENT_OPEN, EVENT_KEEPALIVE, EVENT_DELETE, EVENT_CLEAR, EVENT_POLL_REQUEST};
use anyhow::{Result, anyhow};
use std::sync::Arc;
use serde::Serialize;
use tracing::{error, debug};
use yup_oauth2::{ServiceAccountAuthenticator, ServiceAccountKey};
use reqwest::Client;
use std::collections::HashMap;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_rustls::HttpsConnector;

pub type Authenticator = yup_oauth2::authenticator::Authenticator<HttpsConnector<HttpConnector>>;

#[derive(Clone)]
pub struct FirebaseClient {
    #[allow(dead_code)]
    config: Arc<Config>,
    client: Client,
    authenticator: Arc<Authenticator>,
    project_id: String,
}

#[derive(Serialize, Debug)]
struct FcmMessage {
    message: FcmMessagePayload,
}

#[derive(Serialize, Debug)]
struct FcmMessagePayload {
    topic: String,
    data: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    android: Option<FcmAndroidConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    apns: Option<FcmApnsConfig>,
}

#[derive(Serialize, Debug)]
struct FcmAndroidConfig {
    priority: String,
}

#[derive(Serialize, Debug)]
struct FcmApnsConfig {
    headers: HashMap<String, String>,
    payload: FcmApnsPayload,
}

#[derive(Serialize, Debug)]
struct FcmApnsPayload {
    aps: FcmAps,
    #[serde(flatten)]
    custom_data: HashMap<String, String>,
}

#[derive(Serialize, Debug)]
struct FcmAps {
    #[serde(rename = "mutable-content", skip_serializing_if = "Option::is_none")]
    mutable_content: Option<i32>,
    #[serde(rename = "content-available", skip_serializing_if = "Option::is_none")]
    content_available: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alert: Option<FcmApsAlert>,
}

#[derive(Serialize, Debug)]
struct FcmApsAlert {
    title: Option<String>,
    body: String,
}

impl FirebaseClient {
    pub async fn new(config: Arc<Config>) -> Result<Self> {
        let key_json = std::fs::read_to_string(&config.firebase_key_file)?;
        let key: ServiceAccountKey = serde_json::from_str(&key_json)?;
        let project_id = key.project_id.clone().ok_or_else(|| anyhow!("Missing project_id in Firebase key"))?;

        let authenticator = ServiceAccountAuthenticator::builder(key)
            .build()
            .await?;

        Ok(Self {
            config,
            client: Client::new(),
            authenticator: Arc::new(authenticator),
            project_id,
        })
    }

    pub async fn send(&self, msg: &Message) -> Result<()> {
        let token = self.authenticator.token(&["https://www.googleapis.com/auth/cloud-platform"]).await?;
        
        let fcm_msg = self.to_fcm_message(msg)?;
        let url = format!("https://fcm.googleapis.com/v1/projects/{}/messages:send", self.project_id);

        let response = self.client.post(&url)
            .bearer_auth(token.token().unwrap_or_default())
            .json(&fcm_msg)
            .send()
            .await?;

        if response.status().is_success() {
            debug!("Firebase message sent to topic {}", msg.topic);
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("Failed to send Firebase message: {} - {}", status, body);
            return Err(anyhow!("FCM error: {} - {}", status, body));
        }

        Ok(())
    }

    fn to_fcm_message(&self, m: &Message) -> Result<FcmMessage> {
        let mut data = HashMap::new();
        data.insert("id".to_string(), m.id.clone());
        data.insert("time".to_string(), m.time.to_string());
        data.insert("event".to_string(), m.event.clone());
        data.insert("topic".to_string(), m.topic.clone());

        let mut apns_alert = None;
        let mut apns_headers = HashMap::new();
        let mut apns_content_available = None;
        let mut apns_mutable_content = None;

        match m.event.as_str() {
            EVENT_KEEPALIVE | EVENT_OPEN => {
                apns_content_available = Some(1);
                apns_headers.insert("apns-push-type".to_string(), "background".to_string());
                apns_headers.insert("apns-priority".to_string(), "5".to_string());
            }
            EVENT_POLL_REQUEST => {
                data.insert("message".to_string(), "New message".to_string());
                if let Some(pid) = &m.poll_id {
                    data.insert("poll_id".to_string(), pid.clone());
                }
                apns_alert = Some(FcmApsAlert {
                    title: m.title.clone(),
                    body: "New message".to_string(),
                });
                apns_mutable_content = Some(1);
            }
            EVENT_DELETE | EVENT_CLEAR => {
                if let Some(sid) = &m.sequence_id {
                    data.insert("sequence_id".to_string(), sid.clone());
                }
                apns_content_available = Some(1);
                apns_headers.insert("apns-push-type".to_string(), "background".to_string());
                apns_headers.insert("apns-priority".to_string(), "5".to_string());
            }
            EVENT_MESSAGE => {
                if let Some(sid) = &m.sequence_id { data.insert("sequence_id".to_string(), sid.clone()); }
                if let Some(p) = m.priority { data.insert("priority".to_string(), p.to_string()); }
                if let Some(t) = &m.tags { data.insert("tags".to_string(), t.join(",")); }
                if let Some(c) = &m.click { data.insert("click".to_string(), c.clone()); }
                if let Some(i) = &m.icon { data.insert("icon".to_string(), i.clone()); }
                if let Some(t) = &m.title { data.insert("title".to_string(), t.clone()); }
                if let Some(msg) = &m.message { data.insert("message".to_string(), msg.clone()); }
                if let Some(ct) = &m.content_type { data.insert("content_type".to_string(), ct.clone()); }
                if let Some(enc) = &m.encoding { data.insert("encoding".to_string(), enc.clone()); }
                
                if let Some(actions) = &m.actions {
                    data.insert("actions".to_string(), serde_json::to_string(actions)?);
                }
                
                if let Some(att) = &m.attachment {
                    data.insert("attachment_name".to_string(), att.name.clone());
                    if let Some(t) = &att.type_ { data.insert("attachment_type".to_string(), t.clone()); }
                    if let Some(s) = att.size { data.insert("attachment_size".to_string(), s.to_string()); }
                    if let Some(e) = att.expires { data.insert("attachment_expires".to_string(), e.to_string()); }
                    data.insert("attachment_url".to_string(), att.url.clone());
                }

                apns_alert = Some(FcmApsAlert {
                    title: m.title.clone(),
                    body: m.message.as_deref().unwrap_or("New message").to_string(),
                });
                apns_mutable_content = Some(1);
            }
            _ => {}
        }

        let android = if m.priority.unwrap_or(3) >= 4 {
            Some(FcmAndroidConfig { priority: "high".to_string() })
        } else {
            None
        };

        let apns = Some(FcmApnsConfig {
            headers: apns_headers,
            payload: FcmApnsPayload {
                aps: FcmAps {
                    mutable_content: apns_mutable_content,
                    content_available: apns_content_available,
                    alert: apns_alert,
                },
                custom_data: data.clone(),
            },
        });

        Ok(FcmMessage {
            message: FcmMessagePayload {
                topic: m.topic.clone(),
                data,
                android,
                apns,
            }
        })
    }
}
