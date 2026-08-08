use std::collections::HashMap;
use std::time::Instant;

use chrono::{DateTime, Utc};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::agent::types::{AgentRole, AgentStatus};
use crate::config::Config;
use crate::config::types::CustomCommand;
use crate::db::models::SessionSummary;
use crate::engine::orchestrator::{
    EngineCommand, EngineEvent, McpStatus, SkillInfo, StatusInfo, TimelineEntry,
};
use crate::tui::diff_viewer::DiffViewer;
use crate::tui::keymap::Keymap;
use crate::tui::model_picker::ModelPicker;
use crate::tui::render::render;
use crate::tui::theme::Theme;
use crate::tui::toast::ToastQueue;
use crate::tui::types::{
    AgentInfo, ApprovalRequest, BUILTIN_COMMANDS, Focus, InitFlow, QuestionState, SearchState,
};
use crate::tui::which_key::WhichKeyPopup;

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
    /// Active tab in the Info panel (0 = Skills, 1 = MCPs).
    pub(crate) info_tab: usize,
    /// Selected index in the MCPs sidebar panel.
    pub(crate) mcp_panel_index: usize,
    /// Vertical scroll offset for the MCPs panel.
    pub(crate) mcp_scroll: usize,
    /// Selected index in the Skills sidebar panel.
    pub(crate) skill_panel_index: usize,
    /// Vertical scroll offset for the Skills panel.
    pub(crate) skill_scroll: usize,
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
    pub(crate) stream_committed_index: Option<usize>,
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
    pub(crate) skills_list: Vec<SkillInfo>,
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
            info_tab: 0,
            mcp_panel_index: 0,
            mcp_scroll: 0,
            skill_panel_index: 0,
            skill_scroll: 0,
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
            sent_message: false,
            search: SearchState::default(),
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
            self.push_msg(format!("> {}", item));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{AgentId, AgentStatus};
    use crate::tui::state::fuzzy_score;
    use crossterm::event::{KeyCode, KeyModifiers};

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
