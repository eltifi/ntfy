use sqlx::{SqlitePool, Row};
use crate::config::Config;
use anyhow::Result;
use std::sync::Arc;
use web_push::*;
use serde::{Deserialize, Serialize};
use tracing::{info, error, debug};
use crate::types::Message;

#[derive(Clone)]
pub struct WebPushStore {
    pool: SqlitePool,
    config: Arc<Config>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WebPushSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub topics: Vec<String>,
}

impl WebPushStore {
    pub async fn new(pool: SqlitePool, config: Arc<Config>) -> Result<Self> {
        let store = Self { pool, config };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS webpush_subscription (
                id TEXT PRIMARY KEY,
                endpoint TEXT NOT NULL,
                key_auth TEXT NOT NULL,
                key_p256dh TEXT NOT NULL,
                user_id TEXT,
                subscriber_ip TEXT NOT NULL,
                updated_at INT NOT NULL,
                warned_at INT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_webpush_endpoint ON webpush_subscription (endpoint);
            CREATE INDEX IF NOT EXISTS idx_webpush_user_id ON webpush_subscription (user_id);

            CREATE TABLE IF NOT EXISTS webpush_subscription_topic (
                subscription_id TEXT NOT NULL,
                topic TEXT NOT NULL,
                PRIMARY KEY (subscription_id, topic),
                FOREIGN KEY (subscription_id) REFERENCES webpush_subscription (id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_webpush_topic ON webpush_subscription_topic (topic);
            "#
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_subscription(&self, endpoint: &str, auth: &str, p256dh: &str, user_id: Option<&str>, ip: &str, topics: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let id: String = sqlx::query("SELECT id FROM webpush_subscription WHERE endpoint = ?")
            .bind(endpoint)
            .fetch_optional(&mut *tx)
            .await?
            .map(|r| r.get(0))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            INSERT INTO webpush_subscription (id, endpoint, key_auth, key_p256dh, user_id, subscriber_ip, updated_at, warned_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, 0)
            ON CONFLICT(endpoint) DO UPDATE SET
                key_auth = excluded.key_auth,
                key_p256dh = excluded.key_p256dh,
                user_id = excluded.user_id,
                subscriber_ip = excluded.subscriber_ip,
                updated_at = excluded.updated_at
            "#
        )
        .bind(&id)
        .bind(endpoint)
        .bind(auth)
        .bind(p256dh)
        .bind(user_id)
        .bind(ip)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM webpush_subscription_topic WHERE subscription_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await?;

        for topic in topics {
            sqlx::query("INSERT INTO webpush_subscription_topic (subscription_id, topic) VALUES (?, ?)")
                .bind(&id)
                .bind(topic)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_subscription(&self, endpoint: &str) -> Result<()> {
        sqlx::query("DELETE FROM webpush_subscription WHERE endpoint = ?")
            .bind(endpoint)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_subscriptions_for_topic(&self, topic: &str) -> Result<Vec<WebPushSubscription>> {
        let rows = sqlx::query(
            r#"
            SELECT s.endpoint, s.key_auth as auth, s.key_p256dh as p256dh
            FROM webpush_subscription s
            JOIN webpush_subscription_topic t ON s.id = t.subscription_id
            WHERE t.topic = ?
            "#
        )
        .bind(topic)
        .fetch_all(&self.pool)
        .await?;

        let subs = rows.into_iter().map(|r| WebPushSubscription {
            endpoint: r.get("endpoint"),
            p256dh: r.get("p256dh"),
            auth: r.get("auth"),
            topics: vec![],
        }).collect();

        Ok(subs)
    }

    pub async fn send(&self, msg: &Message) -> Result<()> {
        if self.config.web_push_public_key.is_empty() {
            return Ok(());
        }

        let subs = self.get_subscriptions_for_topic(&msg.topic).await?;
        if subs.is_empty() {
            return Ok(());
        }

        let payload = serde_json::to_string(msg)?;

        for sub in subs {
            let subscription_info = SubscriptionInfo::new(
                sub.endpoint.clone(),
                sub.p256dh.clone(),
                sub.auth.clone(),
            );

            let sig_builder = VapidSignatureBuilder::from_base64(
                &self.config.web_push_private_key,
                &subscription_info,
            )?;
            
            let mut builder = WebPushMessageBuilder::new(&subscription_info);
            builder.set_payload(ContentEncoding::Aes128Gcm, payload.as_bytes());
            
            // In web-push 0.11, VapidSignatureBuilder::build returns Result<VapidSignature, WebPushError>
            // and we can set the subject via set_claim if needed, but it's often not needed for simple usage
            // or it's part of the builder.
            builder.set_vapid_signature(sig_builder.build()?);
            
            let client = IsahcWebPushClient::new()?;
            let response = client.send(builder.build()?).await;

            match response {
                Ok(_) => debug!("WebPush message sent to {}", sub.endpoint),
                Err(WebPushError::EndpointNotValid(_)) | Err(WebPushError::EndpointNotFound(_)) => {
                    info!("WebPush endpoint no longer valid, removing: {}", sub.endpoint);
                    let _ = self.remove_subscription(&sub.endpoint).await;
                }
                Err(e) => error!("Failed to send WebPush to {}: {:?}", sub.endpoint, e),
            }
        }

        Ok(())
    }
}