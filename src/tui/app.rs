use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use chrono::{DateTime, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::{
    Frame, Terminal,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthStr;

use crate::agent::types::{AgentId, AgentRole, AgentStatus};
use crate::config::Config;
use crate::config::types::CustomCommand;
use crate::db::models::SessionSummary;
use crate::engine::orchestrator::{
    EngineCommand, EngineEvent, ExportFormat, InitAnswers, McpStatus, SkillInfo, StatusInfo,
    TimelineEntry,
};
use crate::engine::template::expand_vars;
use crate::tui::diff_viewer::DiffViewer;
use crate::tui::keymap::{Action, Keymap};
use crate::tui::model_picker::ModelPicker;
use crate::tui::toast::{ToastKind, ToastQueue};
use crate::tui::which_key::WhichKeyPopup;

/// All slash commands with a short description, used by the fuzzy command
/// palette and Tab autocomplete.
const BUILTIN_COMMANDS: &[(&str, &str)] = &[
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
struct AgentInfo {
    id: AgentId,
    name: String,
    role: AgentRole,
    status: AgentStatus,
    skills: Vec<String>,
    mcps: Vec<String>,
    model: String,
    parent_id: Option<AgentId>,
    /// Number of child subagents (only for Root agents)
    subagent_count: usize,
}

/// A pending human approval request.
#[derive(Debug, Clone)]
struct ApprovalRequest {
    id: String,
    operation: String,
}

/// State for an inline question dialog (`/question` tool).
struct QuestionState {
    /// Question id (matches the engine's pending_questions key).
    id: String,
    /// The question text.
    question: String,
    /// Optional multiple-choice options.
    options: Vec<String>,
    /// Optional recommended default answer.
    recommended: Option<String>,
    /// Index of the currently selected option (if options present).
    selected: usize,
    /// Free-text answer being typed.
    answer_input: String,
}

/// Color themes selectable via `/themes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Theme {
    Default,
    Nord,
    Dracula,
    Solarized,
}

impl Theme {
    fn name(&self) -> &'static str {
        match self {
            Theme::Default => "default",
            Theme::Nord => "nord",
            Theme::Dracula => "dracula",
            Theme::Solarized => "solarized",
        }
    }

    fn next(&self) -> Theme {
        match self {
            Theme::Default => Theme::Nord,
            Theme::Nord => Theme::Dracula,
            Theme::Dracula => Theme::Solarized,
            Theme::Solarized => Theme::Default,
        }
    }

    /// Accent color used in the status bar and chat border.
    fn accent(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(255, 107, 107),
            Theme::Nord => Color::Rgb(136, 192, 208),
            Theme::Dracula => Color::Rgb(255, 121, 198),
            Theme::Solarized => Color::Rgb(38, 139, 210),
        }
    }
}

/// State for the interactive `/init` flow (sequential prompts).
struct InitFlow {
    /// Current prompt step: 0 = name, 1 = description, 2 = stack.
    step: usize,
    name: String,
    description: String,
    stack: String,
}

impl InitFlow {
    fn prompt(&self) -> &'static str {
        match self.step {
            0 => "Project name: ",
            1 => "Project description: ",
            _ => "Tech stack (comma separated): ",
        }
    }
}

/// Which of the 5 windows currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// (1) Chat panel.
    Chat,
    /// (2) MCPs sidebar panel.
    Mcps,
    /// (3) Skills sidebar panel.
    Skills,
    /// (4) Agents sidebar panel.
    Agents,
    /// (5) Input box.
    Input,
}

/// Application state for the TUI.
pub struct App {
    /// Channel to send commands to the engine.
    pub cmd_tx: mpsc::Sender<EngineCommand>,
    /// Channel to receive events from the engine.
    pub event_rx: mpsc::Receiver<EngineEvent>,
    /// Current user input buffer.
    pub input: String,
    /// Character index of the cursor within `input` (for shell-style editing).
    input_cursor: usize,
    /// Which window currently has keyboard focus.
    focus: Focus,
    /// Selected index in the MCPs sidebar panel.
    mcp_panel_index: usize,
    /// Selected index in the Skills sidebar panel.
    skill_panel_index: usize,
    /// Selected index in the Agents sidebar panel.
    agent_panel_index: usize,
    /// History of previously submitted inputs (for Up/Down arrow navigation).
    input_history: Vec<String>,
    /// Current position in input history while navigating (None = editing fresh).
    history_index: Option<usize>,
    /// Message log (displayed in the chat panel).
    pub messages: Vec<String>,
    /// Current streaming response being accumulated.
    pub current_stream: Option<String>,
    /// Index (into `messages`) of the in-progress stream that was already
    /// committed via `commit_stream`, so that `AgentOutput` replaces exactly
    /// that message instead of duplicating the partial content. Using the index
    /// (rather than `last_mut()`) keeps the replacement correct even if other
    /// messages are pushed in between.
    stream_committed_index: Option<usize>,
    /// Whether the app should exit.
    pub should_exit: bool,
    /// Error message to display.
    pub error: Option<String>,
    /// Current session name (for display).
    pub session_name: String,
    /// Current session ID (for display).
    pub session_id: Option<String>,
    /// Session list (for /sessions display).
    pub session_list: Vec<SessionSummary>,
    /// Whether to show session list panel.
    pub show_session_list: bool,

    // ── Agent info views ──────────────────────────────────────────────
    /// All known agents (root + subagents).
    agents: Vec<AgentInfo>,
    /// Whether to show the agent list overlay.
    pub show_agents: bool,
    /// Whether to show the subagent tree overlay.
    pub show_subagents: bool,
    /// Name of the currently active agent (for display).
    pub active_agent: String,
    /// Configured subagent names per root agent (from config frontmatter),
    /// used to show subagents in `/subagents` even before they are spawned.
    configured_subagents: HashMap<String, Vec<String>>,

    // ── Human-in-the-loop approval ────────────────────────────────────
    /// Pending approval request (None if no pending request).
    pending_approval: Option<ApprovalRequest>,

    // ── Inline question dialog (`/question` tool) ─────────────────────
    /// Pending question from the agent (None if no pending question).
    pending_question: Option<QuestionState>,

    // ── Right panel data ──────────────────────────────────────────────
    /// Total tokens consumed in the current session.
    pub total_tokens: u64,
    /// Percentage of the context window used.
    pub context_window_pct: f64,
    /// Total cost spent (in dollars).
    pub total_cost: f64,
    /// Context window size (in tokens) of the active model.
    pub context_window: u64,
    /// Name of the model currently being executed.
    pub current_model: String,
    /// Current working directory for display.
    pub working_dir: String,
    /// Whether to show the welcome banner (true until first message arrives).
    pub show_welcome: bool,
    /// Whether the terminal supports the Kitty keyboard enhancement protocol.
    pub kb_supported: bool,
    /// Whether debug mode is active (shows LLM JSON payloads).
    pub debug_mode: bool,
    /// Keyboard locale (from $LANG), used for shift mapping with Kitty protocol.
    lang: String,
    /// All slash commands (built-in + custom) for Tab autocomplete and palette.
    commands: Vec<(String, String)>,
    /// Custom slash commands with their templates (for dispatch).
    custom_commands: Vec<CustomCommand>,
    /// Current autocomplete matches for Tab cycling.
    tab_matches: Vec<String>,
    /// Index into tab_matches for cycling.
    tab_index: usize,
    /// Whether the fuzzy command palette is currently open.
    show_command_palette: bool,
    /// Indices into `COMMANDS` for the current fuzzy matches.
    palette_matches: Vec<usize>,
    /// Index of the currently highlighted palette entry.
    palette_index: usize,
    /// Whether the agent-selection combo is open (for `/agent`).
    show_agent_palette: bool,
    /// Root agent names matching the current `/agent` query.
    agent_matches: Vec<String>,
    /// Index of the currently highlighted agent entry.
    agent_index: usize,
    /// Whether the model-selection combo is open (for `/models`).
    show_model_palette: bool,
    /// Model names matching the current `/models` query.
    model_matches: Vec<String>,
    /// Index of the currently highlighted model entry.
    model_index: usize,
    /// Vertical scroll offset for the chat panel (0 = bottom, auto-scroll).
    pub chat_scroll: u16,
    /// Timestamp of the last 'g' press, used to detect a double-'g' (gg) jump.
    last_g_press: Option<Instant>,
    /// Frame counter for animating spinners in the UI.
    pub frame_count: u64,

    // ── OpenCode-style slash command state ───────────────────────────
    /// Current color theme (`/themes`).
    theme: Theme,
    /// Whether to show timestamps next to chat messages (`/timestamps`).
    pub show_timestamps: bool,
    /// Whether to show LLM thinking/streaming output (`/thinking`).
    pub show_thinking: bool,
    /// Timestamps recorded when each chat message was added.
    message_timestamps: Vec<DateTime<Utc>>,
    /// Stash stack for `/stash` (saved prompts).
    stash_stack: Vec<String>,
    /// Skills listed by the engine (`/skills`).
    skills_list: Vec<SkillInfo>,
    /// MCP servers with on/off state (`/mcps`).
    mcps_list: Vec<McpStatus>,
    /// Engine status report (`/status`).
    status_info: Option<StatusInfo>,
    /// Known workspaces (`/workspaces`).
    workspaces_list: Vec<String>,
    /// Session timeline entries (`/timeline`).
    timeline: Vec<TimelineEntry>,
    /// Whether the timeline panel is open.
    pub show_timeline: bool,
    /// Index of the highlighted timeline entry.
    timeline_index: usize,
    /// Whether the MCP list panel is open.
    pub show_mcps: bool,
    /// Index of the highlighted MCP entry.
    mcps_index: usize,
    /// Active `/init` flow (None when not running).
    init_flow: Option<InitFlow>,
    /// Todo list for the active session (from the `todo` tool).
    todos: Vec<crate::db::models::Todo>,

    // ── FASE 4: keymap / which-key / toasts ─────────────────────────
    /// Central keymap dispatching actions to keys.
    pub keymap: Keymap,
    /// Which-key popup state.
    pub which_key: WhichKeyPopup,
    /// Transient toast notifications.
    pub toasts: ToastQueue,
    /// Whether the right-hand sidebar panels are visible.
    pub show_sidebar: bool,
    /// Diff viewer overlay state.
    pub diff_viewer: DiffViewer,
    /// Model picker overlay state.
    pub model_picker: ModelPicker,
    /// External editor command (from config, overrides `$EDITOR`/`$VISUAL`).
    pub editor: Option<String>,
    /// Queue of pending prompts (FASE 4.6).
    pub prompt_queue: Vec<String>,
    /// Whether the prompt queue popup is visible.
    pub show_prompt_queue: bool,
    /// Selected index in the prompt queue popup.
    pub prompt_queue_index: usize,
}

impl App {
    pub fn new(
        cmd_tx: mpsc::Sender<EngineCommand>,
        event_rx: mpsc::Receiver<EngineEvent>,
        kb_supported: bool,
        config: &Config,
    ) -> Self {
        let working_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| String::from("(unknown)"));
        let lang = std::env::var("LANG").unwrap_or_default();

        let mut keymap = Keymap::default();
        if let Some(overrides) = &config.keymap {
            keymap.apply_overrides(overrides);
        }
        let mut model_picker = ModelPicker::default();
        model_picker.set_favorites(config.model_picker.favorites.clone());

        // Map each root agent to its configured subagent names (from frontmatter),
        // so `/subagents` can show them even before they are spawned at runtime.
        let configured_subagents = config
            .agents
            .iter()
            .filter(|a| a.role == AgentRole::Root)
            .map(|a| (a.name.clone(), a.subagents.clone()))
            .collect::<HashMap<_, _>>();

