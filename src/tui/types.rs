//! Shared types for the TUI: focus windows, agent info, approval/question
//! dialogs, the `/init` flow, and the built-in slash command list.

use crate::agent::types::{AgentId, AgentRole, AgentStatus, TaskMode};

/// All slash commands with a short description, used by the fuzzy command
/// palette and Tab autocomplete.
pub(crate) const BUILTIN_COMMANDS: &[(&str, &str)] = &[
    ("/help", "Show help"),
    ("/h", "Show help (alias)"),
    ("/sessions", "List sessions"),
    ("/s", "List sessions (alias)"),
    ("/new", "Start a new session"),
    ("/resume", "Resume a session"),
    ("/r", "Resume a session (alias)"),
    ("/delete", "Delete a session"),
    ("/d", "Delete a session (alias)"),
    ("/rename", "Rename a session"),
    ("/reload", "Reload the active agent (config + skills)"),
    ("/rl", "Reload the active agent (alias)"),
    ("/agents", "List agents"),
    ("/a", "List agents (alias)"),
    ("/agent", "Switch active agent"),
    ("/subagents", "List subagents"),
    ("/sa", "List subagents (alias)"),
    ("/copy", "Copy chat to clipboard"),
    ("/export-editor", "Export chat to external editor"),
    ("/ee", "Export chat to external editor (alias)"),
    ("/compact", "Compact conversation context"),
    ("/c", "Compact conversation context (alias)"),
    ("/debug", "Toggle debug mode"),
    ("/models", "List models"),
    ("/exit", "Exit"),
    ("/quit", "Exit (alias)"),
    // ── OpenCode-style slash commands ────────────────────────────────
    ("/undo", "Undo last message pair"),
    ("/redo", "Redo last undone message pair"),
    ("/fork", "Fork the active session"),
    ("/export", "Export session transcript to file"),
    ("/import", "Import a session transcript from file"),
    ("/share", "Share the active session"),
    ("/unshare", "Unshare the active session"),
    ("/skills", "List skills of the active agent"),
    ("/mcps", "List and toggle MCP servers"),
    ("/status", "Show engine status"),
    ("/init", "Guided AGENTS.md setup"),
    ("/review", "Review git changes"),
    ("/warp", "Set the working directory"),
    ("/workspaces", "List workspaces"),
    ("/move", "Move session to another workspace"),
    ("/worktree", "Manage git worktrees (add|list|remove)"),
    ("/timeline", "Show session timeline"),
    ("/themes", "Change color theme"),
    ("/timestamps", "Toggle timestamps"),
    ("/thinking", "Toggle thinking display"),
    ("/stash", "Stash the current prompt"),
    ("/editor", "Open external editor"),
    // ── FASE 1 y 2: build, jobs y snapshots ─────────────────────────
    ("/build", "Hand off the plan to build mode"),
    ("/jobs", "List running background jobs"),
    ("/parent", "Navigate to the parent session"),
    ("/children", "List child sessions"),
    ("/snapshot", "Create a snapshot of the session"),
    ("/revert", "Revert the session to a snapshot"),
    ("/stage", "Stage the conversation as a pending snapshot"),
    ("/clear", "Clear the staged snapshot"),
    ("/commit", "Commit the staged snapshot"),
];

#[derive(Debug, Clone)]
pub(crate) struct AgentInfo {
    pub(crate) id: AgentId,
    pub(crate) name: String,
    pub(crate) role: AgentRole,
    pub(crate) status: AgentStatus,
    pub(crate) skills: Vec<String>,
    pub(crate) mcps: Vec<String>,
    pub(crate) model: String,
    pub(crate) parent_id: Option<AgentId>,
    /// Number of child subagents (only for Root agents)
    pub(crate) subagent_count: usize,
    /// Name of the configured subagent type (e.g. "reviewer"), or `None` for a
    /// dynamic/generic subagent. Root agents have no type.
    pub(crate) agent_type: Option<String>,
    /// Execution mode (Foreground/Background). Root agents have no mode.
    pub(crate) mode: Option<TaskMode>,
}

/// A pending human approval request.
#[derive(Debug, Clone)]
pub(crate) struct ApprovalRequest {
    pub(crate) id: String,
    pub(crate) operation: String,
}

/// State for an inline question dialog (`/question` tool).
pub(crate) struct QuestionState {
    /// Question id (matches the engine's pending_questions key).
    pub(crate) id: String,
    /// The question text.
    pub(crate) question: String,
    /// Optional multiple-choice options.
    pub(crate) options: Vec<String>,
    /// Optional recommended default answer.
    pub(crate) recommended: Option<String>,
    /// Index of the currently selected option (if options present).
    pub(crate) selected: usize,
    /// Free-text answer being typed.
    pub(crate) answer_input: String,
}

/// State for the interactive `/init` flow (sequential prompts).
pub(crate) struct InitFlow {
    /// Current prompt step: 0 = name, 1 = description, 2 = stack.
    pub(crate) step: usize,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) stack: String,
}

impl InitFlow {
    pub(crate) fn prompt(&self) -> &'static str {
        match self.step {
            0 => "Project name: ",
            1 => "Project description: ",
            _ => "Tech stack (comma separated): ",
        }
    }
}

/// Which of the 5 windows currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    /// (1) Chat panel.
    Chat,
    /// (2) Info panel (unified Skills/MCPs tabs).
    Info,
    /// (3) Agents sidebar panel.
    Agents,
    /// (4) Queue panel (visible prompt queue).
    Queue,
    /// (5) Input box.
    Input,
}

/// State for the conversation history search overlay (Ctrl+R).
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    /// Whether the search overlay is visible.
    pub visible: bool,
    /// The current search query.
    pub query: String,
    /// Indices of matching messages in the conversation.
    pub matches: Vec<usize>,
    /// Currently selected match index.
    pub selected: usize,
}
