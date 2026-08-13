//! Shared types for the TUI: focus windows, agent info, approval/question
//! dialogs, the `/init` flow, and the built-in slash command list.

use crate::agent::types::{AgentId, AgentRole, AgentStatus, TaskMode};

/// Maximum number of chat messages kept in RAM for rendering. Older messages
/// are persisted in the SQLite database by the engine, so dropping them from
/// the in-memory buffer only affects the visible chat history (which the user
/// can recover by scrolling up — see `load_older_messages`).
///
/// This is the primary guard against unbounded RAM growth: a long session
/// with large code blocks can otherwise balloon past 160 MB.
pub(crate) const MAX_MESSAGES: usize = 500;

/// Maximum length (in characters) of a single chat message kept in RAM.
/// Tool outputs and LLM responses can be huge (tens of KB); truncating them
/// at the display layer saves memory without losing the DB-persisted content.
/// The full message is always available in the database.
pub(crate) const MAX_MESSAGE_LENGTH: usize = 12_000;

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
    ("/todos", "Show todo list"),
    ("/t", "Show todo list (alias)"),
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

/// State for the Ctrl+E edit-agent/subagent dialog.
pub(crate) struct EditDialogState {
    /// Whether the dialog is visible.
    pub visible: bool,
    /// Name of the agent or subagent being edited.
    pub target_name: String,
    /// Whether this is a root agent (shows subagents section).
    pub is_root: bool,
    /// All available skill names (union across agents).
    pub all_skills: Vec<String>,
    /// Which skills are currently enabled for the target.
    pub skills_enabled: Vec<bool>,
    /// All available MCP names (union across agents).
    pub all_mcps: Vec<String>,
    /// Which MCPs are currently enabled for the target.
    pub mcps_enabled: Vec<bool>,
    /// All available subagent names for root agents.
    pub all_subagents: Vec<String>,
    /// Which subagents are currently enabled for the target.
    pub subagents_enabled: Vec<bool>,
    /// Currently focused section (0 = Skills, 1 = MCPs, 2 = SubAgents — only for root).
    pub section: usize,
    /// Currently selected index within the section.
    pub index: usize,
}

impl EditDialogState {
    pub(crate) fn new() -> Self {
        Self {
            visible: false,
            target_name: String::new(),
            is_root: false,
            all_skills: Vec::new(),
            skills_enabled: Vec::new(),
            all_mcps: Vec::new(),
            mcps_enabled: Vec::new(),
            all_subagents: Vec::new(),
            subagents_enabled: Vec::new(),
            section: 0,
            index: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with(
        target_name: String,
        is_root: bool,
        all_skills: Vec<String>,
        skills_enabled: Vec<bool>,
        all_mcps: Vec<String>,
        mcps_enabled: Vec<bool>,
        all_subagents: Vec<String>,
        subagents_enabled: Vec<bool>,
    ) -> Self {
        Self {
            visible: true,
            target_name,
            is_root,
            all_skills,
            skills_enabled,
            all_mcps,
            mcps_enabled,
            all_subagents,
            subagents_enabled,
            section: 0,
            index: 0,
        }
    }

    /// The number of sections in this dialog (2 for subagents, 3 for root agents).
    pub(crate) fn section_count(&self) -> usize {
        if self.is_root { 3 } else { 2 }
    }

    /// The number of items in the current section.
    pub(crate) fn section_len(&self) -> usize {
        match self.section {
            0 => self.all_skills.len(),
            1 => self.all_mcps.len(),
            _ => self.all_subagents.len(),
        }
    }

    /// Toggle the currently selected item.
    pub(crate) fn toggle_current(&mut self) {
        let toggled = match self.section {
            0 => self.skills_enabled.get_mut(self.index),
            1 => self.mcps_enabled.get_mut(self.index),
            _ => self.subagents_enabled.get_mut(self.index),
        };
        if let Some(val) = toggled {
            *val = !*val;
        }
    }
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
    /// (1) Input box.
    Input,
    /// (2) Chat panel.
    Chat,
    /// (3) Info panel (unified Skills/MCPs tabs).
    Info,
    /// (4) Agents sidebar panel.
    Agents,
    /// (5) Queue panel (visible prompt queue).
    Queue,
}

/// Represents a single collapsible section in the chat render.
#[derive(Debug, Clone)]
pub(crate) struct CollapsedSection {
    /// Unique identifier: "{type}_{counter}" e.g. "thinking_1"
    pub(crate) id: String,
    /// Section type: "thinking", "tool", "normal", "user", "command"
    pub(crate) section_type: String,
    /// Index of the first line (the ▐ border) in rendered_chat_lines
    pub(crate) start_line: usize,
    /// Number of content lines (excluding the header ▐)
    pub(crate) line_count: usize,
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
