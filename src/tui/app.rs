use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::Terminal;
use ratatui::style::{Color, Modifier, Style};
use ratatui_textarea::{TextArea, WrapMode};
use tokio::sync::mpsc;

use crate::agent::types::{AgentRole, AgentStatus};
use crate::config::Config;
use crate::config::types::CustomCommand;
use crate::db::models::SessionSummary;
use crate::engine::orchestrator::{
    EngineCommand, EngineEvent, McpStatus, SkillInfo, StatusInfo, TimelineEntry,
};
use crate::tui::code_block::{CodeBlockHighlighter, CodeBlockPosition};
use crate::tui::diff_viewer::DiffViewer;
use crate::tui::keymap::Keymap;
use crate::tui::model_picker::ModelPicker;
use crate::tui::render::render;
use crate::tui::theme::Theme;
use crate::tui::toast::ToastQueue;
use crate::tui::types::{
    AgentInfo, ApprovalRequest, BUILTIN_COMMANDS, CollapsedSection, EditDialogState, Focus,
    InitFlow, MAX_MESSAGE_LENGTH, MAX_MESSAGES, QuestionState, SearchState,
};
use crate::tui::which_key::WhichKeyPopup;

/// Application state for the TUI.
pub struct App {
    /// Channel to send commands to the engine.
    pub cmd_tx: mpsc::Sender<EngineCommand>,
    /// Channel to receive events from the engine.
    pub event_rx: mpsc::Receiver<EngineEvent>,
    /// TextArea widget state (buffer, cursor, selection, scroll).
    pub(crate) textarea: TextArea<'static>,
    /// Which window currently has keyboard focus.
    pub(crate) focus: Focus,
    /// Active tab in the Info panel (0 = Skills, 1 = MCPs).
    pub(crate) info_tab: usize,
    /// Selected index in the MCPs sidebar panel.
    pub(crate) mcp_panel_index: usize,
    /// Selected index in the Skills sidebar panel.
    pub(crate) skill_panel_index: usize,
    /// Selected index in the SubAgents sidebar panel.
    pub(crate) subagent_panel_index: usize,
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
    /// Current streaming thinking/reasoning being accumulated.
    pub current_thinking: Option<String>,
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
    /// All available agent names from the merged config (workspace + global),
    /// used to populate the subagent picker in the edit dialog so the user
    /// can add any existing agent as a subagent, even if no root agent
    /// currently references it.
    pub(crate) all_agent_names: Vec<String>,

    // ── Human-in-the-loop approval ────────────────────────────────────
    /// Pending approval request (None if no pending request).
    pub(crate) pending_approval: Option<ApprovalRequest>,

    // ── Inline question dialog (`/question` tool) ─────────────────────
    /// Pending question from the agent (None if no pending question).
    pub(crate) pending_question: Option<QuestionState>,

