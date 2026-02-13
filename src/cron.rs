use crate::state::AppState;
use std::time::Duration;
use tokio::time;
use tracing::{info, error};
use chrono::Utc;

pub struct Manager {
    state: AppState,
}

impl Manager {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    pub async fn run(&self) {
        info!("Maintenance manager started");
        
        // Define intervals
        let message_prune_interval = Duration::from_secs(3600); // 1 hour
        let delayed_message_interval = Duration::from_secs(10); // 10 seconds

        let mut prune_interval = time::interval(message_prune_interval);
        let mut delayed_interval = time::interval(delayed_message_interval);

        loop {
            tokio::select! {
                _ = prune_interval.tick() => {
                    if let Err(e) = self.prune_messages().await {
                        error!("Failed to prune messages: {}", e);
                    }
                    if let Err(e) = self.prune_attachments().await {
                         error!("Failed to prune attachments: {}", e);
                    }
                }
                _ = delayed_interval.tick() => {
                    if let Err(e) = self.send_delayed_messages().await {
                        error!("Failed to send delayed messages: {}", e);
                    }
                }
            }
        }
    }

    async fn send_delayed_messages(&self) -> anyhow::Result<()> {
        let messages = self.state.store.get_due_messages().await?;
        if messages.is_empty() {
            return Ok(());
        }

        info!("Sending {} delayed messages", messages.len());
        for msg in messages {
            let mid = msg.id.clone();
            // In Rust port, we don't have full visitor context here yet, 
            // but we can at least publish to the topic and email.
            
            // 1. Publish to topic
            let topic = self.state.get_topic(&msg.topic);
            topic.publish(std::sync::Arc::new(msg.clone()));

            // 2. Send email if requested (we'd need to re-parse headers or have it in DB)
            // For now, let's just mark as published.
            // Go version uses MarkPublished which updates DB.
            
            if let Err(e) = self.state.store.mark_published(&mid).await {
                error!("Failed to mark message {} as published: {}", mid, e);
            }
        }
        Ok(())
    }

    async fn prune_messages(&self) -> anyhow::Result<()> {
        let cache_duration_secs = self.state.config.cache_duration as i64;
        let older_than = Utc::now().timestamp() - cache_duration_secs;
        
        let count = self.state.store.prune_messages(older_than).await?;
        if count > 0 {
            info!("Pruned {} expired messages", count);
        }
        Ok(())
    }
    
    async fn prune_attachments(&self) -> anyhow::Result<()> {
        // 1. Get expired attachments from DB
        let expired_ids = self.state.store.get_expired_attachments().await?;
        if !expired_ids.is_empty() {
            info!("Found {} expired attachments to delete", expired_ids.len());
            let count = self.state.file_storage.remove_by_ids(&expired_ids).await?;
            info!("Deleted {} expired attachment files", count);
            
            // Mark as deleted in DB
            for id in expired_ids {
                self.state.store.mark_attachment_deleted(&id).await?;
            }
        }
        
        // 2. Scan for orphans (files not in DB)
        // This is expensive and risky if we have race conditions. 
        // For now, let's rely on expiry.
        Ok(())
    }
}
