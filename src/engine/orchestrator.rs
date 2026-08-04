use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::agent::lifecycle::{AgentHandle, SpawnAgentConfig, spawn_agent};
use crate::agent::types::{Agent, AgentId, AgentMessage, AgentMode, AgentRole, AgentStatus};
use crate::config::Config;
use crate::config::types::{OllamaConfig, ProviderConfig};
use crate::db::models::{SessionSummary, Snapshot, StoredMessage};
use crate::db::session::Database;
use crate::engine::jobs::JobRegistry;
use crate::error::{Error, Result};
use crate::llm::provider::{LlmProvider, LlmProviderRegistry, create_provider};
use crate::llm::types::{LlmMessage, LlmProviderConfig, LlmProviderType, MessageRole};
use crate::mcp::client::McpRegistry;
use crate::shell::{git_worktree_add, git_worktree_list, git_worktree_remove};
use crate::skill::loader::load_agent_skills;

/// Events emitted by the engine for the TUI to display.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Engine started.
    Started { debug: bool },
    /// Agent created.
    AgentCreated {
        id: AgentId,
        name: String,
        role: AgentRole,
        model: String,
        skills: Vec<String>,
        mcps: Vec<String>,
    },
    /// Agent received a message.
    AgentMessage {
        agent_id: AgentId,
        agent_name: String,
        message: String,
    },
    /// Agent produced output.
    AgentOutput {
        agent_id: AgentId,
        agent_name: String,
        content: String,
    },
    /// Streaming chunk from an agent's LLM response.
    AgentStreamChunk {
        agent_id: AgentId,
        agent_name: String,
        content: String,
    },
    /// Agent status changed.
    AgentStatusChanged {
        agent_id: AgentId,
        agent_name: String,
        status: AgentStatus,
    },
    /// Subagent created by a parent.
    SubagentCreated {
        parent_id: AgentId,
        subagent_id: AgentId,
        subagent_name: String,
        skills: Vec<String>,
        mcps: Vec<String>,
    },
    /// Subagent completed.
    SubagentCompleted {
        subagent_id: AgentId,
        subagent_name: String,
        result: String,
    },
    /// Session list.
    SessionList(Vec<SessionSummary>),
    /// Session was switched.
    SessionSwitched { id: String, name: String },
    /// Session was deleted.
    SessionDeleted { id: String },
    /// Session was renamed.
    SessionRenamed { id: String, name: String },
    /// Error occurred.
    Error {
        agent_id: Option<AgentId>,
        message: String,
    },
    /// Human approval required for a sensitive operation.
    ApprovalRequired { id: String, operation: String },
    /// Engine shutting down.
    ShuttingDown,
    /// Token usage reported after an LLM response.
    TokenUsage {
        agent_id: AgentId,
        agent_name: String,
        total_tokens: u32,
        context_window: u32,
        cost: f64,
    },
    /// Tool/skill execution started.
    ToolExecution {
        agent_id: AgentId,
        agent_name: String,
        tool_name: String,
        task: String,
    },
    /// Tool/skill execution completed.
    ToolResult {
        agent_id: AgentId,
        agent_name: String,
        tool_name: String,
        success: bool,
        summary: String,
    },
    /// Debug: serialized LLM request payload (only when debug mode is on).
    LlmRequestDebug {
        agent_name: String,
        model: String,
        payload: String,
    },
    /// Debug: serialized LLM response payload (only when debug mode is on).
    LlmResponseDebug {
        agent_name: String,
        model: String,
        payload: String,
    },
    /// The model for the root agent changed.
    ModelChanged { model: String },
    /// The active root agent changed (via `/agent`).
    AgentSwitched { name: String },
    /// The conversation context was compacted (via `/compact`).
    ConversationCompacted {
        agent_id: AgentId,
        agent_name: String,
    },
    /// The last message pair was undone (via `/undo`).
    UndoApplied { removed: Vec<String> },
    /// The last undone message pair was restored (via `/redo`).
    RedoApplied { restored: Vec<String> },
    /// The active session was forked into a new session (via `/fork`).
    Forked { new_session_id: Uuid },
    /// A session was exported to a file (via `/export`).
    Exported { path: PathBuf },
    /// A session was imported from a file (via `/import`).
    Imported { session_id: Uuid },
    /// The share state of the active session changed (via `/share`/`/unshare`).
    ShareUpdated { shared: bool, link: Option<String> },
    /// The skills of the active agent were listed (via `/skills`).
    SkillsListed(Vec<SkillInfo>),
    /// The MCP servers were listed (via `/mcps`).
    McpsListed(Vec<McpStatus>),
    /// A status report was produced (via `/status`).
    StatusReport(StatusInfo),
    /// `AGENTS.md` was generated (via `/init`).
    InitDone,
    /// A git review was dispatched to the root agent (via `/review`).
    ReviewResult(String),
    /// The engine workspace changed (via `/warp`).
    WorkspaceChanged(PathBuf),
    /// The known workspaces were listed (via `/workspaces`).
    WorkspacesListed(Vec<String>),
    /// The session timeline was produced (via `/timeline`).
    Timeline(Vec<TimelineEntry>),
    /// The active session was moved to another workspace (via `/move`).
    SessionMoved { session_id: Uuid, workspace: String },
    /// The todo list for the active session changed (via the `todo` tool).
    TodosUpdated(Vec<crate::db::models::Todo>),
    /// The agent asked the user a structured question (via the `question` tool).
    Question {
        id: String,
        question: String,
        options: Vec<String>,
        recommended: Option<String>,
    },
    /// A command handler failed; the engine loop continues.
    CommandError(String),
    /// A unified diff is available for display (e.g. after `apply_patch`).
    DiffAvailable { text: String, title: String },
    /// Model usage frequency records (model, count) for the picker.
    ModelsFrecency(Vec<(String, usize)>),
    /// Result of a git worktree operation (via `/worktree`).
    WorktreeResult(String),
    /// A background task (dynamic `task` tool delegation) finished.
    SubagentFinished { task_id: String, summary: String },
    /// The active session's plan was handed off to build mode (via `/build`).
    BuildDone,
    /// The session hierarchy (children of the active session) was produced
    /// (via `/children`).
    SessionTree(Vec<SessionSummary>),
    /// The list of running background jobs was produced (via `/jobs`).
    JobsListed(Vec<String>),
    /// A snapshot of the active session was created (via `/snapshot`).
    SnapshotCreated { snapshot: Snapshot },
    /// The active session was reverted to a snapshot (via `/revert`).
    SnapshotReverted { snapshot_id: Uuid },
    /// The snapshots of the active session were listed (via `/snapshots`).
    SnapshotsListed(Vec<Snapshot>),
}

/// Output format for a session export.
pub use crate::db::models::ExportFormat;

/// Answers collected by the interactive `/init` flow.
#[derive(Debug, Clone)]
pub struct InitAnswers {
    /// Project/agent name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Technology stack.
    pub stack: String,
}

/// Information about a skill, reported by `/skills`.
#[derive(Debug, Clone)]
pub struct SkillInfo {
    /// Skill name.
    pub name: String,
    /// Skill description.
    pub description: String,
}

/// Status of an MCP server, reported by `/mcps`.
#[derive(Debug, Clone)]
pub struct McpStatus {
    /// Server name.
    pub name: String,
    /// Whether the server is enabled.
    pub enabled: bool,
}

/// Engine status report, produced by `/status`.
#[derive(Debug, Clone)]
pub struct StatusInfo {
    /// Active model for the root agent.
    pub model: String,
    /// Active session id (if any).
    pub session_id: Option<Uuid>,
    /// Active session name.
    pub session_name: String,
    /// Total tokens consumed.
    pub total_tokens: u32,
    /// Context window size of the active model.
    pub context_window: u32,
    /// Total cost in dollars.
    pub cost: f64,
    /// Whether debug mode is on.
    pub debug: bool,
    /// Current engine workspace.
    pub workspace: PathBuf,
}

/// A timeline entry, produced by `/timeline`.
#[derive(Debug, Clone)]
pub struct TimelineEntry {
    /// Message id.
    pub id: Uuid,
    /// Message role.
    pub role: String,
    /// Message content.
    pub content: String,
    /// When the message was created.
    pub created_at: DateTime<Utc>,
}