        Self {
            cmd_tx,
            event_rx,
            input: String::new(),
            input_cursor: 0,
            focus: Focus::Input,
            mcp_panel_index: 0,
            skill_panel_index: 0,
            agent_panel_index: 0,
            input_history: Vec::new(),
            history_index: None,
            messages: Vec::new(),
            current_stream: None,
            stream_committed_index: None,
            should_exit: false,
            error: None,
            session_name: "default".into(),
            session_id: None,
            session_list: Vec::new(),
            show_session_list: false,
            agents: Vec::new(),
            show_agents: false,
            show_subagents: false,
            active_agent: String::new(),
            configured_subagents,
            pending_approval: None,
            pending_question: None,
            total_tokens: 0,
            context_window_pct: 0.0,
            total_cost: 0.0,
            context_window: 0,
            current_model: String::new(),
            working_dir,
            show_welcome: true,
            kb_supported,
            lang,
            debug_mode: false,
            commands: {
                let mut cmds: Vec<(String, String)> = BUILTIN_COMMANDS
                    .iter()
                    .map(|(c, d)| (c.to_string(), d.to_string()))
                    .collect();
                for cc in &config.commands {
                    cmds.push((cc.name.clone(), cc.description.clone()));
                }
                cmds
            },
            custom_commands: config.commands.clone(),
            tab_matches: Vec::new(),
            tab_index: 0,
            show_command_palette: false,
            palette_matches: Vec::new(),
            palette_index: 0,
            show_agent_palette: false,
            agent_matches: Vec::new(),
            agent_index: 0,
            show_model_palette: false,
            model_matches: Vec::new(),
            model_index: 0,
            chat_scroll: 0,
            last_g_press: None,
            frame_count: 0,
            theme: Theme::Default,
            show_timestamps: false,
            show_thinking: true,
            message_timestamps: Vec::new(),
            stash_stack: Vec::new(),
            skills_list: Vec::new(),
            mcps_list: Vec::new(),
            status_info: None,
            workspaces_list: Vec::new(),
            timeline: Vec::new(),
            show_timeline: false,
            timeline_index: 0,
            show_mcps: false,
            mcps_index: 0,
            init_flow: None,
            todos: Vec::new(),
            keymap,
            which_key: WhichKeyPopup::new(),
            toasts: ToastQueue::default(),
            show_sidebar: true,
            diff_viewer: DiffViewer::new(),
            model_picker,
            editor: config.editor.clone(),
            prompt_queue: Vec::new(),
            show_prompt_queue: false,
            prompt_queue_index: 0,
        }
    }

    /// Append a chat message, recording its timestamp for `/timestamps`.
    fn push_msg(&mut self, msg: impl Into<String>) {
        self.message_timestamps.push(Utc::now());
        self.messages.push(msg.into());
    }

    /// Commit any in-progress streaming response to the message log so that a
    /// newly submitted user message appears AFTER it, preserving chat order.
    fn commit_stream(&mut self) {
        if let Some(stream) = self.current_stream.take() {
            if !stream.is_empty() {
                self.push_msg(stream);
                self.stream_committed_index = Some(self.messages.len() - 1);
            }
        }
    }

    /// Process a single event from the engine.
    pub fn handle_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Started { debug } => {
                self.debug_mode = debug;
                self.push_msg("Anacleto started.");
                self.chat_scroll = 0;
                self.toasts
                    .push("Anacleto listo — pulsa ? para atajos", ToastKind::Info);
            }
            EngineEvent::ModelChanged { model } => {
                self.current_model = model.clone();
                self.push_msg(format!("Model changed to: {}", model));
                self.chat_scroll = 0;
            }
            EngineEvent::ConversationCompacted { .. } => {
                self.push_msg("Conversación compactada.");
                self.chat_scroll = 0;
            }
            EngineEvent::AgentCreated {
                id,
                name,
                role,
                model,
                skills,
                mcps,
            } => {
                self.push_msg(format!("Agent '{}' created.", name));
                self.chat_scroll = 0;
                // Add to agent list
                if !self.agents.iter().any(|a| a.id == id) {
                    self.agents.push(AgentInfo {
                        id,
                        name: name.clone(),
                        role,
                        status: AgentStatus::Idle,
                        skills,
                        mcps,
                        model,
                        parent_id: None,
                        subagent_count: 0,
                    });
                }
            }
            EngineEvent::AgentStreamChunk { content, .. } => {
                *self.current_stream.get_or_insert_with(String::new) += &content;
            }
            EngineEvent::AgentOutput { content, .. } => {
                self.current_stream = None;
                if let Some(idx) = self.stream_committed_index.take() {
                    // The partial stream was already committed; replace exactly
                    // that message with the full content to avoid duplication.
                    if let Some(msg) = self.messages.get_mut(idx) {
                        *msg = content;
                    } else {
                        self.push_msg(content);
                    }
                } else {
                    self.push_msg(content);
                }
                self.chat_scroll = 0;
            }
            EngineEvent::AgentStatusChanged {
                agent_id, status, ..
            } => {
                if let Some(agent) = self.agents.iter_mut().find(|a| a.id == agent_id) {
                    agent.status = status;
                }
            }
            EngineEvent::SubagentCreated {
                parent_id,
                subagent_id,
                subagent_name,
                skills,
                mcps,
            } => {
                self.messages
                    .push(format!("Subagent '{}' created.", subagent_name));
                self.chat_scroll = 0;
                // Track subagent in the list (added later via AgentCreated?)
                // Also bump parent's subagent_count
                if let Some(parent) = self.agents.iter_mut().find(|a| a.id == parent_id) {
                    parent.subagent_count += 1;
                }
                // Add subagent to list (if not already present)
                if !self.agents.iter().any(|a| a.id == subagent_id) {
                    self.agents.push(AgentInfo {
                        id: subagent_id,
                        name: subagent_name,
                        role: AgentRole::SubAgent,
                        status: AgentStatus::Working,
                        skills,
                        mcps,
                        model: String::new(),
                        parent_id: Some(parent_id),
                        subagent_count: 0,
                    });
                }
            }
            EngineEvent::SubagentCompleted {
                subagent_id,
                subagent_name,
                ..
            } => {
                self.messages
                    .push(format!("Subagent '{}' completed.", subagent_name));
                self.chat_scroll = 0;
                if let Some(agent) = self.agents.iter_mut().find(|a| a.id == subagent_id) {
                    agent.status = AgentStatus::Completed;
                }
            }
            EngineEvent::SessionList(sessions) => {
                self.session_list = sessions;
                self.show_session_list = true;
            }
            EngineEvent::SessionSwitched { id, name } => {
                self.session_id = Some(id);
                self.session_name = name.clone();
                self.show_session_list = false;
                self.push_msg(format!("Switched to session: {}", name));
                self.chat_scroll = 0;
            }
            EngineEvent::AgentSwitched { name } => {
                self.active_agent = name.clone();
                self.push_msg(format!("Agente activo: {}", name));
                self.chat_scroll = 0;
            }
            EngineEvent::SessionDeleted { id } => {
                self.push_msg(format!("Session {} deleted.", &id[..8]));
                self.chat_scroll = 0;
                if self.session_id.as_deref() == Some(&id) {
                    self.session_id = None;
                    self.session_name = "none".into();
                }
            }
            EngineEvent::SessionRenamed { name, .. } => {
                self.session_name = name.clone();
                self.push_msg(format!("Session renamed to: {}", name));
                self.chat_scroll = 0;
            }
            EngineEvent::Error { message, .. } => {
                self.error = Some(message.clone());
                self.push_msg(format!("Error: {}", message));
                self.chat_scroll = 0;
            }
            EngineEvent::ShuttingDown => {
                self.push_msg("Anacleto shutting down.");
                self.chat_scroll = 0;
            }
            EngineEvent::ApprovalRequired { id, operation } => {
                self.pending_approval = Some(ApprovalRequest { id, operation });
                self.toasts
                    .push("Aprobación requerida (Y/N)", ToastKind::Info);
            }
            EngineEvent::Question {
                id,
                question,
                options,
                recommended,
            } => {
                self.pending_question = Some(QuestionState {
                    id,
                    question,
                    options,
                    recommended,
                    selected: 0,
                    answer_input: String::new(),
                });
            }
            EngineEvent::TokenUsage {
                total_tokens,
                context_window,
                cost,
                ..
            } => {
                self.total_tokens += total_tokens as u64;
                self.context_window = context_window as u64;
                self.context_window_pct =
                    (self.total_tokens as f64 / context_window as f64) * 100.0;
                // Cost is computed in the engine from per-million-token prices.
                self.total_cost += cost;
            }
            EngineEvent::ToolExecution {
                tool_name, task, ..
            } => {
                self.messages
                    .push(format!("\u{1f527} {}: {}", tool_name, task));
                self.chat_scroll = 0;
            }
            EngineEvent::ToolResult {
                tool_name,
                success,
                summary,
                ..
            } => {
                let icon = if success { "\u{2705}" } else { "\u{274c}" };
                let msg = if success {
                    format!("{} {} \u{2014} {}", icon, tool_name, summary)
                } else {
                    format!("{} {} failed: {}", icon, tool_name, summary)
                };
                self.push_msg(msg);
                self.chat_scroll = 0;
            }
            EngineEvent::LlmRequestDebug {
                agent_name,
                model,
                payload,
                ..
            } => {
                self.push_msg(format!(
                    "\u{1f50d} LLM Request [{}] ({}):",
                    agent_name, model
                ));
                for line in payload.split('\n') {
                    self.push_msg(format!("  {}", line));
                }
                self.chat_scroll = 0;
            }
            EngineEvent::LlmResponseDebug {
                agent_name,
                model,
                payload,
                ..
            } => {
                self.push_msg(format!(
                    "\u{1f50d} LLM Response [{}] ({}):",
                    agent_name, model
                ));
                for line in payload.split('\n') {
                    self.push_msg(format!("  {}", line));
                }
                self.chat_scroll = 0;
            }
            // ── OpenCode-style slash command events ──────────────────
            EngineEvent::UndoApplied { removed } => {
                // Remove the undone messages from the display log.
                let n = removed.len();
                for _ in 0..n {
                    self.messages.pop();
                    self.message_timestamps.pop();
                }
                self.push_msg("\u{21a9} Undo applied.");
                self.chat_scroll = 0;
            }
            EngineEvent::RedoApplied { restored } => {
                // Re-add the restored messages to the display log.
                for msg in restored {
                    self.push_msg(msg);
                }
                self.push_msg("\u{21aa} Redo applied.");
                self.chat_scroll = 0;
            }
            EngineEvent::Forked { new_session_id } => {
                self.session_id = Some(new_session_id.to_string());
                self.push_msg(format!(
                    "\u{2382} Forked into new session: {}",
                    new_session_id
                ));
                self.chat_scroll = 0;
            }
            EngineEvent::Exported { path } => {
                self.push_msg(format!("\u{1f4e4} Session exported to: {}", path.display()));
                self.chat_scroll = 0;
            }
            EngineEvent::Imported { session_id } => {
                self.session_id = Some(session_id.to_string());
                self.push_msg(format!("\u{1f4e5} Session imported: {}", session_id));
                self.chat_scroll = 0;
            }
            EngineEvent::ShareUpdated { shared, link } => {
                if shared {
                    let l = link.as_deref().unwrap_or("(no link)");
                    self.push_msg(format!("\u{1f517} Session shared: {}", l));
                } else {
                    self.push_msg("\u{1f513} Session unshared.");
                }
                self.chat_scroll = 0;
            }
            EngineEvent::SkillsListed(skills) => {
                self.skills_list = skills;
                self.push_msg(format!(
                    "\u{2699} {} skill(s) available.",
                    self.skills_list.len()
                ));
                self.chat_scroll = 0;
            }
            EngineEvent::McpsListed(mcps) => {
                self.mcps_list = mcps;
                self.show_mcps = true;
                self.push_msg(format!("\u{1f50c} {} MCP server(s).", self.mcps_list.len()));
                self.chat_scroll = 0;
            }
            EngineEvent::StatusReport(info) => {
                self.status_info = Some(info);
                self.push_msg("\u{1f4ca} Status updated.");
                self.chat_scroll = 0;
            }
            EngineEvent::InitDone => {
                self.push_msg("\u{2705} AGENTS.md initialized.");
                self.chat_scroll = 0;
            }
            EngineEvent::ReviewResult(result) => {
                self.push_msg(format!("\u{1f50d} Review: {}", result));
                self.chat_scroll = 0;
            }
            EngineEvent::WorkspaceChanged(dir) => {
                self.working_dir = dir.to_string_lossy().to_string();
                self.push_msg(format!("\u{1f4c1} Workspace changed to: {}", dir.display()));
                self.chat_scroll = 0;
            }
            EngineEvent::WorkspacesListed(workspaces) => {
                self.workspaces_list = workspaces;
                self.push_msg(format!(
                    "\u{1f5c2} {} workspace(s).",
                    self.workspaces_list.len()
                ));
                self.chat_scroll = 0;
            }
            EngineEvent::Timeline(entries) => {
                self.timeline = entries;
                self.show_timeline = true;
                self.timeline_index = 0;
                self.push_msg(format!(
                    "\u{1f550} {} timeline entrie(s).",
                    self.timeline.len()
                ));
                self.chat_scroll = 0;
            }
            EngineEvent::SessionMoved {
                session_id,
                workspace,
            } => {
                self.push_msg(format!(
                    "\u{27a1} Session {} moved to workspace '{}'.",
                    session_id, workspace
                ));
                self.chat_scroll = 0;
            }
            EngineEvent::CommandError(msg) => {
                self.push_msg(format!("\u{26a0} Error: {}", msg));
                self.chat_scroll = 0;
            }
            EngineEvent::WorktreeResult(result) => {
                self.push_msg(format!("\u{1f4c2} Worktree: {}", result));
                self.chat_scroll = 0;
            }
            EngineEvent::TodosUpdated(todos) => {
                self.todos = todos;
            }
            EngineEvent::DiffAvailable { text, title } => {
                self.diff_viewer.push_diff(&text, &title);
                self.toasts
                    .push("Diff disponible — pulsa Ctrl+G", ToastKind::Info);
            }
            EngineEvent::ModelsFrecency(frecency) => {
                let recent = frecency.into_iter().map(|(m, _)| m).collect();
                self.model_picker.set_recent(recent);
            }
            // ── FASE 1 y 2: build, jobs y snapshots ─────────────────
            EngineEvent::SubagentFinished { task_id, summary } => {
                self.push_msg(format!(
                    "\u{1f4c4} Tarea '{}' finalizada: {}",
                    task_id, summary
                ));
                self.chat_scroll = 0;
            }
            EngineEvent::BuildDone => {
                self.push_msg("\u{1f3d7} Build completado.");
                self.chat_scroll = 0;
            }
            EngineEvent::SessionTree(sessions) => {
                if sessions.is_empty() {
                    self.push_msg("\u{1f5c2} Sin sesiones hijas.");
                } else {
                    self.push_msg(format!("\u{1f5c2} Árbol de sesiones ({}):", sessions.len()));
                    for s in &sessions {
                        let parent = s
                            .parent_id
                            .map(|p| format!(" (padre: {})", &p.to_string()[..8]))
                            .unwrap_or_default();
                        self.push_msg(format!(
                            "  \u{251c} {} — {} mensajes{}",
                            s.name, s.message_count, parent
                        ));
                    }
                }
                self.chat_scroll = 0;
            }
            EngineEvent::JobsListed(jobs) => {
                if jobs.is_empty() {
                    self.push_msg("\u{1f4cb} Sin jobs activos.");
                } else {
                    self.push_msg(format!("\u{1f4cb} {} job(s) activo(s):", jobs.len()));
                    for job in &jobs {
                        self.push_msg(format!("  \u{2022} {}", job));
                    }
                }
                self.chat_scroll = 0;
            }
            EngineEvent::SnapshotCreated { snapshot } => {
                self.push_msg(format!(
                    "\u{1f4be} Snapshot '{}' creado ({} mensajes).",
                    snapshot.name, snapshot.message_count
                ));
                self.chat_scroll = 0;
            }
            EngineEvent::SnapshotReverted { snapshot_id } => {
                self.push_msg(format!(
                    "\u{21a9} Sesión revertida al snapshot {}.",
                    &snapshot_id.to_string()[..8]
                ));
                self.chat_scroll = 0;
            }
            EngineEvent::SnapshotsListed(snapshots) => {
                if snapshots.is_empty() {
                    self.push_msg("\u{1f4be} Sin snapshots para esta sesión.");
                } else {
                    self.push_msg(format!("\u{1f4be} {} snapshot(s):", snapshots.len()));
                    for s in &snapshots {
                        self.push_msg(format!(
                            "  \u{2022} {} — {} mensajes",
                            s.name, s.message_count
                        ));
                    }
                }
                self.chat_scroll = 0;
            }
            _ => {}
        }
    }

    /// Handle a key event.
    pub fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let key_event = KeyEvent::new(key, modifiers);

        // If the which-key popup is open, any key press closes it.
        if self.which_key.visible {
            self.which_key.visible = false;
            return;
        }

        // If approval dialog is active, Y/N are handled specially
        if self.pending_approval.is_some() {
            match key {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self
                            .cmd_tx
                            .try_send(EngineCommand::ApprovalResponse { id, approved: true });
                        self.toasts.push("Aprobado", ToastKind::Success);
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self.cmd_tx.try_send(EngineCommand::ApprovalResponse {
                            id,
                            approved: false,
                        });
                        self.toasts.push("Denegado", ToastKind::Info);
                    }
                }
                _ if self.keymap.matches(key_event, Action::Approve) => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self
                            .cmd_tx
                            .try_send(EngineCommand::ApprovalResponse { id, approved: true });
                        self.toasts.push("Aprobado", ToastKind::Success);
                    }
                }
                _ if self.keymap.matches(key_event, Action::Deny) => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self.cmd_tx.try_send(EngineCommand::ApprovalResponse {
                            id,
                            approved: false,
                        });
                        self.toasts.push("Denegado", ToastKind::Info);
                    }
                }
                _ => {}
            }
            return;
        }

        // Inline question dialog (`/question` tool): capture answer.
        if self.pending_question.is_some() {
            match key {
                KeyCode::Enter => {
                    if let Some(q) = self.pending_question.take() {
                        let answer = if !q.options.is_empty() {
                            q.options.get(q.selected).cloned().unwrap_or_default()
                        } else {
                            q.answer_input.trim().to_string()
                        };
                        let id = q.id.clone();
                        let _ = self
                            .cmd_tx
                            .try_send(EngineCommand::QuestionAnswer { id, answer });
                    }
                }
                KeyCode::Esc => {
                    if let Some(q) = self.pending_question.take() {
                        let id = q.id.clone();
                        let _ = self.cmd_tx.try_send(EngineCommand::QuestionAnswer {
                            id,
                            answer: String::new(),
                        });
                    }
                }
                KeyCode::Up => {
                    if let Some(q) = self.pending_question.as_mut() {
                        if !q.options.is_empty() {
                            q.selected = q.selected.saturating_sub(1);
                        }
                    }
                }
                KeyCode::Down => {
                    if let Some(q) = self.pending_question.as_mut() {
                        if !q.options.is_empty() {
                            q.selected = (q.selected + 1) % q.options.len();
                        }
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(q) = self.pending_question.as_mut() {
                        if q.options.is_empty() {
                            q.answer_input.push(c);
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Some(q) = self.pending_question.as_mut() {
                        if q.options.is_empty() {
                            q.answer_input.pop();
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        // Interactive `/init` flow: capture answers.
        if self.init_flow.is_some() {
            match key {
                KeyCode::Enter => {
                    self.collect_init_answer();
                }
                KeyCode::Esc => {
                    self.init_flow = None;
                    self.input.clear();
                    self.input_cursor = 0;
                }
                KeyCode::Char(c) => {
                    self.input.push(c);
                    self.input_cursor = self.input.chars().count();
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    self.input_cursor = self.input.chars().count();
                }
                _ => {}
            }
            return;
        }

        // Timeline navigation.
        if self.show_timeline {
            match key {
                KeyCode::Up => {
                    self.timeline_index = self.timeline_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !self.timeline.is_empty() {
                        self.timeline_index = (self.timeline_index + 1) % self.timeline.len();
                    }
                }
                KeyCode::Enter => {
                    self.jump_to_timeline_entry();
                }
                KeyCode::Esc => {
                    self.show_timeline = false;
                }
                _ => {}
            }
            return;
        }

        // MCP list navigation.
        if self.show_mcps {
            match key {
                KeyCode::Up => {
                    self.mcps_index = self.mcps_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !self.mcps_list.is_empty() {
                        self.mcps_index = (self.mcps_index + 1) % self.mcps_list.len();
                    }
                }
                KeyCode::Enter => {
                    self.toggle_selected_mcp();
                }
                KeyCode::Esc => {
                    self.show_mcps = false;
                }
                _ => {}
            }
            return;
        }

        // ── Model picker navigation ──────────────────────────────────
        if self.model_picker.visible {
            match key {
                KeyCode::Up => self.model_picker.previous(),
                KeyCode::Down => self.model_picker.next(),
                KeyCode::Tab | KeyCode::Right => self.model_picker.next_mode(),
                KeyCode::Left => self.model_picker.previous_mode(),
                KeyCode::Enter => {
                    if let Some(model) = self.model_picker.selected_model() {
                        let _ = self.cmd_tx.try_send(EngineCommand::SetModel(model.clone()));
                        let _ = self.cmd_tx.try_send(EngineCommand::RecordModelUsage(model));
                        self.toasts.push("Cambiando modelo…", ToastKind::Info);
                    }
                    self.model_picker.visible = false;
                }
                KeyCode::Esc => {
                    self.model_picker.visible = false;
                }
                _ => {}
            }
            return;
        }

        // ── Diff viewer navigation ───────────────────────────────────
        if self.diff_viewer.visible {
            match key {
                KeyCode::Up => self.diff_viewer.scroll_up(1),
                KeyCode::Down => self.diff_viewer.scroll_down(1),
                KeyCode::PageUp => self.diff_viewer.scroll_up(10),
                KeyCode::PageDown => self.diff_viewer.scroll_down(10),
                KeyCode::Esc => {
                    self.diff_viewer.visible = false;
                }
                _ => {}
            }
            return;
        }

        // ── Prompt queue popup navigation ───────────────────────────
        if self.show_prompt_queue {
            match key {
                KeyCode::Up => {
                    self.prompt_queue_index = self.prompt_queue_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    if self.prompt_queue_index + 1 < self.prompt_queue.len() {
                        self.prompt_queue_index += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some(prompt) = self.prompt_queue.get(self.prompt_queue_index) {
                        let text = prompt.clone();
                        self.prompt_queue.remove(self.prompt_queue_index);
                        if self.prompt_queue.is_empty() {
                            self.show_prompt_queue = false;
                        } else {
                            self.prompt_queue_index =
                                self.prompt_queue_index.min(self.prompt_queue.len() - 1);
                        }
                        let _ = self.cmd_tx.try_send(EngineCommand::UserInput(text));
                    }
                }
                KeyCode::Char('d') => {
                    if !self.prompt_queue.is_empty() {
                        self.prompt_queue.remove(self.prompt_queue_index);
                        if self.prompt_queue.is_empty() {
                            self.show_prompt_queue = false;
                        } else {
                            self.prompt_queue_index =
                                self.prompt_queue_index.min(self.prompt_queue.len() - 1);
                        }
                    }
                }
                KeyCode::Esc => {
                    self.show_prompt_queue = false;
                }
                _ => {}
            }
            return;
        }

        // ── Focus switching (Alt+1..Alt+5) + keymap-driven global actions ──
        // Only dispatch when the key is a special/modified key, or when the
        // input is empty (so plain characters can still be typed normally).
        // Alt+1..Alt+5 are modified keys, so they always apply; the legacy
        // letter bindings ('c'/'i') only switch focus when the input is empty.
        if self.keymap_applies(key_event) {
            if self.keymap.matches(key_event, Action::FocusChat) {
                self.focus = Focus::Chat;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusMcps) {
                self.focus = Focus::Mcps;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusSkills) {
                self.focus = Focus::Skills;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusAgents) {
                self.focus = Focus::Agents;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusInput) {
                self.focus = Focus::Input;
                return;
            }

            if self.keymap.matches(key_event, Action::Quit) {
                self.should_exit = true;
                return;
            }
            if self.keymap.matches(key_event, Action::OpenWhichKey) {
                self.which_key.visible = true;
                return;
            }
            if self.keymap.matches(key_event, Action::ToggleSidebar) {
                self.show_sidebar = !self.show_sidebar;
                return;
            }
            if self.keymap.matches(key_event, Action::ToggleDiffViewer) {
                self.diff_viewer.visible = !self.diff_viewer.visible;
                return;
            }
            if self.keymap.matches(key_event, Action::OpenModelPicker) {
                self.model_picker.visible = true;
                let _ = self.cmd_tx.try_send(EngineCommand::ListModelFrecency);
                return;
            }
            if self.keymap.matches(key_event, Action::OpenEditor) {
                self.open_editor();
                return;
            }
            if self.keymap.matches(key_event, Action::OpenPromptQueue) {
                self.show_prompt_queue = true;
                self.prompt_queue_index = 0;
                return;
            }
            // Quick slots 1..9 resume the pinned session at that index.
            let quick_slots = [
                Action::QuickSlot1,
                Action::QuickSlot2,
                Action::QuickSlot3,
                Action::QuickSlot4,
                Action::QuickSlot5,
                Action::QuickSlot6,
                Action::QuickSlot7,
                Action::QuickSlot8,
                Action::QuickSlot9,
            ];
            for (idx, action) in quick_slots.iter().enumerate() {
                if self.keymap.matches(key_event, *action) {
                    self.resume_quick_slot(idx);
                    return;
                }
            }
        }

        // ── Route the remaining keys by the focused window ───────────
        match self.focus {
            Focus::Input => self.handle_input_key(key, modifiers, key_event),
            Focus::Chat => self.handle_chat_key(key, modifiers, key_event),
            Focus::Mcps => self.handle_mcp_panel_key(key, modifiers, key_event),
            Focus::Skills => self.handle_skill_panel_key(key, modifiers, key_event),
            Focus::Agents => self.handle_agent_panel_key(key, modifiers, key_event),
        }
    }

    /// Handle a key while the Input window (5) has focus.
    fn handle_input_key(&mut self, key: KeyCode, modifiers: KeyModifiers, key_event: KeyEvent) {
        if self.keymap.matches(key_event, Action::TabComplete) {
            // Reset matches if the input has changed since last Tab
            if !self.input.starts_with('/') {
                return;
            }
            let prefix = self.input.to_lowercase();
            if self.tab_index == 0 || self.tab_matches.is_empty() {
                self.tab_matches = self
                    .commands
                    .iter()
                    .filter(|(c, _)| c.starts_with(&prefix))
                    .map(|(c, _)| c.clone())
                    .collect();
            }
            if self.tab_matches.is_empty() {
                return;
            }
            let idx = self.tab_index % self.tab_matches.len();
            self.input = self.tab_matches[idx].clone();
            self.input_cursor = self.input.chars().count();
            self.tab_index += 1;
        } else if self.keymap.matches(key_event, Action::InsertNewline) {
            self.reset_tab_state();
            self.input_insert_char('\n');
        } else if self.keymap.matches(key_event, Action::ClearInput) {
            self.reset_tab_state();
            self.input.clear();
            self.input_cursor = 0;
        } else if self.keymap.matches(key_event, Action::DeleteToStart) {
            self.reset_tab_state();
            self.input_delete_to_start();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteWordBefore) {
            self.reset_tab_state();
            self.input_delete_word_before();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteToEnd) {
            self.reset_tab_state();
            self.input_delete_to_end();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::CursorHome) {
            self.reset_tab_state();
            self.input_cursor = 0;
        } else if self.keymap.matches(key_event, Action::CursorEnd) {
            self.reset_tab_state();
            self.input_cursor = self.input.chars().count();
        } else if self.keymap.matches(key_event, Action::CursorWordLeft) {
            self.reset_tab_state();
            self.input_move_word_left();
        } else if self.keymap.matches(key_event, Action::CursorWordRight) {
            self.reset_tab_state();
            self.input_move_word_right();
        } else if self.keymap.matches(key_event, Action::CursorLeft) {
            self.input_cursor = self.input_cursor.saturating_sub(1);
        } else if self.keymap.matches(key_event, Action::CursorRight) {
            let len = self.input.chars().count();
            self.input_cursor = (self.input_cursor + 1).min(len);
        } else if self.keymap.matches(key_event, Action::DeleteChar) {
            self.input_delete_at();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteCharBefore) {
            self.tab_matches.clear();
            self.tab_index = 0;
            self.input_delete_before();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::HistoryUp) {
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = self
                    .model_index
                    .saturating_sub(1)
                    .min(self.model_matches.len() - 1);
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = self
                    .agent_index
                    .saturating_sub(1)
                    .min(self.agent_matches.len() - 1);
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = self
                    .palette_index
                    .saturating_sub(1)
                    .min(self.palette_matches.len() - 1);
            } else if !self.input_history.is_empty() {
                // Navigate backwards through input history.
                let next = match self.history_index {
                    Some(i) if i > 0 => i - 1,
                    Some(_) => 0,
                    None => self.input_history.len() - 1,
                };
                self.history_index = Some(next);
                self.input = self.input_history[next].clone();
                self.input_cursor = self.input.chars().count();
                self.tab_matches.clear();
                self.tab_index = 0;
            }
        } else if self.keymap.matches(key_event, Action::HistoryDown) {
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = (self.model_index + 1) % self.model_matches.len();
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = (self.agent_index + 1) % self.agent_matches.len();
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = (self.palette_index + 1) % self.palette_matches.len();
            } else if self.history_index.is_some() {
                // Navigate forwards through input history; past the newest returns to empty.
                match self.history_index {
                    Some(i) if i + 1 < self.input_history.len() => {
                        self.history_index = Some(i + 1);
                        self.input = self.input_history[i + 1].clone();
                    }
                    _ => {
                        self.history_index = None;
                        self.input.clear();
                    }
                }
                self.input_cursor = self.input.chars().count();
                self.tab_matches.clear();
                self.tab_index = 0;
            }
        } else if self.keymap.matches(key_event, Action::Send) {
            self.tab_matches.clear();
            self.tab_index = 0;
            if self.show_model_palette && !self.model_matches.is_empty() {
                // Execute `/models <selected>` from the model combo.
                let name = self.model_matches[self.model_index].clone();
                self.show_model_palette = false;
                self.model_matches.clear();
                self.model_index = 0;
                self.input.clear();
                self.input_cursor = 0;
                self.handle_command(format!("/models {}", name));
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                // Execute `/agent <selected>` from the agent combo.
                let name = self.agent_matches[self.agent_index].clone();
                self.show_agent_palette = false;
                self.agent_matches.clear();
                self.agent_index = 0;
                self.input.clear();
                self.input_cursor = 0;
                self.handle_command(format!("/agent {}", name));
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                // Execute the highlighted command from the palette.
                let idx = self.palette_matches[self.palette_index];
                let cmd = self.commands[idx].0.clone();
                self.show_command_palette = false;
                self.palette_matches.clear();
                self.palette_index = 0;
                self.input.clear();
                self.input_cursor = 0;
                self.handle_command(cmd);
            } else {
                let input = std::mem::take(&mut self.input);
                self.input_cursor = 0;
                if !input.is_empty() {
                    // Record in input history (dedupe consecutive repeats).
                    if self.input_history.last() != Some(&input) {
                        self.input_history.push(input.clone());
                    }
                    self.history_index = None;
                    self.process_input(input);
                }
            }
        } else if self.keymap.matches(key_event, Action::CancelInput) {
            // Any non-Tab key resets autocomplete state
            self.tab_matches.clear();
            self.tab_index = 0;
            // Close the command palette first, then other overlays
            if self.show_model_palette {
                self.show_model_palette = false;
                self.model_matches.clear();
                self.model_index = 0;
            } else if self.show_agent_palette {
                self.show_agent_palette = false;
                self.agent_matches.clear();
                self.agent_index = 0;
            } else if self.show_command_palette {
                self.show_command_palette = false;
                self.palette_matches.clear();
                self.palette_index = 0;
            } else if self.show_session_list {
                self.show_session_list = false;
            } else if self.show_agents {
                self.show_agents = false;
            } else if self.show_subagents {
                self.show_subagents = false;
            } else {
                // No overlay open — clear input
                self.input.clear();
                self.input_cursor = 0;
            }
        } else if let KeyCode::Char(c) = key {
            // Any non-Tab key resets autocomplete state
            self.tab_matches.clear();
            self.tab_index = 0;
            if self.kb_supported && modifiers.contains(KeyModifiers::SHIFT) {
                // Kitty protocol: shift is reported as a modifier;
                // apply keyboard-appropriate shift mapping
                self.input_insert_char(shift_char(c, &self.lang));
            } else {
                self.input_insert_char(c);
            }
            self.update_command_palette();
        }
    }

    /// Handle a key while the Chat window (1) has focus.
    fn handle_chat_key(&mut self, key: KeyCode, _modifiers: KeyModifiers, key_event: KeyEvent) {
        if self.keymap.matches(key_event, Action::ScrollUp) {
            self.chat_scroll = self.chat_scroll.saturating_sub(1);
        } else if self.keymap.matches(key_event, Action::ScrollDown) {
            self.chat_scroll = self.chat_scroll.saturating_add(1);
        } else if self.keymap.matches(key_event, Action::PageUp) {
            self.chat_scroll = self.chat_scroll.saturating_add(10);
        } else if self.keymap.matches(key_event, Action::PageDown) {
            self.chat_scroll = self.chat_scroll.saturating_sub(10);
        } else if self.keymap.matches(key_event, Action::ChatTop) {
            if key == KeyCode::Home || (key == KeyCode::Char('g') && self.is_double_g()) {
                // Home or gg: jump to the top of the chat.
                self.chat_scroll = u16::MAX;
            }
        } else if self.keymap.matches(key_event, Action::ChatBottom) {
            // End or G: jump to the bottom (auto-scroll).
            self.chat_scroll = 0;
        } else if self.keymap.matches(key_event, Action::CancelInput) {
            self.focus = Focus::Input;
        }
    }

    /// Handle a key while the MCPs sidebar panel (2) has focus.
    fn handle_mcp_panel_key(&mut self, key: KeyCode, modifiers: KeyModifiers, key_event: KeyEvent) {
        let len = self.unique_mcp_count();
        self.mcp_panel_index =
            self.handle_list_nav_key(key, modifiers, key_event, len, self.mcp_panel_index);
    }

    /// Handle a key while the Skills sidebar panel (3) has focus.
    fn handle_skill_panel_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        let len = self.unique_skill_count();
        self.skill_panel_index =
            self.handle_list_nav_key(key, modifiers, key_event, len, self.skill_panel_index);
    }

    /// Handle a key while the Agents sidebar panel (4) has focus.
    fn handle_agent_panel_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        let len = self.agent_panel_count();
        self.agent_panel_index =
            self.handle_list_nav_key(key, modifiers, key_event, len, self.agent_panel_index);
    }

    /// Shared Vim/arrow navigation for a list panel (MCPs, Skills, Agents).
    /// Returns the updated selection index.
    fn handle_list_nav_key(
        &mut self,
        key: KeyCode,
        _modifiers: KeyModifiers,
        key_event: KeyEvent,
        len: usize,
        mut index: usize,
    ) -> usize {
        if self.keymap.matches(key_event, Action::ListDown) {
            if len > 0 {
                index = (index + 1).min(len - 1);
            }
        } else if self.keymap.matches(key_event, Action::ListUp) {
            index = index.saturating_sub(1);
        } else if self.keymap.matches(key_event, Action::ListTop) {
            if key == KeyCode::Home || (key == KeyCode::Char('g') && self.is_double_g()) {
                index = 0;
            }
        } else if self.keymap.matches(key_event, Action::ListBottom) {
            if len > 0 {
                index = len - 1;
            }
        } else if self.keymap.matches(key_event, Action::CancelInput) {
            self.focus = Focus::Input;
        }
        index
    }

    /// Detect a double-'g' press (gg) within a short window.
    fn is_double_g(&mut self) -> bool {
        let now = Instant::now();
        let double = match self.last_g_press {
            Some(t) => now.duration_since(t) < std::time::Duration::from_millis(500),
            None => false,
        };
        self.last_g_press = Some(now);
        double
    }

    /// Reset the Tab-completion autocomplete state.
    ///
    /// Any non-Tab key that edits the input should clear the cached matches so
    /// the next Tab press recomputes them from the current input.
    fn reset_tab_state(&mut self) {
        self.tab_matches.clear();
        self.tab_index = 0;
    }

    /// Number of unique MCP servers shown in the MCPs sidebar panel.
    fn unique_mcp_count(&self) -> usize {
        self.agents
            .iter()
            .flat_map(|a| a.mcps.iter())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Number of unique skills shown in the Skills sidebar panel.
    fn unique_skill_count(&self) -> usize {
        self.agents
            .iter()
            .flat_map(|a| a.skills.iter())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Number of agents shown in the Agents sidebar panel (non-completed).
    fn agent_panel_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| a.status != AgentStatus::Completed)
            .count()
    }

    /// Convert a character index into a byte index within `input`.
    fn input_char_to_byte(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.input.len())
    }

    /// Insert a character at the cursor position and advance the cursor.
    fn input_insert_char(&mut self, c: char) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        self.input.insert(byte_idx, c);
        self.input_cursor += 1;
    }

    /// Delete the character before the cursor (Backspace).
    fn input_delete_before(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        let prev_len = self.input[..byte_idx]
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.input.replace_range(byte_idx - prev_len..byte_idx, "");
        self.input_cursor -= 1;
    }

    /// Delete the character at the cursor (Delete).
    fn input_delete_at(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        if byte_idx >= self.input.len() {
            return;
        }
        let next_len = self.input[byte_idx..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.input.replace_range(byte_idx..byte_idx + next_len, "");
    }

    /// Move the cursor to the start of the previous word.
    fn input_move_word_left(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        let before: Vec<char> = self.input[..byte_idx].chars().collect();
        let mut i = before.len();
        // Skip trailing whitespace.
        while i > 0 && before[i - 1].is_whitespace() {
            i -= 1;
        }
        // Skip the word.
        while i > 0 && !before[i - 1].is_whitespace() {
            i -= 1;
        }
        self.input_cursor = i;
    }

    /// Move the cursor to the start of the next word.
    fn input_move_word_right(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        let after: Vec<char> = self.input[byte_idx..].chars().collect();
        let mut i = 0;
        // Skip the current word.
        while i < after.len() && !after[i].is_whitespace() {
            i += 1;
        }
        // Skip whitespace.
        while i < after.len() && after[i].is_whitespace() {
            i += 1;
        }
        self.input_cursor = (self.input_cursor + i).min(self.input.chars().count());
    }

    /// Delete the word before the cursor (Ctrl+W).
    fn input_delete_word_before(&mut self) {
        let old_cursor = self.input_cursor;
        self.input_move_word_left();
        let new_cursor = self.input_cursor;
        let start_byte = self.input_char_to_byte(new_cursor);
        let end_byte = self.input_char_to_byte(old_cursor);
        self.input.replace_range(start_byte..end_byte, "");
        self.input_cursor = new_cursor;
    }

    /// Delete from the start of the line to the cursor (Ctrl+U).
    fn input_delete_to_start(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        self.input.replace_range(0..byte_idx, "");
        self.input_cursor = 0;
    }

    /// Delete from the cursor to the end of the line (Ctrl+K).
    fn input_delete_to_end(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        self.input.truncate(byte_idx);
    }

    /// Recompute the fuzzy command palette matches based on the current input.
    /// The palette opens whenever the input starts with `/`.
    fn update_command_palette(&mut self) {
        if !self.input.starts_with('/') {
            self.show_command_palette = false;
            self.palette_matches.clear();
            self.palette_index = 0;
            self.update_agent_palette();
            self.update_model_palette();
            return;
        }

        // `/agent` uses its own agent-selection combo instead of the command list.
        if self.input.starts_with("/agent") {
            self.show_command_palette = false;
            self.palette_matches.clear();
            self.palette_index = 0;
            self.update_agent_palette();
            self.update_model_palette();
            return;
        }

        // `/models` uses its own model-selection combo instead of the command list.
        if self.input.starts_with("/models") {
            self.show_command_palette = false;
            self.palette_matches.clear();
            self.palette_index = 0;
            self.update_agent_palette();
            self.update_model_palette();
            return;
        }

        self.show_agent_palette = false;
        self.agent_matches.clear();
        self.agent_index = 0;
        self.show_model_palette = false;
        self.model_matches.clear();
        self.model_index = 0;

        let query = self.input.trim_start_matches('/');
        let mut scored: Vec<(u32, String, usize)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(i, (cmd, _))| fuzzy_score(query, cmd).map(|s| (s, cmd.to_string(), i)))
            .collect();
        // Sort by score descending (best match first), then alphabetically by
        // command name so the combo is stable and predictable.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        self.palette_matches = scored.into_iter().map(|(_, _, i)| i).collect();
        self.show_command_palette = !self.palette_matches.is_empty();
        if self.palette_index >= self.palette_matches.len() {
            self.palette_index = 0;
        }
    }

    /// Fuzzy agent-selection combo for `/agent`. Only root agents are
    /// switchable, so only those are offered.
    fn update_agent_palette(&mut self) {
        if !self.input.starts_with("/agent") {
            self.show_agent_palette = false;
            self.agent_matches.clear();
            self.agent_index = 0;
            return;
        }

        // Query is the part after `/agent` (e.g. `/agent writ` → "writ").
        let query = self.input.trim_start_matches("/agent").trim_start();

        let mut scored: Vec<(u32, String)> = self
            .agents
            .iter()
            .filter(|a| a.role == AgentRole::Root)
            .map(|a| a.name.clone())
            .filter_map(|name| fuzzy_score(query, &name).map(|s| (s, name)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        self.agent_matches = scored.into_iter().map(|(_, n)| n).collect();
        self.show_agent_palette = !self.agent_matches.is_empty();
        if self.agent_index >= self.agent_matches.len() {
            self.agent_index = 0;
        }
    }

    /// Fuzzy model-selection combo for `/models`.
    fn update_model_palette(&mut self) {
        if !self.input.starts_with("/models") {
            self.show_model_palette = false;
            self.model_matches.clear();
            self.model_index = 0;
            return;
        }

        // Query is the part after `/models` (e.g. `/models gpt` → "gpt").
        let query = self.input.trim_start_matches("/models").trim_start();

        let mut scored: Vec<(u32, String)> = self
            .model_picker
            .all_models()
            .iter()
            .cloned()
            .filter_map(|name| fuzzy_score(query, &name).map(|s| (s, name)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        self.model_matches = scored.into_iter().map(|(_, n)| n).collect();
        self.show_model_palette = !self.model_matches.is_empty();
        if self.model_index >= self.model_matches.len() {
            self.model_index = 0;
        }
    }

    /// Process a line of input — check for slash commands or send to engine.
    fn process_input(&mut self, input: String) {
        // Commit any in-progress stream first so whatever the user does next
        // (slash command, shell, or message) is ordered after the previous
        // assistant response.
        self.commit_stream();
        if input.starts_with('/') {
            self.handle_command(input);
        } else if let Some(cmd) = input.strip_prefix('!') {
            let cmd = cmd.trim().to_string();
            self.push_msg(format!("$ {}", cmd));
            self.chat_scroll = 0;
            // Run synchronously; shell commands are typically fast
            match std::process::Command::new("sh").args(["-c", &cmd]).output() {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    for line in stdout.split('\n') {
                        self.push_msg(format!("\u{2502} {}", line));
                    }
                    if !stderr.is_empty() {
                        for line in stderr.split('\n') {
                            self.push_msg(format!("\u{2514} {}", line));
                        }
                    }
                }
                Err(e) => {
                    self.push_msg(format!("Error: !command failed: {}", e));
                }
            }
            self.chat_scroll = 0;
        } else {
            let msg = format!("> {}", input);
            self.push_msg(msg);
            let cmd = EngineCommand::UserInput(input);
            let _ = self.cmd_tx.try_send(cmd);
        }
    }

    /// Handle a slash command.
    fn handle_command(&mut self, input: String) {
        let parts: Vec<&str> = input.splitn(3, ' ').collect();
        let cmd = parts[0];

        // Dispatch custom slash commands defined in config before built-ins.
        if let Some(cc) = self.custom_commands.iter().find(|c| c.name == cmd) {
            let args = parts.get(1).copied().unwrap_or("");
            let env = std::env::vars().collect::<HashMap<_, _>>();
            let expanded = expand_vars(&cc.template, &env);
            let final_input = if args.is_empty() {
                expanded
            } else {
                format!("{} {}", expanded, args)
            };
            self.push_msg(format!("> {}", cmd));
            let _ = self.cmd_tx.try_send(EngineCommand::UserInput(final_input));
            return;
        }

        match cmd {
            "/sessions" | "/s" => {
                self.push_msg("> /sessions");
                let _ = self.cmd_tx.try_send(EngineCommand::ListSessions);
            }
            "/new" => {
                let name = parts.get(1).unwrap_or(&"default");
                self.push_msg(format!("> /new {}", name));
                let _ = self
                    .cmd_tx
                    .try_send(EngineCommand::NewSession(name.to_string()));
            }
            "/resume" | "/r" => {
                if let Some(id) = parts.get(1) {
                    self.push_msg(format!("> /resume {}", id));
                    let _ = self
                        .cmd_tx
                        .try_send(EngineCommand::ResumeSession(id.to_string()));
                } else {
                    self.push_msg("Usage: /resume <session-id>");
                }
            }
            "/delete" | "/d" => {
                if let Some(id) = parts.get(1) {
                    self.push_msg(format!("> /delete {}", id));
                    let _ = self
                        .cmd_tx
                        .try_send(EngineCommand::DeleteSession(id.to_string()));
                } else {
                    self.push_msg("Usage: /delete <session-id>");
                }
            }
            "/rename" => {
                if let (Some(id), Some(name)) = (parts.get(1), parts.get(2)) {
                    self.push_msg(format!("> /rename {} {}", id, name));
                    let _ = self.cmd_tx.try_send(EngineCommand::RenameSession(
                        id.to_string(),
                        name.to_string(),
                    ));
                } else {
                    self.messages
                        .push("Usage: /rename <session-id> <new-name>".into());
                }
            }
            // ── Session pinning (FASE 4.5) ─────────────────────────
            "/pin" => {
                if let Some(id) = parts.get(1) {
                    self.push_msg(format!("> /pin {}", id));
                    let _ = self.cmd_tx.try_send(EngineCommand::SetSessionPinned {
                        id: id.to_string(),
                        pinned: true,
                    });
                } else {
                    self.push_msg("Usage: /pin <session-id>");
                }
            }
            "/unpin" => {
                if let Some(id) = parts.get(1) {
                    self.push_msg(format!("> /unpin {}", id));
                    let _ = self.cmd_tx.try_send(EngineCommand::SetSessionPinned {
                        id: id.to_string(),
                        pinned: false,
                    });
                } else {
                    self.push_msg("Usage: /unpin <session-id>");
                }
            }
            // ── Prompt queue (FASE 4.6) ────────────────────────────
            "/queue" => {
                self.push_msg("> /queue");
                self.show_prompt_queue = true;
                self.prompt_queue_index = 0;
            }
            "/enqueue" => {
                let text = parts.get(1).unwrap_or(&"").trim();
                if text.is_empty() {
                    self.push_msg("Usage: /enqueue <prompt text>");
                } else {
                    self.prompt_queue.push(text.to_string());
                    self.push_msg(format!("> /enqueue ({} en cola)", self.prompt_queue.len()));
                }
            }
            // ── Agent info commands ────────────────────────────────
            "/agent" => {
                let name = parts.get(1).unwrap_or(&"").trim();
                if name.is_empty() {
                    self.push_msg("Usage: /agent <agent-name>");
                } else {
                    self.push_msg(format!("> /agent {}", name));
                    let _ = self
                        .cmd_tx
                        .try_send(EngineCommand::SwitchAgent(name.to_string()));
                }
            }
            "/agents" | "/a" => {
                self.push_msg("> /agents");
                self.show_agents = !self.show_agents;
                if self.show_agents {
                    self.close_panels();
                    self.show_agents = true;
                }
            }
            "/subagents" | "/sa" => {
                self.push_msg("> /subagents");
                self.show_subagents = !self.show_subagents;
                if self.show_subagents {
                    self.close_panels();
                    self.show_subagents = true;
                }
            }
            "/copy" => {
                self.push_msg("> /copy");
                let content = match parts.get(1).and_then(|n| n.parse::<usize>().ok()) {
                    Some(n) => self
                        .messages
                        .iter()
                        .rev()
                        .take(n)
                        .rev()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None => self.messages.join("\n"),
                };
                match copy_to_clipboard(&content) {
                    Ok(()) => {
                        self.push_msg(format!(
                            "Chat copied to clipboard ({} lines).",
                            self.messages.len()
                        ));
                    }
                    Err(e) => {
                        self.push_msg(format!("Error copying chat: {}", e));
                    }
                }
            }
            "/export-editor" | "/ee" => {
                self.push_msg("> /export-editor");
                let content = self.messages.join("\n");
                let tmp = std::env::temp_dir()
                    .join(format!("anacleto-export-{}.txt", std::process::id()));
                if let Err(e) = std::fs::write(&tmp, &content) {
                    self.push_msg(format!("Error writing export: {}", e));
                } else {
                    self.open_file_in_editor(&tmp);
                    self.push_msg(format!(
                        "Export opened in editor ({} lines).",
                        self.messages.len()
                    ));
                }
            }
            "/compact" | "/c" => {
                self.push_msg("> /compact");
                let _ = self.cmd_tx.try_send(EngineCommand::Compact);
            }
            "/debug" => {
                self.debug_mode = !self.debug_mode;
                self.push_msg(format!(
                    "> /debug — debug mode {}",
                    if self.debug_mode { "ON" } else { "OFF" }
                ));
                let _ = self
                    .cmd_tx
                    .try_send(EngineCommand::SetDebug(self.debug_mode));
            }
            "/models" => match parts.get(1) {
                Some(model) => {
                    self.messages
                        .push(format!("> /models — changing to {}", model));
                    let _ = self
                        .cmd_tx
                        .try_send(EngineCommand::SetModel(model.to_string()));
                }
                None => {
                    self.push_msg("Usage: /models <model-name>");
                }
            },
            "/exit" | "/quit" => {
                self.push_msg("> /exit");
                self.should_exit = true;
            }
            "/help" | "/h" => {
                self.push_msg("> /help");
                self.push_msg(
                    "Commands: /sessions, /new <name>, /resume <id>, /delete <id>, \
                     /rename <id> <name>, /agents, /subagents, /debug, /copy, /compact, /models, /exit, /help",
                );
            }
            // ── OpenCode-style slash commands ────────────────────────
            "/undo" => {
                self.push_msg("> /undo");
                let _ = self.cmd_tx.try_send(EngineCommand::Undo);
            }
            "/redo" => {
                self.push_msg("> /redo");
                let _ = self.cmd_tx.try_send(EngineCommand::Redo);
            }
            "/fork" => {
                self.push_msg("> /fork");
                let _ = self.cmd_tx.try_send(EngineCommand::Fork);
            }
            "/export" => {
                self.push_msg("> /export");
                let path = parts.get(1).map(|p| PathBuf::from(p.to_string()));
                let format = parts.get(2).map(|f| match *f {
                    "md" | "markdown" => ExportFormat::Markdown,
                    _ => ExportFormat::Json,
                });
                let _ = self.cmd_tx.try_send(EngineCommand::Export { path, format });
            }
            "/import" => {
                if let Some(p) = parts.get(1) {
                    self.push_msg(format!("> /import {}", p));
                    let _ = self.cmd_tx.try_send(EngineCommand::Import {
                        path: PathBuf::from(p.to_string()),
                    });
                } else {
                    self.push_msg("Usage: /import <path>");
                }
            }
            "/share" => {
                self.push_msg("> /share");
                let _ = self.cmd_tx.try_send(EngineCommand::Share);
            }
            "/unshare" => {
                self.push_msg("> /unshare");
                let _ = self.cmd_tx.try_send(EngineCommand::Unshare);
            }
            "/skills" => {
                self.push_msg("> /skills");
                let _ = self.cmd_tx.try_send(EngineCommand::ListSkills);
            }
            "/mcps" => match (parts.get(1), parts.get(2)) {
                (Some(name), Some(state)) => {
                    let enabled = matches!(*state, "on" | "enable" | "1" | "true");
                    self.push_msg(format!(
                        "> /mcps {} {}",
                        name,
                        if enabled { "on" } else { "off" }
                    ));
                    let _ = self.cmd_tx.try_send(EngineCommand::ToggleMcp {
                        name: name.to_string(),
                        enabled,
                    });
                }
                _ => {
                    self.push_msg("> /mcps");
                    self.close_panels();
                    self.show_mcps = true;
                    let _ = self.cmd_tx.try_send(EngineCommand::ListMcps);
                }
            },
            "/status" => {
                self.push_msg("> /status");
                let _ = self.cmd_tx.try_send(EngineCommand::Status);
            }
            "/init" => {
                self.push_msg("> /init");
                self.init_flow = Some(InitFlow {
                    step: 0,
                    name: String::new(),
                    description: String::new(),
                    stack: String::new(),
                });
            }
            "/review" => {
                let target = parts.get(1).map(|t| t.to_string());
                self.push_msg("> /review");
                let _ = self.cmd_tx.try_send(EngineCommand::Review { target });
            }
            "/warp" => {
                if let Some(dir) = parts.get(1) {
                    self.push_msg(format!("> /warp {}", dir));
                    let _ = self.cmd_tx.try_send(EngineCommand::Warp {
                        dir: PathBuf::from(dir.to_string()),
                    });
                } else {
                    self.push_msg("Usage: /warp <directory>");
                }
            }
            "/workspaces" => {
                self.push_msg("> /workspaces");
                let _ = self.cmd_tx.try_send(EngineCommand::ListWorkspaces);
            }
            "/move" => {
                if let Some(ws) = parts.get(1) {
                    self.push_msg(format!("> /move {}", ws));
                    let _ = self.cmd_tx.try_send(EngineCommand::MoveSession {
                        workspace: ws.to_string(),
                    });
                } else {
                    self.push_msg("Usage: /move <workspace>");
                }
            }
            "/timeline" => {
                self.push_msg("> /timeline");
                self.close_panels();
                self.show_timeline = true;
                let _ = self.cmd_tx.try_send(EngineCommand::Timeline);
            }
            "/worktree" => match parts.get(1).map(|s| s.to_string()) {
                Some(sub) if sub == "list" => {
                    self.push_msg("> /worktree list");
                    let _ = self.cmd_tx.try_send(EngineCommand::WorktreeList);
                }
                Some(sub) if sub == "add" => {
                    let path = parts.get(2).map(|s| s.to_string());
                    let branch = parts.get(3).map(|s| s.to_string());
                    match path {
                        Some(p) => {
                            self.push_msg(format!("> /worktree add {}", p));
                            let _ = self
                                .cmd_tx
                                .try_send(EngineCommand::WorktreeAdd { path: p, branch });
                        }
                        None => self.push_msg("Usage: /worktree add <path> [branch]"),
                    }
                }
                Some(sub) if sub == "remove" => {
                    let path = parts.get(2).map(|s| s.to_string());
                    match path {
                        Some(p) => {
                            self.push_msg(format!("> /worktree remove {}", p));
                            let _ = self
                                .cmd_tx
                                .try_send(EngineCommand::WorktreeRemove { path: p });
                        }
                        None => self.push_msg("Usage: /worktree remove <path>"),
                    }
                }
                _ => self.push_msg("Usage: /worktree add|list|remove"),
            },
            "/themes" => {
                self.theme = self.theme.next();
                self.push_msg(format!("> /themes — theme: {}", self.theme.name()));
            }
            "/timestamps" => {
                self.show_timestamps = !self.show_timestamps;
                self.push_msg(format!(
                    "> /timestamps — {}",
                    if self.show_timestamps { "ON" } else { "OFF" }
                ));
            }
            "/thinking" => {
                self.show_thinking = !self.show_thinking;
                self.push_msg(format!(
                    "> /thinking — {}",
                    if self.show_thinking { "ON" } else { "OFF" }
                ));
            }
            "/stash" => match parts.get(1) {
                Some(&"pop") => {
                    if let Some(saved) = self.stash_stack.pop() {
                        self.input = saved;
                        self.input_cursor = self.input.chars().count();
                        self.push_msg("> /stash pop — restored prompt.");
                    } else {
                        self.push_msg("> /stash pop — nothing stashed.");
                    }
                }
                Some(&"list") => {
                    self.push_msg(format!(
                        "> /stash list — {} stashed:",
                        self.stash_stack.len()
                    ));
                    let items: Vec<String> = self.stash_stack.to_vec();
                    for (i, s) in items.iter().enumerate() {
                        self.push_msg(format!("  [{}] {}", i, s));
                    }
                }
                _ => {
                    if self.input.trim().is_empty() {
                        self.push_msg("> /stash — nothing to stash.");
                    } else {
                        self.stash_stack.push(self.input.clone());
                        self.input.clear();
                        self.input_cursor = 0;
                        self.push_msg(format!(
                            "> /stash — saved ({} stashed).",
                            self.stash_stack.len()
                        ));
                    }
                }
            },
            "/editor" => {
                self.push_msg("> /editor");
                self.open_editor();
            }
            // ── FASE 1 y 2: build, jobs y snapshots ────────────────
            "/build" => {
                self.push_msg("> /build");
                let _ = self.cmd_tx.try_send(EngineCommand::Build);
            }
            "/jobs" => {
                self.push_msg("> /jobs");
                let _ = self.cmd_tx.try_send(EngineCommand::ListJobs);
            }
            "/parent" => {
                self.push_msg("> /parent");
                let _ = self.cmd_tx.try_send(EngineCommand::Parent);
            }
            "/children" => {
                self.push_msg("> /children");
                let _ = self.cmd_tx.try_send(EngineCommand::Children);
            }
            "/snapshot" => {
                let name = parts.get(1).map(|s| s.to_string());
                self.push_msg(format!("> /snapshot {}", name.as_deref().unwrap_or("")));
                let _ = self.cmd_tx.try_send(EngineCommand::Snapshot { name });
            }
            "/revert" => match parts.get(1).and_then(|s| s.parse::<uuid::Uuid>().ok()) {
                Some(snapshot_id) => {
                    self.push_msg(format!("> /revert {}", snapshot_id));
                    let _ = self.cmd_tx.try_send(EngineCommand::Revert { snapshot_id });
                }
                None => self.push_msg("Usage: /revert <snapshot-id>"),
            },
            "/stage" => {
                let name = parts.get(1).map(|s| s.to_string());
                self.push_msg(format!("> /stage {}", name.as_deref().unwrap_or("")));
                let _ = self.cmd_tx.try_send(EngineCommand::Stage { name });
            }
            "/clear" => {
                self.push_msg("> /clear");
                let _ = self.cmd_tx.try_send(EngineCommand::Clear);
            }
            "/commit" => {
                let name = parts.get(1).map(|s| s.to_string());
                self.push_msg(format!("> /commit {}", name.as_deref().unwrap_or("")));
                let _ = self.cmd_tx.try_send(EngineCommand::Commit { name });
            }
            _ => {
                self.messages
                    .push(format!("Unknown command: {}. Try /help", cmd));
            }
        }
    }

    /// Open the external editor ($EDITOR) with the current input buffer.
    fn open_editor(&mut self) {
        let tmp = std::env::temp_dir().join(format!("anacleto-edit-{}.txt", std::process::id()));
        if std::fs::write(&tmp, &self.input).is_err() {
            self.push_msg("Error: could not write temp file for editor".to_string());
            return;
        }
        self.open_file_in_editor(&tmp);
        if let Ok(contents) = std::fs::read_to_string(&tmp) {
            self.input = contents.trim_end_matches('\n').to_string();
            self.input_cursor = self.input.chars().count();
        }
        let _ = std::fs::remove_file(&tmp);
    }

    /// Open an arbitrary file in the external editor, suspending raw mode.
    fn open_file_in_editor(&mut self, path: &std::path::Path) {
        let editor = self
            .editor
            .clone()
            .or_else(|| std::env::var("EDITOR").ok())
            .or_else(|| std::env::var("VISUAL").ok())
            .unwrap_or_else(|| "vi".to_string());
        // Suspend raw mode and leave the alternate screen so the editor
        // can take over the terminal cleanly.
        let suspended =
            disable_raw_mode().is_ok() && execute!(std::io::stdout(), LeaveAlternateScreen).is_ok();
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{} \"{}\"", editor, path.display()))
            .status();
        // Restore the terminal before reporting the result.
        if suspended {
            let _ = execute!(std::io::stdout(), EnterAlternateScreen);
            let _ = enable_raw_mode();
        }
        if let Err(e) = status {
            self.push_msg(format!("Error launching editor: {}", e));
        }
    }

    /// Resume the pinned session at the given quick-slot index (0-based).
    fn resume_quick_slot(&mut self, index: usize) {
        let pinned: Vec<&SessionSummary> = self.session_list.iter().filter(|s| s.pinned).collect();
        if let Some(session) = pinned.get(index) {
            let id = session.id;
            self.push_msg(format!("> quick-slot {}: resume {}", index + 1, id));
            let _ = self
                .cmd_tx
                .try_send(EngineCommand::ResumeSession(id.to_string()));
        } else {
            self.push_msg(format!("No pinned session in quick slot {}", index + 1));
        }
    }

    /// Advance the `/init` flow with the current input buffer.
    fn collect_init_answer(&mut self) {
        let Some(mut flow) = self.init_flow.take() else {
            return;
        };
        let answer = std::mem::take(&mut self.input);
        match flow.step {
            0 => flow.name = answer,
            1 => flow.description = answer,
            _ => flow.stack = answer,
        }
        if flow.step < 2 {
            flow.step += 1;
            self.init_flow = Some(flow);
        } else {
            let answers = InitAnswers {
                name: flow.name,
                description: flow.description,
                stack: flow.stack,
            };
            let _ = self.cmd_tx.try_send(EngineCommand::Init { answers });
        }
    }

    /// Jump to a timeline entry (scroll chat to it).
    fn jump_to_timeline_entry(&mut self) {
        if let Some(entry) = self.timeline.get(self.timeline_index) {
            let needle = format!("{}: {}", entry.role, entry.content);
            if let Some(pos) = self
                .messages
                .iter()
                .position(|m| m.contains(&entry.content))
            {
                let total = self.messages.len() as u16;
                self.chat_scroll = total.saturating_sub(pos as u16);
            }
            self.show_timeline = false;
            self.push_msg(format!("> /timeline — jumped to {}", needle));
        }
    }

    /// Toggle the selected MCP server on/off.
    fn toggle_selected_mcp(&mut self) {
        if let Some(mcp) = self.mcps_list.get(self.mcps_index) {
            let name = mcp.name.clone();
            let enabled = !mcp.enabled;
            let _ = self
                .cmd_tx
                .try_send(EngineCommand::ToggleMcp { name, enabled });
        }
    }

    /// Close all overlay panels (session list, agents, subagents, timeline, mcps).
    fn close_panels(&mut self) {
        self.show_session_list = false;
        self.show_agents = false;
        self.show_subagents = false;
        self.show_timeline = false;
        self.show_mcps = false;
    }

    /// Whether a key event should be dispatched through the keymap.
    ///
    /// Special keys (Enter, Esc, PageUp, ...) and modified keys (Ctrl+...) are
    /// always dispatched. Plain character keys are only dispatched when the
    /// input buffer is empty, so that typing normally is never intercepted.
    fn keymap_applies(&self, key_event: KeyEvent) -> bool {
        match key_event.code {
            KeyCode::Char(_) => {
                // In the Input window, plain characters are always typed and never
                // trigger global actions, even with an empty buffer.
                if self.focus == Focus::Input && key_event.modifiers == KeyModifiers::NONE {
                    return false;
                }
                key_event.modifiers != KeyModifiers::NONE || self.input.is_empty()
            }
            _ => true,
        }
    }
}

/// Run the TUI event loop.
pub async fn run_tui<B: ratatui::backend::Backend<Error = std::io::Error>>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> std::io::Result<()> {
    loop {
        // Drain ALL pending engine events BEFORE drawing, so the render
        // always shows the latest state (not the state from the previous cycle)
        loop {
            match app.event_rx.try_recv() {
                Ok(event) => {
                    app.handle_event(event);
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return Ok(());
                }
                Err(_) => break, // Empty, continue to draw
            }
        }

        // Draw the UI (now with up-to-date state)
        app.frame_count = app.frame_count.wrapping_add(1);
        app.toasts.tick(Instant::now());
        terminal.draw(|f| render(f, app))?;

        // Check for keyboard input (with timeout for responsiveness)
        if event::poll(std::time::Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key.code, key.modifiers);
        }

        if app.should_exit {
            break;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the TUI.
fn render(f: &mut Frame, app: &mut App) {
    // Hide welcome banner once there's actual content
    if !app.messages.is_empty() || app.current_stream.is_some() {
        app.show_welcome = false;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(1), // status bar
                Constraint::Min(1),    // main content
                Constraint::Length(4), // input
                Constraint::Length(1), // working directory
            ]
            .as_ref(),
        )
        .split(f.area());

    render_status_bar(f, chunks[0], app);
    render_main_content(f, chunks[1], app);
    render_input(f, chunks[2], app);
    render_working_dir(f, chunks[3], app);

    // Render the fuzzy command palette above the input if open.
    if app.show_command_palette && !app.palette_matches.is_empty() {
        render_command_palette(f, chunks[2], app);
    }
    // Render the agent-selection combo above the input if open.
    if app.show_agent_palette && !app.agent_matches.is_empty() {
        render_agent_palette(f, chunks[2], app);
    }
    // Render the model-selection combo above the input if open.
    if app.show_model_palette && !app.model_matches.is_empty() {
        render_model_palette(f, chunks[2], app);
    }

    // Render approval dialog on top if pending
    if app.pending_approval.is_some() {
        render_approval_dialog(f, f.area(), app);
    }

    // Render inline question dialog on top if pending
    if app.pending_question.is_some() {
        render_question_dialog(f, f.area(), app);
    }

    // Render the which-key popup on top if visible.
    app.which_key.render(f, f.area());

    // Render the diff viewer and model picker overlays if visible.
    app.diff_viewer.render(f, f.area());
    app.model_picker.render(f, f.area());

    // Render the prompt queue popup if visible.
    render_prompt_queue(f, f.area(), app);

    // Render transient toasts in the bottom-right corner.
    app.toasts.render(f, f.area());
}

/// Render the prompt queue popup (FASE 4.6).
fn render_prompt_queue(f: &mut Frame, area: Rect, app: &App) {
    if !app.show_prompt_queue {
        return;
    }
    let items: Vec<ListItem> = app
        .prompt_queue
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == app.prompt_queue_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(
                format!("{:>2}. {}", i + 1, p),
                style,
            )))
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Cola de prompts "),
    );
    let popup = Rect {
        x: area.width.saturating_sub(60) / 2,
        y: area.height.saturating_sub(20) / 2,
        width: 60.min(area.width),
        height: 20.min(area.height),
    };
    f.render_widget(Clear, popup);
    f.render_widget(list, popup);
}

