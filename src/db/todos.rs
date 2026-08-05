use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::error::Result;

use super::Database;
use super::models::*;

impl Database {
    /// Add a new todo to a session.
    pub async fn add_todo(
        &self,
        session_id: Uuid,
        content: &str,
        status: &str,
        priority: Option<&str>,
    ) -> Result<Todo> {
        let now = Utc::now();
        let todo = Todo {
            id: Uuid::new_v4(),
            session_id,
            content: content.to_string(),
            status: status.to_string(),
            priority: priority.map(|p| p.to_string()),
            created_at: now,
            updated_at: now,
        };
        sqlx::query(
            r#"
            INSERT INTO todos (id, session_id, content, status, priority, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(todo.id.to_string())
        .bind(todo.session_id.to_string())
        .bind(&todo.content)
        .bind(&todo.status)
        .bind(&todo.priority)
        .bind(todo.created_at.to_rfc3339())
        .bind(todo.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(todo)
    }

    /// Update a todo's status and/or content.
    pub async fn update_todo(
        &self,
        todo_id: Uuid,
        content: Option<&str>,
        status: Option<&str>,
        priority: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            r#"
            UPDATE todos
            SET content = COALESCE(?, content),
                status = COALESCE(?, status),
                priority = COALESCE(?, priority),
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(content)
        .bind(status)
        .bind(priority)
        .bind(now.to_rfc3339())
        .bind(todo_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete a todo by id.
    pub async fn delete_todo(&self, todo_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM todos WHERE id = ?")
            .bind(todo_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List all todos for a session, ordered by creation time.
    pub async fn list_todos(&self, session_id: Uuid) -> Result<Vec<Todo>> {
        let rows = sqlx::query(
            "SELECT id, session_id, content, status, priority, created_at, updated_at \
             FROM todos WHERE session_id = ? ORDER BY created_at ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut todos = Vec::with_capacity(rows.len());
        for row in rows {
            todos.push(Todo {
                id: Uuid::parse_str(&row.get::<String, _>("id")).unwrap_or_default(),
                session_id: Uuid::parse_str(&row.get::<String, _>("session_id"))
                    .unwrap_or_default(),
                content: row.get("content"),
                status: row.get("status"),
                priority: row.get("priority"),
                created_at: DateTime::parse_from_rfc3339(&row.get::<String, _>("created_at"))
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<String, _>("updated_at"))
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            });
        }
        Ok(todos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_todo_crud() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();
        let session = db.create_session("todo-session").await.unwrap();

        // Add
        let t1 = db
            .add_todo(session.id, "Write tests", "pending", Some("high"))
            .await
            .unwrap();
        let t2 = db
            .add_todo(session.id, "Fix bug", "pending", None)
            .await
            .unwrap();
        assert_eq!(t1.status, "pending");
        assert_eq!(t1.priority.as_deref(), Some("high"));

        // List
        let todos = db.list_todos(session.id).await.unwrap();
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].content, "Write tests");

        // Update
        db.update_todo(t1.id, None, Some("in_progress"), None)
            .await
            .unwrap();
        let todos = db.list_todos(session.id).await.unwrap();
        assert_eq!(todos[0].status, "in_progress");

        // Delete
        db.delete_todo(t2.id).await.unwrap();
        let todos = db.list_todos(session.id).await.unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].id, t1.id);
    }
}