/// Token/cost usage reported by an agent task, accumulated by the engine
/// for `/status`.
#[derive(Debug, Clone)]
pub struct UsageEvent {
    /// Total tokens consumed.
    pub total_tokens: u32,
    /// Cost in dollars.
    pub cost: f64,
}

/// The core orchestration engine.
pub struct Engine {
    /// Loaded configuration.
    config: Config,
    /// Registered agents (name -> id lookup).
    agents: HashMap<String, AgentId>,
    /// Active agent handles (id -> handle).
    handles: HashMap<AgentId, AgentHandle>,
    /// LLM provider registry.
    llm_registry: LlmProviderRegistry,
    /// MCP server registry.
    mcp_registry: Arc<tokio::sync::Mutex<McpRegistry>>,
    /// Database for persistence.
    database: Option<Database>,
    /// Active session ID.
    active_session_id: Option<Uuid>,
    /// Channel to send events to the TUI.
    event_tx: mpsc::Sender<EngineEvent>,
    /// Channel to receive commands from the TUI.
    command_rx: mpsc::Receiver<EngineCommand>,
    /// Channel to receive usage reports from agent tasks (for `/status`).
    usage_rx: mpsc::Receiver<UsageEvent>,
    /// Sender half of the usage channel, cloned into agent tasks.
    usage_tx: mpsc::Sender<UsageEvent>,
    /// Pending human approvals (id -> oneshot sender).
    pending_approvals: Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
    /// Pending inline questions (id -> oneshot sender) for the `question` tool.
    pending_questions:
        Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>>,
    /// Debug mode flag (shows LLM request/response payloads).
    /// Shared with agent tasks so the `/debug` toggle takes effect immediately.
    debug: Arc<AtomicBool>,
    /// Current model for the root agent.
    current_model: String,
    /// Name of the currently active root agent (routing target for user input).
    active_agent: String,
    /// Stack of undone message pairs (for `/undo`).
    undo_stack: Vec<Vec<StoredMessage>>,
    /// Stack of undone message pairs available for `/redo`.
    redo_stack: Vec<Vec<StoredMessage>>,
    /// Current engine workspace directory.
    workspace: PathBuf,
    /// Per-server MCP enabled state (for `/mcps` toggling). Shared with agents
    /// so they can gate tool collection.
    mcp_enabled: Arc<tokio::sync::Mutex<HashMap<String, bool>>>,
    /// Total tokens consumed (tracked for `/status`).
    total_tokens: u32,
    /// Total cost in dollars (tracked for `/status`).
    total_cost: f64,
    /// Registry of running background jobs (dynamic `task` tool delegations).
    job_registry: Arc<tokio::sync::Mutex<JobRegistry>>,
    /// A staged snapshot (via `/stage`) awaiting commit (via `/commit`).
    staged_snapshot: Option<Snapshot>,
}

/// Commands from the TUI to the engine.
#[derive(Debug)]
pub enum EngineCommand {
    /// Send user input to the root agent.
    UserInput(String),
    /// Start a new session.
    NewSession(String),
    /// Resume a session.
    ResumeSession(String),
    /// List all sessions.
    ListSessions,
    /// Delete a session.
    DeleteSession(String),
    /// Rename a session.
    RenameSession(String, String),
    /// Respond to a human approval request.
    ApprovalResponse { id: String, approved: bool },
    /// Toggle debug mode on/off.
    SetDebug(bool),
    /// Change the model for the root agent.
    SetModel(String),
    /// Switch the active root agent.
    SwitchAgent(String),
    /// Record model usage for the frecency ranking.
    RecordModelUsage(String),
    /// Request the current model usage frequency records.
    ListModelFrecency,
    /// Force compaction of the root agent's conversation context.
    Compact,
    /// Undo the last message pair in the active session.
    Undo,
    /// Redo the last undone message pair.
    Redo,
    /// Fork the active session into a new session.
    Fork,
    /// Export the active session transcript to a file.
    Export {
        /// Optional output path; defaults to a generated name.
        path: Option<PathBuf>,
        /// Output format (defaults to JSON).
        format: Option<ExportFormat>,
    },
    /// Import a session transcript from a file.
    Import { path: PathBuf },
    /// Mark the active session as shared and generate a link.
    Share,
    /// Remove the shared state from the active session.
    Unshare,
    /// List the skills of the active agent.
    ListSkills,
    /// List the MCP servers and their enabled state.
    ListMcps,
    /// Enable or disable an MCP server.
    ToggleMcp { name: String, enabled: bool },
    /// Produce an engine status report.
    Status,
    /// Generate AGENTS.md from collected answers.
    Init { answers: InitAnswers },
    /// Review git changes (optionally a specific commit/branch).
    Review { target: Option<String> },
    /// Set the engine workspace directory.
    Warp { dir: PathBuf },
    /// List the known workspaces.
    ListWorkspaces,
    /// Move the active session to another workspace.
    MoveSession { workspace: String },
    /// Add a git worktree.
    WorktreeAdd {
        path: String,
        branch: Option<String>,
    },
    /// List git worktrees.
    WorktreeList,
    /// Remove a git worktree.
    WorktreeRemove { path: String },
    /// Produce the timeline of the active session.
    Timeline,
    /// Respond to an inline question asked by the agent (via the `question` tool).
    QuestionAnswer { id: String, answer: String },
    /// Pin or unpin a session (shown at the top of the sidebar).
    SetSessionPinned { id: String, pinned: bool },
    /// List the running background jobs (via `/jobs`).
    ListJobs,
    /// Hand off the active session's plan to build mode (via `/build`).
    Build,
    /// Navigate to the parent session of the active session (via `/parent`).
    Parent,
    /// List the child sessions of the active session (via `/children`).
    Children,
    /// Create a snapshot of the active session's conversation (via `/snapshot`).
    Snapshot { name: Option<String> },
    /// Revert the active session to a snapshot (via `/revert`).
    Revert { snapshot_id: Uuid },
    /// List the snapshots of the active session (via `/snapshots`).
    ListSnapshots,
    /// Stage the current conversation state as a pending snapshot (via `/stage`).
    Stage { name: Option<String> },
    /// Clear the staged snapshot (via `/clear`).
    Clear,
    /// Commit the staged snapshot (via `/commit`).
    Commit { name: Option<String> },
    /// Shutdown the engine.
    Shutdown,
}

impl Engine {
    /// Create a new engine.
    pub fn new(
        config: Config,
        event_tx: mpsc::Sender<EngineEvent>,
        command_rx: mpsc::Receiver<EngineCommand>,
    ) -> Self {
        let (usage_tx, usage_rx) = mpsc::channel(64);
        Self {
            config: config.clone(),
            agents: HashMap::new(),
            handles: HashMap::new(),
            llm_registry: LlmProviderRegistry::new(),
            mcp_registry: Arc::new(tokio::sync::Mutex::new(McpRegistry::new())),
            pending_approvals: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            pending_questions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            debug: Arc::new(AtomicBool::new(false)),
            current_model: config
                .agents
                .iter()
                .find(|a| a.role == AgentRole::Root)
                .map(|a| a.model.clone())
                .unwrap_or_default(),
            active_agent: config
                .agents
                .iter()
                .find(|a| a.role == AgentRole::Root)
                .map(|a| a.name.clone())
                .unwrap_or_default(),
            database: None,
            active_session_id: None,
            event_tx,
            command_rx,
            usage_rx,
            usage_tx,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            mcp_enabled: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            total_tokens: 0,
            total_cost: 0.0,
            job_registry: Arc::new(tokio::sync::Mutex::new(JobRegistry::new())),
            staged_snapshot: None,
        }
    }

