use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use uuid::Uuid;

use serde::{Deserialize, Serialize};

/// Shared state for tracking pending inline questions awaiting a user answer.
pub(crate) type PendingQuestions =
    Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>>;

use crate::config::AgentConfig;
use crate::llm::types::LlmMessage;

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

    /// Subagent names (only for Root agents).
    pub subagent_names: Vec<String>,

    /// Parent agent ID (None for Root agents).
    pub parent_id: Option<AgentId>,

    /// Maximum number of turns (LLM+tool iterations) per task before the agent
    /// is forced to stop and mark the task as incomplete.
    pub max_steps: u32,

    /// List of built-in tool names this agent can use.
    pub tools: Vec<String>,

    /// Additional paths where this agent can write (workspace is always writable).
    pub writable_paths: Vec<PathBuf>,

    /// Temperature for LLM sampling (0.0–2.0). `None` = use provider default.
    pub temperature: Option<f32>,

    /// Maximum output tokens. `None` = use provider default.
    pub max_tokens: Option<u32>,

    /// Top-p nucleus sampling (0.0–1.0). `None` = use provider default.
    pub top_p: Option<f32>,
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
            subagent_names: config.subagents.clone(),
            parent_id: None,
            max_steps: config.max_steps,
            tools: config.tools.clone(),
            writable_paths: config.writable_paths.clone(),
            temperature: config.temperature.map(|t| t as f32),
            max_tokens: config.max_tokens,
            top_p: config.top_p.map(|t| t as f32),
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
        max_steps: u32,
        parent_id: AgentId,
        tools: Vec<String>,
        writable_paths: Vec<PathBuf>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        top_p: Option<f32>,
    ) -> Self {
        Self {
            id: AgentId::new(),
            name,
            role: AgentRole::SubAgent,
            description,
            model,
            skills,
            mcps,
            subagent_names: Vec::new(),
            parent_id: Some(parent_id),
            max_steps,
            tools,
            writable_paths,
            temperature,
            max_tokens,
            top_p,
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
    /// Emergency stop signal — cancel current operation and return to idle.
    Cancel,
    /// Shutdown signal.
    Shutdown,
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

    #[test]
    fn background_task_id_unique_many() {
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = BackgroundTaskId::new();
            assert!(
                ids.insert(id),
                "BackgroundTaskId::new() produced a duplicate"
            );
        }
    }

    #[test]
    fn background_task_id_default() {
        let id = BackgroundTaskId::default();
        let display = format!("{id}");
        assert!(!display.is_empty());
    }

    #[test]
    fn background_task_id_display() {
        let id = BackgroundTaskId::new();
        let display = format!("{id}");
        assert_eq!(display, id.0);
    }

    #[test]
    fn background_task_id_equality() {
        let a = BackgroundTaskId::new();
        let b = BackgroundTaskId::new();
        assert_ne!(a, b);
        assert_eq!(a, a);
    }

    #[test]
    fn background_task_manager_new_has_default_ttl() {
        let mgr = BackgroundTaskManager::new();
        assert_eq!(mgr.ttl, Duration::from_secs(300));
        assert!(mgr.tasks.is_empty());
    }

    #[test]
    fn background_task_manager_with_ttl() {
        let custom = Duration::from_secs(60);
        let mgr = BackgroundTaskManager::with_ttl(custom);
        assert_eq!(mgr.ttl, custom);
    }

    #[tokio::test]
    async fn background_task_manager_insert_get_remove() {
        let task_id = BackgroundTaskId::new();
        let result: Arc<tokio::sync::Mutex<Option<anyhow::Result<String>>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let (tx, rx) = tokio::sync::oneshot::channel();

        let handle = tokio::spawn(async move {
            let _ = rx.await;
        });

        let task = BackgroundTask {
            task_id: task_id.clone(),
            agent_name: "test-agent".to_string(),
            started_at: Instant::now(),
            handle,
            result: result.clone(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        };

        let mut mgr = BackgroundTaskManager::new();
        mgr.insert(task);

        let fetched = mgr.get("test-agent");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().task_id, task_id);

        let removed = mgr.remove("test-agent");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().task_id, task_id);

        // Let the spawned task finish cleanly
        let _ = tx.send(());
    }

    #[tokio::test]
    async fn background_task_manager_cleanup_removes_expired_completed_tasks() {
        let result: Arc<tokio::sync::Mutex<Option<anyhow::Result<String>>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let (tx, rx) = tokio::sync::oneshot::channel();

        let handle = tokio::spawn(async move {
            let _ = rx.await;
        });

        // Mark the task as completed by setting a result
        *result.lock().await = Some(Ok("done".to_string()));

        let task = BackgroundTask {
            task_id: BackgroundTaskId::new(),
            agent_name: "old-agent".to_string(),
            started_at: Instant::now() - Duration::from_secs(600), // older than TTL
            handle,
            result: result.clone(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        };

        let mut mgr = BackgroundTaskManager::with_ttl(Duration::from_secs(300));
        mgr.insert(task);

        mgr.cleanup();

        assert!(
            mgr.get("old-agent").is_none(),
            "expired task should be removed"
        );

        let _ = tx.send(());
    }

    #[tokio::test]
    async fn background_task_manager_cleanup_keeps_running_tasks() {
        let result: Arc<tokio::sync::Mutex<Option<anyhow::Result<String>>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let (tx, rx) = tokio::sync::oneshot::channel();

        let handle = tokio::spawn(async move {
            let _ = rx.await;
        });

        // Result is None — task is still running
        let task = BackgroundTask {
            task_id: BackgroundTaskId::new(),
            agent_name: "active-agent".to_string(),
            started_at: Instant::now() - Duration::from_secs(600), // older than TTL
            handle,
            result: result.clone(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        };

        let mut mgr = BackgroundTaskManager::with_ttl(Duration::from_secs(300));
        mgr.insert(task);
        mgr.cleanup();

        // Running task should NOT be removed even if old
        assert!(
            mgr.get("active-agent").is_some(),
            "running task should be kept"
        );

        let _ = tx.send(());
    }

    #[tokio::test]
    async fn background_task_manager_cleanup_keeps_recent_completed_tasks() {
        let result: Arc<tokio::sync::Mutex<Option<anyhow::Result<String>>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let (tx, rx) = tokio::sync::oneshot::channel();

        let handle = tokio::spawn(async move {
            let _ = rx.await;
        });

        // Mark as completed
        *result.lock().await = Some(Ok("fresh".to_string()));

        let task = BackgroundTask {
            task_id: BackgroundTaskId::new(),
            agent_name: "fresh-agent".to_string(),
            started_at: Instant::now(), // recent — within TTL
            handle,
            result: result.clone(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        };

        let mut mgr = BackgroundTaskManager::with_ttl(Duration::from_secs(300));
        mgr.insert(task);
        mgr.cleanup();

        // Recent completed task should be kept
        assert!(
            mgr.get("fresh-agent").is_some(),
            "recent completed task should be kept"
        );

        let _ = tx.send(());
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

/// Unique identifier for a background task.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackgroundTaskId(pub String);

impl BackgroundTaskId {
    /// Creates a new unique background task identifier using a UUID v4 string.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for BackgroundTaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for BackgroundTaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A background task spawned for an agent, running asynchronously.
///
/// The task's result is stored in a shared [`Arc<Mutex>`] so that any component
/// (e.g., the TUI, the engine) can check whether it has completed without
/// owning the task directly.
#[derive(Debug)]
pub struct BackgroundTask {
    /// Unique task identifier.
    pub task_id: BackgroundTaskId,
    /// Name of the agent that owns this task.
    pub agent_name: String,
    /// Timestamp of when the task was started.
    pub started_at: Instant,
    /// Tokio join handle for the spawned future.
    pub handle: JoinHandle<()>,
    /// Shared, lock-protected result. `None` while the task is still running;
    /// `Some(Ok(...))` or `Some(Err(...))` once it finishes.
    pub result: Arc<tokio::sync::Mutex<Option<anyhow::Result<String>>>>,
    /// Cancel flag for this background task. Set to `true` to request cancellation.
    pub cancel_flag: Arc<AtomicBool>,
}

/// Manages a collection of background tasks keyed by agent name.
///
/// Provides `insert`, `get`, `remove`, and `cleanup` operations. Completed
/// tasks whose age exceeds the configured TTL are removed by [`cleanup`].
///
/// [`cleanup`]: Self::cleanup
#[derive(Debug)]
pub struct BackgroundTaskManager {
    /// Map from agent name to its active background task.
    pub tasks: HashMap<String, BackgroundTask>,
    /// Maximum age for a completed task before it is eligible for cleanup.
    pub ttl: Duration,
}

impl BackgroundTaskManager {
    /// Creates a new manager with the default TTL of 300 seconds.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            ttl: Duration::from_secs(300),
        }
    }

    /// Creates a new manager with a custom TTL.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            tasks: HashMap::new(),
            ttl,
        }
    }

    /// Inserts a background task, keyed by `task.agent_name`.
    ///
    /// If an existing task for the same agent name exists, it is replaced.
    pub fn insert(&mut self, task: BackgroundTask) {
        self.tasks.insert(task.agent_name.clone(), task);
    }

    /// Returns a reference to the background task for the given agent name,
    /// or `None` if no task exists.
    pub fn get(&self, agent_name: &str) -> Option<&BackgroundTask> {
        self.tasks.get(agent_name)
    }

    /// Removes the background task for the given agent name and returns it,
    /// or `None` if no task exists.
    pub fn remove(&mut self, agent_name: &str) -> Option<BackgroundTask> {
        self.tasks.remove(agent_name)
    }

    /// Removes all tasks that have completed and whose age exceeds the TTL.
    ///
    /// A task is considered completed when its `result` is `Some(Ok(..))` or
    /// `Some(Err(..))`. In-flight tasks (result is `None`) are never removed.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.tasks.retain(|_, task| {
            // Check if the task has completed (result is Some).
            let is_completed = task
                .result
                .try_lock()
                .ok()
                .map(|lock| lock.is_some())
                .unwrap_or(false);

            if !is_completed {
                // Task is still running — keep it.
                return true;
            }

            // Task has completed. Remove only if its age exceeds the TTL.
            now.duration_since(task.started_at) < self.ttl
        });
    }
}

impl Default for BackgroundTaskManager {
    fn default() -> Self {
        Self::new()
    }
}
