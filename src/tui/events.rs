//! Engine event handling for the TUI.

use crate::agent::types::{AgentRole, AgentStatus};
use crate::engine::orchestrator::EngineEvent;
use crate::tui::app::App;
use crate::tui::toast::ToastKind;
use crate::tui::types::{AgentInfo, QuestionState};

impl App {
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
            EngineEvent::ConversationCompacted {
                tokens, agent_name, ..
            } => {
                // Reflect the post-compaction buffer size in the status panel.
                self.context_tokens = tokens as u64;
                self.context_window_pct = if self.context_window > 0 {
                    (self.context_tokens as f64 / self.context_window as f64) * 100.0
                } else {
                    0.0
                };
                self.push_msg(format!(
                    "Conversación compactada ({}) — contexto: {} tokens.",
                    agent_name,
                    crate::tui::render::format_tokens(tokens as u64)
                ));
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
                        agent_type: None,
                    });
                }
            }
            EngineEvent::AgentStreamChunk { content, .. } => {
                // Commit any pending thinking as its own block first so
                // thinking appears before the content it precedes.
                commit_thinking_block(self);
                let stream = self.current_stream.get_or_insert_with(String::new);
                // If the stream ends with a tool result line (✅ or ❌) and the
                // new content doesn't start with a newline, insert one to prevent
                // the agent's response text from being concatenated to the same
                // line as the tool result.
                if !content.starts_with('\n')
                    && let Some(last_line) = stream.rsplit('\n').next()
                {
                    let trimmed = last_line.trim_start();
                    if trimmed.starts_with("\u{2705}") || trimmed.starts_with("\u{274c}") {
                        stream.push('\n');
                    }
                }
                stream.push_str(&content);
            }
            EngineEvent::AgentThinkingChunk { content, .. } => {
                if self.show_thinking {
                    // If there's an active stream, commit it as a definitive
                    // message before starting a new thinking block, so the
                    // chronological order (stream arrived before thinking) is
                    // preserved in the message timeline.
                    if self.current_stream.is_some() {
                        commit_stream_block(self);
                    }
                    // Always accumulate thinking in current_thinking.
                    // It will be flushed to the stream by the next
                    // non-thinking event handler (ToolExecution,
                    // ToolResult, AgentStreamChunk, or AgentOutput)
                    // so thinking and tool events stay interleaved.
                    let thinking = self.current_thinking.get_or_insert_with(String::new);
                    thinking.push_str(&content);
                }
            }
            EngineEvent::AgentOutput { content, .. } => {
                // Content was already streamed via AgentStreamChunk events
                // and committed by commit_stream_block (which calls push_msg,
                // which writes to both display AND log via log_msg).
                // Skip if we already streamed — don't push_msg or append_to_log.
                let was_streamed = self.current_stream.is_some();
                if self.current_stream.is_some() {
                    commit_stream_block(self);
                }
                commit_thinking_block(self);
                if !content.is_empty() && !was_streamed {
                    self.push_msg(content.clone());
                }
                self.chat_scroll = 0;
            }
            EngineEvent::AgentStatusChanged {
                agent_id,
                agent_name,
                status,
            } => {
                if let Some(agent) = self.agents.iter_mut().find(|a| a.id == agent_id) {
                    agent.status = status.clone();
                }
                // When the active agent becomes idle, clear the in-flight
                // flag and drain any queued prompts.
                if status == AgentStatus::Idle && agent_name == self.active_agent {
                    self.sent_message = false;
                    self.drain_queue_if_idle();
                }
            }
            EngineEvent::SubagentCreated {
                parent_id,
                subagent_id,
                subagent_name,
                skills,
                mcps,
                agent_type,
            } => {
                self.messages
                    .push(format!("Subagent '{}' created.", subagent_name));
                self.messages_generation = self.messages_generation.wrapping_add(1);
                self.chat_scroll = 0;
                // Track subagent in the list (added later via AgentCreated?)
                // Also bump parent's subagent_count
                if let Some(parent) = self.agents.iter_mut().find(|a| a.id == parent_id) {
                    parent.subagent_count += 1;
                }
                // Mark any previous subagents with the same name as completed
                // (handles re-delegation to the same subagent name)
                for agent in self.agents.iter_mut() {
                    if agent.name == subagent_name && agent.id != subagent_id {
                        agent.status = AgentStatus::Completed;
                    }
                }
                // Add subagent to list (if not already present by ID)
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
                        agent_type,
                    });
                }
            }
            EngineEvent::SubagentCompleted {
                subagent_id,
                subagent_name,
                result,
            } => {
                // Commit any pending stream/thinking block first so partial
                // output from the subagent is not lost (e.g. when it ran out
                // of steps mid-stream).
                if self.current_stream.is_some() {
                    commit_stream_block(self);
                }
                commit_thinking_block(self);
                // Always mark the subagent as Completed so its spinner is
                // removed, regardless of the outcome.
                if let Some(agent) = self.agents.iter_mut().find(|a| a.id == subagent_id) {
                    agent.status = AgentStatus::Completed;
                }
                match result.as_str() {
                    "out_of_steps" => {
                        self.push_msg(format!(
                            "Subagent '{}' sin pasos (se detuvo sin completar la tarea).",
                            subagent_name
                        ));
                        self.toasts.push(
                            format!("Subagente '{}' sin pasos", subagent_name),
                            ToastKind::Warning,
                        );
                    }
                    "error" => {
                        self.push_msg(format!("Subagent '{}' terminó con error.", subagent_name));
                        self.toasts.push(
                            format!("Subagente '{}' con error", subagent_name),
                            ToastKind::Warning,
                        );
                    }
                    _ => {
                        self.messages
                            .push(format!("Subagent '{}' completed.", subagent_name));
                    }
                }
                self.messages_generation = self.messages_generation.wrapping_add(1);
                self.chat_scroll = 0;
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
                // Reset the in-flight flag and re-check the prompt queue: a
                // stale Idle event from the previous agent must not leave
                // `sent_message` latched and deadlock the queue.
                self.sent_message = false;
                self.drain_queue_if_idle();
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
            EngineEvent::Question {
                id,
                question,
                options,
                recommended,
            } => {
                self.pending_question = Some(QuestionState {
                    id,
                    question: question.clone(),
                    options: options.clone(),
                    recommended: recommended.clone(),
                    selected: 0,
                    answer_input: String::new(),
                });
                let opt_str = if options.is_empty() {
                    String::new()
                } else {
                    format!(" [{} opciones]", options.len())
                };
                self.push_msg(format!("[Pregunta{}] {}", opt_str, question));
            }
            EngineEvent::TokenUsage {
                total_tokens,
                prompt_tokens,
                context_window,
                cost,
                ..
            } => {
                self.total_tokens += total_tokens as u64;
                self.context_window = context_window as u64;
                // `context_tokens` is non-cumulative: the prompt sent to the LLM
                // is a good proxy for the current conversation buffer size, so
                // it drops naturally after compaction instead of growing forever.
                self.context_tokens = prompt_tokens as u64;
                self.context_window_pct =
                    (self.context_tokens as f64 / context_window as f64) * 100.0;
                // Cost is computed in the engine from per-million-token prices.
                self.total_cost += cost;

                // Warn once when the LOCAL estimate crosses the 70% threshold
                // (same metric that compaction uses, not the API prompt_tokens)
                let local_pct = if context_window > 0 {
                    (self.local_context_tokens as f64 / context_window as f64) * 100.0
                } else {
                    0.0
                };
                if local_pct >= 70.0 && !self.context_warned {
                    self.context_warned = true;
                    self.toasts.push(
                        "⚠ Contexto alto — usa /compact para compactar",
                        ToastKind::Warning,
                    );
                } else if local_pct < 60.0 {
                    // Reset the flag so the warning can fire again if it climbs back up
                    self.context_warned = false;
                }
            }
            EngineEvent::ToolExecution {
                tool_name, task, ..
            } => {
                // Commit any pending thinking first
                commit_thinking_block(self);
                commit_stream_block(self);

                let (icon, _) = tool_icon_and_label(&tool_name);
                let msg = format!("{} {}: {}", icon, tool_name, one_line(&task, 500));
                self.pending_tool_lines.push(msg);
                self.chat_scroll = 0;
            }
            EngineEvent::ToolResult {
                tool_name,
                success,
                summary,
                ..
            } => {
                // Commit any thinking that arrived between ToolExecution
                // and ToolResult, then push the result as its own message.
                commit_thinking_block(self);
                let (_, label) = tool_icon_and_label(&tool_name);
                let icon = if success { "\u{2705}" } else { "\u{274c}" };
                let msg = if success {
                    format!("{} {} \u{2014} {}", icon, label, summary)
                } else {
                    format!("{} {} failed: {}", icon, label, summary)
                };
                // Split header from output so the renderer can style
                // the output (JSON, shell text) with a dimmed style.
                let (header, rest) = match msg.split_once('\n') {
                    Some((h, r)) => (h.to_string(), r.to_string()),
                    None => (msg.clone(), String::new()),
                };
                self.pending_tool_lines.push(header);
                if !rest.is_empty() {
                    self.pending_tool_lines.push("[tool-output]".to_string());
                    self.pending_tool_lines.push(rest);
                }
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
                self.messages_generation = self.messages_generation.wrapping_add(1);
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
            EngineEvent::SkillsDiscovered { skills } => {
                self.all_discovered_skills = skills;
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
            EngineEvent::LocalTokenEstimate { tokens } => {
                self.local_context_tokens = tokens as u64;
            }
            // ── FASE 1 y 2: build y snapshots ─────────────────
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
            EngineEvent::HookExecuted {
                point,
                command,
                success,
                output,
            } => {
                let icon = if success {
                    "\u{2705}"
                } else {
                    "\u{26a0}\u{fe0f}"
                };
                let msg = format!("{} Hook {}: {}", icon, point, command);
                self.push_msg(msg);
                if !output.is_empty() {
                    self.push_msg(format!(
                        "  \u{2514}\u{2500} {}",
                        output.lines().next().unwrap_or("")
                    ));
                }
                self.chat_scroll = 0;
            }
            _ => {}
        }
    }
}

