//! Miscellaneous command handlers for the engine.
//!
//! These methods are implemented on [`Engine`] and handle the non-session slash
//! commands (`/skills`, `/mcps`, `/status`, `/init`, `/review`, `/warp`,
//! `/workspaces`, `/move`, `/worktree`, `/timeline`, `/build`, `/parent`,
//! `/children`, `/jobs`, `/snapshot`, `/revert`, `/snapshots`, `/stage`,
//! `/clear`, `/commit`, and approval responses).

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use uuid::Uuid;

use crate::agent::types::AgentMessage;
use crate::db::models::StoredMessage;
use crate::engine::events::{
    EngineEvent, InitAnswers, McpStatus, SkillInfo, StatusInfo, TimelineEntry,
};
use crate::engine::orchestrator::Engine;
use crate::error::{Error, Result};
use crate::shell::{git_worktree_add, git_worktree_list, git_worktree_remove};

impl Engine {
    /// Handle `/skills`: list the skills of the root agent.
    pub(crate) async fn handle_list_skills(&self) -> Result<()> {
        let registry = self.skill_registry.read().await;
        let infos: Vec<SkillInfo> = registry
            .list()
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
    pub(crate) async fn handle_list_mcps(&self) -> Result<()> {
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
    pub(crate) async fn handle_toggle_mcp(&mut self, name: &str, enabled: bool) -> Result<()> {
        self.mcp_enabled
            .lock()
            .await
            .insert(name.to_string(), enabled);
        // Re-list so the TUI reflects the new state.
        self.handle_list_mcps().await?;
        Ok(())
    }

    /// Handle `/status`: produce an engine status report.
    pub(crate) async fn handle_status(&self) -> Result<()> {
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
    pub(crate) async fn handle_init(&mut self, answers: InitAnswers) -> Result<()> {
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
    pub(crate) async fn handle_review(&mut self, target: Option<String>) -> Result<()> {
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
    pub(crate) async fn handle_warp(&mut self, dir: PathBuf) -> Result<()> {
        self.workspace = dir.clone();
        self.event_tx
            .send(EngineEvent::WorkspaceChanged(dir))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/workspaces`: list the known workspaces.
    pub(crate) async fn handle_list_workspaces(&self) -> Result<()> {
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
    pub(crate) async fn handle_move_session(&mut self, workspace: &str) -> Result<()> {
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
    pub(crate) async fn handle_worktree_add(&self, path: &str, branch: Option<&str>) -> Result<()> {
        let result = git_worktree_add(&self.workspace, path, branch)
            .unwrap_or_else(|e| format!("error: {}", e));
        self.event_tx
            .send(EngineEvent::WorktreeResult(result))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/worktree list`: list git worktrees.
    pub(crate) async fn handle_worktree_list(&self) -> Result<()> {
        let result = git_worktree_list(&self.workspace).unwrap_or_else(|e| format!("error: {}", e));
        self.event_tx
            .send(EngineEvent::WorktreeResult(result))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/worktree remove`: remove a git worktree.
    pub(crate) async fn handle_worktree_remove(&self, path: &str) -> Result<()> {
        let result =
            git_worktree_remove(&self.workspace, path).unwrap_or_else(|e| format!("error: {}", e));
        self.event_tx
            .send(EngineEvent::WorktreeResult(result))
            .await
            .ok();
        Ok(())
    }

    /// Handle `/timeline`: produce the timeline of the active session.
    pub(crate) async fn handle_timeline(&self) -> Result<()> {
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
    ///
    /// If `PLAN.md` does not exist, a descriptive [`EngineEvent::Error`] is
    /// emitted (instead of aborting the command) so the user gets clear
    /// feedback and the engine loop continues.
    pub(crate) async fn handle_build(&mut self) -> Result<()> {
        let path = self.workspace.join("PLAN.md");
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.event_tx
                    .send(EngineEvent::Error {
                        agent_id: None,
                        message: format!(
                            "No se encontró PLAN.md en el workspace ({}). \
                             Crea el plan antes de usar /build.",
                            path.display()
                        ),
                    })
                    .await
                    .ok();
                return Ok(());
            }
            Err(e) => return Err(Error::Io(e)),
        };
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
    pub(crate) async fn handle_parent(&mut self) -> Result<()> {
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
    pub(crate) async fn handle_children(&self) -> Result<()> {
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
    pub(crate) async fn handle_list_jobs(&self) -> Result<()> {
        let ids = self.job_registry.lock().await.running_ids();
        self.event_tx.send(EngineEvent::JobsListed(ids)).await.ok();
        Ok(())
    }

    /// Handle `/snapshot`: create a snapshot of the active session's conversation.
    ///
    /// The snapshot captures the serialized message list so it can be restored
    /// later via `/revert`.
    pub(crate) async fn handle_snapshot(&mut self, name: Option<&str>) -> Result<()> {
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
    pub(crate) async fn handle_revert(&mut self, snapshot_id: Uuid) -> Result<()> {
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
        // Parse the snapshot's messages BEFORE touching the session, so that a
        // deserialization failure leaves the current messages intact instead of
        // leaving the session empty/corrupt. The deletion only happens after
        // the restore payload has been fully loaded and validated in memory.
        let restored: Vec<StoredMessage> = serde_json::from_str(&snapshot.content)?;
        // Remove all current messages, then restore the snapshot's messages.
        let current = db.get_session_messages(session_id).await?;
        if !current.is_empty() {
            db.delete_messages(session_id, current.len()).await?;
        }
        db.restore_messages(session_id, &restored).await?;
        self.reload_history_to_root(session_id).await?;
        self.event_tx
            .send(EngineEvent::SnapshotReverted { snapshot_id })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/snapshots`: list the snapshots of the active session.
    pub(crate) async fn handle_list_snapshots(&self) -> Result<()> {
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
    pub(crate) async fn handle_stage(&mut self, name: Option<&str>) -> Result<()> {
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
    pub(crate) async fn handle_clear(&mut self) -> Result<()> {
        if let Some(staged) = self.staged_snapshot.take()
            && let Some(ref db) = self.database
        {
            db.delete_snapshot(staged.id).await?;
        }
        Ok(())
    }

    /// Handle `/commit`: persist the staged snapshot as a named snapshot.
    ///
    /// The staged snapshot is renamed (if a name is provided) and kept; the
    /// staging slot is cleared.
    pub(crate) async fn handle_commit(&mut self, name: Option<&str>) -> Result<()> {
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
    pub(crate) async fn handle_approval_response(&self, id: &str, approved: bool) {
        let mut pending = self.pending_approvals.lock().await;
        if let Some(sender) = pending.remove(id) {
            let _ = sender.send(approved);
        }
    }
}
