use std::path::Path;

use chrono::Utc;
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
    async fn run_migrations(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 1,
                metadata TEXT
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

    /// Close the database connection.
    pub async fn close(self) -> Result<()> {
        self.pool.close().await;
        Ok(())
    }
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
}
