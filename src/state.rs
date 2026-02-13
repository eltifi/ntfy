use std::sync::Arc;
use dashmap::DashMap;
use crate::config::Config;
use crate::auth::manager::AuthManager;
use crate::topic::Topic;
use crate::store::Store;
use crate::file_storage::FileStorage;
use crate::types::Message;
use axum::body::Body;
use crate::email::sender::Mailer;
use crate::limits::Limiter;
use crate::webpush::WebPushStore;
use crate::firebase::FirebaseClient;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub topics: Arc<DashMap<String, Topic>>,
    pub store: Store,
    pub file_storage: FileStorage,
    pub auth_manager: AuthManager,
    pub mailer: Mailer,
    pub limiter: Limiter,
    pub webpush: Option<WebPushStore>,
    pub firebase: Option<FirebaseClient>,
}

impl AppState {
    pub fn new(config: Config, store: Store, file_storage: FileStorage, auth_manager: AuthManager, mailer: Mailer, limiter: Limiter, webpush: Option<WebPushStore>, firebase: Option<FirebaseClient>) -> Self {
        Self {
            config: Arc::new(config),
            topics: Arc::new(DashMap::new()),
            store,
            file_storage,
            auth_manager,
            mailer,
            limiter,
            webpush,
            firebase,
        }
    }

    pub fn get_topic(&self, id: &str) -> Topic {
        if let Some(mut topic_ref) = self.topics.get_mut(id) {
            topic_ref.touch();
            return (*topic_ref).clone();
        }
        let topic = Topic::new(id.to_string());
        self.topics.insert(id.to_string(), topic.clone());
        topic
    }

    pub async fn process_publish(&self, msg: Message, _attachment_info: Option<(String, Body)>) -> Result<Message, String> {
        let topic = self.get_topic(&msg.topic);

        // Persist message
        if let Err(e) = self.store.insert_message(&msg).await {
            tracing::error!("Failed to persist message in store: {:?}", e);
            return Err("Failed to persist message".to_string());
        }

        if msg.published {
            let msg_arc = Arc::new(msg.clone());
            topic.publish(msg_arc);

            if let Some(wp) = &self.webpush {
                let wp = wp.clone();
                let msg = msg.clone();
                tokio::spawn(async move {
                    if let Err(e) = wp.send(&msg).await {
                        tracing::error!("Failed to send WebPush: {}", e);
                    }
                });
            }

            if let Some(fb) = &self.firebase {
                let fb = fb.clone();
                let msg = msg.clone();
                tokio::spawn(async move {
                    if let Err(e) = fb.send(&msg).await {
                        tracing::error!("Failed to send Firebase: {}", e);
                    }
                });
            }
        }
        
        Ok(msg)
    }
}