/// Render the top status bar with agent/session info.
fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let root_count = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .count();
    let subagent_count = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::SubAgent)
        .count();
    let active_count = app
        .agents
        .iter()
        .filter(|a| a.status == AgentStatus::Working)
        .count();

    let skill_count: usize = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .map(|a| a.skills.len())
        .sum();
    let mcp_count: usize = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .map(|a| a.mcps.len())
        .sum();

    let session_label = app.session_id.as_deref().unwrap_or("-");
    let mut all_spans: Vec<Span<'static>> = Vec::with_capacity(16);
    all_spans.push(Span::styled(
        " ⬡ anacleto ",
        Style::default()
            .fg(app.theme.accent())
            .add_modifier(Modifier::BOLD),
    ));
    // Keyboard protocol indicator
    if app.kb_supported {
        all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
        all_spans.push(Span::styled(
            " ⌨ ",
            Style::default().fg(Color::Rgb(100, 200, 100)),
        ));
    }
    all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    all_spans.push(Span::styled(
        format!(" {}:{} ", app.session_name, session_label),
        Style::default().fg(Color::Cyan),
    ));
    all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));

    // Active agent indicator
    if !app.active_agent.is_empty() {
        all_spans.push(Span::styled(
            format!(" @{} ", app.active_agent),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
        all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    }

    // Debug mode indicator
    if app.debug_mode {
        all_spans.push(Span::styled(
            " \u{1f41b} DEBUG ",
            Style::default()
                .fg(Color::Rgb(255, 180, 50))
                .add_modifier(Modifier::BOLD),
        ));
        all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    }

    all_spans.push(Span::styled(
        format!(" {}a ", root_count),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ));
    all_spans.push(Span::styled(
        format!("{}sa ", subagent_count),
        Style::default().fg(Color::Yellow),
    ));
    all_spans.push(Span::styled(
        format!("{}⚡ ", active_count),
        Style::default()
            .fg(Color::Rgb(255, 215, 0))
            .add_modifier(Modifier::BOLD),
    ));

    // Right-aligned segment: compute padding
    let left_width: u16 = all_spans.iter().map(|s| s.width() as u16).sum::<u16>() + 2; // leading + trailing spaces
    let right_items = vec![
        Span::styled(
            format!(" ⚙ {} ", skill_count),
            Style::default().fg(Color::Rgb(100, 200, 100)),
        ),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" 🔌 {} ", mcp_count),
            Style::default().fg(Color::Rgb(180, 130, 255)),
        ),
    ];
    let right_width: u16 = right_items.iter().map(|s| s.width() as u16).sum::<u16>();

    let pad = area.width.saturating_sub(left_width + right_width + 2);
    all_spans.push(Span::raw(" ".repeat(pad as usize)));
    all_spans.extend(right_items);
    all_spans.push(Span::styled(" ", Style::default()));

    let bar = Line::from(all_spans);

    let paragraph =
        Paragraph::new(bar).style(Style::default().bg(Color::Rgb(25, 25, 35)).fg(Color::White));
    f.render_widget(paragraph, area);
}

