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
    /// Whether the session is marked as shared.
    pub shared: bool,
    /// Workspace this session belongs to (if any).
    pub workspace: Option<String>,
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

/// A single message in an exported session transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportMessage {
    /// Message role ("user", "assistant", "system", "tool").
    pub role: String,
    /// Message content.
    pub content: String,
    /// When the message was created.
    pub timestamp: DateTime<Utc>,
}

/// A serializable session transcript used for JSON export/import.
///
/// This is the recommended format for lossless round-trips: it preserves the
/// session id, title, creation time and the full ordered message list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportData {
    /// Session ID.
    pub session_id: Uuid,
    /// Session title.
    pub title: String,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// Ordered messages in the session.
    pub messages: Vec<ExportMessage>,
}

/// Output format for a session export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// JSON transcript (lossless, recommended for round-trips).
    Json,
    /// Human-readable Markdown transcript.
    Markdown,
}

/// A task tracked by the `todo` tool, persisted per session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    /// Unique todo ID.
    pub id: Uuid,
    /// Session this todo belongs to.
    pub session_id: Uuid,
    /// Task content.
    pub content: String,
    /// Task status: "pending" | "in_progress" | "completed" | "cancelled".
    pub status: String,
    /// Optional priority ("low" | "medium" | "high").
    pub priority: Option<String>,
    /// When the todo was created.
    pub created_at: DateTime<Utc>,
    /// When the todo was last updated.
    pub updated_at: DateTime<Utc>,
}