/// Commit any pending thinking block as a separate message, wrapped in
/// `[thinking]`/`[/thinking]` markers so the renderer can style it.
fn commit_thinking_block(app: &mut App) {
    if let Some(thinking) = app.current_thinking.take()
        && !thinking.trim().is_empty()
    {
        app.push_msg(format!("[thinking]\n{}\n[/thinking]", thinking.trim()));
    }
}

/// Commit the current stream block as a message.
fn commit_stream_block(app: &mut App) {
    if let Some(stream) = app.current_stream.take()
        && !stream.trim().is_empty()
    {
        app.push_msg(stream);
    }
}

/// Collapse a string to a single line (newlines become spaces) and truncate
/// it to at most `max_chars` characters, appending an ellipsis when cut.
///
/// Used to keep tool execution/result markers compact in the chat.
fn tool_icon_and_label(name: &str) -> (&'static str, String) {
    match name {
        "shell" => ("\u{26a1}", "shell".to_string()),
        "filesystem" => ("\u{1f4c1}", "filesystem".to_string()),
        "read" => ("\u{1f4d6}", "read".to_string()),
        "grep" => ("\u{1f50d}", "grep".to_string()),
        "glob" => ("\u{1f4c2}", "glob".to_string()),
        "webfetch" => ("\u{1f310}", "webfetch".to_string()),
        "websearch" => ("\u{1f50e}", "websearch".to_string()),
        "todo" => ("\u{1f4dd}", "todo".to_string()),
        "question" => ("\u{2753}", "question".to_string()),
        "apply_patch" => ("\u{1f527}", "apply_patch".to_string()),
        "lsp_query" => ("\u{1f52c}", "lsp_query".to_string()),
        "delegate" => ("\u{1f916}", "delegate".to_string()),
        _ if name.starts_with("mcp_") => ("\u{1f50c}", name.to_string()),
        // Passive skills (loaded instructions)
        _ => ("\u{1f4d6}", name.to_string()),
    }
}

