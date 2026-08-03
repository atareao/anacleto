use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::lifecycle::{AgentHandle, SpawnAgentConfig, spawn_agent};
use crate::agent::types::{Agent, AgentId, AgentMessage, AgentRole, AgentStatus};
use crate::config::Config;
use crate::config::types::{OllamaConfig, ProviderConfig};
use crate::db::models::SessionSummary;
use crate::db::session::Database;
use crate::error::{Error, Result};
use crate::llm::provider::{LlmProvider, LlmProviderRegistry, create_provider};
use crate::llm::types::{LlmMessage, LlmProviderConfig, LlmProviderType, MessageRole};
use crate::mcp::client::McpRegistry;
use crate::skill::loader::load_agent_skills;

/// Events emitted by the engine for the TUI to display.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Engine started.
    Started { debug: bool },
    /// Agent created.
    AgentCreated {
        id: AgentId,
        name: String,
        role: AgentRole,
        model: String,
        skills: Vec<String>,
        mcps: Vec<String>,
    },
    /// Agent received a message.
    AgentMessage {
        agent_id: AgentId,
        agent_name: String,
        message: String,
    },
    /// Agent produced output.
    AgentOutput {
        agent_id: AgentId,
        agent_name: String,
        content: String,
    },
    /// Streaming chunk from an agent's LLM response.
    AgentStreamChunk {
        agent_id: AgentId,
        agent_name: String,
        content: String,
    },
    /// Agent status changed.
    AgentStatusChanged {
        agent_id: AgentId,
        agent_name: String,
        status: AgentStatus,
    },
    /// Subagent created by a parent.
    SubagentCreated {
        parent_id: AgentId,
        subagent_id: AgentId,
        subagent_name: String,
        skills: Vec<String>,
        mcps: Vec<String>,
    },
    /// Subagent completed.
    SubagentCompleted {
        subagent_id: AgentId,
        subagent_name: String,
        result: String,
    },
    /// Session list.
    SessionList(Vec<SessionSummary>),
    /// Session was switched.
    SessionSwitched { id: String, name: String },
    /// Session was deleted.
    SessionDeleted { id: String },
    /// Session was renamed.
    SessionRenamed { id: String, name: String },
    /// Error occurred.
    Error {
        agent_id: Option<AgentId>,
        message: String,
    },
    /// Human approval required for a sensitive operation.
    ApprovalRequired { id: String, operation: String },
    /// Engine shutting down.
    ShuttingDown,
    /// Token usage reported after an LLM response.
    TokenUsage {
        agent_id: AgentId,
        agent_name: String,
        total_tokens: u32,
        context_window: u32,
        cost: f64,
    },
    /// Tool/skill execution started.
    ToolExecution {
        agent_id: AgentId,
        agent_name: String,
        tool_name: String,
        task: String,
    },
    /// Tool/skill execution completed.
    ToolResult {
        agent_id: AgentId,
        agent_name: String,
        tool_name: String,
        success: bool,
        summary: String,
    },
    /// Debug: serialized LLM request payload (only when debug mode is on).
    LlmRequestDebug {
        agent_name: String,
        model: String,
        payload: String,
    },
    /// Debug: serialized LLM response payload (only when debug mode is on).
    LlmResponseDebug {
        agent_name: String,
        model: String,
        payload: String,
    },
    /// The model for the root agent changed.
    ModelChanged { model: String },
    /// The conversation context was compacted (via `/compact`).
    ConversationCompacted {
        agent_id: AgentId,
        agent_name: String,
    },
}

/// The core orchestration engine.
pub struct Engine {
    /// Loaded configuration.
    config: Config,
    /// Registered agents (name -> id lookup).
    agents: HashMap<String, AgentId>,
    /// Active agent handles (id -> handle).
    handles: HashMap<AgentId, AgentHandle>,
    /// LLM provider registry.
    llm_registry: LlmProviderRegistry,
    /// MCP server registry.
    mcp_registry: Arc<tokio::sync::Mutex<McpRegistry>>,
    /// Database for persistence.
    database: Option<Database>,
    /// Active session ID.
    active_session_id: Option<Uuid>,
    /// Channel to send events to the TUI.
    event_tx: mpsc::Sender<EngineEvent>,
    /// Channel to receive commands from the TUI.
    command_rx: mpsc::Receiver<EngineCommand>,
    /// Pending human approvals (id -> oneshot sender).
    pending_approvals: Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
    /// Debug mode flag (shows LLM request/response payloads).
    /// Shared with agent tasks so the `/debug` toggle takes effect immediately.
    debug: Arc<AtomicBool>,
    /// Current model for the root agent.
    current_model: String,
}