/// Render the main content area: left (chat/overlays) and right (status panels).
fn render_main_content(f: &mut Frame, area: Rect, app: &App) {
    if app.show_sidebar {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)].as_ref())
            .split(area);

        render_left_panel(f, chunks[0], app);
        render_right_panels(f, chunks[1], app);
    } else {
        // Sidebar hidden: left panel takes the full width.
        render_left_panel(f, area, app);
    }
}

/// Render the left panel: session list, agent list, subagent tree, or chat.
fn render_left_panel(f: &mut Frame, area: Rect, app: &App) {
    if app.show_timeline {
        render_timeline_panel(f, area, app);
    } else if app.show_mcps {
        render_mcp_list_panel(f, area, app);
    } else if app.show_session_list {
        render_session_list(f, area, app);
    } else if app.show_agents {
        render_agent_list(f, area, app);
    } else if app.show_subagents {
        render_subagent_tree(f, area, app);
    } else {
        render_chat(f, area, app);
    }
}

/// Render the session timeline panel (`/timeline`).
fn render_timeline_panel(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .timeline
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let label = format!(
                "{} {}: {}",
                e.created_at.format("%H:%M:%S"),
                e.role,
                e.content.chars().take(60).collect::<String>()
            );
            let style = if i == app.timeline_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(app.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(app.theme.accent()))
                .title(" Timeline "),
        )
        .highlight_style(Style::default().bg(app.theme.accent()));
    f.render_widget(list, area);
}

