use chrono::Utc;
use sqlx::Row;

use crate::error::Result;

use super::Database;

impl Database {
    /// Record that a model was used, incrementing its usage count and updating
    /// its last-used timestamp. If the model is new, it is inserted.
    pub async fn record_model_usage(&self, model: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO model_usage (model, count, last_used)
            VALUES (?, 1, ?)
            ON CONFLICT(model) DO UPDATE SET
                count = count + 1,
                last_used = excluded.last_used
            "#,
        )
        .bind(model)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List models ordered by usage frequency (count desc), then by recency
    /// (last_used desc). Returns `(model, count)` pairs.
    pub async fn list_model_frecency(&self) -> Result<Vec<(String, usize)>> {
        let rows =
            sqlx::query("SELECT model, count FROM model_usage ORDER BY count DESC, last_used DESC")
                .fetch_all(&self.pool)
                .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push((row.get("model"), row.get::<i64, _>("count") as usize));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_model_frecency() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        // Initially empty.
        assert!(db.list_model_frecency().await.unwrap().is_empty());

        // Record usage.
        db.record_model_usage("claude-sonnet-4").await.unwrap();
        db.record_model_usage("claude-sonnet-4").await.unwrap();
        db.record_model_usage("gpt-4o").await.unwrap();

        let frecency = db.list_model_frecency().await.unwrap();
        // Most-used first.
        assert_eq!(frecency[0].0, "claude-sonnet-4");
        assert_eq!(frecency[0].1, 2);
        assert_eq!(frecency[1].0, "gpt-4o");
        assert_eq!(frecency[1].1, 1);
    }
}