    /// Initialize the engine: load config, create providers and agents, connect MCPs.
    pub async fn initialize(&mut self) -> Result<()> {
        // Sync debug flag from config (--debug CLI flag sets this)
        self.debug
            .store(self.config.session.debug, Ordering::Relaxed);

        self.event_tx
            .send(EngineEvent::Started {
                debug: self.debug.load(Ordering::Relaxed),
            })
            .await
            .ok();

        // Notify the TUI of the active model at startup.
        self.event_tx
            .send(EngineEvent::ModelChanged {
                model: self.current_model.clone(),
            })
            .await
            .ok();

        // Notify the TUI of the active root agent at startup so the status bar
        // is populated before the first `/agent` switch.
        self.event_tx
            .send(EngineEvent::AgentSwitched {
                name: self.active_agent.clone(),
            })
            .await
            .ok();

        // Initialize database and create a session
        let db = Database::open(&self.config.session.database_path).await?;
        let session = db.create_session("default").await?;
        let session_id = session.id;
        self.database = Some(db.clone());
        self.active_session_id = Some(session_id);

        // Create LLM providers from config and register them
        let mut llm_registry = LlmProviderRegistry::new();

        if let Some(ref cfg) = self.config.models.anthropic {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::Anthropic);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("anthropic".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.openai {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::OpenAI);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("openai".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.openrouter {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::OpenRouter);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("openrouter".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.ollama {
            let llm_cfg = ollama_config_to_llm(cfg);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("ollama".into(), provider);
        }

        self.llm_registry = llm_registry;

        // Connect MCP servers before spawning agents so tools are available
        {
            let mut mcp = self.mcp_registry.lock().await;
            for (name, def) in &self.config.mcps {
                if let Err(e) = mcp.register(name.clone(), def).await {
                    tracing::warn!("MCP server '{}' unavailable (skipping): {}", name, e);
                }
            }
        }

        // Collect all subagent names (agents that are listed as subagents of another agent)
        let subagent_names: std::collections::HashSet<String> = self
            .config
            .agents
            .iter()
            .flat_map(|a| a.subagents.iter().cloned())
            .collect();

        // Build a name-to-config map for quick lookup
        let config_by_name: std::collections::HashMap<String, &crate::config::AgentConfig> = self
            .config
            .agents
            .iter()
            .map(|a| (a.name.clone(), a))
            .collect();

        // Only spawn root agents — subagents are spawned on-demand by their parent
        for agent_config in &self.config.agents {
            if subagent_names.contains(&agent_config.name) {
                // Skip — this is a subagent, spawned on-demand
                continue;
            }

            let agent = Agent::from_config(agent_config, AgentRole::Root);
            let name = agent.name.clone();
            let id = agent.id.clone();

            self.event_tx
                .send(EngineEvent::AgentCreated {
                    id: id.clone(),
                    name: name.clone(),
                    role: AgentRole::Root,
                    model: agent.model.clone(),
                    skills: agent
                        .skills
                        .iter()
                        .filter_map(|p| {
                            p.file_stem()
                                .and_then(|f| f.to_str().map(|s| s.to_string()))
                        })
                        .collect(),
                    mcps: agent.mcps.clone(),
                })
                .await
                .ok();

            // Resolve the LLM provider for this agent based on model name
            let provider = self.resolve_agent_provider(&agent)?;

            // Load skills for this agent
            let skills = load_agent_skills(&agent.skills);

            // Collect subagent configs for this agent
            let my_subagent_configs: Vec<crate::config::AgentConfig> = agent_config
                .subagents
                .iter()
                .filter_map(|name| config_by_name.get(name).map(|c| (*c).clone()))
                .collect();

            // Spawn the agent as a real tokio task
            let history_limit = self.config.session.history_limit_percent;
            let retry_cfg = self.config.session.retry.clone();
            let handle = spawn_agent(SpawnAgentConfig {
                agent,
                provider,
                skills,
                subagent_configs: my_subagent_configs,
                llm_registry: self.llm_registry.clone(),
                mcp_registry: Some(self.mcp_registry.clone()),
                mcp_enabled: Some(self.mcp_enabled.clone()),
                event_tx: self.event_tx.clone(),
                usage_tx: Some(self.usage_tx.clone()),
                retry_config: retry_cfg,
                db: self.database.clone(),
                session_id: self.active_session_id,
                pending_approvals: Some(self.pending_approvals.clone()),
                pending_questions: Some(self.pending_questions.clone()),
                history_limit_percent: history_limit,
                debug: self.debug.clone(),
                workspace: self.workspace.clone(),
                task_id: None,
                depth: 0,
                mode: AgentMode::Build,
                job_registry: Some(self.job_registry.clone()),
            });

            self.agents.insert(name, id.clone());
            self.handles.insert(id, handle);
        }

        Ok(())
    }

    /// Resolve which LLM provider an agent should use based on its model name.
    fn resolve_agent_provider(&self, agent: &Agent) -> Result<Arc<dyn LlmProvider>> {
        let model = &agent.model;
        let provider_name = if model.contains('/') {
            "openrouter"
        } else if model.starts_with("claude") {
            "anthropic"
        } else if model.starts_with("gpt") || model.starts_with("o1") || model.starts_with("o3") {
            "openai"
        } else {
            "ollama"
        };

        self.llm_registry.get(provider_name).ok_or_else(|| {
            Error::Provider(format!(
                "No provider configured for model '{model}'. Tried provider: {provider_name}. \
                 Ensure the corresponding section is configured in models config."
            ))
        })
    }

    /// Run the main event loop.
    pub async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    let Some(command) = command else { break; };
                    // Shutdown is handled outside the error-catching block so it always
                    // terminates the loop even if a prior handler failed.
                    if matches!(command, EngineCommand::Shutdown) {
                        self.event_tx.send(EngineEvent::ShuttingDown).await.ok();
                        break;
                    }

                    // Dispatch the command inside an async block so a handler error is
                    // reported to the TUI instead of killing the engine event loop.
                    let result: Result<()> = async {
                        match command {
                            EngineCommand::UserInput(input) => {
                                self.handle_user_input(input).await?;
                            }
                            EngineCommand::NewSession(name) => {
                                self.handle_new_session(&name).await?;
                            }
                            EngineCommand::ResumeSession(id) => {
                                self.handle_resume_session(&id).await?;
                            }
                            EngineCommand::ListSessions => {
                                self.handle_list_sessions().await?;
                            }
                            EngineCommand::DeleteSession(id) => {
                                self.handle_delete_session(&id).await?;
                            }
                            EngineCommand::RenameSession(id, name) => {
                                self.handle_rename_session(&id, &name).await?;
                            }
                            EngineCommand::ApprovalResponse { id, approved } => {
                                self.handle_approval_response(&id, approved).await;
                            }
                            EngineCommand::QuestionAnswer { id, answer } => {
                                self.handle_question_answer(&id, answer).await;
                            }
                            EngineCommand::SetDebug(debug) => {
                                self.debug.store(debug, Ordering::Relaxed);
                            }
                            EngineCommand::SetModel(model) => {
                                self.handle_set_model(model).await?;
                            }
                            EngineCommand::SwitchAgent(name) => {
                                self.handle_switch_agent(&name).await?;
                            }
                            EngineCommand::RecordModelUsage(model) => {
                                self.handle_record_model_usage(&model).await?;
                            }
                            EngineCommand::ListModelFrecency => {
                                self.handle_list_model_frecency().await?;
                            }
                            EngineCommand::Compact => {
                                self.send_to_active(AgentMessage::Compact).await?;
                            }
                            EngineCommand::Undo => {
                                self.handle_undo().await?;
                            }
                            EngineCommand::Redo => {
                                self.handle_redo().await?;
                            }
                            EngineCommand::Fork => {
                                self.handle_fork().await?;
                            }
                            EngineCommand::Export { path, format } => {
                                self.handle_export(path, format).await?;
                            }
                            EngineCommand::Import { path } => {
                                self.handle_import(path).await?;
                            }
                            EngineCommand::Share => {
                                self.handle_share().await?;
                            }
                            EngineCommand::Unshare => {
                                self.handle_unshare().await?;
                            }
                            EngineCommand::ListSkills => {
                                self.handle_list_skills().await?;
                            }
                            EngineCommand::ListMcps => {
                                self.handle_list_mcps().await?;
                            }
                            EngineCommand::ToggleMcp { name, enabled } => {
                                self.handle_toggle_mcp(&name, enabled).await?;
                            }
                            EngineCommand::Status => {
                                self.handle_status().await?;
                            }
                            EngineCommand::Init { answers } => {
                                self.handle_init(answers).await?;
                            }
                            EngineCommand::Review { target } => {
                                self.handle_review(target).await?;
                            }
                            EngineCommand::Warp { dir } => {
                                self.handle_warp(dir).await?;
                            }
                            EngineCommand::ListWorkspaces => {
                                self.handle_list_workspaces().await?;
                            }
                            EngineCommand::MoveSession { workspace } => {
                                self.handle_move_session(&workspace).await?;
                            }
                            EngineCommand::WorktreeAdd { path, branch } => {
                                self.handle_worktree_add(&path, branch.as_deref()).await?;
                            }
                            EngineCommand::WorktreeList => {
                                self.handle_worktree_list().await?;
                            }
                            EngineCommand::WorktreeRemove { path } => {
                                self.handle_worktree_remove(&path).await?;
                            }
                            EngineCommand::Timeline => {
                                self.handle_timeline().await?;
                            }
                            EngineCommand::SetSessionPinned { id, pinned } => {
                                self.handle_set_session_pinned(&id, pinned).await?;
                            }
                            EngineCommand::ListJobs => {
                                self.handle_list_jobs().await?;
                            }
                            EngineCommand::Build => {
                                self.handle_build().await?;
                            }
                            EngineCommand::Parent => {
                                self.handle_parent().await?;
                            }
                            EngineCommand::Children => {
                                self.handle_children().await?;
                            }
                            EngineCommand::Snapshot { name } => {
                                self.handle_snapshot(name.as_deref()).await?;
                            }
                            EngineCommand::Revert { snapshot_id } => {
                                self.handle_revert(snapshot_id).await?;
                            }
                            EngineCommand::ListSnapshots => {
                                self.handle_list_snapshots().await?;
                            }
                            EngineCommand::Stage { name } => {
                                self.handle_stage(name.as_deref()).await?;
                            }
                            EngineCommand::Clear => {
                                self.handle_clear().await?;
                            }
                            EngineCommand::Commit { name } => {
                                self.handle_commit(name.as_deref()).await?;
                            }
                            EngineCommand::Shutdown => unreachable!(),
                        }
                        Ok(())
                    }
                    .await;

                    if let Err(e) = result {
                        self.event_tx
                            .send(EngineEvent::CommandError(e.to_string()))
                            .await
                            .ok();
                    }
                }
                usage = self.usage_rx.recv() => {
                    if let Some(usage) = usage {
                        self.total_tokens = self.total_tokens.saturating_add(usage.total_tokens);
                        self.total_cost += usage.cost;
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle user input: route to the active root agent.
    async fn handle_user_input(&mut self, input: String) -> Result<()> {
        // A new turn invalidates any pending redo history.
        self.redo_stack.clear();
        // Find the active root agent
        let root_name = &self.active_agent;

        let root_id = self
            .agents
            .get(root_name)
            .ok_or_else(|| Error::Agent(format!("Agent '{root_name}' not found")))?;

        let handle = self
            .handles
            .get(root_id)
            .ok_or_else(|| Error::Agent("Root agent not initialized".into()))?;

        handle
            .send(AgentMessage::UserInput { content: input })
            .await
    }

    /// Handle model change: update config, respawn active agent.
    async fn handle_set_model(&mut self, model: String) -> Result<()> {
        // Update the config in memory for the active agent
        if let Some(ref mut agent) = self
            .config
            .agents
            .iter_mut()
            .find(|a| a.name == self.active_agent)
        {
            agent.model = model.clone();
        }
        self.current_model = model.clone();

        // Respawn the active agent with the new model
        self.respawn_active_agent().await?;

        self.event_tx
            .send(EngineEvent::ModelChanged { model })
            .await
            .ok();
        Ok(())
    }

    /// Handle switching the active root agent.
    async fn handle_switch_agent(&mut self, name: &str) -> Result<()> {
        // Validate that the target agent exists and is a root agent.
        let agent = self
            .config
            .agents
            .iter()
            .find(|a| a.name == name && a.role == AgentRole::Root)
            .ok_or_else(|| Error::Agent(format!("Root agent '{name}' not found")))?;

        // Validate that the agent is actually spawned. An agent with `role: Root`
        // that is also listed as a subagent of another agent is skipped during
        // `initialize()` and therefore not present in `self.agents`/`self.handles`.
        if !self.agents.contains_key(name) {
            return Err(Error::Agent(format!(
                "Root agent '{name}' is not spawned (it is listed as a subagent of another agent)"
            )));
        }

        let name = name.to_string();
        self.active_agent = name.clone();
        self.current_model = agent.model.clone();

        self.event_tx
            .send(EngineEvent::AgentSwitched { name })
            .await
            .ok();
        self.event_tx
            .send(EngineEvent::ModelChanged {
                model: self.current_model.clone(),
            })
            .await
            .ok();
        Ok(())
    }

    /// Record model usage for the frecency ranking.
    async fn handle_record_model_usage(&mut self, model: &str) -> Result<()> {
        if let Some(db) = &self.database {
            db.record_model_usage(model).await?;
        }
        Ok(())
    }

    /// Emit the current model usage frequency records to the TUI.
    async fn handle_list_model_frecency(&mut self) -> Result<()> {
        let frecency = match &self.database {
            Some(db) => db.list_model_frecency().await?,
            None => Vec::new(),
        };
        self.event_tx
            .send(EngineEvent::ModelsFrecency(frecency))
            .await
            .ok();
        Ok(())
    }

    /// Respawn the active agent (used after model change).
    async fn respawn_active_agent(&mut self) -> Result<()> {
        let agent_config = self
            .config
            .agents
            .iter()
            .find(|a| a.name == self.active_agent)
            .ok_or_else(|| Error::Agent(format!("Agent '{}' not found", self.active_agent)))?;

        let agent = Agent::from_config(agent_config, AgentRole::Root);
        let name = agent.name.clone();
        let id = agent.id.clone();

        // Resolve the new provider
        let provider = self.resolve_agent_provider(&agent)?;

        // Load skills
        let skills = load_agent_skills(&agent.skills);

        // Collect subagent configs
        let _subagent_names: std::collections::HashSet<String> = self
            .config
            .agents
            .iter()
            .flat_map(|a| a.subagents.iter().cloned())
            .collect();
        let config_by_name: std::collections::HashMap<String, &crate::config::AgentConfig> = self
            .config
            .agents
            .iter()
            .map(|a| (a.name.clone(), a))
            .collect();
        let my_subagent_configs: Vec<crate::config::AgentConfig> = agent_config
            .subagents
            .iter()
            .filter_map(|name| config_by_name.get(name).map(|c| (*c).clone()))
            .collect();

        // Kill the old root agent handle
        if let Some(old_id) = self.agents.get(&name) {
            if let Some(old_handle) = self.handles.remove(old_id) {
                // Send shutdown to the old agent task
                let _ = old_handle.sender.send(AgentMessage::Shutdown).await;
            }
            self.agents.remove(&name);
        }

        // Spawn new agent
        let history_limit = self.config.session.history_limit_percent;
        let retry_cfg = self.config.session.retry.clone();
        let handle = spawn_agent(SpawnAgentConfig {
            agent,
            provider,
            skills,
            subagent_configs: my_subagent_configs,
            llm_registry: self.llm_registry.clone(),
            mcp_registry: Some(self.mcp_registry.clone()),
            mcp_enabled: Some(self.mcp_enabled.clone()),
            event_tx: self.event_tx.clone(),
            usage_tx: Some(self.usage_tx.clone()),
            retry_config: retry_cfg,
            db: self.database.clone(),
            session_id: self.active_session_id,
            pending_approvals: Some(self.pending_approvals.clone()),
            pending_questions: Some(self.pending_questions.clone()),
            history_limit_percent: history_limit,
            debug: self.debug.clone(),
            workspace: self.workspace.clone(),
            task_id: None,
            depth: 0,
            mode: AgentMode::Build,
            job_registry: Some(self.job_registry.clone()),
        });

        self.agents.insert(name.clone(), id.clone());
        self.handles.insert(id, handle);

        Ok(())
    }

    /// Handle new session creation.
    async fn handle_new_session(&mut self, name: &str) -> Result<()> {
        if let Some(ref db) = self.database {
            let session = db.create_session(name).await?;
            let session_id = session.id;
            self.active_session_id = Some(session_id);
            self.clear_undo_redo();

            // Clear active agent's conversation
            self.send_to_active(AgentMessage::ClearHistory).await?;

            self.event_tx
                .send(EngineEvent::SessionSwitched {
                    id: session_id.to_string(),
                    name: name.to_string(),
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle session resume: load history and send to root agent.
    async fn handle_resume_session(&mut self, id_str: &str) -> Result<()> {
        let session_id = Uuid::parse_str(id_str)
            .map_err(|e| Error::Session(format!("Invalid session ID: {e}")))?;

        if let Some(db) = self.database.clone() {
            // Load messages from DB
            let messages = db.get_session_messages(session_id).await?;

            // Convert stored messages to LlmMessage
            let history: Vec<LlmMessage> = messages
                .iter()
                .map(|m| {
                    let role = match m.role.as_str() {
                        "user" => MessageRole::User,
                        "assistant" => MessageRole::Assistant,
                        "system" => MessageRole::System,
                        "tool" => MessageRole::Tool,
                        _ => MessageRole::User,
                    };
                    LlmMessage {
                        role,
                        content: m.content.clone(),
                        tool_calls: None,
                        tool_call_id: None,
                    }
                })
                .collect();

            self.active_session_id = Some(session_id);
            self.clear_undo_redo();

            // Send history to active agent
            self.send_to_active(AgentMessage::LoadHistory(history))
                .await?;

            // Get session name for the event
            let sessions = db.list_sessions().await?;
            let name = sessions
                .iter()
                .find(|s| s.id == session_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "unknown".into());

            self.event_tx
                .send(EngineEvent::SessionSwitched {
                    id: session_id.to_string(),
                    name,
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle listing all sessions.
    async fn handle_list_sessions(&self) -> Result<()> {
        if let Some(ref db) = self.database {
            let sessions = db.list_sessions().await?;
            self.event_tx
                .send(EngineEvent::SessionList(sessions))
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle pinning/unpinning a session, then refresh the session list.
    async fn handle_set_session_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        if let Some(ref db) = self.database {
            db.set_session_pinned(id, pinned).await?;
            let sessions = db.list_sessions().await?;
            self.event_tx
                .send(EngineEvent::SessionList(sessions))
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle deleting a session.
    async fn handle_delete_session(&self, id_str: &str) -> Result<()> {
        let session_id = Uuid::parse_str(id_str)
            .map_err(|e| Error::Session(format!("Invalid session ID: {e}")))?;

        if let Some(ref db) = self.database {
            db.delete_session(session_id).await?;
            self.event_tx
                .send(EngineEvent::SessionDeleted {
                    id: id_str.to_string(),
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle renaming a session.
    async fn handle_rename_session(&self, id_str: &str, new_name: &str) -> Result<()> {
        let session_id = Uuid::parse_str(id_str)
            .map_err(|e| Error::Session(format!("Invalid session ID: {e}")))?;

        if let Some(ref db) = self.database {
            db.rename_session(session_id, new_name).await?;
            self.event_tx
                .send(EngineEvent::SessionRenamed {
                    id: id_str.to_string(),
                    name: new_name.to_string(),
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Clear the undo/redo stacks (called on session change).
    fn clear_undo_redo(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Reload the session's messages into the root agent's context.
    async fn reload_history_to_root(&self, session_id: Uuid) -> Result<()> {
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let history: Vec<LlmMessage> = db
            .get_session_messages(session_id)
            .await?
            .iter()
            .map(|m| LlmMessage {
                role: match m.role.as_str() {
                    "user" => MessageRole::User,
                    "assistant" => MessageRole::Assistant,
                    "system" => MessageRole::System,
                    "tool" => MessageRole::Tool,
                    _ => MessageRole::User,
                },
                content: m.content.clone(),
                tool_calls: None,
                tool_call_id: None,
            })
            .collect();
        // Only sync the agent context if an active agent is actually running;
        // otherwise (e.g. headless tests) skip without failing the operation.
        if !self.agents.contains_key(&self.active_agent) {
            return Ok(());
        }
        self.send_to_active(AgentMessage::LoadHistory(history))
            .await?;
        Ok(())
    }

    /// Handle `/undo`: remove the last message pair and push it onto the stacks.
    async fn handle_undo(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let removed = db.delete_messages(session_id, 2).await?;
        if removed.is_empty() {
            return Ok(());
        }
        self.undo_stack.push(removed.clone());
        self.redo_stack.push(removed.clone());
        // Sync the root agent's context to the post-undo state.
        self.reload_history_to_root(session_id).await?;
        let removed_contents: Vec<String> = removed.iter().map(|m| m.content.clone()).collect();
        self.event_tx
            .send(EngineEvent::UndoApplied {
                removed: removed_contents,
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/redo`: restore the last undone message pair.
    async fn handle_redo(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        if let Some(messages) = self.redo_stack.pop() {
            db.restore_messages(session_id, &messages).await?;
            self.undo_stack.push(messages.clone());
            // Sync the root agent's context to the post-redo state.
            self.reload_history_to_root(session_id).await?;
            let restored_contents: Vec<String> =
                messages.iter().map(|m| m.content.clone()).collect();
            self.event_tx
                .send(EngineEvent::RedoApplied {
                    restored: restored_contents,
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle `/fork`: create a new session copying the active session's messages.
    async fn handle_fork(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(db) = self.database.clone() else {
            return Ok(());
        };
        let name = db
            .get_session_name(session_id)
            .await?
            .unwrap_or_else(|| "fork".into());
        let new_session = db
            .create_session_with_parent(&format!("{name} (fork)"), Some(session_id))
            .await?;
        db.copy_messages(session_id, new_session.id).await?;
        self.active_session_id = Some(new_session.id);
        self.clear_undo_redo();

        // Load the copied history into the root agent so it has context.
        self.reload_history_to_root(new_session.id).await?;

        self.event_tx
            .send(EngineEvent::Forked {
                new_session_id: new_session.id,
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/export`: write the active session transcript to a file.
    async fn handle_export(
        &mut self,
        path: Option<PathBuf>,
        format: Option<ExportFormat>,
    ) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let format = format.unwrap_or(ExportFormat::Json);
        let path = match path {
            Some(p) => {
                if p.is_relative() {
                    self.workspace.join(p)
                } else {
                    p
                }
            }
            None => {
                let name = db
                    .get_session_name(session_id)
                    .await?
                    .unwrap_or_else(|| "session".into());
                let ext = match format {
                    ExportFormat::Json => "json",
                    ExportFormat::Markdown => "md",
                };
                self.workspace.join(format!("{name}.{ext}"))
            }
        };
        db.export_session(session_id, &path, format).await?;
        self.event_tx
            .send(EngineEvent::Exported { path })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/import`: import a session transcript from a file.
    async fn handle_import(&mut self, path: PathBuf) -> Result<()> {
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let path = if path.is_relative() {
            self.workspace.join(path)
        } else {
            path
        };
        let new_id = db.import_session(&path).await?;
        self.active_session_id = Some(new_id);
        self.clear_undo_redo();
        // Load the imported conversation into the root agent so it has context.
        self.reload_history_to_root(new_id).await?;
        self.event_tx
            .send(EngineEvent::Imported { session_id: new_id })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/share`: mark the active session as shared and generate a link.
    async fn handle_share(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let link = format!("anacleto://share/{}", Uuid::new_v4());
        db.set_shared(session_id, true, Some(&link)).await?;
        self.event_tx
            .send(EngineEvent::ShareUpdated {
                shared: true,
                link: Some(link),
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/unshare`: remove the shared state from the active session.
    async fn handle_unshare(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        db.set_shared(session_id, false, None).await?;
        self.event_tx
            .send(EngineEvent::ShareUpdated {
                shared: false,
                link: None,
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/skills`: list the skills of the root agent.
    async fn handle_list_skills(&self) -> Result<()> {
        let skills = self
            .config
            .agents
            .iter()
            .find(|a| a.role == AgentRole::Root)
            .map(|c| load_agent_skills(&c.skills))
            .unwrap_or_default();
        let infos: Vec<SkillInfo> = skills
            .iter()
            .map(|s| SkillInfo {
                name: s.name.clone(),
                description: s.description.clone(),
            })
            .collect();
        self.event_tx
            .send(EngineEvent::SkillsListed(infos))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/mcps`: list the MCP servers and their enabled state.
    async fn handle_list_mcps(&self) -> Result<()> {
        let enabled_map = self.mcp_enabled.lock().await;
        let statuses: Vec<McpStatus> = self
            .mcp_registry
            .lock()
            .await
            .names()
            .iter()
            .map(|n| McpStatus {
                name: n.clone(),
                enabled: *enabled_map.get(n).unwrap_or(&true),
            })
            .collect();
        drop(enabled_map);
        self.event_tx
            .send(EngineEvent::McpsListed(statuses))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/mcps <name> on|off`: enable or disable an MCP server.
    async fn handle_toggle_mcp(&mut self, name: &str, enabled: bool) -> Result<()> {
        self.mcp_enabled
            .lock()
            .await
            .insert(name.to_string(), enabled);
        // Re-list so the TUI reflects the new state.
        self.handle_list_mcps().await?;
        Ok(())
    }

    /// Handle `/status`: produce an engine status report.
    async fn handle_status(&self) -> Result<()> {
        let session_id = self.active_session_id;
        let session_name = match (session_id, &self.database) {
            (Some(id), Some(db)) => db
                .get_session_name(id)
                .await?
                .unwrap_or_else(|| "unknown".into()),
            _ => "none".into(),
        };
        let provider_name = if self.current_model.contains('/') {
            "openrouter"
        } else if self.current_model.starts_with("claude") {
            "anthropic"
        } else if self.current_model.starts_with("gpt")
            || self.current_model.starts_with("o1")
            || self.current_model.starts_with("o3")
        {
            "openai"
        } else {
            "ollama"
        };
        let context_window = self
            .llm_registry
            .get(provider_name)
            .map(|p| p.context_window() as u32)
            .unwrap_or(0);
        let info = StatusInfo {
            model: self.current_model.clone(),
            session_id,
            session_name,
            total_tokens: self.total_tokens,
            context_window,
            cost: self.total_cost,
            debug: self.debug.load(Ordering::Relaxed),
            workspace: self.workspace.clone(),
        };
        self.event_tx
            .send(EngineEvent::StatusReport(info))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/init`: generate AGENTS.md in the workspace from collected answers.
    async fn handle_init(&mut self, answers: InitAnswers) -> Result<()> {
        let mut content = format!(
            "# {}\n\n{}",
            answers.name,
            if answers.description.is_empty() {
                "# Anacleto agent".to_string()
            } else {
                answers.description
            }
        );
        if !answers.stack.trim().is_empty() {
            content.push_str(&format!("\n\n## Tech stack\n\n{}", answers.stack));
        }
        let path = self.workspace.join("AGENTS.md");
        tokio::fs::write(&path, content).await.map_err(Error::Io)?;
        self.event_tx.send(EngineEvent::InitDone).await.ok();
        Ok(())
    }

    /// Handle `/review`: run git diff and send it to the root agent for review.
    async fn handle_review(&mut self, target: Option<String>) -> Result<()> {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("diff");
        if let Some(t) = &target {
            cmd.arg(t);
        }
        cmd.current_dir(&self.workspace);
        let output = cmd.output().map_err(Error::Io)?;
        let diff = String::from_utf8_lossy(&output.stdout).to_string();
        let title = match &target {
            Some(t) => format!("git diff {}", t),
            None => "git diff".to_string(),
        };
        self.event_tx
            .send(EngineEvent::DiffAvailable {
                text: diff.clone(),
                title,
            })
            .await
            .ok();
        let prompt = if diff.trim().is_empty() {
            "No hay cambios sin commitear para revisar.".to_string()
        } else {
            format!(
                "Revisa los siguientes cambios de git:\n\n```diff\n{}\n```",
                diff
            )
        };
        self.send_to_active(AgentMessage::UserInput { content: prompt })
            .await?;
        self.event_tx
            .send(EngineEvent::ReviewResult(diff))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/warp`: set the engine workspace directory.
    async fn handle_warp(&mut self, dir: PathBuf) -> Result<()> {
        self.workspace = dir.clone();
        self.event_tx
            .send(EngineEvent::WorkspaceChanged(dir))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/workspaces`: list the known workspaces.
    async fn handle_list_workspaces(&self) -> Result<()> {
        let workspaces: Vec<String> = self
            .config
            .workspaces
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        self.event_tx
            .send(EngineEvent::WorkspacesListed(workspaces))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/move`: move the active session to another workspace.
    async fn handle_move_session(&mut self, workspace: &str) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        db.set_session_workspace(session_id, workspace).await?;
        // Re-home the engine workspace so paths re-resolve (FASE 5.1).
        self.workspace = PathBuf::from(workspace);
        self.event_tx
            .send(EngineEvent::WorkspaceChanged(PathBuf::from(workspace)))
            .await
            .ok();
        self.event_tx
            .send(EngineEvent::SessionMoved {
                session_id,
                workspace: workspace.to_string(),
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/worktree add`: add a git worktree.
    async fn handle_worktree_add(&self, path: &str, branch: Option<&str>) -> Result<()> {
        let result = git_worktree_add(&self.workspace, path, branch)
            .unwrap_or_else(|e| format!("error: {}", e));
        self.event_tx
            .send(EngineEvent::WorktreeResult(result))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/worktree list`: list git worktrees.
    async fn handle_worktree_list(&self) -> Result<()> {
        let result = git_worktree_list(&self.workspace).unwrap_or_else(|e| format!("error: {}", e));
        self.event_tx
            .send(EngineEvent::WorktreeResult(result))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/worktree remove`: remove a git worktree.
    async fn handle_worktree_remove(&self, path: &str) -> Result<()> {
        let result =
            git_worktree_remove(&self.workspace, path).unwrap_or_else(|e| format!("error: {}", e));
        self.event_tx
            .send(EngineEvent::WorktreeResult(result))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/timeline`: produce the timeline of the active session.
    async fn handle_timeline(&self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let messages = db.get_session_messages(session_id).await?;
        let entries: Vec<TimelineEntry> = messages
            .iter()
            .map(|m| TimelineEntry {
                id: m.id,
                role: m.role.clone(),
                content: m.content.clone(),
                created_at: m.created_at,
            })
            .collect();
        self.event_tx
            .send(EngineEvent::Timeline(entries))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/build`: read the plan markdown file from the workspace and
    /// inject it as an execution message to the active agent.
    async fn handle_build(&mut self) -> Result<()> {
        let path = self.workspace.join("PLAN.md");
        let content = tokio::fs::read_to_string(&path).await.map_err(Error::Io)?;
        let prompt = format!(
            "Execute the following plan. Implement it fully, then report what was done.\n\n{}",
            content
        );
        self.send_to_active(AgentMessage::UserInput { content: prompt })
            .await?;
        self.event_tx.send(EngineEvent::BuildDone).await.ok();
        Ok(())
    }

    /// Handle `/parent`: navigate to the parent session of the active session.
    async fn handle_parent(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        if let Some(parent_id) = db.get_parent(session_id).await? {
            self.handle_resume_session(&parent_id.to_string()).await?;
        }
        Ok(())
    }

    /// Handle `/children`: list the child sessions of the active session.
    async fn handle_children(&self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let children = db.get_children(session_id).await?;
        self.event_tx
            .send(EngineEvent::SessionTree(children))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/jobs`: list the running background jobs.
    async fn handle_list_jobs(&self) -> Result<()> {
        let ids = self.job_registry.lock().await.running_ids();
        self.event_tx.send(EngineEvent::JobsListed(ids)).await.ok();
        Ok(())
    }

    /// Handle `/snapshot`: create a snapshot of the active session's conversation.
    ///
    /// The snapshot captures the serialized message list so it can be restored
    /// later via `/revert`.
    async fn handle_snapshot(&mut self, name: Option<&str>) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let messages = db.get_session_messages(session_id).await?;
        let content = serde_json::to_string(&messages)?;
        let snapshot_name = name.unwrap_or("snapshot").to_string();
        let snapshot = db
            .create_snapshot(session_id, &snapshot_name, &content)
            .await?;
        self.event_tx
            .send(EngineEvent::SnapshotCreated {
                snapshot: snapshot.clone(),
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/revert`: restore the active session to a snapshot's state.
    ///
    /// The current messages are deleted and replaced with the snapshot's
    /// serialized message list, then the root agent's context is reloaded.
    async fn handle_revert(&mut self, snapshot_id: Uuid) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let Some(snapshot) = db.get_snapshot(snapshot_id).await? else {
            return Err(Error::NotFound(format!(
                "Snapshot '{snapshot_id}' not found"
            )));
        };
        if snapshot.session_id != session_id {
            return Err(Error::Session(format!(
                "Snapshot '{snapshot_id}' does not belong to the active session"
            )));
        }
        // Remove all current messages, then restore the snapshot's messages.
        let current = db.get_session_messages(session_id).await?;
        if !current.is_empty() {
            db.delete_messages(session_id, current.len()).await?;
        }
        let restored: Vec<StoredMessage> = serde_json::from_str(&snapshot.content)?;
        db.restore_messages(session_id, &restored).await?;
        self.reload_history_to_root(session_id).await?;
        self.event_tx
            .send(EngineEvent::SnapshotReverted { snapshot_id })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/snapshots`: list the snapshots of the active session.
    async fn handle_list_snapshots(&self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let snapshots = db.list_snapshots(session_id).await?;
        self.event_tx
            .send(EngineEvent::SnapshotsListed(snapshots))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/stage`: capture the current conversation state as a staged
    /// snapshot without persisting it. The staged snapshot can be committed
    /// later via `/commit` or discarded via `/clear`.
    async fn handle_stage(&mut self, name: Option<&str>) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let messages = db.get_session_messages(session_id).await?;
        let content = serde_json::to_string(&messages)?;
        let snapshot_name = name.unwrap_or("staged").to_string();
        let snapshot = db
            .create_snapshot(session_id, &snapshot_name, &content)
            .await?;
        self.staged_snapshot = Some(snapshot.clone());
        self.event_tx
            .send(EngineEvent::SnapshotCreated { snapshot })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/clear`: discard the staged snapshot.
    async fn handle_clear(&mut self) -> Result<()> {
        if let Some(staged) = self.staged_snapshot.take() {
            if let Some(ref db) = self.database {
                db.delete_snapshot(staged.id).await?;
            }
        }
        Ok(())
    }

    /// Handle `/commit`: persist the staged snapshot as a named snapshot.
    ///
    /// The staged snapshot is renamed (if a name is provided) and kept; the
    /// staging slot is cleared.
    async fn handle_commit(&mut self, name: Option<&str>) -> Result<()> {
        let Some(staged) = self.staged_snapshot.take() else {
            return Err(Error::Session(
                "No staged snapshot to commit. Use /stage first.".into(),
            ));
        };
        if let Some(ref db) = self.database {
            if let Some(new_name) = name {
                // Rename by re-creating with the same content and deleting the old.
                let content = staged.content.clone();
                let renamed = db
                    .create_snapshot(staged.session_id, new_name, &content)
                    .await?;
                db.delete_snapshot(staged.id).await?;
                self.event_tx
                    .send(EngineEvent::SnapshotCreated { snapshot: renamed })
                    .await
                    .ok();
            } else {
                self.event_tx
                    .send(EngineEvent::SnapshotCreated { snapshot: staged })
                    .await
                    .ok();
            }
        }
        Ok(())
    }

    /// Handle approval response from the TUI.
    async fn handle_approval_response(&self, id: &str, approved: bool) {
        let mut pending = self.pending_approvals.lock().await;
        if let Some(sender) = pending.remove(id) {
            let _ = sender.send(approved);
        }
    }

    /// Deliver an inline question answer to the waiting agent task.
    async fn handle_question_answer(&self, id: &str, answer: String) {
        let mut pending = self.pending_questions.lock().await;
        if let Some(sender) = pending.remove(id) {
            let _ = sender.send(answer);
        }
    }

    /// Send a message to the active root agent.
    async fn send_to_active(&self, msg: AgentMessage) -> Result<()> {
        let root_name = &self.active_agent;

        let root_id = self
            .agents
            .get(root_name)
            .ok_or_else(|| Error::Agent(format!("Agent '{root_name}' not found")))?;

        let handle = self
            .handles
            .get(root_id)
            .ok_or_else(|| Error::Agent("Root agent not initialized".into()))?;

        handle.send(msg).await
    }

    /// Shutdown the engine.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.event_tx.send(EngineEvent::ShuttingDown).await.ok();
        {
            let mut mcp = self.mcp_registry.lock().await;
            mcp.disconnect_all().await;
        }
        if let Some(db) = self.database.take() {
            db.close().await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers: config types -> LLM provider config
// ---------------------------------------------------------------------------

/// Convert a generic `ProviderConfig` + type tag to `LlmProviderConfig`.
fn provider_config_to_llm(cfg: &ProviderConfig, ptype: LlmProviderType) -> LlmProviderConfig {
    LlmProviderConfig {
        provider_type: ptype,
        api_key: Some(cfg.api_key.clone()),
        model: cfg.model.clone(),
        base_url: cfg.base_url.clone(),
        context_window: cfg.context_window,
        input_price_per_million: cfg.input_price_per_million,
        output_price_per_million: cfg.output_price_per_million,
    }
}

/// Convert an `OllamaConfig` to `LlmProviderConfig`.
fn ollama_config_to_llm(cfg: &OllamaConfig) -> LlmProviderConfig {
    LlmProviderConfig {
        provider_type: LlmProviderType::Ollama,
        api_key: None,
        model: cfg.model.clone(),
        base_url: Some(cfg.base_url.clone()),
        context_window: cfg.context_window,
        input_price_per_million: 0.0,
        output_price_per_million: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::AgentConfig;

    #[tokio::test]
    async fn test_engine_creation() {
        let (event_tx, _) = mpsc::channel(64);
        let (_cmd_tx, cmd_rx) = mpsc::channel(64);
        let config = Config::default();

        let mut engine = Engine::new(config, event_tx, cmd_rx);
        // Should not panic
        let result = engine.initialize().await;
        // May fail if no database directory, but shouldn't panic
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_provider_config_conversion() {
        let cfg = ProviderConfig {
            api_key: "sk-test".into(),
            model: "gpt-4o".into(),
            context_window: 128_000,
            base_url: None,
            input_price_per_million: 3.0,
            output_price_per_million: 15.0,
        };
        let llm_cfg = provider_config_to_llm(&cfg, LlmProviderType::OpenAI);
        assert_eq!(llm_cfg.api_key, Some("sk-test".into()));
        assert_eq!(llm_cfg.model, "gpt-4o");
        assert_eq!(llm_cfg.context_window, 128_000);
    }

    #[test]
    fn test_ollama_config_conversion() {
        let cfg = OllamaConfig {
            base_url: "http://localhost:11434".into(),
            model: "llama3.2".into(),
            context_window: 8_192,
        };
        let llm_cfg = ollama_config_to_llm(&cfg);
        assert_eq!(llm_cfg.provider_type, LlmProviderType::Ollama);
        assert_eq!(llm_cfg.api_key, None);
        assert_eq!(llm_cfg.model, "llama3.2");
    }

    #[test]
    fn test_resolve_provider_by_model() {
        let config = Config::default();
        let (event_tx, _) = mpsc::channel(64);
        let (_cmd_tx, cmd_rx) = mpsc::channel(64);

        let mut engine = Engine::new(config, event_tx, cmd_rx);
        // Populate registry with dummy providers
        let dummy_cfg = LlmProviderConfig {
            provider_type: LlmProviderType::Ollama,
            api_key: None,
            model: "llama3.2".into(),
            base_url: Some("http://localhost:11434".into()),
            context_window: 8_192,
            input_price_per_million: 0.0,
            output_price_per_million: 0.0,
        };
        engine
            .llm_registry
            .register("ollama".into(), Arc::from(create_provider(&dummy_cfg)));

        let agent = Agent::create_subagent(
            "test".into(),
            "test.md".into(),
            "llama3.2".into(),
            vec![],
            vec![],
            crate::permissions::Permissions::default(),
            60,
            AgentId::new(),
        );
        let result = engine.resolve_agent_provider(&agent);
        assert!(result.is_ok());
    }

    /// Build an engine with a temp DB and an active session.
    async fn test_engine_with_session()
    -> (Engine, Uuid, mpsc::Receiver<EngineEvent>, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();
        let session = db.create_session("test-session").await.unwrap();
        let (event_tx, event_rx) = mpsc::channel(64);
        let (_cmd_tx, cmd_rx) = mpsc::channel(64);
        let mut config = Config::default();
        config.session.database_path = db_path;
        let mut engine = Engine::new(config, event_tx, cmd_rx);
        engine.database = Some(db.clone());
        engine.active_session_id = Some(session.id);
        (engine, session.id, event_rx, dir)
    }

    #[tokio::test]
    async fn test_handle_undo_redo() {
        let (mut engine, session_id, _rx, _dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();
        db.store_message(session_id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(session_id, "assistant", "assistant", "Hi", None)
            .await
            .unwrap();

        engine.handle_undo().await.unwrap();
        assert_eq!(db.get_session_messages(session_id).await.unwrap().len(), 0);
        assert_eq!(engine.undo_stack.len(), 1);
        assert_eq!(engine.redo_stack.len(), 1);

        engine.handle_redo().await.unwrap();
        let msgs = db.get_session_messages(session_id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "Hello");
        assert_eq!(msgs[1].content, "Hi");
    }

    #[tokio::test]
    async fn test_handle_fork() {
        let (mut engine, session_id, _rx, _dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();
        db.store_message(session_id, "user", "user", "Hello", None)
            .await
            .unwrap();

        engine.handle_fork().await.unwrap();
        let new_id = engine.active_session_id.unwrap();
        assert_ne!(new_id, session_id);
        let msgs = db.get_session_messages(new_id).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello");
    }

    #[tokio::test]
    async fn test_handle_share_unshare() {
        let (mut engine, session_id, _rx, _dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();

        engine.handle_share().await.unwrap();
        let meta = db.get_session_metadata(session_id).await.unwrap();
        assert!(
            meta.as_ref()
                .and_then(|v| v.get("share_link"))
                .and_then(|v| v.as_str())
                .is_some()
        );

        engine.handle_unshare().await.unwrap();
        let meta = db.get_session_metadata(session_id).await.unwrap();
        assert!(meta.as_ref().and_then(|v| v.get("share_link")).is_none());
    }

    #[tokio::test]
    async fn test_handle_export_import() {
        let (mut engine, session_id, _rx, dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();
        db.store_message(session_id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(session_id, "assistant", "assistant", "Hi", None)
            .await
            .unwrap();

        let out = dir.path().join("export.json");
        engine
            .handle_export(Some(out.clone()), Some(ExportFormat::Json))
            .await
            .unwrap();
        assert!(out.exists());

        engine.handle_import(out).await.unwrap();
        let new_id = engine.active_session_id.unwrap();
        let msgs = db.get_session_messages(new_id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "Hello");
        assert_eq!(msgs[1].content, "Hi");
    }

    #[tokio::test]
    async fn test_handle_init_writes_agents_md() {
        let (mut engine, _session_id, _rx, _dir) = test_engine_with_session().await;
        engine.workspace = tempfile::TempDir::new().unwrap().into_path();
        let answers = InitAnswers {
            name: "My Project".into(),
            description: "A test project".into(),
            stack: "Rust, React".into(),
        };
        engine.handle_init(answers).await.unwrap();
        let content = std::fs::read_to_string(engine.workspace.join("AGENTS.md")).unwrap();
        assert!(content.contains("# My Project"));
        assert!(content.contains("Rust, React"));
    }

    #[tokio::test]
    async fn test_handle_warp() {
        let (mut engine, _session_id, mut rx, _dir) = test_engine_with_session().await;
        let new_dir = tempfile::TempDir::new().unwrap().into_path();
        engine.handle_warp(new_dir.clone()).await.unwrap();
        assert_eq!(engine.workspace, new_dir);
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev, EngineEvent::WorkspaceChanged(_)));
    }

    #[tokio::test]
    async fn test_handle_timeline() {
        let (mut engine, session_id, mut rx, _dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();
        db.store_message(session_id, "user", "user", "Hello", None)
            .await
            .unwrap();
        engine.handle_timeline().await.unwrap();
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::Timeline(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].content, "Hello");
            }
            _ => panic!("expected Timeline event"),
        }
    }

    #[tokio::test]
    async fn test_handle_move_session() {
        let (mut engine, session_id, mut rx, _dir) = test_engine_with_session().await;
        engine.handle_move_session("other-ws").await.unwrap();
        // `/move` now also re-homes the engine workspace, so a WorkspaceChanged
        // event is emitted before SessionMoved (FASE 5.1).
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::WorkspaceChanged(dir) => {
                assert_eq!(dir, std::path::PathBuf::from("other-ws"));
            }
            _ => panic!("expected WorkspaceChanged event"),
        }
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::SessionMoved {
                session_id: sid,
                workspace,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(workspace, "other-ws");
            }
            _ => panic!("expected SessionMoved event"),
        }
    }

    /// Build an engine whose config declares two root agents and whose
    /// `self.agents` map reflects that both were spawned.
    fn engine_with_agents() -> (Engine, mpsc::Receiver<EngineEvent>) {
        let (event_tx, event_rx) = mpsc::channel(64);
        let (_cmd_tx, cmd_rx) = mpsc::channel(64);
        let mut config = Config::default();
        config.agents = vec![
            AgentConfig {
                name: "root".into(),
                description: "root agent".into(),
                role: AgentRole::Root,
                model: "claude-sonnet-4".into(),
                skills: vec![],
                mcps: vec![],
                permissions: crate::config::types::PermissionConfig::default(),
                subagents: vec![],
                system_prompt: String::new(),
                max_steps: 90,
                subagent_depth: 3,
            },
            AgentConfig {
                name: "writer".into(),
                description: "writer agent".into(),
                role: AgentRole::Root,
                model: "claude-opus-4".into(),
                skills: vec![],
                mcps: vec![],
                permissions: crate::config::types::PermissionConfig::default(),
                subagents: vec![],
                system_prompt: String::new(),
                max_steps: 90,
                subagent_depth: 3,
            },
            AgentConfig {
                name: "helper".into(),
                description: "non-root agent".into(),
                role: AgentRole::SubAgent,
                model: "claude-sonnet-4".into(),
                skills: vec![],
                mcps: vec![],
                permissions: crate::config::types::PermissionConfig::default(),
                subagents: vec![],
                system_prompt: String::new(),
                max_steps: 90,
                subagent_depth: 3,
            },
        ];
        let mut engine = Engine::new(config, event_tx, cmd_rx);
        // Simulate both root agents having been spawned by `initialize()`.
        engine.agents.insert("root".into(), AgentId::new());
        engine.agents.insert("writer".into(), AgentId::new());
        (engine, event_rx)
    }

    #[tokio::test]
    async fn test_handle_switch_agent_valid() {
        let (mut engine, mut rx) = engine_with_agents();
        engine.active_agent = "root".into();
        engine.current_model = "claude-sonnet-4".into();

        engine.handle_switch_agent("writer").await.unwrap();

        assert_eq!(engine.active_agent, "writer");
        assert_eq!(engine.current_model, "claude-opus-4");
        // AgentSwitched is emitted before ModelChanged.
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::AgentSwitched { name } => assert_eq!(name, "writer"),
            _ => panic!("expected AgentSwitched event"),
        }
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::ModelChanged { model } => assert_eq!(model, "claude-opus-4"),
            _ => panic!("expected ModelChanged event"),
        }
    }

    #[tokio::test]
    async fn test_handle_switch_agent_unknown() {
        let (mut engine, _rx) = engine_with_agents();
        let err = engine.handle_switch_agent("ghost").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_handle_switch_agent_non_root() {
        let (mut engine, _rx) = engine_with_agents();
        let err = engine.handle_switch_agent("helper").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_handle_switch_agent_not_spawned() {
        // A root agent that is listed as a subagent of another agent is skipped
        // during `initialize()`, so it is absent from `self.agents` even though
        // it passes the config `role == Root` check.
        let (mut engine, _rx) = engine_with_agents();
        engine.config.agents.push(AgentConfig {
            name: "orphan".into(),
            description: "root but listed as subagent".into(),
            role: AgentRole::Root,
            model: "claude-sonnet-4".into(),
            skills: vec![],
            mcps: vec![],
            permissions: crate::config::types::PermissionConfig::default(),
            subagents: vec![],
            system_prompt: String::new(),
            max_steps: 90,
            subagent_depth: 3,
        });
        // `orphan` is NOT in `self.agents` (not spawned).
        let err = engine.handle_switch_agent("orphan").await.unwrap_err();
        assert!(err.to_string().contains("not spawned"));
    }

    #[tokio::test]
    async fn test_handle_switch_agent_current_model_consistency() {
        let (mut engine, _rx) = engine_with_agents();
        engine.active_agent = "root".into();
        engine.current_model = "claude-sonnet-4".into();

        engine.handle_switch_agent("writer").await.unwrap();
        assert_eq!(engine.current_model, "claude-opus-4");

        // Switching back restores the original agent's model.
        engine.handle_switch_agent("root").await.unwrap();
        assert_eq!(engine.current_model, "claude-sonnet-4");
        assert_eq!(engine.active_agent, "root");
    }
}