/// Render the MCP server list panel (`/mcps`).
fn render_mcp_list_panel(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .mcps_list
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let state = if m.enabled { "● ON" } else { "○ OFF" };
            let label = format!("{} {}", state, m.name);
            let style = if i == app.mcps_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(app.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(app.theme.accent()))
                .title(" MCP Servers "),
        )
        .highlight_style(Style::default().bg(app.theme.accent()));
    f.render_widget(list, area);
}

/// Render the right panel: 4 stacked info panels (Status, MCPs, Skills, Running agents).
fn render_right_panels(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(6),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ]
            .as_ref(),
        )
        .split(area);

    render_status_panel(f, chunks[0], app);
    render_mcp_panel(f, chunks[1], app);
    render_skill_panel(f, chunks[2], app);
    render_agent_panel(f, chunks[3], app);
}

/// Panel 1: Status — tokens, coste y contexto en tres líneas.
fn render_status_panel(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(1)].as_ref())
        .split(area);

    let text = format!(
        "Tokens: {}\nCost: ${:.2}\nContext: {:.1}% ({} / {})",
        format_tokens(app.total_tokens),
        app.total_cost,
        app.context_window_pct,
        format_tokens(app.total_tokens),
        format_tokens(app.context_window)
    );

    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Status "),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, chunks[0]);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Rgb(20, 40, 60)))
        .percent((app.context_window_pct.min(100.0)) as u16)
        .label(format!("Context: {:.1}%", app.context_window_pct));
    f.render_widget(gauge, chunks[1]);
}