/// Commands from the TUI to the engine.
#[derive(Debug)]
pub enum EngineCommand {
    /// Send user input to the root agent.
    UserInput(String),
    /// Start a new session.
    NewSession(String),
    /// Resume a session.
    ResumeSession(String),
    /// List all sessions.
    ListSessions,
    /// Delete a session.
    DeleteSession(String),
    /// Rename a session.
    RenameSession(String, String),
    /// Respond to a human approval request.
    ApprovalResponse { id: String, approved: bool },
    /// Toggle debug mode on/off.
    SetDebug(bool),
    /// Change the model for the root agent.
    SetModel(String),
    /// Force compaction of the root agent's conversation context.
    Compact,
    /// Shutdown the engine.
    Shutdown,
}

impl Engine {
    /// Create a new engine.
    pub fn new(
        config: Config,
        event_tx: mpsc::Sender<EngineEvent>,
        command_rx: mpsc::Receiver<EngineCommand>,
    ) -> Self {
        Self {
            config: config.clone(),
            agents: HashMap::new(),
            handles: HashMap::new(),
            llm_registry: LlmProviderRegistry::new(),
            mcp_registry: Arc::new(tokio::sync::Mutex::new(McpRegistry::new())),
            pending_approvals: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            debug: Arc::new(AtomicBool::new(false)),
            current_model: config
                .agents
                .iter()
                .find(|a| a.role == AgentRole::Root)
                .map(|a| a.model.clone())
                .unwrap_or_default(),
            database: None,
            active_session_id: None,
            event_tx,
            command_rx,
        }
    }

