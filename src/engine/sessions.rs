//! Session lifecycle command handlers for the engine.
//!
//! These methods are implemented on [`Engine`] and handle the session-related
//! slash commands (`/new`, `/resume`, `/sessions`, `/delete`, `/rename`,
//! `/undo`, `/redo`, `/fork`, `/export`, `/import`, `/share`, `/unshare`).

use std::path::PathBuf;

use uuid::Uuid;

use crate::engine::events::{EngineEvent, ExportFormat};
use crate::engine::orchestrator::Engine;
use crate::error::{Error, Result};
use crate::llm::types::{LlmMessage, MessageRole};

impl Engine {
    /// Handle new session creation.
    pub(crate) async fn handle_new_session(&mut self, name: &str) -> Result<()> {
        if let Some(ref db) = self.database {
            let session = db.create_session(name).await?;
            let session_id = session.id;
            self.active_session_id = Some(session_id);
            self.clear_undo_redo();

            // Clear active agent's conversation
            self.with_active_session(|s| s.conversation.clear()).await?;

            self.event_tx
                .send(EngineEvent::SessionSwitched {
                    id: session_id.to_string(),
                    name: name.to_string(),
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle session resume: load history and send to root agent.
    pub(crate) async fn handle_resume_session(&mut self, id_str: &str) -> Result<()> {
        let session_id = Uuid::parse_str(id_str)
            .map_err(|e| Error::Session(format!("Invalid session ID: {e}")))?;

        if let Some(db) = self.database.clone() {
            // Load messages from DB
            let messages = db.get_session_messages(session_id).await?;

            // Convert stored messages to LlmMessage
            let history: Vec<LlmMessage> = messages
                .iter()
                .map(|m| {
                    let role = match m.role.as_str() {
                        "user" => MessageRole::User,
                        "assistant" => MessageRole::Assistant,
                        "system" => MessageRole::System,
                        "tool" => MessageRole::Tool,
                        _ => MessageRole::User,
                    };
                    LlmMessage {
                        role,
                        content: m.content.clone(),
                        tool_calls: None,
                        tool_call_id: None,
                    }
                })
                .collect();

            self.active_session_id = Some(session_id);
            self.clear_undo_redo();

            // Send history to active agent
            self.with_active_session(move |s| {
                s.conversation = history;
            })
            .await?;

            // Get session name for the event
            let sessions = db.list_sessions().await?;
            let name = sessions
                .iter()
                .find(|s| s.id == session_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "unknown".into());

            self.event_tx
                .send(EngineEvent::SessionSwitched {
                    id: session_id.to_string(),
                    name,
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle listing all sessions.
    pub(crate) async fn handle_list_sessions(&self) -> Result<()> {
        if let Some(ref db) = self.database {
            let sessions = db.list_sessions().await?;
            self.event_tx
                .send(EngineEvent::SessionList(sessions))
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle pinning/unpinning a session, then refresh the session list.
    pub(crate) async fn handle_set_session_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        if let Some(ref db) = self.database {
            db.set_session_pinned(id, pinned).await?;
            let sessions = db.list_sessions().await?;
            self.event_tx
                .send(EngineEvent::SessionList(sessions))
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle deleting a session.
    pub(crate) async fn handle_delete_session(&self, id_str: &str) -> Result<()> {
        let session_id = Uuid::parse_str(id_str)
            .map_err(|e| Error::Session(format!("Invalid session ID: {e}")))?;

        if let Some(ref db) = self.database {
            db.delete_session(session_id).await?;
            self.event_tx
                .send(EngineEvent::SessionDeleted {
                    id: id_str.to_string(),
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle renaming a session.
    pub(crate) async fn handle_rename_session(&self, id_str: &str, new_name: &str) -> Result<()> {
        let session_id = Uuid::parse_str(id_str)
            .map_err(|e| Error::Session(format!("Invalid session ID: {e}")))?;

        if let Some(ref db) = self.database {
            db.rename_session(session_id, new_name).await?;
            self.event_tx
                .send(EngineEvent::SessionRenamed {
                    id: id_str.to_string(),
                    name: new_name.to_string(),
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Clear the undo/redo stacks (called on session change).
    pub(crate) fn clear_undo_redo(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Reload the session's messages into the root agent's context.
    pub(crate) async fn reload_history_to_root(&self, session_id: Uuid) -> Result<()> {
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let history: Vec<LlmMessage> = db
            .get_session_messages(session_id)
            .await?
            .iter()
            .map(|m| LlmMessage {
                role: match m.role.as_str() {
                    "user" => MessageRole::User,
                    "assistant" => MessageRole::Assistant,
                    "system" => MessageRole::System,
                    "tool" => MessageRole::Tool,
                    _ => MessageRole::User,
                },
                content: m.content.clone(),
                tool_calls: None,
                tool_call_id: None,
            })
            .collect();
        // Only sync the agent context if an active agent is actually running;
        // otherwise (e.g. headless tests) skip without failing the operation.
        if !self.agents.contains_key(&self.active_agent) {
            return Ok(());
        }
        self.with_active_session(move |s| {
            s.conversation = history;
        })
        .await?;
        Ok(())
    }

    /// Handle `/undo`: remove the last message pair and push it onto the stacks.
    pub(crate) async fn handle_undo(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let removed = db.delete_messages(session_id, 2).await?;
        if removed.is_empty() {
            return Ok(());
        }
        self.undo_stack.push(removed.clone());
        self.redo_stack.push(removed.clone());
        // Sync the root agent's context to the post-undo state.
        self.reload_history_to_root(session_id).await?;
        let removed_contents: Vec<String> = removed.iter().map(|m| m.content.clone()).collect();
        self.event_tx
            .send(EngineEvent::UndoApplied {
                removed: removed_contents,
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/redo`: restore the last undone message pair.
    pub(crate) async fn handle_redo(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        if let Some(messages) = self.redo_stack.pop() {
            db.restore_messages(session_id, &messages).await?;
            self.undo_stack.push(messages.clone());
            // Sync the root agent's context to the post-redo state.
            self.reload_history_to_root(session_id).await?;
            let restored_contents: Vec<String> =
                messages.iter().map(|m| m.content.clone()).collect();
            self.event_tx
                .send(EngineEvent::RedoApplied {
                    restored: restored_contents,
                })
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle `/fork`: create a new session copying the active session's messages.
    pub(crate) async fn handle_fork(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(db) = self.database.clone() else {
            return Ok(());
        };
        let name = db
            .get_session_name(session_id)
            .await?
            .unwrap_or_else(|| "fork".into());
        let new_session = db
            .create_session_with_parent(&format!("{name} (fork)"), Some(session_id))
            .await?;
        db.copy_messages(session_id, new_session.id).await?;
        self.active_session_id = Some(new_session.id);
        self.clear_undo_redo();

        // Load the copied history into the root agent so it has context.
        self.reload_history_to_root(new_session.id).await?;

        self.event_tx
            .send(EngineEvent::Forked {
                new_session_id: new_session.id,
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/export`: write the active session transcript to a file.
    pub(crate) async fn handle_export(
        &mut self,
        path: Option<PathBuf>,
        format: Option<ExportFormat>,
    ) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let format = format.unwrap_or(ExportFormat::Json);
        let path = match path {
            Some(p) => {
                if p.is_relative() {
                    self.workspace.join(p)
                } else {
                    p
                }
            }
            None => {
                let name = db
                    .get_session_name(session_id)
                    .await?
                    .unwrap_or_else(|| "session".into());
                let ext = match format {
                    ExportFormat::Json => "json",
                    ExportFormat::Markdown => "md",
                };
                self.workspace.join(format!("{name}.{ext}"))
            }
        };
        db.export_session(session_id, &path, format).await?;
        self.event_tx
            .send(EngineEvent::Exported { path })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/import`: import a session transcript from a file.
    pub(crate) async fn handle_import(&mut self, path: PathBuf) -> Result<()> {
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let path = if path.is_relative() {
            self.workspace.join(path)
        } else {
            path
        };
        let new_id = db.import_session(&path).await?;
        self.active_session_id = Some(new_id);
        self.clear_undo_redo();
        // Load the imported conversation into the root agent so it has context.
        self.reload_history_to_root(new_id).await?;
        self.event_tx
            .send(EngineEvent::Imported { session_id: new_id })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/share`: mark the active session as shared and generate a link.
    pub(crate) async fn handle_share(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        let link = format!("anacleto://share/{}", Uuid::new_v4());
        db.set_shared(session_id, true, Some(&link)).await?;
        self.event_tx
            .send(EngineEvent::ShareUpdated {
                shared: true,
                link: Some(link),
            })
            .await
            .ok();
        Ok(())
    }

    /// Handle `/unshare`: remove the shared state from the active session.
    pub(crate) async fn handle_unshare(&mut self) -> Result<()> {
        let Some(session_id) = self.active_session_id else {
            return Ok(());
        };
        let Some(ref db) = self.database else {
            return Ok(());
        };
        db.set_shared(session_id, false, None).await?;
        self.event_tx
            .send(EngineEvent::ShareUpdated {
                shared: false,
                link: None,
            })
            .await
            .ok();
        Ok(())
    }
}
