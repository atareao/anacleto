use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{Error, Result};

use super::Database;
use super::models::*;

impl Database {
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
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
}
