//! Integration tests for Anacleto.
//
// These tests exercise the full stack: config parsing, permission checking,
// database operations, and agent lifecycle. They do NOT connect to real
// LLM or MCP servers.

use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Config parsing
// ---------------------------------------------------------------------------

#[test]
fn test_config_parse_minimal() {
    use anacleto::config::Config;

    let yaml = r#"
models:
  ollama:
    base_url: "http://localhost:11434"
    model: "llama3.2"
    context_window: 8192
agents: []
"#;
    let config: Config = serde_yaml::from_str(yaml).unwrap();
    assert!(config.models.ollama.is_some());
    assert!(config.agents.is_empty());
}

#[test]
fn test_config_parse_with_agents_ignored() {
    use anacleto::config::Config;

    let yaml = r#"
models:
  ollama:
    base_url: "http://localhost:11434"
    model: "llama3.2"
    context_window: 8192
agents:
  - name: root
    description: "agents/root.md"
    model: "llama3.2"
    subagents: [reviewer]
  - name: reviewer
    description: "agents/reviewer.md"
    model: "llama3.2"
    subagents: []
"#;
    // Agents are no longer defined in YAML — the `agents:` key is ignored.
    let config: Config = serde_yaml::from_str(yaml).unwrap();
    assert!(config.agents.is_empty());
}

#[test]
fn test_config_parse_with_skills_and_mcps() {
    use anacleto::config::Config;

    let yaml = r#"
models:
  ollama:
    base_url: "http://localhost:11434"
    model: "llama3.2"
    context_window: 8192
mcps:
  filesystem:
    transport: stdio
    command: "mcp-fs"
    args: ["--dir", "/tmp"]
agents:
  - name: root
    description: "root.md"
    model: "llama3.2"
    skills:
      - "skills/shell/"
    mcps:
      - filesystem
    subagents: []
"#;
    let config: Config = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.mcps.len(), 1);
    assert!(config.mcps.contains_key("filesystem"));
    // Agents are no longer defined in YAML — the `agents:` key is ignored.
    assert!(config.agents.is_empty());
}

#[test]
fn test_config_merge_does_not_merge_agents() {
    use anacleto::agent::types::AgentRole;
    use anacleto::config::{AgentConfig, Config};

    let global = Config {
        agents: vec![AgentConfig {
            name: "root".into(),
            description: "root".into(),
            when_to_use: String::new(),
            role: AgentRole::Root,
            model: "llama2".into(),
            skills: vec![],
            mcps: vec![],
            subagents: vec![],
            system_prompt: "You are root.".into(),
            max_steps: 60,
            tools: vec![],
            writable_paths: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
        }],
        ..Default::default()
    };

    let project = Config {
        agents: vec![AgentConfig {
            name: "root".into(),
            description: "new-root".into(),
            when_to_use: String::new(),
            role: AgentRole::Root,
            model: "llama3.2".into(),
            skills: vec![],
            mcps: vec![],
            subagents: vec!["reviewer".into()],
            system_prompt: "You are the new root.".into(),
            max_steps: 60,
            tools: vec![],
            writable_paths: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
        }],
        ..Default::default()
    };

    let mut merged = global.clone();
    anacleto::config::loader::merge_configs(&mut merged, project);

    // Agents are NOT merged by merge_configs — they come from the Markdown loader.
    assert_eq!(merged.agents.len(), 1);
    assert_eq!(merged.agents[0].model, "llama2");
}

// ---------------------------------------------------------------------------
// Permission checking
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Agent types
// ---------------------------------------------------------------------------

#[test]
fn test_agent_id_generation() {
    use anacleto::agent::AgentId;

    let id = AgentId::new();
    assert!(!id.to_string().is_empty());

    let id2 = AgentId::new();
    assert_ne!(id, id2);
}

#[test]
fn test_agent_message_types() {
    use anacleto::agent::AgentMessage;

    let msg = AgentMessage::UserInput {
        content: "hello".into(),
    };
    match &msg {
        AgentMessage::UserInput { content } => assert_eq!(content, "hello"),
        _ => panic!("Expected UserInput"),
    }
}

// ---------------------------------------------------------------------------
// Database operations
// ---------------------------------------------------------------------------