    // ── Right panel data ──────────────────────────────────────────────
    /// Total tokens consumed in the current session (cumulative).
    pub total_tokens: u64,
    /// Current context size (non-cumulative, reflects actual conversation buffer).
    pub context_tokens: u64,
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
    /// Current git branch name (None if not a git repo or on detached HEAD).
    pub git_branch: Option<String>,
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
    pub(crate) skills_list: Vec<SkillInfo>,
    /// All skills discovered on disk (workspace + global), for edit dialog.
    pub(crate) all_discovered_skills: Vec<String>,
    /// MCP servers with on/off state (`/mcps`).
    pub(crate) mcps_list: Vec<McpStatus>,
    /// Engine status report (`/status`).
    pub(crate) status_info: Option<StatusInfo>,
    /// Known workspaces (`/workspaces`).
    pub(crate) workspaces_list: Vec<String>,
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
    pub(crate) todos: Vec<crate::db::models::Todo>,

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
    /// Whether a message has been sent to the agent and we're awaiting idle.
    pub sent_message: bool,
    /// Search overlay state (Ctrl+R).
    pub(crate) search: SearchState,
    /// Ctrl+E edit-agent/subagent dialog state.
    pub(crate) edit_dialog: EditDialogState,
    /// Cached syntect highlighter (loaded once).
    pub(crate) code_block_hl: CodeBlockHighlighter,
    /// Positions of code block [copy] lines in the rendered chat, populated
    /// each frame during render and used by mouse-click handling.
    pub(crate) code_block_positions: Vec<CodeBlockPosition>,
    /// Accumulates consecutive tool/skill lines so they render as a single
    /// [tool] block without redundant borders between them.
    pub(crate) pending_tool_lines: Vec<String>,
    /// Monotonic counter for unique section IDs across all batches and renders.
    /// The full rendered chat lines from the last frame, used by mouse-click
    /// handling to map a click row back to an absolute rendered line.
    pub(crate) rendered_chat_lines: Vec<ratatui::text::Line<'static>>,
    /// Set of collapsed section IDs (ephemeral, per-session).
    pub(crate) collapsed_sections: HashSet<String>,
    /// Per-frame mapping: line index in rendered_chat_lines → section_id.
    pub(crate) section_line_map: Vec<Option<String>>,
    /// Per-frame section info for collapse rendering.
    pub(crate) section_info: Vec<CollapsedSection>,
    /// Cached rendered chat lines from committed messages (excluding live
    /// streaming/thinking content). Invalidated when `messages_generation`
    /// changes, the terminal is resized, or display settings change.
    pub(crate) chat_cache: Option<ChatCache>,
    /// Timestamp of the last render, used for throttling (max 30fps).
    pub(crate) last_render_time: Instant,
    /// Monotonic generation counter bumped every time `messages` changes.
    /// Used by the render cache to skip re-rendering unchanged messages.
    pub(crate) messages_generation: u64,
}

