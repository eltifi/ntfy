use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub listen_http: String,
    #[serde(default)]
    pub cache_file: PathBuf,
    #[serde(default)]
    pub cache_duration: u64, // Seconds
    #[serde(default)]
    pub auth_file: PathBuf,
    #[serde(default)]
    pub attachment_cache_dir: PathBuf,
    #[serde(default)]
    pub web_root: PathBuf,
    #[serde(default)]
    pub total_attachment_size_limit: usize,
    #[serde(default)]
    pub attachment_expiry_duration: u64, // Seconds
    #[serde(default)]
    pub visitor_attachment_total_size_limit: usize,
    #[serde(default)]
    pub visitor_attachment_daily_bandwidth_limit: usize,
    #[serde(default)]
    pub message_size_limit: usize,
    #[serde(default)]
    pub global_topic_limit: usize,
    #[serde(default)]
    pub visitor_subscription_limit: usize,
    #[serde(default)]
    pub visitor_request_limit_burst: usize,
    #[serde(default)]
    pub message_delay_min: u64, // Seconds
    #[serde(default)]
    pub message_delay_max: u64, // Seconds
    #[serde(default)]
    pub behind_proxy: bool,
    #[serde(default)]
    pub auth_default_access: String,
    #[serde(default)]
    pub smtp_sender_addr: String,
    #[serde(default)]
    pub smtp_sender_user: String,
    #[serde(default)]
    pub smtp_sender_pass: String,
    #[serde(default)]
    pub smtp_sender_from: String,
    #[serde(default)]
    pub smtp_server_listen: String,
    #[serde(default)]
    pub smtp_server_domain: String,
    #[serde(default)]
    pub smtp_server_addr_prefix: String,
    #[serde(default)]
    pub web_push_public_key: String,
    #[serde(default)]
    pub web_push_private_key: String,
    #[serde(default)]
    pub web_push_file: PathBuf,
    #[serde(default)]
    pub web_push_email_address: String,
    #[serde(default)]
    pub firebase_key_file: PathBuf,
    #[serde(default)]
    pub upstream_base_url: String,
    #[serde(default)]
    pub enable_login: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "http://localhost".to_string(),
            listen_http: ":80".to_string(),
            cache_file: PathBuf::from("cache.db"),
            cache_duration: 12 * 3600,
            auth_file: PathBuf::from("auth.db"),
            attachment_cache_dir: PathBuf::from("attachments"),
            web_root: PathBuf::from("client"),
            total_attachment_size_limit: 5 * 1024 * 1024 * 1024,
            attachment_expiry_duration: 3 * 3600,
            visitor_attachment_total_size_limit: 100 * 1024 * 1024,
            visitor_attachment_daily_bandwidth_limit: 500 * 1024 * 1024,
            message_size_limit: 4096,
            global_topic_limit: 15000,
            visitor_subscription_limit: 30,
            visitor_request_limit_burst: 60,
            message_delay_min: 10,
            message_delay_max: 3 * 24 * 3600,
            behind_proxy: false,
            auth_default_access: "read-write".to_string(),
            smtp_sender_addr: "".to_string(),
            smtp_sender_user: "".to_string(),
            smtp_sender_pass: "".to_string(),
            smtp_sender_from: "".to_string(),
            smtp_server_listen: "".to_string(),
            smtp_server_domain: "ntfy.sh".to_string(),
            smtp_server_addr_prefix: "".to_string(),
            web_push_public_key: "".to_string(),
            web_push_private_key: "".to_string(),
            web_push_file: PathBuf::from("webpush.db"),
            web_push_email_address: "".to_string(),
            firebase_key_file: PathBuf::from(""),
            upstream_base_url: "".to_string(),
            enable_login: false,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, config::ConfigError> {
        let builder = config::Config::builder()
            .set_default("base_url", "http://localhost")?
            .set_default("listen_http", ":80")?
            .set_default("cache_file", "cache.db")?
            .set_default("cache_duration", 43200 as i64)?
            .set_default("auth_file", "auth.db")?
            .set_default("attachment_cache_dir", "attachments")?
            .set_default("web_root", "client")?
            .set_default("total_attachment_size_limit", 5368709120 as i64)?
            .set_default("attachment_expiry_duration", 10800 as i64)?
            .set_default("visitor_attachment_total_size_limit", 104857600 as i64)?
            .set_default("visitor_attachment_daily_bandwidth_limit", 524288000 as i64)?
            .set_default("message_size_limit", 4096 as i64)?
            .set_default("global_topic_limit", 15000 as i64)?
            .set_default("visitor_subscription_limit", 30 as i64)?
            .set_default("visitor_request_limit_burst", 60 as i64)?
            .set_default("message_delay_min", 10 as i64)?
            .set_default("message_delay_max", 259200 as i64)?
            .set_default("web_push_file", "webpush.db")?
            .set_default("web_push_public_key", "")?
            .set_default("web_push_private_key", "")?
            .set_default("web_push_email_address", "")?
            .set_default("firebase_key_file", "")?
            .set_default("upstream_base_url", "")?
            .set_default("enable_login", false)?
            .set_default("auth_default_access", "read-write")?
            .set_default("behind_proxy", false)?
            .set_default("smtp_sender_addr", "")?
            .set_default("smtp_sender_user", "")?
            .set_default("smtp_sender_pass", "")?
            .set_default("smtp_sender_from", "")?
            .set_default("smtp_server_listen", "")?
            .set_default("smtp_server_domain", "ntfy.sh")?
            .set_default("smtp_server_addr_prefix", "")?
            .add_source(config::File::with_name("server.yml").required(false))
            .add_source(config::Environment::with_prefix("NTFY").separator("_"));

        builder.build()?.try_deserialize()
    }
}