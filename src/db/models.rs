use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A persisted session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID.
    pub id: Uuid,
    /// Human-readable session name.
    pub name: String,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session was last active.
    pub updated_at: DateTime<Utc>,
    /// Whether the session is active.
    pub is_active: bool,
    /// Session metadata.
    pub metadata: Option<serde_json::Value>,
}

/// A message stored in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    /// Unique message ID.
    pub id: Uuid,
    /// Session this message belongs to.
    pub session_id: Uuid,
    /// Agent that produced this message.
    pub agent_name: String,
    /// Message role.
    pub role: String,
    /// Message content.
    pub content: String,
    /// When the message was created.
    pub created_at: DateTime<Utc>,
    /// Token count (if available).
    pub token_count: Option<u32>,
}

/// Summary of a session for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: i64,
    pub is_active: bool,
}