/// Format a token count as thousands (K) or millions (M).
fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Fuzzy-match `query` against `candidate` (case-insensitive subsequence).
/// Returns a score if every character of `query` appears in order in
/// `candidate`, or `None` otherwise. Higher scores rank better: consecutive
/// matches, matches near the start, and shorter candidates are preferred.
fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let c: Vec<char> = candidate.chars().flat_map(|c| c.to_lowercase()).collect();
    if q.is_empty() {
        return Some(0);
    }

    let mut qi = 0;
    let mut score = 0u32;
    let mut prev: Option<usize> = None;
    for (ci, &ch) in c.iter().enumerate() {
        if qi < q.len() && ch == q[qi] {
            // Bonus for consecutive matches.
            score += match prev {
                Some(p) if ci == p + 1 => 8,
                _ => 2,
            };
            // Bonus for a match at the very start of the candidate.
            if ci == 0 {
                score += 5;
            }
            prev = Some(ci);
            qi += 1;
        }
    }
    if qi == q.len() {
        // Prefer shorter candidates (fewer extra characters).
        score += (c.len() as u32).saturating_sub(q.len() as u32).max(1);
        Some(score)
    } else {
        None
    }
}

/// Panel 2: MCPs — connected MCP server names.
fn render_mcp_panel(f: &mut Frame, area: Rect, app: &App) {
    let unique_mcps: Vec<&str> = {
        let set: std::collections::BTreeSet<&str> = app
            .agents
            .iter()
            .flat_map(|a| a.mcps.iter().map(|s| s.as_str()))
            .collect();
        set.into_iter().collect()
    };

    let focused = app.focus == Focus::Mcps;
    let border_color = if focused {
        app.theme.accent()
    } else {
        Color::Magenta
    };

    let items: Vec<ListItem> = if unique_mcps.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(none)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        unique_mcps
            .iter()
            .enumerate()
            .map(|(i, mcp)| {
                let style = if focused && i == app.mcp_panel_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(app.theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(*mcp, style)))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(" (2) MCPs "),
    );

    f.render_widget(list, area);
}

/// Panel 3: Skills — loaded skill names.
fn render_skill_panel(f: &mut Frame, area: Rect, app: &App) {
    let unique_skills: Vec<&str> = {
        let set: std::collections::BTreeSet<&str> = app
            .agents
            .iter()
            .flat_map(|a| a.skills.iter().map(|s| s.as_str()))
            .collect();
        set.into_iter().collect()
    };

    let focused = app.focus == Focus::Skills;
    let border_color = if focused {
        app.theme.accent()
    } else {
        Color::Green
    };

    let items: Vec<ListItem> = if unique_skills.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(none)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        unique_skills
            .iter()
            .enumerate()
            .map(|(i, skill)| {
                let style = if focused && i == app.skill_panel_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(app.theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(*skill, style)))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(" (3) Skills "),
    );

    f.render_widget(list, area);
}

/// Spinner animation frames (Braille dots).
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Panel 4: Running agents — agents with Working status.
fn render_agent_panel(f: &mut Frame, area: Rect, app: &App) {
    let display_agents: Vec<&AgentInfo> = app
        .agents
        .iter()
        .filter(|a| a.status != AgentStatus::Completed)
        .collect();

    let focused = app.focus == Focus::Agents;
    let border_color = if focused {
        app.theme.accent()
    } else {
        Color::Yellow
    };

    let items: Vec<ListItem> = if display_agents.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(none)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        display_agents
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let selected = focused && i == app.agent_panel_index;
                let active = a.name == app.active_agent;
                let (dot, dot_color) = match &a.status {
                    AgentStatus::Working => ("🟢", Color::Green),
                    AgentStatus::Idle => ("⏸", Color::Yellow),
                    AgentStatus::WaitingForSubAgent => ("⏳", Color::Blue),
                    AgentStatus::Completed => ("✅", Color::DarkGray),
                    AgentStatus::Error(_) => ("❌", Color::Red),
                };
                let role = match a.role {
                    AgentRole::Root => "Root",
                    AgentRole::SubAgent => "SubAgent",
                };
                let status_str = match &a.status {
                    AgentStatus::Working => "working",
                    AgentStatus::Idle => "idle",
                    AgentStatus::WaitingForSubAgent => "waiting",
                    AgentStatus::Completed => "done",
                    AgentStatus::Error(_) => "error",
                };
                let item_style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(app.theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if active { "▶ " } else { "  " },
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {} ", dot),
                        Style::default().fg(dot_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        &a.name,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                            .bg(if active { Color::Magenta } else { Color::Reset }),
                    ),
                    if a.status == AgentStatus::Working {
                        Span::styled(
                            format!(
                                " {}",
                                SPINNER_FRAMES[(app.frame_count as usize) % SPINNER_FRAMES.len()]
                            ),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::raw("")
                    },
                    Span::styled(format!(" [{}]", role), Style::default().fg(Color::DarkGray)),
                    Span::styled(format!(" ({})", status_str), Style::default().fg(dot_color)),
                ]))
                .style(item_style)
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(" (4) Agents "),
    );

    f.render_widget(list, area);
}

/// Render the current working directory (left) and active model (right).
/// When the directory path is too long, it is truncated with an ellipsis (...)
/// to ensure the model name always fits on the right side.
fn render_working_dir(f: &mut Frame, area: Rect, app: &App) {
    let dir_text = format!(" 📁 {}", app.working_dir);
    let model_text = format!("🤖 {}", app.current_model);
    let width = area.width as usize;

    // Use display width (emoji count as 2 columns) so the model ends exactly
    // at the right edge of the terminal.
    let dir_width = dir_text.width();
    let model_width = model_text.width();
    // Leave at least 1 space between dir and model
    let max_dir_width = width.saturating_sub(model_width + 1);

    let truncated_dir = if dir_width > max_dir_width && max_dir_width > 3 {
        // Truncate with ellipsis at the end
        let keep = max_dir_width.saturating_sub(1);
        let mut s: String = String::new();
        let mut w = 0;
        for ch in dir_text.chars() {
            let cw = ch.to_string().width();
            if w + cw > keep {
                break;
            }
            s.push(ch);
            w += cw;
        }
        s.push('…');
        s
    } else {
        dir_text
    };

    let padding = width.saturating_sub(truncated_dir.width() + model_width);
    let line = Line::from(vec![
        Span::styled(truncated_dir, Style::default().fg(Color::DarkGray)),
        Span::raw(" ".repeat(padding)),
        Span::styled(model_text, Style::default().fg(Color::Cyan)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
fn render_chat(f: &mut Frame, area: Rect, app: &App) {
    if app.show_welcome {
        render_welcome_banner(f, area, app);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(app.messages.len() + 4);

    for (idx, m) in app.messages.iter().enumerate() {
        let ts = if app.show_timestamps {
            app.message_timestamps
                .get(idx)
                .map(|t| format!("[{}] ", t.format("%H:%M:%S")))
                .unwrap_or_default()
        } else {
            String::new()
        };
        if m.starts_with("> ") && !m.starts_with("> /") {
            let style = Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD);
            for line_text in m.split('\n') {
                lines.push(Line::from(Span::styled(
                    format!("{}{}", ts, line_text),
                    style,
                )));
            }
        } else if m.starts_with("> /") {
            let style = Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD);
            lines.push(Line::from(Span::styled(m.clone(), style)));
        } else if m.starts_with("Error:") || m.starts_with("Error :") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
        } else if m.starts_with("Subagent '") && m.contains("created") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Yellow),
            )));
        } else if m.starts_with("Subagent '") && m.contains("completed") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Green),
            )));
        } else if m.starts_with("Switched to session:")
            || m.starts_with("Session renamed to:")
            || (m.starts_with("Session ") && (m.contains("deleted") || m.contains("deleted.")))
            || m.starts_with("Anacleto shutting down")
            || m.starts_with("Anacleto started")
        {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Blue),
            )));
        } else if m.starts_with("Agent '") && m.contains("created") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Cyan),
            )));
        } else if m.starts_with("Unknown command") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Red),
            )));
        } else if m.starts_with("Usage:") || m.starts_with("Commands:") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Blue),
            )));
        } else if m.starts_with("$ ") {
            // !command prompt — yellow bold
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if m.starts_with("\u{2502} ") {
            // stdout from !command — gray, dimmed
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Rgb(160, 160, 180))
                    .add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{2514} ") {
            // stderr from !command — red, dimmed
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Rgb(220, 120, 120))
                    .add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{1f527}") {
            // Tool execution tracing — cyan
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{2705}") {
            // Tool result success — green dim
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{274c}") {
            // Tool result failure — red dim
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Red).add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{1f50d}") {
            // Debug header — purple bold
            let style = Style::default()
                .fg(Color::Rgb(180, 100, 255))
                .add_modifier(Modifier::BOLD);
            lines.push(Line::from(Span::styled(m.clone(), style)));
        } else if m.starts_with("  ") && app.debug_mode {
            // Debug payload — purple dim
            let style = Style::default()
                .fg(Color::Rgb(180, 100, 255))
                .add_modifier(Modifier::DIM);
            lines.push(Line::from(Span::styled(m.clone(), style)));
        } else {
            // AI responses — split by newline, render markdown per line
            let base = Style::default().fg(Color::Rgb(200, 220, 255));
            for (i, line_text) in m.split('\n').enumerate() {
                let prefix = if i == 0 { ts.as_str() } else { "" };
                lines.push(render_markdown_line(
                    &format!("{}{}", prefix, line_text),
                    base,
                ));
            }
        }
    }

    // Add streaming indicator if active
    // IMPORTANT: split by newlines so each logical Line = (roughly) one visual line.
    // Without this split, a long streaming response wraps to many visual lines but
    // counts as a single logical line, making bottom content invisible & unscrollable.
    if let Some(stream) = &app.current_stream {
        let style = Style::default()
            .fg(Color::Rgb(100, 200, 255))
            .add_modifier(Modifier::DIM);
        for (idx, line_text) in stream.split('\n').enumerate() {
            let prefix = if idx == 0 { "\u{258c}" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("{}{}", prefix, line_text),
                style,
            )));
        }
    }

    let title = format!(" (1) \u{1f4ac} Chat [{}] ", app.session_name);
    let chat_border = if app.focus == Focus::Chat {
        app.theme.accent()
    } else {
        Color::DarkGray
    };

    // Instead of using Paragraph::scroll() — which can leave content hidden when
    // wrapping creates more visual lines than logical ones — we pre-select the
    // subset of lines that fits the visible area, then render with scroll(0,0).
    let content_width = (area.width.saturating_sub(2)).max(1) as usize; // minus borders
    let visible = (area.height.max(2) as usize) - 2; // minus borders

    // Select visible portion: walk backwards from the end accumulating visual rows
    // (accounting for wrapping) until we fill the visible rows.
    let start_idx = select_visible_start(&lines, visible, content_width, app.chat_scroll);
    let display_lines: Vec<Line> = lines.into_iter().skip(start_idx as usize).collect();

    let paragraph = Paragraph::new(display_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(chat_border))
                .title(title),
        )
        .scroll((0, 0))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

