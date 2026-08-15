//! Slash-command dispatch and input processing.
//!
//! Contains the `App` methods that turn a submitted line of input into
//! engine commands or local actions: `process_input`, the large
//! `handle_command` dispatcher, the external-editor helpers, and the
//! small actions used by the key handlers (quick slots, `/init` flow,
//! timeline/MCP navigation, panel closing).

use std::collections::HashMap;
use std::path::PathBuf;

use super::app::App;
use super::render::copy_to_clipboard;
use super::types::InitFlow;
use crate::db::models::SessionSummary;
use crate::engine::orchestrator::{EngineCommand, ExportFormat, InitAnswers};
use crate::engine::template::expand_vars;

impl App {
    /// Process a line of input — check for slash commands or send to engine.
    pub(crate) fn process_input(&mut self, input: String) {
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
            // Enqueue the prompt and drain immediately if the active agent is
            // idle; otherwise it waits in the visible/editable queue until the
            // agent is free.
            self.prompt_queue.push(input);
            self.drain_queue_if_idle();
        }
    }

    /// Handle a slash command.
    pub(crate) fn handle_command(&mut self, input: String) {
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
                match (parts.get(1), parts.get(2)) {
                    (Some(name), None) => {
                        // Rename active session
                        if let Some(ref id) = self.session_id {
                            let id = id.clone();
                            self.push_msg(format!("> /rename {} {}", id, name));
                            let _ = self
                                .cmd_tx
                                .try_send(EngineCommand::RenameSession(id, name.to_string()));
                        } else {
                            self.push_msg("No active session to rename.");
                        }
                    }
                    (Some(id), Some(name)) => {
                        self.push_msg(format!("> /rename {} {}", id, name));
                        let _ = self.cmd_tx.try_send(EngineCommand::RenameSession(
                            id.to_string(),
                            name.to_string(),
                        ));
                    }
                    _ => {
                        self.messages
                            .push("Usage: /rename [<session-id>] <new-name>".into());
                    }
                }
            }
            "/log" => {
                if !self.log_enabled {
                    // Enable logging
                    if let Some(ref session_id) = self.session_id {
                        let log_dir = dirs::data_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("anacleto")
                            .join("logs");
                        if let Err(e) = std::fs::create_dir_all(&log_dir) {
                            self.push_msg(format!("Error creating log dir: {}", e));
                            return;
                        }
                        let log_path = log_dir.join(format!("{}.md", session_id));
                        // Write header
                        let header = format!("# Session log: {}\n\n", session_id);
                        if let Err(e) = std::fs::write(&log_path, &header) {
                            self.push_msg(format!("Error creating log file: {}", e));
                            return;
                        }
                        self.log_path = Some(log_path.clone());
                        self.log_enabled = true;
                        self.push_msg(format!("Logging enabled -> {}", log_path.display()));
                    } else {
                        self.push_msg("No active session to log.");
                    }
                } else {
                    // Disable logging
                    self.log_enabled = false;
                    if let Some(ref path) = self.log_path {
                        self.push_msg(format!("Logging disabled -> {}", path.display()));
                    }
                    self.log_path = None;
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
                    // Drain immediately if the active agent is idle, otherwise
                    // the item would sit in the queue until a future Idle event.
                    self.drain_queue_if_idle();
                }
            }
            // ── Agent info commands ────────────────────────────────
            "/agent" | "/agents" | "/a" => {
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
            "/reload" | "/rl" => {
                self.push_msg("> /reload");
                let _ = self.cmd_tx.try_send(EngineCommand::ReloadAgent);
            }
            "/exit" | "/quit" => {
                self.push_msg("> /exit");
                self.should_exit = true;
            }
            "/help" | "/h" => {
                self.push_msg("> /help");
                self.push_msg(
                    "Commands: /sessions, /new <name>, /resume <id>, /delete <id>, \
                     /rename <id> <name>, /reload, /agents, /subagents, /debug, /copy, \
                     /compact, /models, /exit, /help",
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
            "/todos" | "/t" => {
                self.push_msg("> /todos");
                self.close_panels();
                self.show_todos = true;
                self.todos_index = 0;
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
                        self.set_textarea_text(saved.as_str());
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
                    if self.textarea.lines().join("\n").trim().is_empty() {
                        self.push_msg("> /stash — nothing to stash.");
                    } else {
                        self.stash_stack.push(self.textarea.lines().join("\n"));
                        self.reset_textarea();
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
    pub(crate) fn open_editor(&mut self) {
        let tmp = std::env::temp_dir().join(format!("anacleto-edit-{}.txt", std::process::id()));
        if std::fs::write(&tmp, self.textarea.lines().join("\n")).is_err() {
            self.push_msg("Error: could not write temp file for editor".to_string());
            return;
        }
        self.open_file_in_editor(&tmp);
        if let Ok(contents) = std::fs::read_to_string(&tmp) {
            self.set_textarea_text(contents.trim_end_matches('\n'));
        }
        let _ = std::fs::remove_file(&tmp);
    }

    /// Open an arbitrary file in the external editor, suspending raw mode.
    pub(crate) fn open_file_in_editor(&mut self, path: &std::path::Path) {
        let editor = self
            .editor
            .clone()
            .or_else(|| std::env::var("EDITOR").ok())
            .or_else(|| std::env::var("VISUAL").ok())
            .unwrap_or_else(|| "vi".to_string());
        // Suspend raw mode and leave the alternate screen so the editor
        // can take over the terminal cleanly.
        let suspended = crossterm::terminal::disable_raw_mode().is_ok()
            && crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)
                .is_ok();
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{} \"{}\"", editor, path.display()))
            .status();
        // Restore the terminal before reporting the result.
        if suspended {
            let _ =
                crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen);
            let _ = crossterm::terminal::enable_raw_mode();
        }
        if let Err(e) = status {
            self.push_msg(format!("Error launching editor: {}", e));
        }
    }

    /// Resume the pinned session at the given quick-slot index (0-based).
    pub(crate) fn resume_quick_slot(&mut self, index: usize) {
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
    pub(crate) fn collect_init_answer(&mut self) {
        let Some(mut flow) = self.init_flow.take() else {
            return;
        };
        let answer = self.textarea.lines().join("\n");
        self.reset_textarea();
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
    pub(crate) fn jump_to_timeline_entry(&mut self) {
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
    pub(crate) fn toggle_selected_mcp(&mut self) {
        if let Some(mcp) = self.mcps_list.get(self.mcps_index) {
            let name = mcp.name.clone();
            let enabled = !mcp.enabled;
            let _ = self
                .cmd_tx
                .try_send(EngineCommand::ToggleMcp { name, enabled });
        }
    }

    /// Close all overlay panels (session list, agents, subagents, timeline, mcps).
    pub(crate) fn close_panels(&mut self) {
        self.show_session_list = false;
        self.show_agents = false;
        self.show_subagents = false;
        self.show_timeline = false;
        self.show_mcps = false;
        self.show_todos = false;
        self.show_workspace_palette = false;
        self.show_skill_palette = false;
    }
}
