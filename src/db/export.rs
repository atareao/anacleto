use std::path::Path;

use chrono::Utc;
use uuid::Uuid;

use crate::error::{Error, Result};

use super::Database;
use super::models::*;

impl Database {
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
}