/// Walk backwards through `lines` accumulating visual row counts (accounting for
/// wrapping at `content_width` columns) and return the index of the first logical
/// line to display so that the bottommost content fits in `visible_rows`.
///
/// When `chat_scroll > 0` the user has manually scrolled up that many logical
/// lines from the auto-scroll position.
fn select_visible_start(
    lines: &[Line],
    visible_rows: usize,
    content_width: usize,
    chat_scroll: u16,
) -> u16 {
    if lines.is_empty() {
        return 0;
    }

    // Walk backwards, accumulating visual rows until we fill the visible area
    let mut remaining = visible_rows;
    let mut bottom: usize = 0;

    'walk: for (i, line) in lines.iter().enumerate().rev() {
        let visual = visual_line_count(line, content_width);

        if remaining == 0 {
            // Filled the visible area; start from the line AFTER this one
            bottom = (i + 1).min(lines.len() - 1);
            break 'walk;
        }

        if visual > remaining {
            // This line overflows but must be partially shown
            bottom = i;
            break 'walk;
        }

        remaining -= visual;
    }
    // If loop completes without break: all lines fit (bottom stays 0)

    // Apply manual scroll offset (if any)
    let scroll = bottom.saturating_sub(chat_scroll as usize);
    scroll as u16
}

/// Estimate how many visual rows a logical Line occupies when wrapped at
/// `content_width` columns.
fn visual_line_count(line: &Line, content_width: usize) -> usize {
    let w = line.width();
    if w == 0 || content_width == 0 {
        1
    } else {
        w.div_ceil(content_width)
    }
}

/// Parse a line of text for inline markdown and return styled Spans.
/// Uses COLOR changes (not modifiers) for italic since color is
/// far more visible in terminals than font-weight/italic.
///
///   `**bold**`  -> bright white foreground
///   `*italic*`  -> warm yellow foreground
///   `` `code` `` -> amber on dark background
fn render_markdown_line(text: &str, base_style: Style) -> Line<'static> {
    if text.is_empty() {
        return Line::from("");
    }

    let trimmed = text.trim_start();

    // Line-level constructs (must be at start of trimmed line)
    if let Some(content) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        let bullet = Span::styled(" \u{2022} ", base_style.fg(Color::Rgb(255, 180, 100)));
        let mut spans = vec![bullet];
        spans.extend(parse_inline(content, base_style));
        return Line::from(spans);
    }

    if let Some(content) = trimmed.strip_prefix("> ") {
        let bar = Span::styled(" \u{2502} ", base_style.fg(Color::Rgb(100, 120, 140)));
        let quote_style = base_style
            .fg(Color::Rgb(140, 160, 180))
            .add_modifier(Modifier::DIM);
        let mut spans = vec![bar];
        spans.extend(parse_inline(content, quote_style));
        return Line::from(spans);
    }

    if let Some(content) = trimmed.strip_prefix("### ") {
        return Line::from(Span::styled(
            content.to_string(),
            base_style.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }

    // Regular paragraph line
    Line::from(parse_inline(text, base_style))
}

/// Parse inline markdown tokens: `**bold**`, `*italic*`, `` `code` ``
fn parse_inline(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Check for `code` first (backtick)
        if chars[i] == '`' {
            let mut content = String::new();
            i += 1;
            while i < len && chars[i] != '`' {
                content.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1;
            }
            spans.push(Span::styled(
                content,
                Style::default()
                    .fg(Color::Rgb(255, 200, 100))
                    .bg(Color::Rgb(50, 35, 15)),
            ));
            continue;
        }

        // Check for **bold**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            let mut content = String::new();
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                content.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            }
            spans.push(Span::styled(
                content,
                base_style
                    .fg(Color::Rgb(255, 255, 255))
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }

        // Check for *italic*
        if chars[i] == '*' {
            let mut content = String::new();
            i += 1;
            while i < len && chars[i] != '*' {
                content.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1;
            }
            // Italic = warm yellow (more visible than italic modifier)
            spans.push(Span::styled(
                content,
                base_style.fg(Color::Rgb(255, 220, 120)),
            ));
            continue;
        }

        // Regular character
        let mut plain = String::new();
        while i < len && chars[i] != '*' && chars[i] != '`' {
            plain.push(chars[i]);
            i += 1;
        }
        if !plain.is_empty() {
            spans.push(Span::styled(plain, base_style));
        }
    }

    spans
}

/// Render a welcome banner centered in the chat area when there are no messages yet.
fn render_welcome_banner(f: &mut Frame, area: Rect, app: &App) {
    let version = env!("CARGO_PKG_VERSION");
    let banner_lines = vec![
        Line::from(Span::styled(
            format!(" ⬡ anacleto v{} ", version),
            Style::default()
                .fg(Color::Rgb(255, 107, 107))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            " Agent Orchestration Engine ",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            " Type /help for commands ",
            Style::default()
                .fg(Color::Rgb(150, 150, 180))
                .add_modifier(Modifier::DIM),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            if app.kb_supported {
                " ⌨  Shift+Enter: newline "
            } else {
                " ⚠  Ctrl+J: newline (Shift+Enter unsupported) "
            },
            Style::default().fg(if app.kb_supported {
                Color::Rgb(100, 200, 100)
            } else {
                Color::Rgb(255, 180, 80)
            }),
        )),
    ];

    let banner = Paragraph::new(banner_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(80, 80, 120)))
                .style(Style::default().bg(Color::Rgb(20, 20, 30))),
        )
        .alignment(Alignment::Center);

    // Center the banner vertically by padding
    let banner_height = 7u16;
    let vert_pad = area.height.saturating_sub(banner_height) / 2;
    let banner_area = Rect {
        x: area.x + area.width.saturating_sub(46).min(area.width) / 2,
        y: area.y + vert_pad,
        width: 46.min(area.width),
        height: banner_height.min(area.height),
    };

    f.render_widget(banner, banner_area);
}

fn render_session_list(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .session_list
        .iter()
        .map(|s| {
            let active_marker = if Some(s.id.to_string()) == app.session_id {
                " ◀"
            } else {
                ""
            };
            let pinned_marker = if s.pinned { "📌" } else { "  " };
            let style = if Some(s.id.to_string()) == app.session_id {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    "{} {}  msgs:{}  {}  {}{}",
                    pinned_marker,
                    &s.id.to_string()[..8],
                    s.message_count,
                    s.name,
                    s.updated_at.format("%Y-%m-%d %H:%M"),
                    active_marker,
                ),
                style,
            )))
        })
        .collect();

    let sessions_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Sessions (Esc to close)"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(sessions_list, area);
}

/// Render the agent list overlay.
fn render_agent_list(f: &mut Frame, area: Rect, app: &App) {
    // Separate roots from subagents
    let roots: Vec<&AgentInfo> = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .collect();
    let subagents: Vec<&AgentInfo> = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::SubAgent)
        .collect();

    let mut items: Vec<ListItem> = Vec::new();

    // Root agents section
    if !roots.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "─── Root Agents ───",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))));
        for agent in &roots {
            items.push(build_agent_list_item(agent, agent.name == app.active_agent));
        }
    }

    // Subagents section
    if !subagents.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "─── SubAgents ───",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ))));
        for agent in &subagents {
            items.push(build_agent_list_item(agent, agent.name == app.active_agent));
        }
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No agents loaded.",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let agent_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Agents (Esc to close)"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(agent_list, area);
}

fn build_agent_list_item(agent: &AgentInfo, active: bool) -> ListItem<'static> {
    // Status badge
    let (status_color, badge) = match &agent.status {
        AgentStatus::Idle => (Color::Green, " IDLE "),
        AgentStatus::Working => (Color::Yellow, " BUSY "),
        AgentStatus::WaitingForSubAgent => (Color::Blue, " WAIT "),
        AgentStatus::Completed => (Color::DarkGray, " DONE "),
        AgentStatus::Error(_) => (Color::Red, " ERR  "),
    };

    let badge_span = Span::styled(
        badge.to_string(),
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::REVERSED),
    );

    // Active agent marker: a ▶ prefix with a highlighted background on the name.
    let marker_span = if active {
        Span::styled(
            "▶ ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    let name_span = Span::styled(
        agent.name.clone(),
        Style::default().add_modifier(Modifier::BOLD).bg(if active {
            Color::Magenta
        } else {
            Color::Reset
        }),
    );

    let mut spans = vec![
        marker_span,
        badge_span,
        Span::raw(" ".to_string()),
        name_span,
    ];

    // Model info
    if !agent.model.is_empty() {
        spans.push(Span::raw(" [".to_string()));
        spans.push(Span::styled(
            agent.model.clone(),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::raw("]".to_string()));
    }

    // Skills
    if !agent.skills.is_empty() {
        spans.push(Span::raw("  skills: ".to_string()));
        spans.push(Span::styled(
            agent.skills.join(", "),
            Style::default().fg(Color::Cyan),
        ));
    }

    // MCPs
    if !agent.mcps.is_empty() {
        spans.push(Span::raw("  mcps: ".to_string()));
        spans.push(Span::styled(
            agent.mcps.join(", "),
            Style::default().fg(Color::Magenta),
        ));
    }

    // Subagent count
    if agent.subagent_count > 0 {
        spans.push(Span::raw("  children: ".to_string()));
        spans.push(Span::styled(
            agent.subagent_count.to_string(),
            Style::default().fg(Color::Blue),
        ));
    }

    ListItem::new(Line::from(spans))
}

/// Render the subagent tree overlay showing the hierarchy.
fn render_subagent_tree(f: &mut Frame, area: Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();

    // Find root agents with subagents
    let roots: Vec<&AgentInfo> = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .collect();

    if roots.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No agents loaded.",
            Style::default().fg(Color::DarkGray),
        ))));
    } else {
        for root in &roots {
            // Root agent line
            let (status_color, badge) = match &root.status {
                AgentStatus::Idle => (Color::Green, " IDLE "),
                AgentStatus::Working => (Color::Yellow, " BUSY "),
                AgentStatus::WaitingForSubAgent => (Color::Blue, " WAIT "),
                AgentStatus::Completed => (Color::DarkGray, " DONE "),
                AgentStatus::Error(_) => (Color::Red, " ERR  "),
            };

            let mut root_spans = vec![
                Span::styled("┌─ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    badge,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::REVERSED),
                ),
                Span::raw(" "),
                Span::styled(
                    &root.name,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if !root.model.is_empty() {
                root_spans.push(Span::raw(" ["));
                root_spans.push(Span::styled(
                    &root.model,
                    Style::default().fg(Color::DarkGray),
                ));
                root_spans.push(Span::raw("]"));
            }
            items.push(ListItem::new(Line::from(root_spans)));

            // Find children (subagents whose parent_id matches this root)
            let children: Vec<&AgentInfo> = app
                .agents
                .iter()
                .filter(|a| a.parent_id == Some(root.id.clone()))
                .collect();

            // Configured subagents for this root that haven't been spawned yet.
            let spawned_names: std::collections::HashSet<&str> =
                children.iter().map(|c| c.name.as_str()).collect();
            let pending: Vec<&String> = app
                .configured_subagents
                .get(&root.name)
                .map(|names| {
                    names
                        .iter()
                        .filter(|n| !spawned_names.contains(n.as_str()))
                        .collect()
                })
                .unwrap_or_default();

            let total = children.len() + pending.len();

            if total == 0 {
                items.push(ListItem::new(Line::from(Span::styled(
                    "  │  (no subagents)",
                    Style::default().fg(Color::DarkGray),
                ))));
            } else {
                for (i, child) in children.iter().enumerate() {
                    let is_last = i == total - 1;
                    let (child_status_color, child_badge) = match &child.status {
                        AgentStatus::Idle => (Color::Green, " IDLE  "),
                        AgentStatus::Working => (Color::Yellow, " BUSY  "),
                        AgentStatus::WaitingForSubAgent => (Color::Blue, " WAIT  "),
                        AgentStatus::Completed => (Color::DarkGray, " DONE  "),
                        AgentStatus::Error(_) => (Color::Red, " ERR   "),
                    };

                    let prefix = if is_last { "└── " } else { "├── " };
                    let child_spans = vec![
                        Span::styled(
                            format!("│ {}", prefix),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            child_badge,
                            Style::default()
                                .fg(child_status_color)
                                .add_modifier(Modifier::REVERSED),
                        ),
                        Span::raw(" "),
                        Span::styled(&child.name, Style::default().fg(Color::Magenta)),
                    ];
                    items.push(ListItem::new(Line::from(child_spans)));
                }

                // Configured but not yet spawned subagents.
                for (j, name) in pending.iter().enumerate() {
                    let idx = children.len() + j;
                    let is_last = idx == total - 1;
                    let prefix = if is_last { "└── " } else { "├── " };
                    let child_spans = vec![
                        Span::styled(
                            format!("│ {}", prefix),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            " PEND ",
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::REVERSED),
                        ),
                        Span::raw(" "),
                        Span::styled(name.as_str(), Style::default().fg(Color::DarkGray)),
                        Span::styled(" (not created)", Style::default().fg(Color::DarkGray)),
                    ];
                    items.push(ListItem::new(Line::from(child_spans)));
                }
            }

            // Blank separator between roots
            items.push(ListItem::new(Line::from(Span::raw(""))));
        }
    }

    let tree_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Subagent Tree (Esc to close)"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(tree_list, area);
}

