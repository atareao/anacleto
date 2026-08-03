use std::path::Path;

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::{Error, Result};

use super::models::*;

/// Database manager for session persistence.
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Open or create the database at the given path.
    pub async fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(Error::Io)?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;

        let db = Self { pool };
        db.run_migrations().await?;
        Ok(db)
    }

    /// Run database migrations.
    ///
    /// The `sessions` table is created with the full current schema for fresh
    /// databases, and the newer columns (`shared`, `workspace`) are added to
    /// pre-existing databases via guarded `ALTER TABLE` statements so existing
    /// sessions are never broken.
    async fn run_migrations(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 1,
                metadata TEXT,
                shared INTEGER NOT NULL DEFAULT 0,
                workspace TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_name TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                token_count INTEGER,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_messages_session_id
            ON messages(session_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Additive migrations for databases created before these columns existed.
        self.ensure_column("sessions", "shared", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column("sessions", "workspace", "TEXT").await?;

        Ok(())
    }

    /// Add a column to a table if it does not already exist.
    ///
    /// SQLite does not support `ADD COLUMN IF NOT EXISTS` on all bundled
    /// versions, so we inspect `PRAGMA table_info` first and only issue the
    /// `ALTER TABLE` when the column is missing.
    async fn ensure_column(&self, table: &str, column: &str, definition: &str) -> Result<()> {
        let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(&self.pool)
            .await?;

        let exists = rows
            .iter()
            .any(|row| row.get::<String, _>("name") == column);

        if !exists {
            sqlx::query(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Create a new session.
    pub async fn create_session(&self, name: &str) -> Result<Session> {
        let now = Utc::now();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO sessions (id, name, created_at, updated_at, is_active)
            VALUES (?, ?, ?, ?, 1)
            "#,
        )
        .bind(id.to_string())
        .bind(name)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(Session {
            id,
            name: name.to_string(),
            created_at: now,
            updated_at: now,
            is_active: true,
            metadata: None,
            shared: false,
            workspace: None,
        })
    }

    /// Store a message in a session.
    pub async fn store_message(
        &self,
        session_id: Uuid,
        agent_name: &str,
        role: &str,
        content: &str,
        token_count: Option<u32>,
    ) -> Result<StoredMessage> {
        let now = Utc::now();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO messages (id, session_id, agent_name, role, content, created_at, token_count)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(session_id.to_string())
        .bind(agent_name)
        .bind(role)
        .bind(content)
        .bind(now.to_rfc3339())
        .bind(token_count.map(|t| t as i64))
        .execute(&self.pool)
        .await?;

        // Update session's updated_at
        sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
            .bind(now.to_rfc3339())
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(StoredMessage {
            id,
            session_id,
            agent_name: agent_name.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: now,
            token_count,
        })
    }

    /// Insert a message with an explicit timestamp and agent name (used by
    /// `/import` to preserve the original transcript faithfully).
    pub async fn store_message_full(
        &self,
        session_id: Uuid,
        agent_name: &str,
        role: &str,
        content: &str,
        created_at: DateTime<Utc>,
        token_count: Option<u32>,
    ) -> Result<StoredMessage> {
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO messages (id, session_id, agent_name, role, content, created_at, token_count)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(session_id.to_string())
        .bind(agent_name)
        .bind(role)
        .bind(content)
        .bind(created_at.to_rfc3339())
        .bind(token_count.map(|t| t as i64))
        .execute(&self.pool)
        .await?;

        Ok(StoredMessage {
            id,
            session_id,
            agent_name: agent_name.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            created_at,
            token_count,
        })
    }

    /// Get messages for a session.
    pub async fn get_session_messages(&self, session_id: Uuid) -> Result<Vec<StoredMessage>> {
        let rows = sqlx::query(
            r#"
            SELECT id, session_id, agent_name, role, content, created_at, token_count
            FROM messages
            WHERE session_id = ?
            ORDER BY created_at ASC
            "#,
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let messages = rows
            .iter()
            .map(|row| -> Result<StoredMessage> {
                Ok(StoredMessage {
                    id: Uuid::parse_str(row.get::<&str, _>("id"))
                        .map_err(|e| Error::Session(e.to_string()))?,
                    session_id: Uuid::parse_str(row.get::<&str, _>("session_id"))
                        .map_err(|e| Error::Session(e.to_string()))?,
                    agent_name: row.get("agent_name"),
                    role: row.get("role"),
                    content: row.get("content"),
                    created_at: chrono::DateTime::parse_from_rfc3339(
                        row.get::<&str, _>("created_at"),
                    )
                    .map_err(|e| Error::Session(e.to_string()))?
                    .with_timezone(&chrono::Utc),
                    token_count: row.get::<Option<i64>, _>("token_count").map(|t| t as u32),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(messages)
    }

    /// List all sessions.
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let rows = sqlx::query(
            r#"
            SELECT s.id, s.name, s.created_at, s.updated_at, s.is_active,
                   COUNT(m.id) as message_count
            FROM sessions s
            LEFT JOIN messages m ON m.session_id = s.id
            GROUP BY s.id
            ORDER BY s.updated_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let summaries = rows
            .iter()
            .map(|row| -> Result<SessionSummary> {
                Ok(SessionSummary {
                    id: Uuid::parse_str(row.get::<&str, _>("id"))
                        .map_err(|e| Error::Session(e.to_string()))?,
                    name: row.get("name"),
                    created_at: chrono::DateTime::parse_from_rfc3339(
                        row.get::<&str, _>("created_at"),
                    )
                    .map_err(|e| Error::Session(e.to_string()))?
                    .with_timezone(&chrono::Utc),
                    updated_at: chrono::DateTime::parse_from_rfc3339(
                        row.get::<&str, _>("updated_at"),
                    )
                    .map_err(|e| Error::Session(e.to_string()))?
                    .with_timezone(&chrono::Utc),
                    message_count: row.get("message_count"),
                    is_active: row.get::<i32, _>("is_active") != 0,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(summaries)
    }

    /// Delete a session and all its messages.
    pub async fn delete_session(&self, session_id: Uuid) -> Result<()> {
        // Delete messages first (foreign key)
        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Rename a session.
    pub async fn rename_session(&self, session_id: Uuid, new_name: &str) -> Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE sessions SET name = ?, updated_at = ? WHERE id = ?")
            .bind(new_name)
            .bind(now.to_rfc3339())
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete the last `limit` messages of a session and return the deleted
    /// messages so they can be restored later (used by `/undo`).
    pub async fn delete_messages(
        &self,
        session_id: Uuid,
        limit: usize,
    ) -> Result<Vec<StoredMessage>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        // Select the last `limit` messages (most recent first).
        let rows = sqlx::query(
            r#"
            SELECT id, session_id, agent_name, role, content, created_at, token_count
            FROM messages
            WHERE session_id = ?
            ORDER BY created_at DESC, id DESC
            LIMIT ?
            "#,
        )
        .bind(session_id.to_string())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let messages = rows
            .iter()
            .map(|row| -> Result<StoredMessage> {
                Ok(StoredMessage {
                    id: Uuid::parse_str(row.get::<&str, _>("id"))
                        .map_err(|e| Error::Session(e.to_string()))?,
                    session_id: Uuid::parse_str(row.get::<&str, _>("session_id"))
                        .map_err(|e| Error::Session(e.to_string()))?,
                    agent_name: row.get("agent_name"),
                    role: row.get("role"),
                    content: row.get("content"),
                    created_at: chrono::DateTime::parse_from_rfc3339(
                        row.get::<&str, _>("created_at"),
                    )
                    .map_err(|e| Error::Session(e.to_string()))?
                    .with_timezone(&chrono::Utc),
                    token_count: row.get::<Option<i64>, _>("token_count").map(|t| t as u32),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        // Delete the selected messages by id (single statement, no N+1).
        if !messages.is_empty() {
            let ids: Vec<String> = messages.iter().map(|m| m.id.to_string()).collect();
            let placeholders = vec!["?"; ids.len()].join(",");
            let sql = format!("DELETE FROM messages WHERE id IN ({})", placeholders);
            let mut q = sqlx::query(&sql);
            for id in &ids {
                q = q.bind(id);
            }
            q.execute(&self.pool).await?;
        }

        Ok(messages)
    }

    /// Re-insert previously deleted messages (with their original ids) into a
    /// session (used by `/redo`).
    pub async fn restore_messages(
        &self,
        session_id: Uuid,
        messages: &[StoredMessage],
    ) -> Result<()> {
        for msg in messages {
            sqlx::query(
                r#"
                INSERT INTO messages (id, session_id, agent_name, role, content, created_at, token_count)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(msg.id.to_string())
            .bind(session_id.to_string())
            .bind(&msg.agent_name)
            .bind(&msg.role)
            .bind(&msg.content)
            .bind(msg.created_at.to_rfc3339())
            .bind(msg.token_count.map(|t| t as i64))
            .execute(&self.pool)
            .await?;
        }

        if !messages.is_empty() {
            let now = Utc::now();
            sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
                .bind(now.to_rfc3339())
                .bind(session_id.to_string())
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Copy all messages from one session to another (used by `/fork`).
    ///
    /// Messages are copied with fresh ids so the primary key stays unique.
    pub async fn copy_messages(&self, from_session: Uuid, to_session: Uuid) -> Result<()> {
        let messages = self.get_session_messages(from_session).await?;

        for msg in &messages {
            sqlx::query(
                r#"
                INSERT INTO messages (id, session_id, agent_name, role, content, created_at, token_count)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(Uuid::new_v4().to_string())
            .bind(to_session.to_string())
            .bind(&msg.agent_name)
            .bind(&msg.role)
            .bind(&msg.content)
            .bind(msg.created_at.to_rfc3339())
            .bind(msg.token_count.map(|t| t as i64))
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Mark a session as shared (or not) and persist an optional share link in
    /// its metadata (used by `/share` and `/unshare`).
    pub async fn set_shared(
        &self,
        session_id: Uuid,
        shared: bool,
        link: Option<&str>,
    ) -> Result<()> {
        // Merge the share link into the existing metadata JSON.
        let existing = self.get_session_metadata(session_id).await?;
        let mut metadata = existing.unwrap_or_else(|| serde_json::json!({}));
        if let Some(obj) = metadata.as_object_mut() {
            match link {
                Some(l) => {
                    obj.insert(
                        "share_link".to_string(),
                        serde_json::Value::String(l.to_string()),
                    );
                }
                None => {
                    obj.remove("share_link");
                }
            }
        }

        let metadata_str = serde_json::to_string(&metadata)?;
        sqlx::query("UPDATE sessions SET shared = ?, metadata = ? WHERE id = ?")
            .bind(if shared { 1 } else { 0 })
            .bind(metadata_str)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get the metadata JSON of a session, if any.
    pub async fn get_session_metadata(
        &self,
        session_id: Uuid,
    ) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query("SELECT metadata FROM sessions WHERE id = ?")
            .bind(session_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let raw: Option<String> = row.get("metadata");
                match raw {
                    Some(s) if !s.is_empty() => Ok(Some(serde_json::from_str(&s)?)),
                    _ => Ok(None),
                }
            }
            None => Ok(None),
        }
    }

    /// Get the name of a session, if it exists.
    pub async fn get_session_name(&self, session_id: Uuid) -> Result<Option<String>> {
        let row = sqlx::query("SELECT name FROM sessions WHERE id = ?")
            .bind(session_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|r| r.get("name")))
    }

    /// Export a session transcript to a file in JSON or Markdown format.
    pub async fn export_session(
        &self,
        session_id: Uuid,
        path: &Path,
        format: ExportFormat,
    ) -> Result<()> {
        let messages = self.get_session_messages(session_id).await?;
        let title = self
            .get_session_name(session_id)
            .await?
            .unwrap_or_else(|| "session".to_string());

        let data = ExportData {
            session_id,
            title: title.clone(),
            created_at: Utc::now(),
            messages: messages
                .iter()
                .map(|m| ExportMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                    timestamp: m.created_at,
                })
                .collect(),
        };

        let contents = match format {
            ExportFormat::Json => serde_json::to_string_pretty(&data)?,
            ExportFormat::Markdown => render_markdown(&data),
        };

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(Error::Io)?;
        }
        tokio::fs::write(path, contents).await.map_err(Error::Io)?;

        Ok(())
    }

    /// Import a session transcript from a JSON file, creating a new session
    /// and re-inserting its messages. Returns the new session id.
    pub async fn import_session(&self, path: &Path) -> Result<Uuid> {
        let contents = tokio::fs::read_to_string(path).await.map_err(Error::Io)?;
        let data: ExportData = serde_json::from_str(&contents)?;

        let session = self.create_session(&data.title).await?;
        let new_id = session.id;

        for msg in &data.messages {
            self.store_message_full(
                new_id,
                &msg.role,
                &msg.role,
                &msg.content,
                msg.timestamp,
                None,
            )
            .await?;
        }

        Ok(new_id)
    }

    /// Set the workspace a session belongs to (used by `/move`).
    pub async fn set_session_workspace(&self, session_id: Uuid, workspace: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET workspace = ? WHERE id = ?")
            .bind(workspace)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Close the database connection.
    pub async fn close(self) -> Result<()> {
        self.pool.close().await;
        Ok(())
    }
}

/// Render an [`ExportData`] transcript as human-readable Markdown.
fn render_markdown(data: &ExportData) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", data.title));
    out.push_str(&format!("- Session: `{}`\n", data.session_id));
    out.push_str(&format!("- Created: {}\n\n", data.created_at.to_rfc3339()));

    for msg in &data.messages {
        let role = match msg.role.as_str() {
            "user" => "**User**",
            "assistant" => "**Assistant**",
            "system" => "**System**",
            "tool" => "**Tool**",
            other => other,
        };
        out.push_str(&format!("### {}\n\n", role));
        out.push_str(&format!("{}\n\n", msg.content));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_session() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("test-session").await.unwrap();
        assert_eq!(session.name, "test-session");
        assert!(session.is_active);
    }

    #[tokio::test]
    async fn test_store_and_retrieve_messages() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("test-session").await.unwrap();

        db.store_message(session.id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(session.id, "assistant", "assistant", "Hi there!", None)
            .await
            .unwrap();

        let messages = db.get_session_messages(session.id).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].content, "Hi there!");
    }

    #[tokio::test]
    async fn test_delete_and_restore_messages() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("test-session").await.unwrap();

        db.store_message(session.id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(session.id, "assistant", "assistant", "Hi there!", None)
            .await
            .unwrap();
        db.store_message(session.id, "user", "user", "Third", None)
            .await
            .unwrap();

        // Delete the last 2 messages.
        let deleted = db.delete_messages(session.id, 2).await.unwrap();
        assert_eq!(deleted.len(), 2);
        assert_eq!(deleted[0].content, "Third");
        assert_eq!(deleted[1].content, "Hi there!");

        // Only the first message remains.
        let remaining = db.get_session_messages(session.id).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].content, "Hello");

        // Restore the deleted messages.
        db.restore_messages(session.id, &deleted).await.unwrap();
        let restored = db.get_session_messages(session.id).await.unwrap();
        assert_eq!(restored.len(), 3);
        assert_eq!(restored[0].content, "Hello");
        assert_eq!(restored[1].content, "Hi there!");
        assert_eq!(restored[2].content, "Third");
    }

    #[tokio::test]
    async fn test_delete_messages_zero_limit() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("test-session").await.unwrap();
        db.store_message(session.id, "user", "user", "Hello", None)
            .await
            .unwrap();

        let deleted = db.delete_messages(session.id, 0).await.unwrap();
        assert!(deleted.is_empty());
        assert_eq!(db.get_session_messages(session.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_copy_messages() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let from = db.create_session("from").await.unwrap();
        let to = db.create_session("to").await.unwrap();

        db.store_message(from.id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(from.id, "assistant", "assistant", "Hi there!", None)
            .await
            .unwrap();

        db.copy_messages(from.id, to.id).await.unwrap();

        let copied = db.get_session_messages(to.id).await.unwrap();
        assert_eq!(copied.len(), 2);
        assert_eq!(copied[0].content, "Hello");
        assert_eq!(copied[1].content, "Hi there!");
        // The original session is unchanged.
        assert_eq!(db.get_session_messages(from.id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_set_shared_and_metadata() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("test-session").await.unwrap();

        // Initially not shared, no metadata.
        let meta = db.get_session_metadata(session.id).await.unwrap();
        assert!(meta.is_none());

        // Share with a link.
        db.set_shared(session.id, true, Some("https://share/anacleto/abc"))
            .await
            .unwrap();
        let meta = db.get_session_metadata(session.id).await.unwrap().unwrap();
        assert_eq!(
            meta.get("share_link").and_then(|v| v.as_str()),
            Some("https://share/anacleto/abc")
        );

        // Unshare clears the link.
        db.set_shared(session.id, false, None).await.unwrap();
        let meta = db.get_session_metadata(session.id).await.unwrap().unwrap();
        assert!(meta.get("share_link").is_none());
    }

    #[tokio::test]
    async fn test_export_import_round_trip() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("round-trip").await.unwrap();
        db.store_message(session.id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(session.id, "assistant", "assistant", "Hi there!", None)
            .await
            .unwrap();

        let export_path = dir.path().join("export.json");
        db.export_session(session.id, &export_path, ExportFormat::Json)
            .await
            .unwrap();

        // Import into a brand new session.
        let imported_id = db.import_session(&export_path).await.unwrap();
        assert_ne!(imported_id, session.id);

        let imported = db.get_session_messages(imported_id).await.unwrap();
        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].content, "Hello");
        assert_eq!(imported[1].content, "Hi there!");
        assert_eq!(imported[0].role, "user");
        assert_eq!(imported[1].role, "assistant");
    }

    #[tokio::test]
    async fn test_export_markdown() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("md-export").await.unwrap();
        db.store_message(session.id, "user", "user", "Hello", None)
            .await
            .unwrap();

        let export_path = dir.path().join("export.md");
        db.export_session(session.id, &export_path, ExportFormat::Markdown)
            .await
            .unwrap();

        let contents = std::fs::read_to_string(&export_path).unwrap();
        assert!(contents.contains("# md-export"));
        assert!(contents.contains("**User**"));
        assert!(contents.contains("Hello"));
    }

    #[tokio::test]
    async fn test_set_session_workspace() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("test-session").await.unwrap();
        db.set_session_workspace(session.id, "/some/workspace")
            .await
            .unwrap();

        let row = sqlx::query("SELECT workspace FROM sessions WHERE id = ?")
            .bind(session.id.to_string())
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let workspace: Option<String> = row.get("workspace");
        assert_eq!(workspace.as_deref(), Some("/some/workspace"));
    }

    #[tokio::test]
    async fn test_migration_adds_new_columns_to_legacy_schema() {
        // Simulate a database created before the `shared`/`workspace` columns
        // existed, then verify `run_migrations` adds them via `ensure_column`.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("legacy.db");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();

        // Legacy schema without the new columns.
        sqlx::query(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 1,
                metadata TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_name TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                token_count INTEGER,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        // Run migrations on the legacy schema.
        let db = Database { pool };
        db.run_migrations().await.unwrap();

        // The new columns must now exist.
        let cols = sqlx::query("PRAGMA table_info(sessions)")
            .fetch_all(&db.pool)
            .await
            .unwrap();
        let names: Vec<String> = cols.iter().map(|r| r.get::<String, _>("name")).collect();
        assert!(names.contains(&"shared".to_string()));
        assert!(names.contains(&"workspace".to_string()));

        // And a second run must be idempotent (no error, no duplicate columns).
        db.run_migrations().await.unwrap();
        let cols2 = sqlx::query("PRAGMA table_info(sessions)")
            .fetch_all(&db.pool)
            .await
            .unwrap();
        let names2: Vec<String> = cols2.iter().map(|r| r.get::<String, _>("name")).collect();
        assert_eq!(names2.iter().filter(|n| *n == "shared").count(), 1);
        assert_eq!(names2.iter().filter(|n| *n == "workspace").count(), 1);
    }
}
