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
    pub(crate) pool: SqlitePool,
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
                workspace TEXT,
                pinned INTEGER NOT NULL DEFAULT 0
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

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS todos (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_todos_session_id
            ON todos(session_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS model_usage (
                model TEXT PRIMARY KEY,
                count INTEGER NOT NULL DEFAULT 1,
                last_used TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS snapshots (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                message_count INTEGER NOT NULL DEFAULT 0,
                content TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_snapshots_session_id
            ON snapshots(session_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Additive migrations for databases created before these columns existed.
        self.ensure_column("sessions", "shared", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column("sessions", "workspace", "TEXT").await?;
        self.ensure_column("sessions", "pinned", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column("sessions", "parent_id", "TEXT").await?;

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
        self.create_session_with_parent(name, None).await
    }

    /// Create a new session, optionally linked to a parent session (for forks).
    pub async fn create_session_with_parent(
        &self,
        name: &str,
        parent_id: Option<Uuid>,
    ) -> Result<Session> {
        let now = Utc::now();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO sessions (id, name, created_at, updated_at, is_active, parent_id)
            VALUES (?, ?, ?, ?, 1, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(name)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(parent_id.map(|p| p.to_string()))
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
            parent_id,
        })
    }

    /// List all sessions.
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let rows = sqlx::query(
            r#"
            SELECT s.id, s.name, s.created_at, s.updated_at, s.is_active, s.pinned, s.parent_id,
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
                    pinned: row.get::<i32, _>("pinned") != 0,
                    parent_id: row
                        .get::<Option<String>, _>("parent_id")
                        .and_then(|s| Uuid::parse_str(&s).ok()),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(summaries)
    }

    /// Set the pinned flag on a session.
    pub async fn set_session_pinned(&self, session_id: &str, pinned: bool) -> Result<()> {
        sqlx::query("UPDATE sessions SET pinned = ? WHERE id = ?")
            .bind(if pinned { 1 } else { 0 })
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List pinned sessions, ordered by most recently updated first.
    pub async fn list_pinned_sessions(&self) -> Result<Vec<SessionSummary>> {
        let rows = sqlx::query(
            r#"
            SELECT s.id, s.name, s.created_at, s.updated_at, s.is_active, s.pinned, s.parent_id,
                   COUNT(m.id) as message_count
            FROM sessions s
            LEFT JOIN messages m ON m.session_id = s.id
            WHERE s.pinned = 1
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
                    pinned: true,
                    parent_id: row
                        .get::<Option<String>, _>("parent_id")
                        .and_then(|s| Uuid::parse_str(&s).ok()),
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

    /// Set the workspace a session belongs to (used by `/move`).
    pub async fn set_session_workspace(&self, session_id: Uuid, workspace: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET workspace = ? WHERE id = ?")
            .bind(workspace)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set (or clear) the parent session of a session (used by `/fork`).
    pub async fn set_parent(&self, session_id: Uuid, parent_id: Option<Uuid>) -> Result<()> {
        sqlx::query("UPDATE sessions SET parent_id = ? WHERE id = ?")
            .bind(parent_id.map(|p| p.to_string()))
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Get the parent session id of a session, if any.
    pub async fn get_parent(&self, session_id: Uuid) -> Result<Option<Uuid>> {
        let row = sqlx::query("SELECT parent_id FROM sessions WHERE id = ?")
            .bind(session_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .and_then(|r| r.get::<Option<String>, _>("parent_id"))
            .and_then(|s| Uuid::parse_str(&s).ok()))
    }

    /// List the child sessions (direct forks) of a session.
    pub async fn get_children(&self, session_id: Uuid) -> Result<Vec<SessionSummary>> {
        let rows = sqlx::query(
            r#"
            SELECT s.id, s.name, s.created_at, s.updated_at, s.is_active, s.pinned, s.parent_id,
                   COUNT(m.id) as message_count
            FROM sessions s
            LEFT JOIN messages m ON m.session_id = s.id
            WHERE s.parent_id = ?
            GROUP BY s.id
            ORDER BY s.updated_at DESC
            "#,
        )
        .bind(session_id.to_string())
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
                    pinned: row.get::<i32, _>("pinned") != 0,
                    parent_id: row
                        .get::<Option<String>, _>("parent_id")
                        .and_then(|s| Uuid::parse_str(&s).ok()),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(summaries)
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

    #[tokio::test]
    async fn test_session_pinning() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();
        let s1 = db.create_session("one").await.unwrap();
        let s2 = db.create_session("two").await.unwrap();

        // Initially nothing pinned.
        assert!(db.list_pinned_sessions().await.unwrap().is_empty());

        // Pin s1.
        db.set_session_pinned(&s1.id.to_string(), true)
            .await
            .unwrap();
        let pinned = db.list_pinned_sessions().await.unwrap();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].id, s1.id);
        assert!(pinned[0].pinned);

        // Pin s2 too; both listed.
        db.set_session_pinned(&s2.id.to_string(), true)
            .await
            .unwrap();
        assert_eq!(db.list_pinned_sessions().await.unwrap().len(), 2);

        // Unpin s1.
        db.set_session_pinned(&s1.id.to_string(), false)
            .await
            .unwrap();
        let pinned = db.list_pinned_sessions().await.unwrap();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].id, s2.id);

        // list_sessions reflects the pinned flag.
        let all = db.list_sessions().await.unwrap();
        let s1_summary = all.iter().find(|s| s.id == s1.id).unwrap();
        assert!(!s1_summary.pinned);
        let s2_summary = all.iter().find(|s| s.id == s2.id).unwrap();
        assert!(s2_summary.pinned);
    }

    #[tokio::test]
    async fn test_session_parent_children() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        let parent = db.create_session("parent").await.unwrap();
        assert_eq!(parent.parent_id, None);

        // Create a child linked to the parent at creation time.
        let child = db
            .create_session_with_parent("child", Some(parent.id))
            .await
            .unwrap();
        assert_eq!(child.parent_id, Some(parent.id));

        // get_parent resolves the link.
        assert_eq!(db.get_parent(child.id).await.unwrap(), Some(parent.id));
        assert_eq!(db.get_parent(parent.id).await.unwrap(), None);

        // get_children lists direct forks.
        let children = db.get_children(parent.id).await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, child.id);
        assert_eq!(children[0].parent_id, Some(parent.id));

        // set_parent can re-link or clear.
        let other = db.create_session("other").await.unwrap();
        db.set_parent(child.id, Some(other.id)).await.unwrap();
        assert_eq!(db.get_parent(child.id).await.unwrap(), Some(other.id));
        assert!(db.get_children(parent.id).await.unwrap().is_empty());
        assert_eq!(db.get_children(other.id).await.unwrap().len(), 1);

        db.set_parent(child.id, None).await.unwrap();
        assert_eq!(db.get_parent(child.id).await.unwrap(), None);
    }
}
