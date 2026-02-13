use crate::types::{Message, Attachment, Action};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite, Row};
use std::path::Path;
use tracing::{info, error};
use anyhow::Result;
use serde_json;
use chrono;

#[derive(Clone)]
pub struct Store {
    pool: Pool<Sqlite>,
}

impl Store {
    pub async fn new(path: Option<&Path>) -> Result<Self> {
        let db_url = match path {
            Some(p) if !p.as_os_str().is_empty() => format!("sqlite://{}?mode=rwc", p.display()),
            _ => "sqlite::memory:".to_string(),
        };
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await?;

        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<()> {
        // Create schema version table if not exists
        let res = sqlx::query(
            "CREATE TABLE IF NOT EXISTS schemaVersion (id INT PRIMARY KEY, version INT NOT NULL)"
        )
        .execute(&self.pool)
        .await;
        if let Err(ref e) = res {
            error!("Failed to create schemaVersion table: {:?}", e);
        }
        res?;

        // Get current version
        let version: i32 = sqlx::query("SELECT version FROM schemaVersion WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?
            .map(|row| row.get(0))
            .unwrap_or(0);

        let target_version = 14; 
        
        info!("Checking database version (current={}, target={})", version, target_version);

        if version < target_version {
            info!("Migrating database from version {} to {}", version, target_version);

            if version < 1 {
                let schema = r#"
                CREATE TABLE IF NOT EXISTS messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    mid TEXT NOT NULL,
                    sequence_id TEXT NOT NULL,
                    time INT NOT NULL,
                    event TEXT NOT NULL,
                    expires INT NOT NULL,
                    topic TEXT NOT NULL,
                    message TEXT NOT NULL,
                    title TEXT NOT NULL,
                    priority INT NOT NULL,
                    tags TEXT NOT NULL,
                    click TEXT NOT NULL,
                    icon TEXT NOT NULL,
                    actions TEXT NOT NULL,
                    attachment_name TEXT NOT NULL,
                    attachment_type TEXT NOT NULL,
                    attachment_size INT NOT NULL,
                    attachment_expires INT NOT NULL,
                    attachment_url TEXT NOT NULL,
                    attachment_deleted INT NOT NULL,
                    sender TEXT NOT NULL,
                    user TEXT NOT NULL,
                    content_type TEXT NOT NULL,
                    encoding TEXT NOT NULL,
                    published INT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_mid ON messages (mid);
                CREATE INDEX IF NOT EXISTS idx_sequence_id ON messages (sequence_id);
                CREATE INDEX IF NOT EXISTS idx_time ON messages (time);
                CREATE INDEX IF NOT EXISTS idx_topic ON messages (topic);
                CREATE INDEX IF NOT EXISTS idx_expires ON messages (expires);
                CREATE INDEX IF NOT EXISTS idx_sender ON messages (sender);
                CREATE INDEX IF NOT EXISTS idx_user ON messages (user);
                CREATE INDEX IF NOT EXISTS idx_attachment_expires ON messages (attachment_expires);
                "#;
                let res = sqlx::query(schema).execute(&self.pool).await;
                if let Err(ref e) = res {
                    error!("Failed to create messages table: {:?}", e);
                }
                res?;
            }
            
            // Always ensure stats table exists and is initialized
            sqlx::query("CREATE TABLE IF NOT EXISTS stats (key TEXT PRIMARY KEY, value INT)")
                .execute(&self.pool)
                .await?;
            sqlx::query("INSERT OR IGNORE INTO stats (key, value) VALUES ('messages', 0)")
                .execute(&self.pool)
                .await?;

            sqlx::query("INSERT OR REPLACE INTO schemaVersion (id, version) VALUES (1, ?)")
                .bind(target_version)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    pub async fn insert_message(&self, m: &Message) -> Result<()> {
        let tags = m.tags.clone().unwrap_or_default().join(",");
        let actions = if let Some(a) = &m.actions {
            serde_json::to_string(a)?
        } else {
            String::new()
        };
        
        let (att_name, att_type, att_size, att_expires, att_url) = if let Some(a) = &m.attachment {
            (a.name.clone(), a.type_.clone().unwrap_or_default(), a.size.unwrap_or(0), a.expires.unwrap_or(0), a.url.clone())
        } else {
            (String::new(), String::new(), 0, 0, String::new())
        };

        let res = sqlx::query(
            r#"
            INSERT INTO messages (
                mid, sequence_id, time, event, expires, topic, message, title, priority, tags, 
                click, icon, actions, attachment_name, attachment_type, attachment_size, 
                attachment_expires, attachment_url, attachment_deleted, sender, user, 
                content_type, encoding, published
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&m.id)
        .bind(m.sequence_id.as_deref().unwrap_or(""))
        .bind(m.time)
        .bind(&m.event)
        .bind(m.expires.unwrap_or(0))
        .bind(&m.topic)
        .bind(m.message.as_deref().unwrap_or(""))
        .bind(m.title.as_deref().unwrap_or(""))
        .bind(m.priority.unwrap_or(0))
        .bind(tags)
        .bind(m.click.as_deref().unwrap_or(""))
        .bind(m.icon.as_deref().unwrap_or(""))
        .bind(actions)
        .bind(att_name)
        .bind(att_type)
        .bind(att_size)
        .bind(att_expires)
        .bind(att_url)
        .bind(0) // attachment_deleted
        .bind(m.sender.as_deref().unwrap_or(""))
        .bind(m.user.as_deref().unwrap_or(""))
        .bind(m.content_type.as_deref().unwrap_or(""))
        .bind(m.encoding.as_deref().unwrap_or(""))
        .bind(if m.published { 1 } else { 0 })
        .execute(&self.pool)
        .await;

        if let Err(ref e) = res {
            tracing::error!("SQL error in insert_message: {:?}", e);
        }
        res?;

        Ok(())
    }

    pub async fn get_messages(&self, topic: &str, since: i64) -> Result<Vec<Message>> {
        let rows = sqlx::query(
            r#"
            SELECT 
                mid, sequence_id, time, event, expires, topic, message, title, priority, tags, 
                click, icon, actions, attachment_name, attachment_type, attachment_size, 
                attachment_expires, attachment_url, sender, user, content_type, encoding, published
            FROM messages
            WHERE topic = ? AND time >= ? AND published = 1
            ORDER BY time, id
            "#
        )
        .bind(topic)
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::new();
        for row in rows {
            let tags_str: String = row.try_get("tags")?;
            let tags = if tags_str.is_empty() { None } else { Some(tags_str.split(',').map(String::from).collect()) };
            
            let actions_str: String = row.try_get("actions")?;
            let actions: Option<Vec<Action>> = if actions_str.is_empty() { None } else { serde_json::from_str(&actions_str).ok() };

            let att_name: String = row.try_get("attachment_name")?;
            let attachment = if att_name.is_empty() {
                None
            } else {
                Some(Attachment {
                    name: att_name,
                    type_: Some(row.try_get("attachment_type")?),
                    size: Some(row.try_get("attachment_size")?),
                    expires: Some(row.try_get("attachment_expires")?),
                    url: row.try_get("attachment_url")?,
                })
            };

            messages.push(Message {
                id: row.try_get("mid")?,
                sequence_id: Some(row.try_get("sequence_id")?),
                time: row.try_get("time")?,
                expires: Some(row.try_get("expires")?),
                event: row.try_get("event")?,
                topic: row.try_get("topic")?,
                title: Some(row.try_get("title")?),
                message: Some(row.try_get("message")?),
                priority: Some(row.try_get("priority")?),
                tags,
                click: Some(row.try_get("click")?),
                icon: Some(row.try_get("icon")?),
                actions,
                attachment,
                poll_id: None, 
                content_type: Some(row.try_get("content_type")?),
                encoding: Some(row.try_get("encoding")?),
                sender: Some(row.try_get("sender")?),
                user: Some(row.try_get("user")?),
                published: row.try_get::<i32, _>("published")? != 0,
            });
        }
        
        Ok(messages)
    }

    pub async fn get_due_messages(&self) -> Result<Vec<Message>> {
        let now = chrono::Utc::now().timestamp();
        let rows = sqlx::query(
            r#"
            SELECT 
                mid, sequence_id, time, event, expires, topic, message, title, priority, tags, 
                click, icon, actions, attachment_name, attachment_type, attachment_size, 
                attachment_expires, attachment_url, sender, user, content_type, encoding, published
            FROM messages
            WHERE time <= ? AND published = 0
            ORDER BY time, id
            "#
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::new();
        for row in rows {
            let tags_str: String = row.try_get("tags")?;
            let tags = if tags_str.is_empty() { None } else { Some(tags_str.split(',').map(String::from).collect()) };
            
            let actions_str: String = row.try_get("actions")?;
            let actions: Option<Vec<Action>> = if actions_str.is_empty() { None } else { serde_json::from_str(&actions_str).ok() };

            let att_name: String = row.try_get("attachment_name")?;
            let attachment = if att_name.is_empty() {
                None
            } else {
                Some(Attachment {
                    name: att_name,
                    type_: Some(row.try_get("attachment_type")?),
                    size: Some(row.try_get("attachment_size")?),
                    expires: Some(row.try_get("attachment_expires")?),
                    url: row.try_get("attachment_url")?,
                })
            };

            messages.push(Message {
                id: row.try_get("mid")?,
                sequence_id: Some(row.try_get("sequence_id")?),
                time: row.try_get("time")?,
                expires: Some(row.try_get("expires")?),
                event: row.try_get("event")?,
                topic: row.try_get("topic")?,
                title: Some(row.try_get("title")?),
                message: Some(row.try_get("message")?),
                priority: Some(row.try_get("priority")?),
                tags,
                click: Some(row.try_get("click")?),
                icon: Some(row.try_get("icon")?),
                actions,
                attachment,
                poll_id: None, 
                content_type: Some(row.try_get("content_type")?),
                encoding: Some(row.try_get("encoding")?),
                sender: Some(row.try_get("sender")?),
                user: Some(row.try_get("user")?),
                published: false,
            });
        }
        
        Ok(messages)
    }

    pub async fn mark_published(&self, mid: &str) -> Result<()> {
        sqlx::query("UPDATE messages SET published = 1 WHERE mid = ?")
            .bind(mid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune_messages(&self, age: i64) -> Result<u64> {
        let result = sqlx::query("DELETE FROM messages WHERE time < ?")
            .bind(age)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn get_expired_attachments(&self) -> Result<Vec<String>> {
        let now = chrono::Utc::now().timestamp();
        let rows = sqlx::query("SELECT mid FROM messages WHERE attachment_expires > 0 AND attachment_expires < ? AND attachment_deleted = 0")
            .bind(now)
            .fetch_all(&self.pool)
            .await?;
            
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.try_get("mid")?);
        }
        Ok(ids)
    }

    pub async fn mark_attachment_deleted(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE messages SET attachment_deleted = 1 WHERE mid = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_sender_attachment_size(&self, sender: &str) -> Result<i64> {
        let now = chrono::Utc::now().timestamp();
        let row: (i64,) = sqlx::query_as("SELECT IFNULL(SUM(attachment_size), 0) FROM messages WHERE user = '' AND sender = ? AND attachment_expires >= ? AND attachment_deleted = 0")
            .bind(sender)
            .bind(now)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    pub async fn get_user_attachment_size(&self, user_id: &str) -> Result<i64> {
        let now = chrono::Utc::now().timestamp();
        let row: (i64,) = sqlx::query_as("SELECT IFNULL(SUM(attachment_size), 0) FROM messages WHERE user = ? AND attachment_expires >= ? AND attachment_deleted = 0")
            .bind(user_id)
            .bind(now)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }
}
