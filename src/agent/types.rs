use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::config::{AgentConfig, ToolSettings};
use crate::llm::types::LlmMessage;
use crate::permissions::Permissions;

/// Unique identifier for an agent or subagent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// Creates a new unique agent identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The role of an agent in the hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    /// Top-level agent, invocable by the user.
    Root,
    /// Subagent, created by a parent agent, disposable.
    SubAgent,
}

/// An agent or subagent instance.
#[derive(Debug, Clone)]
pub struct Agent {
    /// Unique identifier.
    pub id: AgentId,

    /// Agent name from config.
    pub name: String,

    /// Role in hierarchy.
    pub role: AgentRole,

    /// System prompt (persona) for this agent, loaded from the Markdown body.
    pub description: String,

    /// LLM model to use.
    pub model: String,

    /// Skill paths.
    pub skills: Vec<PathBuf>,

    /// MCP names (references global MCP definitions).
    pub mcps: Vec<String>,

    /// Permissions for this agent.
    pub permissions: Permissions,

    /// Subagent names (only for Root agents).
    pub subagent_names: Vec<String>,

    /// Parent agent ID (None for Root agents).
    pub parent_id: Option<AgentId>,

    /// Maximum number of turns (LLM+tool iterations) per task before the agent
    /// is forced to stop and mark the task as incomplete.
    pub max_steps: u32,

    /// Maximum depth of dynamic subagent delegation via the `task` tool.
    pub subagent_depth: u32,

    /// Per-tool configuration overrides.
    pub tool_settings: HashMap<String, ToolSettings>,
}

impl Agent {
    /// Create a new agent from configuration.
    pub fn from_config(config: &AgentConfig, role: AgentRole) -> Self {
        Self {
            id: AgentId::new(),
            name: config.name.clone(),
            role,
            description: config.system_prompt.clone(),
            model: config.model.clone(),
            skills: config.skills.clone(),
            mcps: config.mcps.clone(),
            permissions: Permissions::from_config(&config.permissions),
            subagent_names: config.subagents.clone(),
            parent_id: None,
            max_steps: config.max_steps,
            subagent_depth: config.subagent_depth,
            tool_settings: config.tools.clone(),
        }
    }

    /// Create a subagent from a parent agent's subagent config.
    #[allow(clippy::too_many_arguments)]
    pub fn create_subagent(
        name: String,
        description: String,
        model: String,
        skills: Vec<PathBuf>,
        mcps: Vec<String>,
        permissions: Permissions,
        max_steps: u32,
        parent_id: AgentId,
        tool_settings: HashMap<String, ToolSettings>,
    ) -> Self {
        Self {
            id: AgentId::new(),
            name,
            role: AgentRole::SubAgent,
            description,
            model,
            skills,
            mcps,
            permissions,
            subagent_names: Vec::new(),
            parent_id: Some(parent_id),
            max_steps,
            subagent_depth: 3,
            tool_settings,
        }
    }

    /// Whether this agent is a root-level agent.
    pub fn is_root(&self) -> bool {
        self.role == AgentRole::Root
    }

    /// Whether this agent is a subagent.
    pub fn is_subagent(&self) -> bool {
        self.role == AgentRole::SubAgent
    }
}

/// Message sent between agents or between engine and agent.
#[derive(Debug, Clone)]
pub enum AgentMessage {
    /// A user message to process.
    UserInput { content: String },
    /// A message from a parent agent to a subagent.
    Delegate {
        task: String,
        context: Vec<MessageEntry>,
    },
    /// A response from a subagent back to parent.
    Response { content: String },
    /// Internal system message.
    System { content: String },
    /// Tool/skill execution request.
    ToolCall {
        tool_name: String,
        arguments: serde_json::Value,
    },
    /// Tool/skill execution result.
    ToolResult {
        tool_name: String,
        result: serde_json::Value,
    },
    /// Load conversation history into the agent.
    LoadHistory(Vec<LlmMessage>),
    /// Clear the agent's conversation history.
    ClearHistory,
    /// Force compaction of the conversation context (summarize old messages).
    Compact,
    /// A dynamic task delegation request (via the `task` tool).
    Task {
        task_id: String,
        description: String,
        mode: TaskMode,
        model: Option<String>,
        tools: Vec<String>,
    },
    /// Emergency stop signal — cancel current operation and return to idle.
    Cancel,
    /// Shutdown signal.
    Shutdown,
}

/// Execution mode for a dynamically delegated task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskMode {
    /// Run the subagent synchronously and wait for its result.
    Foreground,
    /// Launch the subagent in the background and return immediately.
    Background,
}

/// The operational mode of an agent, controlling which tools are available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMode {
    /// Read-only: write tools are disabled.
    Plan,
    /// Full read/write access.
    Build,
}

/// A single entry in a message history.
#[derive(Debug, Clone)]
pub struct MessageEntry {
    /// Role of the message sender.
    pub role: MessageRole,
    /// Text content of the message.
    pub content: String,
    /// Timestamp when the message was sent.
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Role of a message sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    /// Message from a human user.
    User,
    /// Message from the LLM assistant.
    Assistant,
    /// System-level instruction message.
    System,
    /// Result of a tool or skill execution.
    Tool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_agent_id_unique_many() {
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = AgentId::new();
            assert!(ids.insert(id), "AgentId::new() produced a duplicate");
        }
    }

    #[test]
    fn test_agent_id_display_debug() {
        let id = AgentId::new();
        let display = format!("{}", id);
        let debug = format!("{:?}", id);
        assert!(!display.is_empty());
        assert!(!debug.is_empty());
        assert_eq!(display, id.0.to_string());
    }

    proptest! {
        #[test]
        fn message_entry_content_preserved(content: String, role_kind: u8) {
            let role = match role_kind % 4 {
                0 => MessageRole::User,
                1 => MessageRole::Assistant,
                2 => MessageRole::System,
                _ => MessageRole::Tool,
            };
            let entry = MessageEntry {
                content: content.clone(),
                role: role.clone(),
                timestamp: chrono::Utc::now(),
            };
            prop_assert_eq!(entry.content, content);
            prop_assert_eq!(entry.role, role.clone());
        }
    }
}

/// Status of an agent's lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    /// Agent is initialized and ready.
    Idle,
    /// Agent is processing a message.
    Working,
    /// Agent is waiting for a subagent response.
    WaitingForSubAgent,
    /// Agent has completed and is awaiting destruction (subagents only).
    Completed,
    /// Agent encountered an error.
    Error(String),
}
