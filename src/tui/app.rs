use std::collections::HashMap;
use std::time::Instant;

use chrono::{DateTime, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::agent::types::{AgentId, AgentRole, AgentStatus};
use crate::config::Config;
use crate::config::types::CustomCommand;
use crate::db::models::SessionSummary;
use crate::engine::orchestrator::{
    EngineCommand, EngineEvent, McpStatus, SkillInfo, StatusInfo, TimelineEntry,
};
use crate::tui::diff_viewer::DiffViewer;
use crate::tui::keymap::{Action, Keymap};
use crate::tui::model_picker::ModelPicker;
use crate::tui::render::render;
use crate::tui::theme::Theme;
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
    pub(crate) input_cursor: usize,
    /// Which window currently has keyboard focus.
    pub(crate) focus: Focus,
    /// Selected index in the MCPs sidebar panel.
    pub(crate) mcp_panel_index: usize,
    /// Selected index in the Skills sidebar panel.
    pub(crate) skill_panel_index: usize,
    /// Selected index in the Agents sidebar panel.
    pub(crate) agent_panel_index: usize,
    /// History of previously submitted inputs (for Up/Down arrow navigation).
    pub(crate) input_history: Vec<String>,
    /// Current position in input history while navigating (None = editing fresh).
    pub(crate) history_index: Option<usize>,
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
    pub(crate) show_session_list: bool,

    // ── Agent info views ──────────────────────────────────────────────
    /// All known agents (root + subagents).
    pub(crate) agents: Vec<AgentInfo>,
    /// Whether to show the agent list overlay.
    pub show_agents: bool,
    /// Whether to show the subagent tree overlay.
    pub show_subagents: bool,
    /// Name of the currently active agent (for display).
    pub active_agent: String,
    /// Configured subagent names per root agent (from config frontmatter),
    /// used to show subagents in `/subagents` even before they are spawned.
    pub(crate) configured_subagents: HashMap<String, Vec<String>>,

    // ── Human-in-the-loop approval ────────────────────────────────────
    /// Pending approval request (None if no pending request).
    pub(crate) pending_approval: Option<ApprovalRequest>,

    // ── Inline question dialog (`/question` tool) ─────────────────────
    /// Pending question from the agent (None if no pending question).
    pub(crate) pending_question: Option<QuestionState>,

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
    pub(crate) lang: String,
    /// All slash commands (built-in + custom) for Tab autocomplete and palette.
    pub(crate) commands: Vec<(String, String)>,
    /// Custom slash commands with their templates (for dispatch).
    pub(crate) custom_commands: Vec<CustomCommand>,
    /// Current autocomplete matches for Tab cycling.
    pub(crate) tab_matches: Vec<String>,
    /// Index into tab_matches for cycling.
    pub(crate) tab_index: usize,
    /// Whether the fuzzy command palette is currently open.
    pub(crate) show_command_palette: bool,
    /// Indices into `COMMANDS` for the current fuzzy matches.
    pub(crate) palette_matches: Vec<usize>,
    /// Index of the currently highlighted palette entry.
    pub(crate) palette_index: usize,
    /// Whether the agent-selection combo is open (for `/agent`).
    pub(crate) show_agent_palette: bool,
    /// Root agent names matching the current `/agent` query.
    pub(crate) agent_matches: Vec<String>,
    /// Index of the currently highlighted agent entry.
    pub(crate) agent_index: usize,
    /// Whether the model-selection combo is open (for `/models`).
    pub(crate) show_model_palette: bool,
    /// Model names matching the current `/models` query.
    pub(crate) model_matches: Vec<String>,
    /// Index of the currently highlighted model entry.
    pub(crate) model_index: usize,
    /// Vertical scroll offset for the chat panel (0 = bottom, auto-scroll).
    pub chat_scroll: u16,
    /// Timestamp of the last 'g' press, used to detect a double-'g' (gg) jump.
    pub(crate) last_g_press: Option<Instant>,
    /// Frame counter for animating spinners in the UI.
    pub frame_count: u64,

    // ── OpenCode-style slash command state ───────────────────────────
    /// Current color theme (`/themes`).
    pub(crate) theme: Theme,
    /// Whether to show timestamps next to chat messages (`/timestamps`).
    pub show_timestamps: bool,
    /// Whether to show LLM thinking/streaming output (`/thinking`).
    pub show_thinking: bool,
    /// Timestamps recorded when each chat message was added.
    pub(crate) message_timestamps: Vec<DateTime<Utc>>,
    /// Stash stack for `/stash` (saved prompts).
    pub(crate) stash_stack: Vec<String>,
    /// Skills listed by the engine (`/skills`).
    skills_list: Vec<SkillInfo>,
    /// MCP servers with on/off state (`/mcps`).
    pub(crate) mcps_list: Vec<McpStatus>,
    /// Engine status report (`/status`).
    status_info: Option<StatusInfo>,
    /// Known workspaces (`/workspaces`).
    workspaces_list: Vec<String>,
    /// Session timeline entries (`/timeline`).
    pub(crate) timeline: Vec<TimelineEntry>,
    /// Whether the timeline panel is open.
    pub show_timeline: bool,
    /// Index of the highlighted timeline entry.
    pub(crate) timeline_index: usize,
    /// Whether the MCP list panel is open.
    pub show_mcps: bool,
    /// Index of the highlighted MCP entry.
    pub(crate) mcps_index: usize,
    /// Active `/init` flow (None when not running).
    pub(crate) init_flow: Option<InitFlow>,
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
    pub(crate) fn push_msg(&mut self, msg: impl Into<String>) {
        self.message_timestamps.push(Utc::now());
        self.messages.push(msg.into());
    }

    /// Commit any in-progress streaming response to the message log so that a
    /// newly submitted user message appears AFTER it, preserving chat order.
    pub(crate) fn commit_stream(&mut self) {
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

    /// Number of unique MCP servers shown in the MCPs sidebar panel.
    pub(crate) fn unique_mcp_count(&self) -> usize {
        self.agents
            .iter()
            .flat_map(|a| a.mcps.iter())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Number of unique skills shown in the Skills sidebar panel.
    pub(crate) fn unique_skill_count(&self) -> usize {
        self.agents
            .iter()
            .flat_map(|a| a.skills.iter())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Number of agents shown in the Agents sidebar panel (non-completed).
    pub(crate) fn agent_panel_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| a.status != AgentStatus::Completed)
            .count()
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

/// Fuzzy-match `query` against `candidate` (case-insensitive subsequence).
/// Returns a score if every character of `query` appears in order in
/// `candidate`, or `None` otherwise. Higher scores rank better: consecutive
/// matches, matches near the start, and shorter candidates are preferred.
pub(crate) fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
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