/// Cached rendering of the committed chat messages (everything except the
/// live streaming/thinking blocks, which change every frame and are rendered
/// on top of this cache). Rebuilt only when the underlying messages or
/// display settings change.
pub(crate) struct ChatCache {
    /// `messages_generation` value when the cache was built.
    pub generation: u64,
    /// Content width (terminal width minus borders) when the cache was built.
    pub content_width: usize,
    /// Whether timestamps were shown when the cache was built.
    pub show_timestamps: bool,
    /// Whether dark mode was active when the cache was built.
    pub is_dark: bool,
    /// Section-type counters (for unique section IDs) when the cache was built.
    pub counters: HashMap<String, u32>,
    /// Fully rendered, prewrapped chat lines (committed messages only).
    pub lines: Vec<ratatui::text::Line<'static>>,
    /// Per-line section map matching `lines` (pre-prewrap coordinates).
    pub section_line_map: Vec<Option<String>>,
    /// Section info (collapse regions) in pre-prewrap coordinates.
    pub section_info: Vec<CollapsedSection>,
    /// Code block positions from committed messages.
    pub code_block_positions: Vec<CodeBlockPosition>,
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
        let git_branch = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if s.is_empty() || s == "HEAD" {
                        None // detached HEAD or no commits
                    } else {
                        Some(s)
                    }
                } else {
                    None
                }
            });
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

        // Collect all available agent names from the merged config
        // (workspace .agents/agents/ + global $HOME/.agents/agents/)
        // for the subagent picker in the edit dialog.
        let all_agent_names: Vec<String> = config.agents.iter().map(|a| a.name.clone()).collect();

        Self {
            cmd_tx,
            event_rx,
            textarea: {
                let mut ta = TextArea::default();
                ta.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
                ta.set_cursor_line_style(Style::default());
                ta.set_wrap_mode(WrapMode::Word);
                ta
            },
            focus: Focus::Input,
            info_tab: 0,
            mcp_panel_index: 0,
            skill_panel_index: 0,
            subagent_panel_index: 0,
            agent_panel_index: 0,
            input_history: Vec::new(),
            history_index: None,
            messages: Vec::new(),
            current_stream: None,
            current_thinking: None,
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
            all_agent_names,
            pending_approval: None,
            pending_question: None,
            total_tokens: 0,
            context_tokens: 0,
            context_window_pct: 0.0,
            total_cost: 0.0,
            context_window: 0,
            current_model: String::new(),
            working_dir,
            git_branch,
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
            all_discovered_skills: Vec::new(),
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
            sent_message: false,
            search: SearchState::default(),
            edit_dialog: EditDialogState::new(),
            code_block_hl: CodeBlockHighlighter::default(),
            code_block_positions: Vec::new(),
            pending_tool_lines: Vec::new(),
            rendered_chat_lines: Vec::new(),
            collapsed_sections: HashSet::new(),
            section_line_map: Vec::new(),
            section_info: Vec::new(),
            chat_cache: None,
            last_render_time: Instant::now(),
            messages_generation: 0,
        }
    }

    /// Append a chat message, recording its timestamp for `/timestamps`.
    /// Flushes any pending tool lines first so tool messages are always
    /// batched together before non-tool content.
    /// Wraps non-tool, non-thinking messages in [normal] markers so the
    /// renderer can detect section transitions and add borders.
    /// Long messages are truncated to `MAX_MESSAGE_LENGTH` and the in-memory
    /// buffer is capped at `MAX_MESSAGES` entries (older messages are dropped;
    /// they remain persisted in the database).
    pub(crate) fn push_msg(&mut self, msg: impl Into<String>) {
        self.flush_tool_lines();
        let mut msg = msg.into();
        // Truncate oversized messages at the display layer. The full content
        // is still persisted by the engine in SQLite, so nothing is lost.
        let char_len = msg.chars().count();
        if char_len > MAX_MESSAGE_LENGTH {
            let truncated: String = msg.chars().take(MAX_MESSAGE_LENGTH).collect();
            msg = format!(
                "{}…\n[truncado: {} chars → {}]",
                truncated, char_len, MAX_MESSAGE_LENGTH
            );
        }
        // Only wrap in [normal] if not already wrapped in a section marker.
        let trimmed = msg.trim_start();
        if !trimmed.starts_with("[thinking]")
            && !trimmed.starts_with("[tool]")
            && !trimmed.starts_with("[normal]")
            && !trimmed.starts_with("[user]")
            && !trimmed.starts_with("[command]")
        {
            self.message_timestamps.push(Utc::now());
            // Slash-command echoes (`> /cmd`) get their own section type so
            // they render with the command style (magenta border/text).
            if trimmed.starts_with("> /") {
                self.messages
                    .push(crate::tui::render::trim_block_blank_lines(&format!(
                        "[command]\n{}\n[/command]",
                        msg
                    )));
            } else {
                self.messages
                    .push(crate::tui::render::trim_block_blank_lines(&format!(
                        "[normal]\n{}\n[/normal]",
                        msg
                    )));
            }
        } else {
            self.message_timestamps.push(Utc::now());
            self.messages
                .push(crate::tui::render::trim_block_blank_lines(&msg));
        }
        self.enforce_message_limit();
    }

    /// Cap the in-memory message buffer at `MAX_MESSAGES` entries, dropping
    /// the oldest ones. The engine persists every message in SQLite, so this
    /// only limits what is kept in RAM for rendering — the primary guard
    /// against unbounded memory growth on long sessions.
    fn enforce_message_limit(&mut self) {
        let overflow = self.messages.len().saturating_sub(MAX_MESSAGES);
        if overflow > 0 {
            self.messages.drain(0..overflow);
            self.message_timestamps.drain(0..overflow);
        }
        // Any mutation of `messages` invalidates the render cache.
        self.messages_generation = self.messages_generation.wrapping_add(1);
    }

    /// Flush any accumulated tool lines as a single [tool] block.
    /// Uses direct push to self.messages (not push_msg) to avoid recursion.
    pub(crate) fn flush_tool_lines(&mut self) {
        if self.pending_tool_lines.is_empty() {
            return;
        }
        let mut block = format!("[tool]\n{}\n[/tool]", self.pending_tool_lines.join("\n"));
        // Truncate oversized tool blocks at the display layer.
        let char_len = block.chars().count();
        if char_len > MAX_MESSAGE_LENGTH {
            let truncated: String = block.chars().take(MAX_MESSAGE_LENGTH).collect();
            block = format!(
                "{}…\n[truncado: {} chars → {}]",
                truncated, char_len, MAX_MESSAGE_LENGTH
            );
        }
        self.pending_tool_lines.clear();
        self.message_timestamps.push(Utc::now());
        self.messages.push(block);
        self.enforce_message_limit();
    }

    /// Commit any in-progress streaming response to the message log so that a
    /// newly submitted user message appears AFTER it, preserving chat order.
    pub(crate) fn commit_stream(&mut self) {
        if let Some(stream) = self.current_stream.take()
            && !stream.is_empty()
        {
            self.push_msg(stream);
        }
    }

    /// Replace the textarea with a new empty one, preserving standard
    /// configuration (no underline on cursor line, word wrap).
    pub(crate) fn reset_textarea(&mut self) {
        let mut ta = TextArea::default();
        ta.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
        ta.set_cursor_line_style(Style::default());
        ta.set_wrap_mode(WrapMode::Word);
        self.textarea = ta;
    }

    /// Replace the textarea with one containing the given text, preserving
    /// standard configuration (no underline on cursor line, word wrap).
    pub(crate) fn set_textarea_text(&mut self, text: &str) {
        let mut ta = TextArea::from([text]);
        ta.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
        ta.set_cursor_line_style(Style::default());
        ta.set_wrap_mode(WrapMode::Word);
        self.textarea = ta;
    }

    /// Whether the active agent is currently busy (working or waiting for a
    /// subagent). If the active agent is not tracked in `self.agents` or
    /// `active_agent` is empty, the agent is treated as idle.
    pub(crate) fn active_agent_is_busy(&self) -> bool {
        if self.active_agent.is_empty() {
            return false;
        }
        self.agents
            .iter()
            .find(|a| a.name == self.active_agent)
            .map(|a| {
                matches!(
                    a.status,
                    AgentStatus::Working | AgentStatus::WaitingForSubAgent
                )
            })
            .unwrap_or(false)
    }

    /// Send a user prompt to the engine and mark a message as in-flight on a
    /// successful send. Returns `true` when the send succeeded. On failure the
    /// caller is responsible for retrying or re-queuing the prompt.
    pub(crate) fn send_prompt(&mut self, text: String) -> bool {
        match self.cmd_tx.try_send(EngineCommand::UserInput(text)) {
            Ok(()) => {
                self.sent_message = true;
                true
            }
            Err(_) => false,
        }
    }

    /// Drain the prompt queue one item at a time. Sends the first queued
    /// prompt to the engine only if no message is already in flight (i.e.
    /// `sent_message` is false) AND the active agent is idle. When the agent
    /// finishes processing and emits `Idle`, `handle_event` clears
    /// `sent_message` and calls this again.
    pub(crate) fn drain_queue_if_idle(&mut self) {
        if self.sent_message || self.active_agent_is_busy() || self.prompt_queue.is_empty() {
            return;
        }
        let item = self.prompt_queue.remove(0);
        if self.send_prompt(item.clone()) {
            self.push_msg(format!("[user]\n> {}\n[/user]", item));
        } else {
            // Channel full (or closed): put the prompt back at the front of
            // the queue so it is not lost; it will be retried on the next
            // drain.
            self.prompt_queue.insert(0, item);
        }
    }

    /// Update the list of matching message indices from the current search query.
    pub(crate) fn update_search_matches(&mut self) {
        if self.search.query.is_empty() {
            self.search.matches.clear();
            self.search.selected = 0;
            return;
        }
        let query = self.search.query.to_lowercase();
        self.search.matches = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, msg)| msg.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        self.search.selected = 0;
    }

    /// Approximate the vertical scroll offset for a message index.
    /// This is a rough estimate since each message may wrap multiple lines.
    pub(crate) fn chat_height_at(&self, msg_index: usize) -> u16 {
        // Each message is at least 1 line, plus some overhead for spacing.
        // The exact value depends on terminal width, but we use the index
        // as a rough scroll offset so Enter jumps to the right area.
        msg_index as u16 * 3
    }

    // ── Code block helpers ───────────────────────────────────────────

    /// Find all fenced code blocks in all stored messages.
    /// Returns `(lang, code)` pairs in order of appearance.
    pub(crate) fn find_code_blocks(&self) -> Vec<(String, String)> {
        let mut blocks: Vec<(String, String)> = Vec::new();
        for msg in &self.messages {
            let mut in_block = false;
            let mut lang = String::new();
            let mut code_lines: Vec<String> = Vec::new();
            for line in msg.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("```") && !in_block {
                    in_block = true;
                    lang = trimmed.trim_start_matches("```").trim().to_string();
                    code_lines.clear();
                } else if trimmed == "```" && in_block {
                    in_block = false;
                    blocks.push((lang.clone(), code_lines.join("\n")));
                } else if in_block {
                    code_lines.push(line.to_string());
                }
            }
        }
        blocks
    }

    /// Toggle expand/collapse of the last code block in the message log.
    /// No-op since code blocks are always fully visible.
    pub(crate) fn toggle_last_code_block(&mut self) {
        // No-op: code blocks are always visible
    }

    /// Copy the content of the last code block to the clipboard.
    /// Returns a user-facing status message.
    pub(crate) fn copy_last_code_block(&self) -> Option<String> {
        let blocks = self.find_code_blocks();
        if let Some((lang, code)) = blocks.last() {
            match crate::tui::render::copy_to_clipboard(code) {
                Ok(()) => {
                    return Some(format!(
                        "Código '{}' copiado al portapapeles ({} líneas)",
                        lang,
                        code.lines().count()
                    ));
                }
                Err(e) => return Some(format!("Error al copiar: {}", e)),
            }
        }
        None // no code blocks found
    }

    /// Open the edit dialog for a given agent/subagent.
    pub(crate) fn open_edit_dialog(
        &mut self,
        target_name: String,
        is_root: bool,
        skills: &[String],
        mcps: &[String],
        subagents: Option<&[String]>,
    ) {
        // Collect all unique skills across all agents AND from the skill registry
        let all_skills: Vec<String> = {
            let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for agent in &self.agents {
                for s in &agent.skills {
                    set.insert(s.clone());
                }
            }
            // Also include skills from the registry (loaded from $HOME/.agents/skills etc.)
            for skill in &self.skills_list {
                set.insert(skill.name.clone());
            }
            // Include ALL skills discovered on disk (workspace + global)
            for name in &self.all_discovered_skills {
                set.insert(name.clone());
            }
            set.into_iter().collect()
        };

        let skills_enabled: Vec<bool> = all_skills.iter().map(|s| skills.contains(s)).collect();

        // Collect all unique MCPs across all agents
        let all_mcps: Vec<String> = {
            let mut set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            for agent in &self.agents {
                for m in &agent.mcps {
                    set.insert(m.as_str());
                }
            }
            set.into_iter().map(String::from).collect()
        };

        let mcps_enabled: Vec<bool> = all_mcps.iter().map(|m| mcps.contains(m)).collect();

        // Collect all unique subagent names from the merged config
        // (workspace .agents/agents/ + global $HOME/.agents/agents/)
        let all_subagents: Vec<String> = self.all_agent_names.clone();

        let subagents_enabled: Vec<bool> = if let Some(sa) = subagents {
            all_subagents.iter().map(|s| sa.contains(s)).collect()
        } else {
            vec![false; all_subagents.len()]
        };

        self.edit_dialog = EditDialogState::new_with(
            target_name,
            is_root,
            all_skills,
            skills_enabled,
            all_mcps,
            mcps_enabled,
            all_subagents,
            subagents_enabled,
        );
    }

    /// Open the edit dialog for the currently focused panel's selection.
    ///
    /// Returns `true` if a dialog was opened. Used by the Ctrl+E handler in
    /// `handle_key`, which runs before the keymap dispatch so that Ctrl+E is
    /// not swallowed by `Action::OpenEditor`.
    pub(crate) fn open_edit_dialog_for_focus(&mut self) -> bool {
        match self.focus {
            Focus::Agents => {
                let selected = {
                    let display_agents: Vec<&AgentInfo> = self
                        .agents
                        .iter()
                        .filter(|a| a.status != AgentStatus::Completed)
                        .collect();
                    display_agents
                        .get(self.agent_panel_index)
                        .cloned()
                        .map(|a| {
                            let subagents = if a.role == AgentRole::Root {
                                self.configured_subagents
                                    .get(&a.name)
                                    .cloned()
                                    .unwrap_or_default()
                            } else {
                                Vec::new()
                            };
                            (
                                a.name.clone(),
                                a.role == AgentRole::Root,
                                a.skills.clone(),
                                a.mcps.clone(),
                                subagents,
                            )
                        })
                };
                if let Some((name, is_root, skills, mcps, subagents)) = selected {
                    self.open_edit_dialog(
                        name,
                        is_root,
                        &skills,
                        &mcps,
                        if is_root { Some(&subagents) } else { None },
                    );
                    true
                } else {
                    false
                }
            }
            Focus::Info if self.info_tab == 2 => {
                let unique_subagents: Vec<&str> = {
                    let set: std::collections::BTreeSet<&str> = self
                        .configured_subagents
                        .values()
                        .flat_map(|v| v.iter().map(|s| s.as_str()))
                        .collect();
                    set.into_iter().collect()
                };
                if let Some(&name) = unique_subagents.get(self.subagent_panel_index) {
                    // Find the first agent that has this subagent type to get its skills/MCPs.
                    let (skills, mcps) = self
                        .agents
                        .iter()
                        .find(|a| a.agent_type.as_deref() == Some(name))
                        .map(|a| (a.skills.clone(), a.mcps.clone()))
                        .unwrap_or_default();
                    self.open_edit_dialog(name.to_string(), false, &skills, &mcps, None);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Determine whether the current theme is a dark theme by checking the
    /// brightness of the accent color. Darker themes have avg < 128 per
    /// channel (total brightness < 384).
    pub(crate) fn is_dark(&self) -> bool {
        match self.theme.accent() {
            Color::Rgb(r, g, b) => {
                let brightness = r as u16 + g as u16 + b as u16;
                brightness < 384
            }
            _ => true, // non-RGB colors default to dark
        }
    }
}

/// Minimum interval between renders (in microseconds) when streaming is
/// active. This caps the render rate at ~30 fps to avoid burning CPU on
/// full re-renders when only a few streamed characters have changed.
const RENDER_INTERVAL_US: u64 = 33_000; // ≈ 30 fps

/// Run the TUI event loop.
pub async fn run_tui<B: ratatui::backend::Backend<Error = std::io::Error>>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> std::io::Result<()> {
    loop {
        // Drain ALL pending engine events BEFORE drawing, so the render
        // always shows the latest state (not the state from the previous cycle)
        let mut had_events = false;
        loop {
            match app.event_rx.try_recv() {
                Ok(event) => {
                    app.handle_event(event);
                    had_events = true;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return Ok(());
                }
                Err(_) => break, // Empty, continue to draw
            }
        }

        // Render throttling: skip this frame if we rendered less than
        // RENDER_INTERVAL_US ago, unless there were pending events (which
        // means state changed and we need to refresh).
        let now = Instant::now();
        let elapsed = now.duration_since(app.last_render_time);
        if !had_events && elapsed.as_micros() < RENDER_INTERVAL_US as u128 {
            // Still within the throttle window: just poll for keyboard input
            // without rendering. This keeps the UI responsive while avoiding
            // wasteful re-renders when nothing is happening.
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        app.handle_key(key.code, key.modifiers);
                    }
                    Event::Mouse(mouse) => {
                        app.handle_mouse(mouse);
                    }
                    _ => {}
                }
            }
            if app.should_exit {
                break;
            }
            continue;
        }

        // Draw the UI (now with up-to-date state)
        app.last_render_time = now;
        app.frame_count = app.frame_count.wrapping_add(1);
        app.toasts.tick(Instant::now());
        terminal.draw(|f| render(f, app))?;

        // Check for keyboard input (with timeout for responsiveness)
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.handle_key(key.code, key.modifiers);
                }
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse);
                }
                _ => {}
            }
        }

        if app.should_exit {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{AgentId, AgentStatus};
    use crate::tui::state::fuzzy_score;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui_textarea::CursorMove;

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
    fn typing_c_with_nonempty_input_inserts_char_not_focus() {
        // Regression: 'c' must be typed, not switch focus to Chat, when input
        // already has text.
        let mut app = test_app();
        app.textarea = TextArea::from(["he"]);
        app.textarea.move_cursor(CursorMove::End);
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(app.textarea.lines().join("\n"), "hec");
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn c_with_empty_input_typing_inserts_char() {
        // Regression: 'c' with empty input must be typed, not switch to Chat.
        let mut app = test_app();
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(app.textarea.lines().join("\n"), "c");
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn q_with_empty_input_typing_inserts_char_not_quit() {
        // Regression: 'q' with empty input must be typed, not quit.
        let mut app = test_app();
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(app.textarea.lines().join("\n"), "q");
        assert_eq!(app.focus, Focus::Input);
        assert!(!app.should_exit);
    }

    #[test]
    fn n_with_empty_input_typing_inserts_char() {
        // Regression: 'N' with empty input must be typed.
        let mut app = test_app();
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Char('N'), KeyModifiers::NONE);
        assert_eq!(app.textarea.lines().join("\n"), "N");
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
    fn alt_1_switches_focus_to_input() {
        let mut app = test_app();
        app.textarea = TextArea::from(["some text"]);
        app.focus = Focus::Chat;
        app.handle_key(KeyCode::Char('1'), KeyModifiers::ALT);
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn alt_5_switches_focus_to_queue() {
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.handle_key(KeyCode::Char('5'), KeyModifiers::ALT);
        assert_eq!(app.focus, Focus::Queue);
    }

    #[test]
    fn input_left_moves_cursor_back() {
        let mut app = test_app();
        app.textarea = TextArea::from(["hola"]);
        // Cursor is at end (4), press Left to move back.
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Left, KeyModifiers::NONE);
        // TextArea cursor is internal — we verify by the text being unchanged
        // and focus staying on Input.
        assert_eq!(app.textarea.lines().join("\n"), "hola");
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn input_cursor_home_jumps_to_start() {
        let mut app = test_app();
        app.textarea = TextArea::from(["hola"]);
        app.focus = Focus::Input;
        app.handle_key(KeyCode::Home, KeyModifiers::NONE);
        // TextArea handles cursor internally; verify no crash & focus unchanged.
        assert_eq!(app.textarea.lines().join("\n"), "hola");
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn chat_j_scrolls_down_and_k_scrolls_up() {
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.chat_scroll = 5;
        // j scrolls down (shows newer content, decreases scroll).
        app.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.chat_scroll, 4);
        // k scrolls up (shows older content, increases scroll).
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

    /// Build an `AgentInfo` with the given name and status.
    fn agent(name: &str, status: AgentStatus) -> AgentInfo {
        AgentInfo {
            id: AgentId::new(),
            name: name.to_string(),
            role: AgentRole::Root,
            status,
            skills: Vec::new(),
            mcps: Vec::new(),
            model: String::new(),
            parent_id: None,
            subagent_count: 0,
            agent_type: None,
            mode: None,
        }
    }

    #[test]
    fn drain_queue_if_idle_sends_first_item() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.active_agent = "root".to_string();
        app.agents.push(agent("root", AgentStatus::Idle));
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];

        app.drain_queue_if_idle();

        assert_eq!(app.prompt_queue, vec!["second".to_string()]);
        match cmd_rx.try_recv() {
            Ok(EngineCommand::UserInput(text)) => assert_eq!(text, "first"),
            other => panic!("expected UserInput, got {:?}", other),
        }
    }

    #[test]
    fn drain_queue_if_idle_does_nothing_when_sent_message_is_true() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.sent_message = true;
        app.prompt_queue = vec!["first".to_string()];

        app.drain_queue_if_idle();

        assert_eq!(app.prompt_queue, vec!["first".to_string()]);
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn drain_queue_if_idle_does_nothing_when_agent_busy() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.active_agent = "root".to_string();
        app.agents.push(agent("root", AgentStatus::Working));
        app.prompt_queue = vec!["first".to_string()];

        app.drain_queue_if_idle();

        // sent_message is false, but the agent is busy —
        // the item must stay queued.
        assert_eq!(app.prompt_queue, vec!["first".to_string()]);
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn drain_queue_if_idle_does_nothing_when_queue_empty() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.active_agent = "root".to_string();
        app.agents.push(agent("root", AgentStatus::Idle));

        app.drain_queue_if_idle();

        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn drain_queue_if_idle_drains_one_item_at_a_time() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.active_agent = "root".to_string();
        app.agents.push(agent("root", AgentStatus::Idle));
        app.prompt_queue = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];

        // First drain sends only the first item; the rest stay queued.
        app.drain_queue_if_idle();
        assert_eq!(
            app.prompt_queue,
            vec!["second".to_string(), "third".to_string()]
        );
        match cmd_rx.try_recv() {
            Ok(EngineCommand::UserInput(text)) => assert_eq!(text, "first"),
            other => panic!("expected UserInput, got {:?}", other),
        }

        // Second drain sends the next item, one at a time.
        // Clear sent_message first to simulate receiving an Idle event.
        app.sent_message = false;
        app.drain_queue_if_idle();
        assert_eq!(app.prompt_queue, vec!["third".to_string()]);
        match cmd_rx.try_recv() {
            Ok(EngineCommand::UserInput(text)) => assert_eq!(text, "second"),
            other => panic!("expected UserInput, got {:?}", other),
        }
    }

    #[test]
    fn drain_queue_if_idle_reinserts_item_when_channel_full() {
        // Channel with capacity 1, pre-filled so try_send fails with Full.
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.active_agent = "root".to_string();
        app.agents.push(agent("root", AgentStatus::Idle));
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];

        // Fill the channel so the next try_send fails.
        let _ = app.cmd_tx.try_send(EngineCommand::Status);

        app.drain_queue_if_idle();

        // The item must be re-inserted at the front, not lost.
        assert_eq!(
            app.prompt_queue,
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn search_empty_query_clears_matches() {
        let mut app = test_app();
        app.messages = vec!["hello world".to_string(), "foo bar".to_string()];
        app.search.query = "".to_string();
        app.update_search_matches();
        assert!(app.search.matches.is_empty());
    }

    #[test]
    fn search_finds_matching_messages() {
        let mut app = test_app();
        app.messages = vec![
            "hello world".to_string(),
            "foo bar".to_string(),
            "hello again".to_string(),
        ];
        app.search.query = "hello".to_string();
        app.update_search_matches();
        assert_eq!(app.search.matches, vec![0, 2]);
        assert_eq!(app.search.selected, 0);
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut app = test_app();
        app.messages = vec!["Hello World".to_string(), "goodbye".to_string()];
        app.search.query = "hello".to_string();
        app.update_search_matches();
        assert_eq!(app.search.matches, vec![0]);
    }

    #[test]
    fn search_no_match_returns_empty() {
        let mut app = test_app();
        app.messages = vec!["abc".to_string(), "def".to_string()];
        app.search.query = "xyz".to_string();
        app.update_search_matches();
        assert!(app.search.matches.is_empty());
    }

    #[test]
    fn search_resets_selected_on_update() {
        let mut app = test_app();
        app.messages = vec!["a".to_string(), "b".to_string(), "a".to_string()];
        app.search.query = "a".to_string();
        app.update_search_matches();
        assert_eq!(app.search.selected, 0);
        app.search.selected = 1;
        app.search.query = "b".to_string();
        app.update_search_matches();
        assert_eq!(app.search.selected, 0);
    }
}