/// Render the fuzzy command palette as a dropdown above the input area.
fn render_command_palette(f: &mut Frame, input_area: Rect, app: &App) {
    let max_items = 8usize;
    let count = app.palette_matches.len().min(max_items);
    let width = input_area.width.min(60);
    let height = (count as u16) + 2; // +2 for borders
    let x = input_area.x;
    // Place the dropdown directly above the input area.
    let y = input_area.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);

    let items: Vec<ListItem> = app
        .palette_matches
        .iter()
        .take(max_items)
        .map(|&i| {
            let (cmd, desc) = &app.commands[i];
            let line = Line::from(vec![
                Span::styled(
                    format!(" {:<12}", cmd),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc, Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Commands "),
        )
        .highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(
        list,
        area,
        &mut ratatui::widgets::ListState::default().with_selected(Some(app.palette_index)),
    );
}

/// Render the agent-selection combo as a dropdown above the input area.
fn render_agent_palette(f: &mut Frame, input_area: Rect, app: &App) {
    let max_items = 8usize;
    let count = app.agent_matches.len().min(max_items);
    let width = input_area.width.min(60);
    let height = (count as u16) + 2; // +2 for borders
    let x = input_area.x;
    // Place the dropdown directly above the input area.
    let y = input_area.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);

    let items: Vec<ListItem> = app
        .agent_matches
        .iter()
        .take(max_items)
        .map(|name| {
            let line = Line::from(vec![
                Span::styled(
                    format!(" {:<16}", name),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("root", Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta))
                .title(" Agents "),
        )
        .highlight_style(Style::default().bg(Color::Rgb(60, 40, 60)))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(
        list,
        area,
        &mut ratatui::widgets::ListState::default().with_selected(Some(app.agent_index)),
    );
}

/// Render the model-selection combo as a dropdown above the input area.
fn render_model_palette(f: &mut Frame, input_area: Rect, app: &App) {
    let max_items = 8usize;
    let count = app.model_matches.len().min(max_items);
    let width = input_area.width.min(60);
    let height = (count as u16) + 2; // +2 for borders
    let x = input_area.x;
    // Place the dropdown directly above the input area.
    let y = input_area.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);

    let items: Vec<ListItem> = app
        .model_matches
        .iter()
        .take(max_items)
        .map(|name| {
            let line = Line::from(vec![Span::styled(
                format!(" {:<24}", name),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Models "),
        )
        .highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(
        list,
        area,
        &mut ratatui::widgets::ListState::default().with_selected(Some(app.model_index)),
    );
}

fn render_input(f: &mut Frame, area: Rect, app: &App) {
    let input_style = Style::default()
        .fg(app.theme.accent())
        .add_modifier(Modifier::BOLD);
    let prompt = Span::styled(" ❯ ", input_style);

    // Split input into lines
    let lines: Vec<&str> = app.input.split('\n').collect();

    // Build rendered lines: first line gets prompt, rest get 3-space indent
    let mut rendered: Vec<Line> = Vec::with_capacity(lines.len());
    for (i, line_text) in lines.iter().enumerate() {
        if i == 0 {
            rendered.push(Line::from(vec![prompt.clone(), Span::raw(*line_text)]));
        } else {
            rendered.push(Line::from(vec![
                Span::raw("   "), // 3-space indent to align with text after " ❯ "
                Span::raw(*line_text),
            ]));
        }
    }

    // Content width available for text (minus borders and prompt/indent).
    let inner_width = area.width.saturating_sub(2) as usize; // 2 for borders
    let first_line_width = inner_width.saturating_sub(3); // " ❯ " prompt
    let rest_line_width = inner_width.saturating_sub(3); // 3-space indent

    // Compute how many visual rows each logical line occupies when wrapped.
    let mut visual_rows: Vec<usize> = Vec::with_capacity(lines.len());
    for (i, line_text) in lines.iter().enumerate() {
        let w = if i == 0 {
            first_line_width
        } else {
            rest_line_width
        };
        let len = line_text.chars().count();
        visual_rows.push(if w == 0 { 1 } else { len.div_ceil(w).max(1) });
    }
    let total_visual: usize = visual_rows.iter().sum();

    // Bottom-anchored scroll: show last N visual rows where N = visible rows minus borders
    let visible_rows = (area.height.saturating_sub(2)) as usize; // 2 for borders
    let scroll_offset = total_visual.saturating_sub(visible_rows);

    let title = if let Some(flow) = &app.init_flow {
        format!(" Init — {} ", flow.prompt())
    } else {
        " (5) Input ".to_string()
    };
    let input_border = if app.focus == Focus::Input {
        app.theme.accent()
    } else {
        Color::DarkGray
    };

    let paragraph = Paragraph::new(rendered)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(input_border))
                .title(title),
        )
        .scroll((scroll_offset as u16, 0))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));

    f.render_widget(paragraph, area);

    // Cursor: position at `input_cursor` (char index), accounting for wrap.
    let cursor_char = app.input_cursor.min(app.input.chars().count());
    // Find the logical line containing the cursor and the char offset within it.
    let mut remaining = cursor_char;
    let mut cursor_line_idx = 0usize;
    let mut col_in_line = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let line_chars = line.chars().count();
        if remaining <= line_chars {
            cursor_line_idx = i;
            col_in_line = remaining;
            break;
        }
        remaining -= line_chars + 1; // +1 for the '\n' separator
        cursor_line_idx = i + 1;
    }
    let cursor_w = if cursor_line_idx == 0 {
        first_line_width
    } else {
        rest_line_width
    };
    // Visual row of the cursor within its logical line (0-based).
    let cursor_visual_in_line = col_in_line.checked_div(cursor_w).unwrap_or(0);
    // Column within the wrapped row (0-based), plus prompt/indent offset.
    let col_in_row = col_in_line.checked_rem(cursor_w).unwrap_or(0);
    // Visual row of the cursor's logical line start (sum of previous lines' visual rows).
    let cursor_line_start: usize = visual_rows[..cursor_line_idx].iter().sum();
    let cursor_visual = cursor_line_start + cursor_visual_in_line;
    let cursor_row = area.y + 1 + (cursor_visual.saturating_sub(scroll_offset)) as u16;
    let cursor_col = area.x + 1 + 3 + col_in_row as u16;
    f.set_cursor_position((cursor_col, cursor_row));
}

/// Render the human-in-the-loop approval dialog as a centered overlay.
fn render_approval_dialog(f: &mut Frame, area: Rect, app: &App) {
    let Some(ref approval) = app.pending_approval else {
        return;
    };

    // Dialog dimensions
    let dialog_width = area.width.min(60);
    let dialog_height = 7;
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

    // Clear area behind dialog with a semi-transparent effect
    let overlay = ratatui::widgets::Clear;
    f.render_widget(overlay, dialog_area);

    // Build dialog content
    let lines = vec![
        Line::from(Span::styled(
            " ⚠  Approval Required ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            &approval.operation,
            Style::default().fg(Color::White),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            " Press Y to approve  |  Press N to deny ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )),
    ];

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow))
                .style(Style::default().bg(Color::Rgb(40, 30, 0))),
        )
        .alignment(ratatui::layout::Alignment::Center);

    f.render_widget(dialog, dialog_area);
}

/// Render the inline question dialog (`/question` tool).
fn render_question_dialog(f: &mut Frame, area: Rect, app: &App) {
    let Some(ref q) = app.pending_question else {
        return;
    };

    let dialog_width = area.width.min(70);
    let dialog_height = 12;
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

    let overlay = ratatui::widgets::Clear;
    f.render_widget(overlay, dialog_area);

    let mut lines = vec![
        Line::from(Span::styled(
            " ❓ Question ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(&q.question, Style::default().fg(Color::White))),
        Line::from(Span::raw("")),
    ];

    if !q.options.is_empty() {
        for (i, opt) in q.options.iter().enumerate() {
            let marker = if i == q.selected { "▸" } else { " " };
            let style = if i == q.selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(
                format!(" {} {}", marker, opt),
                style,
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!(" ❯ {}", q.answer_input),
            Style::default().fg(Color::Green),
        )));
    }

    if let Some(rec) = &q.recommended {
        lines.push(Line::from(Span::styled(
            format!(" (recomendado: {})", rec),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        " Enter: submit  |  Esc: cancel  |  ↑/↓: select option ",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
    )));

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .style(Style::default().bg(Color::Rgb(0, 30, 40))),
        )
        .alignment(ratatui::layout::Alignment::Left);

    f.render_widget(dialog, dialog_area);
}

/// Apply Shift mapping for a character under the Kitty keyboard enhancement protocol.
///
/// With `REPORT_ALL_KEYS_AS_ESCAPE_CODES`, Kitty sends the unshifted physical key
/// code plus a SHIFT modifier, instead of the pre-shifted character. The terminal
/// no longer performs keyboard-layout-dependent shift mapping, so we must do it
/// ourselves. This function uses the `$LANG` locale to determine the layout:
/// `es_*` → Spanish, anything else → US English.
fn shift_char(c: char, lang: &str) -> char {
    let es = lang.starts_with("es_");
    match c {
        'A'..='Z' => c, // already uppercased by crossterm parser
        'a'..='z' => c.to_ascii_uppercase(),
        '1' => '!',
        '2' => {
            if es {
                '"'
            } else {
                '@'
            }
        }
        '3' => {
            if es {
                '·'
            } else {
                '#'
            }
        }
        '4' => '$',
        '5' => '%',
        '6' => '&',
        '7' => {
            if es {
                '/'
            } else {
                '&'
            }
        }
        '8' => '(',
        '9' => ')',
        '0' => {
            if es {
                '='
            } else {
                ')'
            }
        }
        '-' => '_',
        '\'' => '?',
        '`' => {
            if es {
                '^'
            } else {
                '~'
            }
        }
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        ',' => {
            if es {
                ';'
            } else {
                '<'
            }
        }
        '.' => {
            if es {
                ':'
            } else {
                '>'
            }
        }
        '/' => '?',
        _ => c,
    }
}

/// Copy text to the system clipboard.
/// Tries `wl-copy` (Wayland) first, then `xclip` (X11).
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    // Try wl-copy (Wayland)
    if let Ok(mut child) = std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        use std::io::Write;
        let _ = child.stdin.as_mut().unwrap().write_all(text.as_bytes());
        let _ = child.wait();
        return Ok(());
    }

    // Try xclip (X11)
    if let Ok(mut child) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        use std::io::Write;
        let _ = child.stdin.as_mut().unwrap().write_all(text.as_bytes());
        let _ = child.wait();
        return Ok(());
    }

    Err("No clipboard tool found. Install wl-clipboard (Wayland) or xclip (X11)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matches_subsequence() {
        // "sess" matches "/sessions" (subsequence).
        assert!(fuzzy_score("sess", "/sessions").is_some());
        // "sa" matches "/subagents" (s-u-b-a...).
        assert!(fuzzy_score("sa", "/subagents").is_some());
        // "mdl" matches "/models" (m-o-d-e-l-s).
        assert!(fuzzy_score("mdl", "/models").is_some());
    }

    #[test]
    fn fuzzy_rejects_non_subsequence() {
        // Characters out of order must not match.
        assert!(fuzzy_score("xs", "/sessions").is_none());
        assert!(fuzzy_score("zzz", "/help").is_none());
    }

    #[test]
    fn fuzzy_is_case_insensitive() {
        assert!(fuzzy_score("HELP", "/help").is_some());
        assert!(fuzzy_score("Help", "/help").is_some());
    }

    #[test]
    fn fuzzy_prefers_consecutive_and_shorter() {
        // Consecutive matches score higher than scattered ones.
        let consecutive = fuzzy_score("sess", "/sessions").unwrap();
        let scattered = fuzzy_score("sns", "/sessions").unwrap();
        assert!(consecutive > scattered);
    }

    #[test]
    fn fuzzy_empty_query_matches_everything() {
        assert!(fuzzy_score("", "/help").is_some());
    }

    /// Build an `App` with empty input for testing cursor helpers.
    fn test_app() -> App {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        App::new(cmd_tx, event_rx, false, &Config::default())
    }

    #[test]
    fn input_insert_char_advances_cursor() {
        let mut app = test_app();
        app.input = String::from("hola");
        app.input_cursor = 2;
        app.input_insert_char('X');
        assert_eq!(app.input, "hoXla");
        assert_eq!(app.input_cursor, 3);
    }

    #[test]
    fn input_insert_char_handles_multibyte() {
        let mut app = test_app();
        app.input = String::from("héllo");
        app.input_cursor = 1; // after 'h'
        app.input_insert_char('X');
        assert_eq!(app.input, "hXéllo");
        assert_eq!(app.input_cursor, 2);
    }

    #[test]
    fn input_delete_before_removes_char() {
        let mut app = test_app();
        app.input = String::from("hola");
        app.input_cursor = 4;
        app.input_delete_before();
        assert_eq!(app.input, "hol");
        assert_eq!(app.input_cursor, 3);
    }

    #[test]
    fn input_delete_before_at_start_is_noop() {
        let mut app = test_app();
        app.input = String::from("hola");
        app.input_cursor = 0;
        app.input_delete_before();
        assert_eq!(app.input, "hola");
        assert_eq!(app.input_cursor, 0);
    }

    #[test]
    fn input_delete_before_handles_multibyte() {
        let mut app = test_app();
        app.input = String::from("héllo");
        app.input_cursor = 2; // after 'é'
        app.input_delete_before();
        assert_eq!(app.input, "hllo");
        assert_eq!(app.input_cursor, 1);
    }

    #[test]
    fn input_delete_at_removes_char_after_cursor() {
        let mut app = test_app();
        app.input = String::from("hola");
        app.input_cursor = 1;
        app.input_delete_at();
        assert_eq!(app.input, "hla");
        assert_eq!(app.input_cursor, 1);
    }

    #[test]
    fn input_delete_at_at_end_is_noop() {
        let mut app = test_app();
        app.input = String::from("hola");
        app.input_cursor = 4;
        app.input_delete_at();
        assert_eq!(app.input, "hola");
    }

    #[test]
    fn input_move_word_left_jumps_to_previous_word() {
        let mut app = test_app();
        app.input = String::from("hola mundo rust");
        app.input_cursor = 15; // end
        app.input_move_word_left();
        assert_eq!(app.input_cursor, 11); // start of "rust"
        app.input_move_word_left();
        assert_eq!(app.input_cursor, 5); // start of "mundo"
    }

    #[test]
    fn input_move_word_right_jumps_to_next_word() {
        let mut app = test_app();
        app.input = String::from("hola mundo rust");
        app.input_cursor = 0;
        app.input_move_word_right();
        assert_eq!(app.input_cursor, 5); // start of "mundo"
        app.input_move_word_right();
        assert_eq!(app.input_cursor, 11); // start of "rust"
    }

    #[test]
    fn input_delete_word_before_removes_previous_word() {
        let mut app = test_app();
        app.input = String::from("hola mundo");
        app.input_cursor = 11;
        app.input_delete_word_before();
        assert_eq!(app.input, "hola ");
        assert_eq!(app.input_cursor, 5);
    }

    #[test]
    fn input_delete_to_start_clears_prefix() {
        let mut app = test_app();
        app.input = String::from("hola mundo");
        app.input_cursor = 5;
        app.input_delete_to_start();
        assert_eq!(app.input, "mundo");
        assert_eq!(app.input_cursor, 0);
    }

    #[test]
    fn input_delete_to_end_clears_suffix() {
        let mut app = test_app();
        app.input = String::from("hola mundo");
        app.input_cursor = 5;
        app.input_delete_to_end();
        assert_eq!(app.input, "hola ");
        assert_eq!(app.input_cursor, 5);
    }

    #[test]
    fn input_char_to_byte_maps_char_index_to_byte() {
        let mut app = test_app();
        app.input = String::from("héllo");
        // char index 1 ('é') starts at byte 1.
        assert_eq!(app.input_char_to_byte(1), 1);
        // char index 2 ('l') starts at byte 3.
        assert_eq!(app.input_char_to_byte(2), 3);
        // Out-of-range maps to the end.
        assert_eq!(app.input_char_to_byte(99), app.input.len());
    }

    #[test]
    fn typing_c_with_nonempty_input_inserts_char_not_focus() {
        // Regression: 'c' must be typed, not switch focus to Chat, when input
        // already has text.
        let mut app = test_app();
        app.input = String::from("he");
        app.input_cursor = 2;
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(app.input, "hec");
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn c_with_empty_input_typing_inserts_char() {
        // Regression: 'c' with empty input must be typed, not switch to Chat.
        let mut app = test_app();
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(app.input, "c");
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn q_with_empty_input_typing_inserts_char_not_quit() {
        // Regression: 'q' with empty input must be typed, not quit.
        let mut app = test_app();
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(app.input, "q");
        assert_eq!(app.focus, Focus::Input);
        assert!(!app.should_exit);
    }

    #[test]
    fn n_with_empty_input_typing_inserts_char() {
        // Regression: 'N' with empty input must be typed.
        let mut app = test_app();
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('N'), KeyModifiers::NONE);
        assert_eq!(app.input, "N");
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn ctrl_q_with_input_focus_quits() {
        // Ctrl+q must still quit even while focused on the Input window.
        let mut app = test_app();
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(app.should_exit);
    }

    #[test]
    fn plain_letter_in_chat_focus_does_not_trigger_global_action() {
        // With focus on Chat (not Input) and empty input, a plain letter must
        // not trigger a global action such as Quit.
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!app.should_exit);
    }

    #[test]
    fn alt_1_switches_focus_to_chat() {
        let mut app = test_app();
        app.input = String::from("some text");
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('1'), KeyModifiers::ALT);
        assert_eq!(app.focus, Focus::Chat);
    }

    #[test]
    fn alt_5_switches_focus_to_input() {
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.handle_key(KeyCode::Char('5'), KeyModifiers::ALT);
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn input_left_moves_cursor_back() {
        let mut app = test_app();
        app.input = String::from("hola");
        app.input_cursor = 3;
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(app.input_cursor, 2);
    }

    #[test]
    fn input_cursor_home_jumps_to_start() {
        let mut app = test_app();
        app.input = String::from("hola");
        app.input_cursor = 3;
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(app.input_cursor, 0);
    }

    #[test]
    fn chat_j_scrolls_down_and_k_scrolls_up() {
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.chat_scroll = 5;
        app.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.chat_scroll, 6);
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(app.chat_scroll, 5);
    }

    #[test]
    fn chat_pageup_scrolls_by_page() {
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.chat_scroll = 3;
        app.handle_key(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(app.chat_scroll, 13);
    }
}