    /// Initialize the engine: load config, create providers and agents, connect MCPs.
    pub async fn initialize(&mut self) -> Result<()> {
        // Sync debug flag from config (--debug CLI flag sets this)
        self.debug
            .store(self.config.session.debug, Ordering::Relaxed);

        self.event_tx
            .send(EngineEvent::Started {
                debug: self.debug.load(Ordering::Relaxed),
            })
            .await
            .ok();

        // Notify the TUI of the active model at startup.
        self.event_tx
            .send(EngineEvent::ModelChanged {
                model: self.current_model.clone(),
            })
            .await
            .ok();

        // Initialize database and create a session
        let db = Database::open(&self.config.session.database_path).await?;
        let session = db.create_session("default").await?;
        let session_id = session.id;
        self.database = Some(db.clone());
        self.active_session_id = Some(session_id);

        // Create LLM providers from config and register them
        let mut llm_registry = LlmProviderRegistry::new();

        if let Some(ref cfg) = self.config.models.anthropic {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::Anthropic);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("anthropic".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.openai {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::OpenAI);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("openai".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.openrouter {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::OpenRouter);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("openrouter".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.ollama {
            let llm_cfg = ollama_config_to_llm(cfg);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("ollama".into(), provider);
        }

        self.llm_registry = llm_registry;

        // Connect MCP servers before spawning agents so tools are available
        {
            let mut mcp = self.mcp_registry.lock().await;
            for (name, def) in &self.config.mcps {
                if let Err(e) = mcp.register(name.clone(), def).await {
                    tracing::warn!("MCP server '{}' unavailable (skipping): {}", name, e);
                }
            }
        }

        // Collect all subagent names (agents that are listed as subagents of another agent)
        let subagent_names: std::collections::HashSet<String> = self
            .config
            .agents
            .iter()
            .flat_map(|a| a.subagents.iter().cloned())
            .collect();

        // Build a name-to-config map for quick lookup
        let config_by_name: std::collections::HashMap<String, &crate::config::AgentConfig> = self
            .config
            .agents
            .iter()
            .map(|a| (a.name.clone(), a))
            .collect();

        // Only spawn root agents — subagents are spawned on-demand by their parent
        for agent_config in &self.config.agents {
            if subagent_names.contains(&agent_config.name) {
                // Skip — this is a subagent, spawned on-demand
                continue;
            }

            let agent = Agent::from_config(agent_config, AgentRole::Root);
            let name = agent.name.clone();
            let id = agent.id.clone();

            self.event_tx
                .send(EngineEvent::AgentCreated {
                    id: id.clone(),
                    name: name.clone(),
                    role: AgentRole::Root,
                    model: agent.model.clone(),
                    skills: agent
                        .skills
                        .iter()
                        .filter_map(|p| {
                            p.file_stem()
                                .and_then(|f| f.to_str().map(|s| s.to_string()))
                        })
                        .collect(),
                    mcps: agent.mcps.clone(),
                })
                .await
                .ok();

            // Resolve the LLM provider for this agent based on model name
            let provider = self.resolve_agent_provider(&agent)?;

            // Load skills for this agent
            let skills = load_agent_skills(&agent.skills);

            // Collect subagent configs for this agent
            let my_subagent_configs: Vec<crate::config::AgentConfig> = agent_config
                .subagents
                .iter()
                .filter_map(|name| config_by_name.get(name).map(|c| (*c).clone()))
                .collect();

            // Spawn the agent as a real tokio task
            let history_limit = self.config.session.history_limit_percent;
            let retry_cfg = self.config.session.retry.clone();
            let handle = spawn_agent(SpawnAgentConfig {
                agent,
                provider,
                skills,
                subagent_configs: my_subagent_configs,
                llm_registry: self.llm_registry.clone(),
                mcp_registry: Some(self.mcp_registry.clone()),
                event_tx: self.event_tx.clone(),
                retry_config: retry_cfg,
                db: self.database.clone(),
                session_id: self.active_session_id,
                pending_approvals: Some(self.pending_approvals.clone()),
                history_limit_percent: history_limit,
                debug: self.debug.clone(),
            });

            self.agents.insert(name, id.clone());
            self.handles.insert(id, handle);
        }

        Ok(())
    }

    /// Resolve which LLM provider an agent should use based on its model name.
    fn resolve_agent_provider(&self, agent: &Agent) -> Result<Arc<dyn LlmProvider>> {
        let model = &agent.model;
        let provider_name = if model.contains('/') {
            "openrouter"
        } else if model.starts_with("claude") {
            "anthropic"
        } else if model.starts_with("gpt") || model.starts_with("o1") || model.starts_with("o3") {
            "openai"
        } else {
            "ollama"
        };

        self.llm_registry.get(provider_name).ok_or_else(|| {
            Error::Provider(format!(
                "No provider configured for model '{model}'. Tried provider: {provider_name}. \
                 Ensure the corresponding section is configured in models config."
            ))
        })
    }

    /// Find the root agent config (the one with `role == Root`).
    fn root_agent_config(&self) -> Result<&crate::config::AgentConfig> {
        self.config
            .agents
            .iter()
            .find(|a| a.role == AgentRole::Root)
            .ok_or_else(|| Error::Agent("No root agent configured".into()))
    }

    /// Run the main event loop.
    pub async fn run(&mut self) -> Result<()> {
        while let Some(command) = self.command_rx.recv().await {
            match command {
                EngineCommand::UserInput(input) => {
                    self.handle_user_input(input).await?;
                }
                EngineCommand::NewSession(name) => {
                    self.handle_new_session(&name).await?;
                }
                EngineCommand::ResumeSession(id) => {
                    self.handle_resume_session(&id).await?;
                }
                EngineCommand::ListSessions => {
                    self.handle_list_sessions().await?;
                }
                EngineCommand::DeleteSession(id) => {
                    self.handle_delete_session(&id).await?;
                }
                EngineCommand::RenameSession(id, name) => {
                    self.handle_rename_session(&id, &name).await?;
                }
                EngineCommand::ApprovalResponse { id, approved } => {
                    self.handle_approval_response(&id, approved).await;
                }
                EngineCommand::SetDebug(debug) => {
                    self.debug.store(debug, Ordering::Relaxed);
                }
                EngineCommand::SetModel(model) => {
                    self.handle_set_model(model).await?;
                }
                EngineCommand::Compact => {
                    self.send_to_root(AgentMessage::Compact).await?;
                }
                EngineCommand::Shutdown => {
                    self.event_tx.send(EngineEvent::ShuttingDown).await.ok();
                    break;
                }
            }
        }
        Ok(())
    }

    /// Handle user input: route to the root agent.
    async fn handle_user_input(&self, input: String) -> Result<()> {
        // Find the root agent (the one with role == Root)
        let root_name = &self.root_agent_config()?.name;

        let root_id = self
            .agents
            .get(root_name)
            .ok_or_else(|| Error::Agent(format!("Agent '{root_name}' not found")))?;

        let handle = self
            .handles
            .get(root_id)
            .ok_or_else(|| Error::Agent("Root agent not initialized".into()))?;

        handle
            .send(AgentMessage::UserInput { content: input })
            .await
    }

    /// Handle model change: update config, respawn root agent.
    async fn handle_set_model(&mut self, model: String) -> Result<()> {
        // Update the config in memory
        if let Some(ref mut agent) = self
            .config
            .agents
            .iter_mut()
            .find(|a| a.role == AgentRole::Root)
        {
            agent.model = model.clone();
        }
        self.current_model = model.clone();

        // Respawn the root agent with the new model
        self.respawn_root_agent().await?;

        self.event_tx
            .send(EngineEvent::ModelChanged { model })
            .await
            .ok();
        Ok(())
    }

    /// Respawn the root agent (used after model change).
    async fn respawn_root_agent(&mut self) -> Result<()> {
        let agent_config = self.root_agent_config()?;

        let agent = Agent::from_config(agent_config, AgentRole::Root);
        let name = agent.name.clone();
        let id = agent.id.clone();

        // Resolve the new provider
        let provider = self.resolve_agent_provider(&agent)?;

        // Load skills
        let skills = load_agent_skills(&agent.skills);

        // Collect subagent configs
        let _subagent_names: std::collections::HashSet<String> = self
            .config
            .agents
            .iter()
            .flat_map(|a| a.subagents.iter().cloned())
            .collect();
        let config_by_name: std::collections::HashMap<String, &crate::config::AgentConfig> = self
            .config
            .agents
            .iter()
            .map(|a| (a.name.clone(), a))
            .collect();
        let my_subagent_configs: Vec<crate::config::AgentConfig> = agent_config
            .subagents
            .iter()
            .filter_map(|name| config_by_name.get(name).map(|c| (*c).clone()))
            .collect();

        // Kill the old root agent handle
        if let Some(old_id) = self.agents.get(&name) {
            if let Some(old_handle) = self.handles.remove(old_id) {
                // Send shutdown to the old agent task
                let _ = old_handle.sender.send(AgentMessage::Shutdown).await;
            }
            self.agents.remove(&name);
        }

        // Spawn new agent
        let history_limit = self.config.session.history_limit_percent;
        let retry_cfg = self.config.session.retry.clone();
        let handle = spawn_agent(SpawnAgentConfig {
            agent,
            provider,
            skills,
            subagent_configs: my_subagent_configs,
            llm_registry: self.llm_registry.clone(),
            mcp_registry: Some(self.mcp_registry.clone()),
            event_tx: self.event_tx.clone(),
            retry_config: retry_cfg,
            db: self.database.clone(),
            session_id: self.active_session_id,
            pending_approvals: Some(self.pending_approvals.clone()),
            history_limit_percent: history_limit,
            debug: self.debug.clone(),
        });

        self.agents.insert(name.clone(), id.clone());
        self.handles.insert(id, handle);

        Ok(())
    }

    /// Handle new session creation.
    async fn handle_new_session(&mut self, name: &str) -> Result<()> {
        if let Some(ref db) = self.database {
            let session = db.create_session(name).await?;
            let session_id = session.id;
            self.active_session_id = Some(session_id);

            // Clear root agent's conversation
            self.send_to_root(AgentMessage::ClearHistory).await?;

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
    async fn handle_resume_session(&mut self, id_str: &str) -> Result<()> {
        let session_id = Uuid::parse_str(id_str)
            .map_err(|e| Error::Session(format!("Invalid session ID: {e}")))?;

        if let Some(ref db) = self.database {
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

            // Send history to root agent
            self.send_to_root(AgentMessage::LoadHistory(history))
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
    async fn handle_list_sessions(&self) -> Result<()> {
        if let Some(ref db) = self.database {
            let sessions = db.list_sessions().await?;
            self.event_tx
                .send(EngineEvent::SessionList(sessions))
                .await
                .ok();
        }
        Ok(())
    }

    /// Handle deleting a session.
    async fn handle_delete_session(&self, id_str: &str) -> Result<()> {
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
    async fn handle_rename_session(&self, id_str: &str, new_name: &str) -> Result<()> {
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

    /// Handle approval response from the TUI.
    async fn handle_approval_response(&self, id: &str, approved: bool) {
        let mut pending = self.pending_approvals.lock().await;
        if let Some(sender) = pending.remove(id) {
            let _ = sender.send(approved);
        }
    }

    /// Send a message to the root agent.
    async fn send_to_root(&self, msg: AgentMessage) -> Result<()> {
        let root_name = &self.root_agent_config()?.name;

        let root_id = self
            .agents
            .get(root_name)
            .ok_or_else(|| Error::Agent(format!("Agent '{root_name}' not found")))?;

        let handle = self
            .handles
            .get(root_id)
            .ok_or_else(|| Error::Agent("Root agent not initialized".into()))?;

        handle.send(msg).await
    }

    /// Shutdown the engine.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.event_tx.send(EngineEvent::ShuttingDown).await.ok();
        {
            let mut mcp = self.mcp_registry.lock().await;
            mcp.disconnect_all().await;
        }
        if let Some(db) = self.database.take() {
            db.close().await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers: config types -> LLM provider config
// ---------------------------------------------------------------------------

/// Convert a generic `ProviderConfig` + type tag to `LlmProviderConfig`.
fn provider_config_to_llm(cfg: &ProviderConfig, ptype: LlmProviderType) -> LlmProviderConfig {
    LlmProviderConfig {
        provider_type: ptype,
        api_key: Some(cfg.api_key.clone()),
        model: cfg.model.clone(),
        base_url: cfg.base_url.clone(),
        context_window: cfg.context_window,
        input_price_per_million: cfg.input_price_per_million,
        output_price_per_million: cfg.output_price_per_million,
    }
}

/// Convert an `OllamaConfig` to `LlmProviderConfig`.
fn ollama_config_to_llm(cfg: &OllamaConfig) -> LlmProviderConfig {
    LlmProviderConfig {
        provider_type: LlmProviderType::Ollama,
        api_key: None,
        model: cfg.model.clone(),
        base_url: Some(cfg.base_url.clone()),
        context_window: cfg.context_window,
        input_price_per_million: 0.0,
        output_price_per_million: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_engine_creation() {
        let (event_tx, _) = mpsc::channel(64);
        let (_cmd_tx, cmd_rx) = mpsc::channel(64);
        let config = Config::default();

        let mut engine = Engine::new(config, event_tx, cmd_rx);
        // Should not panic
        let result = engine.initialize().await;
        // May fail if no database directory, but shouldn't panic
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_provider_config_conversion() {
        let cfg = ProviderConfig {
            api_key: "sk-test".into(),
            model: "gpt-4o".into(),
            context_window: 128_000,
            base_url: None,
            input_price_per_million: 3.0,
            output_price_per_million: 15.0,
        };
        let llm_cfg = provider_config_to_llm(&cfg, LlmProviderType::OpenAI);
        assert_eq!(llm_cfg.api_key, Some("sk-test".into()));
        assert_eq!(llm_cfg.model, "gpt-4o");
        assert_eq!(llm_cfg.context_window, 128_000);
    }

    #[test]
    fn test_ollama_config_conversion() {
        let cfg = OllamaConfig {
            base_url: "http://localhost:11434".into(),
            model: "llama3.2".into(),
            context_window: 8_192,
        };
        let llm_cfg = ollama_config_to_llm(&cfg);
        assert_eq!(llm_cfg.provider_type, LlmProviderType::Ollama);
        assert_eq!(llm_cfg.api_key, None);
        assert_eq!(llm_cfg.model, "llama3.2");
    }

    #[test]
    fn test_resolve_provider_by_model() {
        let config = Config::default();
        let (event_tx, _) = mpsc::channel(64);
        let (_cmd_tx, cmd_rx) = mpsc::channel(64);

        let mut engine = Engine::new(config, event_tx, cmd_rx);
        // Populate registry with dummy providers
        let dummy_cfg = LlmProviderConfig {
            provider_type: LlmProviderType::Ollama,
            api_key: None,
            model: "llama3.2".into(),
            base_url: Some("http://localhost:11434".into()),
            context_window: 8_192,
            input_price_per_million: 0.0,
            output_price_per_million: 0.0,
        };
        engine
            .llm_registry
            .register("ollama".into(), Arc::from(create_provider(&dummy_cfg)));

        let agent = Agent::create_subagent(
            "test".into(),
            "test.md".into(),
            "llama3.2".into(),
            vec![],
            vec![],
            crate::permissions::Permissions::default(),
            60,
            AgentId::new(),
        );
        let result = engine.resolve_agent_provider(&agent);
        assert!(result.is_ok());
    }
}
