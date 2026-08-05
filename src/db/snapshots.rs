use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{Error, Result};

use super::Database;
use super::models::*;

impl Database {
    /// Create a snapshot of a session's current conversation state.
    ///
    /// The `content` field stores the serialized JSON array of the session's
    /// messages so the state can be restored later via [`Database::get_snapshot`].
    pub async fn create_snapshot(
        &self,
        session_id: Uuid,
        name: &str,
        content: &str,
    ) -> Result<Snapshot> {
        let messages = self.get_session_messages(session_id).await?;
        let now = Utc::now();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO snapshots (id, session_id, name, created_at, message_count, content)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(session_id.to_string())
        .bind(name)
        .bind(now.to_rfc3339())
        .bind(messages.len() as i64)
        .bind(content)
        .execute(&self.pool)
        .await?;

        Ok(Snapshot {
            id,
            session_id,
            name: name.to_string(),
            created_at: now,
            message_count: messages.len() as i64,
            content: content.to_string(),
        })
    }

    /// List all snapshots for a session, newest first.
    pub async fn list_snapshots(&self, session_id: Uuid) -> Result<Vec<Snapshot>> {
        let rows = sqlx::query(
            r#"
            SELECT id, session_id, name, created_at, message_count, content
            FROM snapshots
            WHERE session_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| -> Result<Snapshot> {
                Ok(Snapshot {
                    id: Uuid::parse_str(row.get::<&str, _>("id"))
                        .map_err(|e| Error::Session(e.to_string()))?,
                    session_id: Uuid::parse_str(row.get::<&str, _>("session_id"))
                        .map_err(|e| Error::Session(e.to_string()))?,
                    name: row.get("name"),
                    created_at: chrono::DateTime::parse_from_rfc3339(
                        row.get::<&str, _>("created_at"),
                    )
                    .map_err(|e| Error::Session(e.to_string()))?
                    .with_timezone(&chrono::Utc),
                    message_count: row.get("message_count"),
                    content: row.get("content"),
                })
            })
            .collect()
    }

    /// Get a single snapshot by id.
    pub async fn get_snapshot(&self, snapshot_id: Uuid) -> Result<Option<Snapshot>> {
        let rows = sqlx::query(
            r#"
            SELECT id, session_id, name, created_at, message_count, content
            FROM snapshots
            WHERE id = ?
            "#,
        )
        .bind(snapshot_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let Some(row) = rows.first() else {
            return Ok(None);
        };

        Ok(Some(Snapshot {
            id: Uuid::parse_str(row.get::<&str, _>("id"))
                .map_err(|e| Error::Session(e.to_string()))?,
            session_id: Uuid::parse_str(row.get::<&str, _>("session_id"))
                .map_err(|e| Error::Session(e.to_string()))?,
            name: row.get("name"),
            created_at: chrono::DateTime::parse_from_rfc3339(row.get::<&str, _>("created_at"))
                .map_err(|e| Error::Session(e.to_string()))?
                .with_timezone(&chrono::Utc),
            message_count: row.get("message_count"),
            content: row.get("content"),
        }))
    }

    /// Delete a snapshot by id.
    pub async fn delete_snapshot(&self, snapshot_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM snapshots WHERE id = ?")
            .bind(snapshot_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_snapshot_crud() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let session = db.create_session("snap-session").await.unwrap();
        db.store_message(session.id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(session.id, "assistant", "assistant", "Hi", None)
            .await
            .unwrap();

        let snap = db
            .create_snapshot(session.id, "checkpoint", r#"[{"role":"user"}]"#)
            .await
            .unwrap();
        assert_eq!(snap.name, "checkpoint");
        assert_eq!(snap.message_count, 2);
        assert_eq!(snap.session_id, session.id);

        // list returns the snapshot.
        let listed = db.list_snapshots(session.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, snap.id);

        // get by id.
        let fetched = db.get_snapshot(snap.id).await.unwrap().expect("snapshot");
        assert_eq!(fetched.content, r#"[{"role":"user"}]"#);

        // delete removes it.
        db.delete_snapshot(snap.id).await.unwrap();
        assert!(db.get_snapshot(snap.id).await.unwrap().is_none());
        assert!(db.list_snapshots(session.id).await.unwrap().is_empty());
    }
}