fn one_line(s: &str, max_chars: usize) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        let cut: String = collapsed.chars().take(max_chars).collect();
        format!("{}…", cut.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::AgentId;
    use crate::config::Config;
    use crate::engine::orchestrator::EngineCommand;
    use tokio::sync::mpsc;

    fn agent(id: AgentId, name: &str, status: AgentStatus) -> AgentInfo {
        AgentInfo {
            id,
            name: name.to_string(),
            role: AgentRole::Root,
            status,
            skills: Vec::new(),
            mcps: Vec::new(),
            model: String::new(),
            parent_id: None,
            subagent_count: 0,
            agent_type: None,
        }
    }

    #[test]
    fn one_line_collapses_and_truncates() {
        assert_eq!(one_line("a\nb\nc", 100), "a b c");
        assert_eq!(one_line("  spaced   out  ", 100), "spaced out");
        // Truncation appends an ellipsis.
        let long = "x".repeat(200);
        let out = one_line(&long, 100);
        assert_eq!(out.chars().count(), 101);
        assert!(out.ends_with('…'));
        // Short content is returned unchanged (collapsed).
        assert_eq!(one_line("ok", 100), "ok");
    }

    #[test]
    fn idle_event_of_active_agent_drains_queue() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        let id = AgentId::new();
        app.active_agent = "root".to_string();
        app.agents
            .push(agent(id.clone(), "root", AgentStatus::Working));
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];

        app.handle_event(EngineEvent::AgentStatusChanged {
            agent_id: id,
            agent_name: "root".to_string(),
            status: AgentStatus::Idle,
        });

        // The first item was sent and removed from the queue.
        assert_eq!(app.prompt_queue, vec!["second".to_string()]);
        match cmd_rx.try_recv() {
            Ok(EngineCommand::UserInput(text)) => assert_eq!(text, "first"),
            other => panic!("expected UserInput, got {:?}", other),
        }
    }

    #[test]
    fn idle_event_of_non_active_agent_does_not_drain() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        let id = AgentId::new();
        app.active_agent = "root".to_string();
        app.agents
            .push(agent(id.clone(), "sub", AgentStatus::Working));
        app.prompt_queue = vec!["first".to_string()];

        app.handle_event(EngineEvent::AgentStatusChanged {
            agent_id: id,
            agent_name: "sub".to_string(),
            status: AgentStatus::Idle,
        });

        // Queue untouched, nothing sent.
        assert_eq!(app.prompt_queue, vec!["first".to_string()]);
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn agent_output_persists_thinking_before_response() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.show_thinking = true;

        // Accumulate thinking and stream during generation.
        app.handle_event(EngineEvent::AgentThinkingChunk {
            agent_id: AgentId::new(),
            agent_name: "root".to_string(),
            content: "reasoning step".to_string(),
        });
        app.handle_event(EngineEvent::AgentStreamChunk {
            agent_id: AgentId::new(),
            agent_name: "root".to_string(),
            content: "hello world".to_string(),
        });

        // On completion the thinking is committed as its own block and the
        // streamed response text becomes a separate message.
        app.handle_event(EngineEvent::AgentOutput {
            agent_id: AgentId::new(),
            agent_name: "root".to_string(),
            content: String::new(),
        });

        assert_eq!(
            app.messages,
            vec![
                "[thinking]\nreasoning step\n[/thinking]".to_string(),
                "[normal]\nhello world\n[/normal]".to_string(),
            ]
        );
        assert!(app.current_thinking.is_none());
        assert!(app.current_stream.is_none());
    }

    #[test]
    fn agent_output_skips_empty_thinking() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.show_thinking = true;

        // Whitespace-only thinking should not add a marker to the message.
        app.handle_event(EngineEvent::AgentThinkingChunk {
            agent_id: AgentId::new(),
            agent_name: "root".to_string(),
            content: " \n ".to_string(),
        });
        app.handle_event(EngineEvent::AgentOutput {
            agent_id: AgentId::new(),
            agent_name: "root".to_string(),
            content: "answer".to_string(),
        });

        assert_eq!(
            app.messages,
            vec!["[normal]\nanswer\n[/normal]".to_string()]
        );
    }

    #[test]
    fn subagent_completed_out_of_steps_marks_completed_and_warns() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        let id = AgentId::new();
        app.agents
            .push(agent(id.clone(), "sub", AgentStatus::Working));

        app.handle_event(EngineEvent::SubagentCompleted {
            subagent_id: id,
            subagent_name: "sub".to_string(),
            result: "out_of_steps".to_string(),
        });

        // Spinner must be removed: status is no longer Working.
        assert_eq!(app.agents[0].status, AgentStatus::Completed);
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("sin pasos") && m.contains("sub"))
        );
        assert!(
            !app.toasts.is_empty(),
            "expected a warning toast for out_of_steps"
        );
    }

    #[test]
    fn subagent_completed_error_marks_completed_and_warns() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        let id = AgentId::new();
        app.agents
            .push(agent(id.clone(), "sub", AgentStatus::Working));

        app.handle_event(EngineEvent::SubagentCompleted {
            subagent_id: id,
            subagent_name: "sub".to_string(),
            result: "error".to_string(),
        });

        assert_eq!(app.agents[0].status, AgentStatus::Completed);
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("error") && m.contains("sub"))
        );
    }

    #[test]
    fn subagent_completed_success_marks_completed() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        let id = AgentId::new();
        app.agents
            .push(agent(id.clone(), "sub", AgentStatus::Working));

        app.handle_event(EngineEvent::SubagentCompleted {
            subagent_id: id,
            subagent_name: "sub".to_string(),
            result: "completed".to_string(),
        });

        assert_eq!(app.agents[0].status, AgentStatus::Completed);
        assert!(
            app.messages
                .iter()
                .any(|m| m.contains("completed") && m.contains("sub"))
        );
    }

    #[test]
    fn subagent_completed_commits_pending_stream() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.show_thinking = true;
        let id = AgentId::new();
        app.agents
            .push(agent(id.clone(), "sub", AgentStatus::Working));

        // Accumulate partial stream output before the subagent reports
        // it ran out of steps mid-stream.
        app.handle_event(EngineEvent::AgentStreamChunk {
            agent_id: id.clone(),
            agent_name: "sub".to_string(),
            content: "partial result".to_string(),
        });
        app.handle_event(EngineEvent::SubagentCompleted {
            subagent_id: id,
            subagent_name: "sub".to_string(),
            result: "out_of_steps".to_string(),
        });

        // Partial output must not be lost.
        assert!(app.messages.iter().any(|m| m.contains("partial result")));
        assert!(app.current_stream.is_none());
        assert_eq!(app.agents[0].status, AgentStatus::Completed);
    }
}