#[test]
fn test_database_session_crud() {
    use anacleto::db::Database;
    use tempfile::TempDir;

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();

        // Create a session
        let session = db.create_session("test-session").await.unwrap();
        assert!(!session.id.to_string().is_empty());
        assert_eq!(session.name, "test-session");

        // List sessions
        let sessions = db.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);

        // Store messages
        db.store_message(session.id, "root", "user", "hello", None)
            .await
            .unwrap();
        db.store_message(session.id, "root", "assistant", "world", None)
            .await
            .unwrap();

        // Load messages
        let messages = db.get_session_messages(session.id).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].content, "world");

        // Rename session
        db.rename_session(session.id, "renamed").await.unwrap();
        let sessions = db.list_sessions().await.unwrap();
        assert_eq!(sessions[0].name, "renamed");

        // Delete session
        db.delete_session(session.id).await.unwrap();
        let sessions = db.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 0);
    });
}

// ---------------------------------------------------------------------------
// Skill parsing
// ---------------------------------------------------------------------------

#[test]
fn test_skill_parse() {
    use anacleto::skill::loader;

    let content = r#"---
name: test-skill
description: A test skill
---

# Test Skill

This is a test skill description.
"#;

    let skill = loader::parse_skill(content).unwrap();
    assert_eq!(skill.name, "test-skill");
    assert_eq!(skill.description, "A test skill");
    assert!(skill.instructions.contains("Test Skill"));
}

// ---------------------------------------------------------------------------
// MCP types
// ---------------------------------------------------------------------------

#[test]
fn test_mcp_transport_stdio() {
    use anacleto::config::McpDefinition;
    use anacleto::mcp::types::McpTransport;

    let def = McpDefinition {
        transport: "stdio".into(),
        command: Some("npx".into()),
        args: vec![
            "-y".into(),
            "@modelcontextprotocol/server-filesystem".into(),
        ],
        host: None,
        port: None,
    };

    let transport = McpTransport::from(&def);
    match transport {
        McpTransport::Stdio { command, args } => {
            assert_eq!(command, "npx");
            assert!(!args.is_empty());
        }
        _ => panic!("Expected Stdio transport"),
    }
}

#[test]
fn test_mcp_transport_tcp() {
    use anacleto::config::McpDefinition;
    use anacleto::mcp::types::McpTransport;

    let def = McpDefinition {
        transport: "tcp".into(),
        command: None,
        args: vec![],
        host: Some("localhost".into()),
        port: Some(5432),
    };

    let transport = McpTransport::from(&def);
    match transport {
        McpTransport::Tcp { host, port } => {
            assert_eq!(host, "localhost");
            assert_eq!(port, 5432);
        }
        _ => panic!("Expected Tcp transport"),
    }
}

// ---------------------------------------------------------------------------
// Retry config
// ---------------------------------------------------------------------------

#[test]
fn test_retry_config_defaults() {
    use anacleto::config::RetryConfig;

    let config = RetryConfig::default();
    assert_eq!(config.max_retries, 3);
    assert_eq!(config.base_delay_ms, 1000);
    assert_eq!(config.max_delay_ms, 30000);
}

fn retry_config() -> impl Strategy<Value = anacleto::config::RetryConfig> {
    (0u32..10, 0u64..60000u64, 0u64..60000u64).prop_map(
        |(max_retries, base_delay_ms, max_delay_ms)| anacleto::config::RetryConfig {
            max_retries,
            base_delay_ms,
            max_delay_ms,
        },
    )
}

proptest! {
    #[test]
    fn retry_config_roundtrip(config in retry_config()) {
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: anacleto::config::RetryConfig = serde_yaml::from_str(&yaml).unwrap();
        prop_assert_eq!(config.max_retries, deserialized.max_retries);
        prop_assert_eq!(config.base_delay_ms, deserialized.base_delay_ms);
        prop_assert_eq!(config.max_delay_ms, deserialized.max_delay_ms);
    }

    #[test]
    fn config_yaml_parses_without_panic(agent_count in 0u32..5u32) {
        let mut agents_yaml = String::new();
        for i in 0..agent_count {
            agents_yaml.push_str(&format!(
                r#"  - name: "agent-{}"
    description: "agents/agent-{}.md"
    model: "llama3.2"
    subagents: []
"#,
                i, i
            ));
        }
        let yaml = format!(
            r#"
models:
  ollama:
    base_url: "http://localhost:11434"
    model: "llama3.2"
    context_window: 8192
agents:
{}
"#,
            agents_yaml
        );
        let config: std::result::Result<anacleto::config::Config, _> = serde_yaml::from_str(&yaml);
        prop_assert!(config.is_ok());
        let config = config.unwrap();
        // Agents are no longer defined in YAML — the `agents:` key is ignored,
        // so the parsed config always has zero agents regardless of agent_count.
        prop_assert!(config.agents.is_empty());
    }
}
