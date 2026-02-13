use sqlx::SqlitePool;
use crate::config::Config;
use crate::auth::types::{User, Permission};
use anyhow::Result;
use std::sync::Arc;
use bcrypt;

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    name: String,
    hash: String,
    role: String,
    tier: String,
}

#[derive(Clone)]
pub struct AuthManager {
    pool: SqlitePool,
    config: Arc<Config>,
}

impl AuthManager {
    pub async fn new(pool: SqlitePool, config: Arc<Config>) -> Result<Self> {
        Ok(Self { pool, config })
    }

    pub async fn authenticate(&self, username: &str, password: &str) -> Result<Option<User>> {
        // 1. Fetch user by username
        let user_opt = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, user as name, pass as hash, role, tier
            FROM user
            WHERE user = ?
            "#
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(user_row) = user_opt {
             // 2. Verify password (bcrypt)
             // Go uses bcrypt.Verify(hash, password)
             // Rust bcrypt::verify(password, hash)
             let valid = bcrypt::verify(password, &user_row.hash)?;
             if valid {
                 return Ok(Some(User {
                     id: user_row.id,
                     name: user_row.name,
                     hash: user_row.hash,
                     role: user_row.role.into(),
                     tier: user_row.tier.into(),
                 }));
             }
        }
        
        Ok(None)
    }

    pub async fn authenticate_token(&self, token: &str) -> Result<Option<User>> {
         let user_opt = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT u.id, u.user as name, u.pass as hash, u.role, u.tier
            FROM user u
            JOIN user_token tk on u.id = tk.user_id
            WHERE tk.token = ? AND (tk.expires = 0 OR tk.expires >= strftime('%s', 'now'))
            "#
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(user_row) = user_opt {
             return Ok(Some(User {
                 id: user_row.id,
                 name: user_row.name,
                 hash: user_row.hash,
                 role: user_row.role.into(),
                 tier: user_row.tier.into(),
             }));
        }
        Ok(None)
    }

    pub async fn authorize(&self, user: &User, topic: &str, permission: Permission) -> Result<bool> {
        // Go logic:
        // 1. Check if user has specific access to topic (or wildcard)
        // 2. Check if user has reservation for topic
        // 3. Check for anonymous access (if user is everyone)

        // For now, let's implement a simplified version that checks `user_access` table
        // Matches `selectTopicPermsQuery` in Go:
        // SELECT read, write FROM user_access ... WHERE (user = ? OR user = ?) AND ? LIKE topic ...
        
        let username = &user.name;
        let anon_username = "*"; // Everyone

        // We need to check both the specific user and the anonymous user
        // And we need to match glob patterns (Go does `LIKE topic ESCAPE '\'`)
        // SQLite `LIKE` is case-insensitive by default in some builds, but we want case-insensitive for topics?
        // ntfy Go topics are case-sensitive?
        // Go query: WHERE (u.user = ? OR u.user = ?) AND ? LIKE a.topic ESCAPE '\'
        
        #[derive(sqlx::FromRow)]
        struct PermRow {
            read: i64,
            write: i64,
        }

        // Check explicit permissions
        let perms: Vec<PermRow> = sqlx::query_as::<_, PermRow>(
            r#"
            SELECT read, write
            FROM user_access a
            JOIN user u ON u.id = a.user_id
            WHERE (u.user = ? OR u.user = ?) 
              -- SQLite LIKE matching: topic pattern matching the topic
            AND ? LIKE a.topic ESCAPE '\'
            ORDER BY u.user DESC, length(a.topic) DESC, a.write DESC
            "#
        )
        .bind(username)
        .bind(anon_username)
        .bind(topic)
        .fetch_all(&self.pool)
        .await?;
        
        // Iterate and find first match?
        // Go sorts by user DESC (specific user first, then anonymous), Length DESC (most specific match), Write DESC.
        // So the first result is the most specific permission.
        
        if let Some(perm) = perms.first() {
            let p = Permission::new(perm.read != 0, perm.write != 0);
            if permission == Permission::Read && p.is_read() {
                return Ok(true);
            }
            if permission == Permission::Write && p.is_write() {
                return Ok(true);
            }
            if permission == Permission::ReadWrite && p.is_read() && p.is_write() {
                return Ok(true);
            }
            // If explicit permission exists but doesn't grant enough access, we DENY?
            // Go logic: "If strict matching found, return it."
            // So yes, returning false here is correct.
            return Ok(false);
        }
        
        // If no explicit permission, apply default policy
        // We use config.auth_default_access
        
        // Parse default permission string
        // "read-write" -> ReadWrite
        // "read-only" -> Read
        // "write-only" -> Write
        // "deny-all" -> DenyAll
        
        let default_perm_str = &self.config.auth_default_access;
        let default_perm = match default_perm_str.as_str() {
            "read-write" | "rw" => Permission::ReadWrite,
            "read-only" | "read" | "ro" => Permission::Read,
            "write-only" | "write" | "wo" => Permission::Write,
            _ => Permission::DenyAll,
        };
        
        if permission == Permission::Read && default_perm.is_read() {
            return Ok(true);
        }
        if permission == Permission::Write && default_perm.is_write() {
            return Ok(true);
        }
        if permission == Permission::ReadWrite && default_perm.is_read() && default_perm.is_write() {
             return Ok(true);
        }

        Ok(false) 
    }

    pub async fn _has_reservation(&self, username: &str, topic: &str) -> Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM user_access a JOIN user u ON u.id = a.user_id WHERE u.user = ? AND a.topic = ? AND a.owner_user_id = u.id")
            .bind(username)
            .bind(topic)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn add_user(&self, username: &str, password: &str, role: crate::auth::types::Role) -> Result<()> {
        let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)?;
        let id = uuid::Uuid::new_v4().to_string();
        let role_str: String = role.into();
        
        sqlx::query("INSERT INTO user (id, user, pass, role, tier) VALUES (?, ?, ?, ?, ?)")
            .bind(id)
            .bind(username)
            .bind(hash)
            .bind(role_str)
            .bind("free") // Default tier
            .execute(&self.pool)
            .await?;
            
        Ok(())
    }

    pub async fn delete_user(&self, username: &str) -> Result<()> {
        sqlx::query("DELETE FROM user WHERE user = ?")
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn change_password(&self, username: &str, password: &str) -> Result<()> {
        let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)?;
        sqlx::query("UPDATE user SET pass = ? WHERE user = ?")
            .bind(hash)
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn change_role(&self, username: &str, role: crate::auth::types::Role) -> Result<()> {
        let role_str: String = role.into();
        sqlx::query("UPDATE user SET role = ? WHERE user = ?")
            .bind(role_str)
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query_as::<_, UserRow>("SELECT id, user as name, pass as hash, role, tier FROM user")
            .fetch_all(&self.pool)
            .await?;
            
        let users = rows.into_iter().map(|row| User {
            id: row.id,
            name: row.name,
            hash: row.hash,
            role: row.role.into(),
            tier: row.tier.into(),
        }).collect();
        
        Ok(users)
    }
}
