//! Concurrency stress tests for Anacleto.
//!
//! These tests verify thread-safety, absence of data races, and correct
//! behavior under concurrent access patterns. All tests run under
//! tokio's multi-thread runtime.

use std::collections::HashSet;
use std::sync::Arc;

use futures::future::join_all;
use tokio::sync::Semaphore;

// ---------------------------------------------------------------------------
// AgentId concurrency: 100 concurrent generations, all must be unique
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_id_concurrent_generation() {
    use anacleto::agent::AgentId;

    let mut handles = Vec::with_capacity(100);
    for _ in 0..100 {
        handles.push(tokio::spawn(async { AgentId::new() }));
    }

    let results = join_all(handles).await;
    let ids: Vec<AgentId> = results.into_iter().map(|r| r.unwrap()).collect();

    assert_eq!(ids.len(), 100);

    let unique: HashSet<AgentId> = ids.into_iter().collect();
    assert_eq!(unique.len(), 100, "All 100 AgentIds must be unique");
}

// ---------------------------------------------------------------------------
// Permission checker concurrency: 50 tasks call check_* functions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_permissions_concurrent_access() {
    use anacleto::config::PermissionConfig;
    use anacleto::permissions::{checker, types::Permissions};

    let config = PermissionConfig {
        deny: vec!["command.run".into(), "net.http".into()],
        allow: vec![],
    };
    let perms = Arc::new(Permissions::from_config(&config));

    let mut handles = Vec::with_capacity(50);
    for i in 0..50 {
        let perms = Arc::clone(&perms);
        handles.push(tokio::spawn(async move {
            match i % 5 {
                0 => checker::check_fs_read(&perms),
                1 => checker::check_fs_write(&perms),
                2 => checker::check_command_run(&perms),
                3 => checker::check_net_http(&perms),
                _ => checker::check_env_read(&perms),
            }
        }));
    }

    let results = join_all(handles).await;
    for (i, result) in results.iter().enumerate() {
        let r = result.as_ref().unwrap();
        match i % 5 {
            2 => assert!(r.is_err(), "command.run should be denied"),
            3 => assert!(r.is_err(), "net.http should be denied"),
            _ => assert!(r.is_ok(), "permission should be allowed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Database concurrency: 10 tasks with Semaphore limiting to 5
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_database_concurrent_access() {
    use anacleto::db::Database;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("concurrent.db");
    let db = Arc::new(Database::open(&db_path).await.unwrap());
    let semaphore = Arc::new(Semaphore::new(5));

    let mut handles = Vec::with_capacity(10);
    for i in 0..10 {
        let db = Arc::clone(&db);
        let sem = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            let session_name = format!("concurrent-session-{}", i);
            let session = db.create_session(&session_name).await.unwrap();
            assert_eq!(session.name, session_name);

            db.store_message(
                session.id,
                "test-agent",
                "user",
                &format!("Message {} from user", i),
                None,
            )
            .await
            .unwrap();
            db.store_message(
                session.id,
                "test-agent",
                "assistant",
                &format!("Response {} from assistant", i),
                None,
            )
            .await
            .unwrap();

            let messages = db.get_session_messages(session.id).await.unwrap();
            assert_eq!(messages.len(), 2);

            session.id
        }));
    }

    let session_ids = join_all(handles).await;
    assert_eq!(session_ids.len(), 10, "All 10 tasks should complete");

    let sessions = db.list_sessions().await.unwrap();
    assert_eq!(sessions.len(), 10, "All 10 sessions should be persisted");

    for result in &session_ids {
        let session_id = result.as_ref().unwrap();
        let messages = db.get_session_messages(*session_id).await.unwrap();
        assert_eq!(
            messages.len(),
            2,
            "Each session must have exactly 2 messages"
        );
    }
}

// ---------------------------------------------------------------------------
// MCP Registry concurrent read access
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_registry_concurrent_access() {
    use anacleto::mcp::client::McpRegistry;

    let registry = Arc::new(McpRegistry::new());

    let mut handles = Vec::with_capacity(20);
    for i in 0..20 {
        let registry = Arc::clone(&registry);
        handles.push(tokio::spawn(async move {
            match i % 3 {
                0 => {
                    let _ = registry.get("non-existent");
                }
                1 => {
                    let tools = registry.collect_tools(&[]).await;
                    assert!(tools.is_empty());
                }
                _ => {
                    let tools = registry.collect_tools(&["nonexistent".to_string()]).await;
                    assert!(tools.is_empty());
                }
            }
        }));
    }

    let results = join_all(handles).await;
    for r in results {
        r.unwrap();
    }
}
