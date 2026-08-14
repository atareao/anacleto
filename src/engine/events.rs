//! Event and command types shared between the engine and the TUI.
//!
//! These types are defined here (rather than in `orchestrator`) so the engine
//! core, session handlers and command handlers can all reference them without
//! creating a dependency cycle. They are re-exported from `orchestrator` for
//! backwards compatibility with the rest of the crate.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use crate::agent::types::{AgentId, AgentRole, AgentStatus, TaskMode};
use crate::config::types::ToolSettings;
use crate::db::models::{SessionSummary, Snapshot};

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
    /// Thinking/reasoning chunk from an agent's LLM response.
    AgentThinkingChunk {
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
        /// Name of the configured subagent type (e.g. "reviewer"), or `None`
        /// for a dynamic/generic subagent.
        agent_type: Option<String>,
        /// Execution mode of the subagent (Foreground/Background).
        mode: TaskMode,
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
        /// Total tokens (prompt + completion) for this response.
        total_tokens: u32,
        /// Prompt tokens sent to the LLM (proxy for current conversation size).
        prompt_tokens: u32,
        /// Context window of the provider.
        context_window: u32,
        /// Cost estimated from per-million-token prices.
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
    /// Tool settings for the current agent (fired once on agent start).
    ToolSettingsUpdated(HashMap<String, ToolSettings>),
    /// The model for the root agent changed.
    ModelChanged { model: String },
    /// The active root agent changed (via `/agent`).
    AgentSwitched { name: String },
    /// The conversation context was compacted.
    ConversationCompacted {
        agent_id: AgentId,
        agent_name: String,
        /// Estimated token count of the conversation buffer after compaction.
        tokens: u32,
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
    /// A hook was executed (fire-and-forget).
    HookExecuted {
        /// The hook point name (e.g. "AfterApply").
        point: String,
        /// The command that was executed.
        command: String,
        /// Whether the hook exited successfully (exit code 0).
        success: bool,
        /// Captured stdout/stderr (truncated).
        output: String,
    },
    /// Configuration was reloaded from disk (triggered by SIGHUP).
    ConfigReloaded,
    /// Skills have been re-discovered on disk (triggered by `ScanSkills` command).
    SkillsDiscovered {
        /// List of all discovered skill names.
        skills: Vec<String>,
    },
    /// Local token estimate emitted after conversation modifications.
    /// Used by the TUI to show the same metric that compaction uses.
    LocalTokenEstimate {
        /// Estimated token count from conversation_tokens().
        tokens: usize,
    },
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
    /// Reload configuration from disk (triggered by SIGHUP).
    ReloadConfig,
    /// Re-scan skill directories and reload the skill registry.
    ScanSkills,
    /// Update the configuration of an agent/subagent (skills, mcps, subagents).
    UpdateAgentConfig {
        name: String,
        skills: Vec<String>,
        mcps: Vec<String>,
        /// Only for root agents: the list of configured subagents.
        subagents: Option<Vec<String>>,
    },
    /// Reload the active agent: respawn with fresh config, skills, plugins, and system prompt.
    ReloadAgent,
    /// Emergency stop: cancel all in-flight agent activity.
    StopAgent,
    /// Shutdown the engine.
    Shutdown,
}
