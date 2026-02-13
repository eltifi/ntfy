use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Role {
    #[serde(rename = "admin")]
    Admin,
    #[serde(rename = "user")]
    User,
    #[serde(rename = "anonymous")]
    Anonymous,
}

impl From<Role> for String {
    fn from(r: Role) -> Self {
        match r {
            Role::Admin => "admin".to_string(),
            Role::User => "user".to_string(),
            Role::Anonymous => "anonymous".to_string(),
        }
    }
}

impl From<String> for Role {
    fn from(s: String) -> Self {
        match s.as_str() {
            "admin" => Role::Admin,
            "user" => Role::User,
            _ => Role::Anonymous,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Copy)]
pub enum Tier {
    #[serde(rename = "free")]
    Free,
    #[serde(rename = "pro")]
    Pro,
    #[serde(rename = "business")]
    Business,
}

impl Tier {
    pub fn info(&self) -> TierInfo {
        match self {
            Tier::Free => TierInfo {
                message_limit: 256,
                message_expiry_duration: 12 * 3600,
                email_limit: 16,
                reservation_limit: 1,
                attachment_file_size_limit: 15 * 1024 * 1024,
                attachment_total_size_limit: 100 * 1024 * 1024,
                attachment_expiry_duration: 3 * 3600,
                attachment_bandwidth_limit: 500 * 1024 * 1024,
            },
            Tier::Pro => TierInfo {
                message_limit: 5000,
                message_expiry_duration: 24 * 3600,
                email_limit: 500,
                reservation_limit: 10,
                attachment_file_size_limit: 500 * 1024 * 1024,
                attachment_total_size_limit: 5 * 1024 * 1024 * 1024,
                attachment_expiry_duration: 24 * 3600,
                attachment_bandwidth_limit: 10 * 1024 * 1024 * 1024,
            },
            Tier::Business => TierInfo {
                message_limit: 100000,
                message_expiry_duration: 30 * 24 * 3600,
                email_limit: 10000,
                reservation_limit: 100,
                attachment_file_size_limit: 5 * 1024 * 1024 * 1024,
                attachment_total_size_limit: 100 * 1024 * 1024 * 1024,
                attachment_expiry_duration: 30 * 24 * 3600,
                attachment_bandwidth_limit: 100 * 1024 * 1024 * 1024,
            },
        }
    }
}

pub struct TierInfo {
    pub message_limit: i64,
    pub message_expiry_duration: i64,
    pub email_limit: i64,
    pub reservation_limit: i64,
    pub attachment_file_size_limit: i64,
    pub attachment_total_size_limit: i64,
    pub attachment_expiry_duration: i64,
    pub attachment_bandwidth_limit: i64,
}

impl Default for Tier {
    fn default() -> Self {
        Tier::Free
    }
}

impl From<String> for Tier {
    fn from(s: String) -> Self {
        match s.as_str() {
            "pro" => Tier::Pro,
            "business" => Tier::Business,
            _ => Tier::Free,
        }
    }
}

impl From<Tier> for String {
    fn from(t: Tier) -> Self {
        match t {
            Tier::Free => "free".to_string(),
            Tier::Pro => "pro".to_string(),
            Tier::Business => "business".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    #[allow(dead_code)]
    pub hash: String,
    pub role: Role,
    #[serde(default)]
    pub tier: Tier,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Permission {
    DenyAll = 0,
    Read = 1,
    Write = 2,
    ReadWrite = 3,
}

impl Permission {
    pub fn new(read: bool, write: bool) -> Self {
        match (read, write) {
            (true, true) => Permission::ReadWrite,
            (true, false) => Permission::Read,
            (false, true) => Permission::Write,
            (false, false) => Permission::DenyAll,
        }
    }

    pub fn is_read(&self) -> bool {
        matches!(self, Permission::Read | Permission::ReadWrite)
    }

    pub fn is_write(&self) -> bool {
        matches!(self, Permission::Write | Permission::ReadWrite)
    }
}
