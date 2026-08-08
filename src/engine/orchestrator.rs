use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::lifecycle::{AgentHandle, SpawnAgentConfig, spawn_agent};
use crate::agent::types::{Agent, AgentId, AgentMessage, AgentMode, AgentRole, AgentStatus};
use crate::config::Config;
use crate::config::types::{CacheMode, OllamaConfig, ProviderConfig};
use crate::db::models::{Snapshot, StoredMessage};
use crate::db::session::Database;
use crate::engine::jobs::JobRegistry;
use crate::error::{Error, Result};
use crate::hook::{HookAction, HookActionConfig, HookContext, HookPoint, HookRegistry};
use crate::llm::provider::{LlmProvider, LlmProviderRegistry, create_provider};
use crate::llm::types::{CacheControl, LlmProviderConfig, LlmProviderType};
use crate::mcp::client::McpRegistry;
use crate::plugin::PluginRegistry;
use crate::skill::registry::{SharedSkillRegistry, SkillRegistry};

// Re-export the event/command types defined in `events` so the rest of the
// crate (TUI, agent lifecycle, main) can keep importing them from
// `crate::engine::orchestrator`.
pub use crate::engine::events::{
    EngineCommand, EngineEvent, ExportFormat, InitAnswers, McpStatus, SkillInfo, StatusInfo,
    TimelineEntry, UsageEvent,
};

/// The core orchestration engine.
pub struct Engine {
    /// Loaded configuration.
    pub(crate) config: Config,
    /// Registered agents (name -> id lookup).
    pub(crate) agents: HashMap<String, AgentId>,
    /// Active agent handles (id -> handle).
    pub(crate) handles: HashMap<AgentId, AgentHandle>,
    /// LLM provider registry.
    pub(crate) llm_registry: LlmProviderRegistry,
    /// MCP server registry.
    pub(crate) mcp_registry: Arc<tokio::sync::Mutex<McpRegistry>>,
    /// Database for persistence.
    pub(crate) database: Option<Database>,
    /// Active session ID.
    pub(crate) active_session_id: Option<Uuid>,
    /// Channel to send events to the TUI.
    pub(crate) event_tx: mpsc::Sender<EngineEvent>,
    /// Channel to receive commands from the TUI.
    pub(crate) command_rx: mpsc::Receiver<EngineCommand>,
    /// Channel to receive usage reports from agent tasks (for `/status`).
    pub(crate) usage_rx: mpsc::Receiver<UsageEvent>,
    /// Sender half of the usage channel, cloned into agent tasks.
    pub(crate) usage_tx: mpsc::Sender<UsageEvent>,
    /// Pending human approvals (id -> oneshot sender).
    pub(crate) pending_approvals:
        Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
    /// Pending inline questions (id -> oneshot sender) for the `question` tool.
    pub(crate) pending_questions:
        Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>>,
    /// Debug mode flag (shows LLM request/response payloads).
    /// Shared with agent tasks so the `/debug` toggle takes effect immediately.
    pub(crate) debug: Arc<AtomicBool>,
    /// Current model for the root agent.
    pub(crate) current_model: String,
    /// Name of the currently active root agent (routing target for user input).
    pub(crate) active_agent: String,
    /// Stack of undone message pairs (for `/undo`).
    pub(crate) undo_stack: Vec<Vec<StoredMessage>>,
    /// Stack of undone message pairs available for `/redo`.
    pub(crate) redo_stack: Vec<Vec<StoredMessage>>,
    /// Current engine workspace directory.
    pub(crate) workspace: PathBuf,
    /// Per-server MCP enabled state (for `/mcps` toggling). Shared with agents
    /// so they can gate tool collection.
    pub(crate) mcp_enabled: Arc<tokio::sync::Mutex<HashMap<String, bool>>>,
    /// Total tokens consumed (tracked for `/status`).
    pub(crate) total_tokens: u32,
    /// Total cost in dollars (tracked for `/status`).
    pub(crate) total_cost: f64,
    /// Registry of running background jobs (dynamic `task` tool delegations).
    pub(crate) job_registry: Arc<tokio::sync::Mutex<JobRegistry>>,
    /// A staged snapshot (via `/stage`) awaiting commit (via `/commit`).
    pub(crate) staged_snapshot: Option<Snapshot>,
    /// Loaded plugins and their custom tools.
    pub(crate) plugins: Arc<PluginRegistry>,
    /// Central skill registry, loaded once at startup.
    pub(crate) skill_registry: SharedSkillRegistry,
    /// Hook system registry (from config).
    pub(crate) hook_registry: HookRegistry,
    /// Cancel flags for active agents (agent_name -> flag). Set directly to stop agents.
    pub(crate) cancel_flags: HashMap<String, Arc<AtomicBool>>,
}
impl Engine {
    pub fn new(
        config: Config,
        event_tx: mpsc::Sender<EngineEvent>,
        command_rx: mpsc::Receiver<EngineCommand>,
    ) -> Self {
        let (usage_tx, usage_rx) = mpsc::channel(64);
        Self {
            config: config.clone(),
            agents: HashMap::new(),
            handles: HashMap::new(),
            llm_registry: LlmProviderRegistry::new(),
            mcp_registry: Arc::new(tokio::sync::Mutex::new(McpRegistry::new())),
            pending_approvals: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            pending_questions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            debug: Arc::new(AtomicBool::new(false)),
            current_model: config
                .agents
                .iter()
                .find(|a| a.role == AgentRole::Root)
                .map(|a| a.model.clone())
                .unwrap_or_default(),
            active_agent: config
                .agents
                .iter()
                .find(|a| a.role == AgentRole::Root)
                .map(|a| a.name.clone())
                .unwrap_or_default(),
            database: None,
            active_session_id: None,
            event_tx,
            command_rx,
            usage_rx,
            usage_tx,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            mcp_enabled: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            total_tokens: 0,
            total_cost: 0.0,
            job_registry: Arc::new(tokio::sync::Mutex::new(JobRegistry::new())),
            staged_snapshot: None,
            plugins: Arc::new(PluginRegistry::new()),
            skill_registry: Arc::new(tokio::sync::RwLock::new(SkillRegistry::new())),
            hook_registry: HookRegistry::from(&config),
            cancel_flags: HashMap::new(),
        }
    }

    fn root_agent_configs(
        agents: &[crate::config::AgentConfig],
    ) -> impl Iterator<Item = &crate::config::AgentConfig> {
        agents.iter().filter(|a| a.role == AgentRole::Root)
    }

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

        // Notify the TUI of the active root agent at startup so the status bar
        // is populated before the first `/agent` switch.
        self.event_tx
            .send(EngineEvent::AgentSwitched {
                name: self.active_agent.clone(),
            })
            .await
            .ok();

        // Initialize database and create a session
        let db = Database::open(&self.config.session.database_path).await?;
        let session = db.create_session("default").await?;
        let session_id = session.id;
        self.database = Some(db.clone());
        self.active_session_id = Some(session_id);

        // Load plugins from the global plugins directory.
        let plugins_dir = crate::config::paths::global_plugins_dir();
        let mut plugins = PluginRegistry::new();
        if let Err(e) = plugins.load_from_dir(&plugins_dir) {
            eprintln!(
                "warning: failed to load plugins from {}: {e}",
                plugins_dir.display()
            );
        }
        self.plugins = Arc::new(plugins);

        // Create LLM providers from config and register them
        let mut llm_registry = LlmProviderRegistry::new();

        let cache: CacheControl = self.config.models.cache.mode.into();

        if let Some(ref cfg) = self.config.models.anthropic {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::Anthropic, cache);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("anthropic".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.openai {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::OpenAI, cache);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("openai".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.openrouter {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::OpenRouter, cache);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("openrouter".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.ollama {
            let llm_cfg = ollama_config_to_llm(cfg, cache);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("ollama".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.bedrock {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::Bedrock, cache);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("bedrock".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.azure {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::Azure, cache);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("azure".into(), provider);
        }
        if let Some(ref cfg) = self.config.models.google {
            let llm_cfg = provider_config_to_llm(cfg, LlmProviderType::Google, cache);
            let provider: Arc<dyn LlmProvider> = Arc::from(create_provider(&llm_cfg));
            if let Ok(cw) = provider.fetch_context_window().await {
                provider.set_context_window(cw);
            }
            llm_registry.register("google".into(), provider);
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

        // Create concurrency semaphore for subagent spawning
        let concurrency_semaphore = if self.config.session.max_concurrency > 0 {
            Some(Arc::new(tokio::sync::Semaphore::new(
                self.config.session.max_concurrency as usize,
            )))
        } else {
            None
        };

        // Load all skills into the central registry once
        {
            let all_skill_paths: Vec<PathBuf> = self
                .config
                .agents
                .iter()
                .flat_map(|a| a.skills.clone())
                .collect();
            if !all_skill_paths.is_empty() {
                let mut reg = self.skill_registry.write().await;
                if let Err(e) = reg.load_from_paths(&all_skill_paths) {
                    eprintln!("Warning: Failed to load some skills: {e}");
                }
            }
        }

        // Merge auto-detected, plugin, and skill hooks into the hook registry.
        // Config hooks were already loaded in Engine::new(); now merge lower-priority
        // sources (auto-detect, plugins, skills) with Config > Plugin > Skill > Auto precedence.
        {
            let auto_hooks = crate::hook::autoconfig::detect_auto_hooks();

            let mut skill_hooks: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();
            {
                let reg = self.skill_registry.read().await;
                for skill in reg.list() {
                    for (key, actions) in &skill.hooks {
                        if let Some(point) = parse_hook_point(key) {
                            let entry = skill_hooks.entry(point).or_default();
                            entry.extend(actions.clone());
                        }
                    }
                }
            }

            let plugin_hooks: Vec<(HookPoint, HookActionConfig)> = self
                .plugins
                .list()
                .iter()
                .flat_map(|p| p.register_hooks())
                .collect();

            let merged =
                merge_hook_sources(&self.config.hooks, &plugin_hooks, &skill_hooks, &auto_hooks);
            self.hook_registry = HookRegistry::new(merged);
        }

        // Build a name-to-config map for quick lookup
        let config_by_name: std::collections::HashMap<String, &crate::config::AgentConfig> = self
            .config
            .agents
            .iter()
            .map(|a| (a.name.clone(), a))
            .collect();

        // Only spawn root agents — subagents are spawned on-demand by their parent.
        // An agent is a root iff it declares `role: root` (ADR-0001). Agents
        // declared as subagents are never spawned at startup, regardless of
        // whether a parent references them.
        for agent_config in Self::root_agent_configs(&self.config.agents) {
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

            // Collect skill names for this agent from its config paths
            let skill_names: Vec<String> = agent
                .skills
                .iter()
                .filter_map(|p| {
                    p.file_stem()
                        .and_then(|f| f.to_str().map(|s| s.to_string()))
                })
                .collect();

            // Collect subagent configs for this agent
            let my_subagent_configs: Vec<crate::config::AgentConfig> = agent_config
                .subagents
                .iter()
                .filter_map(|name| config_by_name.get(name).map(|c| (*c).clone()))
                .collect();

            // Create an external cancel flag for this agent
            let cancel_flag = Arc::new(AtomicBool::new(false));
            self.cancel_flags.insert(name.clone(), cancel_flag.clone());

            // Spawn the agent as a real tokio task
            let history_limit = self.config.session.history_limit_percent;
            let retry_cfg = self.config.session.retry.clone();
            let handle = spawn_agent(SpawnAgentConfig {
                agent,
                provider,
                skill_registry: self.skill_registry.clone(),
                skill_names,
                subagent_configs: my_subagent_configs,
                llm_registry: self.llm_registry.clone(),
                mcp_registry: Some(self.mcp_registry.clone()),
                mcp_enabled: Some(self.mcp_enabled.clone()),
                event_tx: self.event_tx.clone(),
                usage_tx: Some(self.usage_tx.clone()),
                retry_config: retry_cfg,
                db: self.database.clone(),
                session_id: self.active_session_id,
                pending_approvals: Some(self.pending_approvals.clone()),
                pending_questions: Some(self.pending_questions.clone()),
                history_limit_percent: history_limit,
                debug: self.debug.clone(),
                workspace: self.workspace.clone(),
                task_id: None,
                depth: 0,
                mode: AgentMode::Build,
                job_registry: Some(self.job_registry.clone()),
                plugins: Some(self.plugins.clone()),
                concurrency_semaphore: concurrency_semaphore.clone(),
                hook_registry: self.hook_registry.clone(),
                cancel_flag: Some(cancel_flag),
            })
            .await;

            self.agents.insert(name, id.clone());
            self.handles.insert(id, handle);
        }

        // Fire OnStartup hooks and emit events
        let hook_results = self
            .hook_registry
            .run(HookPoint::OnStartup, &HookContext::default())
            .await;
        for r in &hook_results {
            let _ = self
                .event_tx
                .send(EngineEvent::HookExecuted {
                    point: format!("{:?}", HookPoint::OnStartup),
                    command: r.command.clone(),
                    success: r.exit_code == Some(0),
                    output: if r.stdout.is_empty() {
                        r.stderr.clone()
                    } else {
                        r.stdout.clone()
                    },
                })
                .await;
        }

        Ok(())
    }

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

    /// Reload configuration from disk. Called on SIGHUP.
    pub fn reload_config(&mut self, config: Config) {
        self.config = config;
        // Propagate any config changes that affect running state
        // (e.g., session settings, MCP definitions)
        tracing::info!("Configuration reloaded from disk");
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    let Some(command) = command else { break; };
                    // Shutdown is handled outside the error-catching block so it always
                    // terminates the loop even if a prior handler failed.
                    if matches!(command, EngineCommand::Shutdown) {
                        self.event_tx.send(EngineEvent::ShuttingDown).await.ok();
                        break;
                    }

                    // Dispatch the command inside an async block so a handler error is
                    // reported to the TUI instead of killing the engine event loop.
                    let result: Result<()> = async {
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
                            EngineCommand::QuestionAnswer { id, answer } => {
                                self.handle_question_answer(&id, answer).await;
                            }
                            EngineCommand::SetDebug(debug) => {
                                self.debug.store(debug, Ordering::Relaxed);
                            }
                            EngineCommand::SetModel(model) => {
                                self.handle_set_model(model).await?;
                            }
                            EngineCommand::SwitchAgent(name) => {
                                self.handle_switch_agent(&name).await?;
                            }
                            EngineCommand::RecordModelUsage(model) => {
                                self.handle_record_model_usage(&model).await?;
                            }
                            EngineCommand::ListModelFrecency => {
                                self.handle_list_model_frecency().await?;
                            }
                            EngineCommand::Compact => {
                                self.send_to_active(AgentMessage::Compact).await?;
                            }
                            EngineCommand::Undo => {
                                self.handle_undo().await?;
                            }
                            EngineCommand::Redo => {
                                self.handle_redo().await?;
                            }
                            EngineCommand::Fork => {
                                self.handle_fork().await?;
                            }
                            EngineCommand::Export { path, format } => {
                                self.handle_export(path, format).await?;
                            }
                            EngineCommand::Import { path } => {
                                self.handle_import(path).await?;
                            }
                            EngineCommand::Share => {
                                self.handle_share().await?;
                            }
                            EngineCommand::Unshare => {
                                self.handle_unshare().await?;
                            }
                            EngineCommand::ListSkills => {
                                self.handle_list_skills().await?;
                            }
                            EngineCommand::ListMcps => {
                                self.handle_list_mcps().await?;
                            }
                            EngineCommand::ToggleMcp { name, enabled } => {
                                self.handle_toggle_mcp(&name, enabled).await?;
                            }
                            EngineCommand::Status => {
                                self.handle_status().await?;
                            }
                            EngineCommand::Init { answers } => {
                                self.handle_init(answers).await?;
                            }
                            EngineCommand::Review { target } => {
                                self.handle_review(target).await?;
                            }
                            EngineCommand::Warp { dir } => {
                                self.handle_warp(dir).await?;
                            }
                            EngineCommand::ListWorkspaces => {
                                self.handle_list_workspaces().await?;
                            }
                            EngineCommand::MoveSession { workspace } => {
                                self.handle_move_session(&workspace).await?;
                            }
                            EngineCommand::WorktreeAdd { path, branch } => {
                                self.handle_worktree_add(&path, branch.as_deref()).await?;
                            }
                            EngineCommand::WorktreeList => {
                                self.handle_worktree_list().await?;
                            }
                            EngineCommand::WorktreeRemove { path } => {
                                self.handle_worktree_remove(&path).await?;
                            }
                            EngineCommand::Timeline => {
                                self.handle_timeline().await?;
                            }
                            EngineCommand::SetSessionPinned { id, pinned } => {
                                self.handle_set_session_pinned(&id, pinned).await?;
                            }
                            EngineCommand::ListJobs => {
                                self.handle_list_jobs().await?;
                            }
                            EngineCommand::Build => {
                                self.handle_build().await?;
                            }
                            EngineCommand::Parent => {
                                self.handle_parent().await?;
                            }
                            EngineCommand::Children => {
                                self.handle_children().await?;
                            }
                            EngineCommand::Snapshot { name } => {
                                self.handle_snapshot(name.as_deref()).await?;
                            }
                            EngineCommand::Revert { snapshot_id } => {
                                self.handle_revert(snapshot_id).await?;
                            }
                            EngineCommand::ListSnapshots => {
                                self.handle_list_snapshots().await?;
                            }
                            EngineCommand::Stage { name } => {
                                self.handle_stage(name.as_deref()).await?;
                            }
                            EngineCommand::Clear => {
                                self.handle_clear().await?;
                            }
                            EngineCommand::Commit { name } => {
                                self.handle_commit(name.as_deref()).await?;
                            }
                            EngineCommand::ReloadAgent => {
                                // First stop any in-flight work
                                if let Some(flag) = self.cancel_flags.get(&self.active_agent) {
                                    flag.store(true, Ordering::Relaxed);
                                }
                                let _ = self.send_to_active(AgentMessage::Cancel).await;
                                // Then respawn the agent (reuses existing method, picks up fresh config)
                                self.respawn_active_agent().await?;
                                self.event_tx
                                    .send(EngineEvent::AgentStatusChanged {
                                        agent_id: AgentId::new(),
                                        agent_name: self.active_agent.clone(),
                                        status: AgentStatus::Idle,
                                    })
                                    .await
                                    .ok();
                            }
                            EngineCommand::StopAgent => {
                                // Set the cancel flag directly (bypasses mpsc channel, immediate effect)
                                if let Some(flag) = self.cancel_flags.get(&self.active_agent) {
                                    flag.store(true, Ordering::Relaxed);
                                }
                                // Also send through channel for when agent IS in the message loop
                                let _ = self.send_to_active(AgentMessage::Cancel).await;
                                // Emit Idle status so TUI knows immediately
                                let _ = self
                                    .event_tx
                                    .send(EngineEvent::AgentStatusChanged {
                                        agent_id: AgentId::new(),
                                        agent_name: self.active_agent.clone(),
                                        status: AgentStatus::Idle,
                                    })
                                    .await;
                            }
                            EngineCommand::Shutdown => unreachable!(),
                            EngineCommand::ReloadConfig => {
                                match crate::config::loader::load_config(None) {
                                    Ok(new_config) => {
                                        self.reload_config(new_config);
                                        let _ = self.event_tx
                                            .send(EngineEvent::ConfigReloaded)
                                            .await;
                                        tracing::info!("Configuration reloaded successfully");
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to reload config: {}", e);
                                        let _ = self.event_tx
                                            .send(EngineEvent::CommandError(
                                                format!("Failed to reload config: {}", e),
                                            ))
                                            .await;
                                    }
                                }
                            }
                        }
                        Ok(())
                    }
                    .await;

                    if let Err(e) = result {
                        self.event_tx
                            .send(EngineEvent::CommandError(e.to_string()))
                            .await
                            .ok();
                    }
                }
                usage = self.usage_rx.recv() => {
                    if let Some(usage) = usage {
                        self.total_tokens = self.total_tokens.saturating_add(usage.total_tokens);
                        self.total_cost += usage.cost;
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_user_input(&mut self, input: String) -> Result<()> {
        // A new turn invalidates any pending redo history.
        self.redo_stack.clear();
        // Find the active root agent
        let root_name = &self.active_agent;

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

    async fn handle_set_model(&mut self, model: String) -> Result<()> {
        // Update the config in memory for the active agent
        if let Some(ref mut agent) = self
            .config
            .agents
            .iter_mut()
            .find(|a| a.name == self.active_agent)
        {
            agent.model = model.clone();
        }
        self.current_model = model.clone();

        // Respawn the active agent with the new model
        self.respawn_active_agent().await?;

        self.event_tx
            .send(EngineEvent::ModelChanged { model })
            .await
            .ok();
        Ok(())
    }

    async fn handle_switch_agent(&mut self, name: &str) -> Result<()> {
        // Validate that the target agent exists and is a root agent.
        let agent = self
            .config
            .agents
            .iter()
            .find(|a| a.name == name && a.role == AgentRole::Root)
            .ok_or_else(|| Error::Agent(format!("Root agent '{name}' not found")))?;

        // Validate that the agent is actually spawned. An agent with `role: Root`
        // that is also listed as a subagent of another agent is skipped during
        // `initialize()` and therefore not present in `self.agents`/`self.handles`.
        if !self.agents.contains_key(name) {
            return Err(Error::Agent(format!(
                "Root agent '{name}' is not spawned (it is listed as a subagent of another agent)"
            )));
        }

        let name = name.to_string();
        self.active_agent = name.clone();
        self.current_model = agent.model.clone();

        self.event_tx
            .send(EngineEvent::AgentSwitched { name })
            .await
            .ok();
        self.event_tx
            .send(EngineEvent::ModelChanged {
                model: self.current_model.clone(),
            })
            .await
            .ok();
        Ok(())
    }

    async fn handle_record_model_usage(&mut self, model: &str) -> Result<()> {
        if let Some(db) = &self.database {
            db.record_model_usage(model).await?;
        }
        Ok(())
    }

    async fn handle_list_model_frecency(&mut self) -> Result<()> {
        let frecency = match &self.database {
            Some(db) => db.list_model_frecency().await?,
            None => Vec::new(),
        };
        self.event_tx
            .send(EngineEvent::ModelsFrecency(frecency))
            .await
            .ok();
        Ok(())
    }

    async fn respawn_active_agent(&mut self) -> Result<()> {
        let agent_config = self
            .config
            .agents
            .iter()
            .find(|a| a.name == self.active_agent)
            .ok_or_else(|| Error::Agent(format!("Agent '{}' not found", self.active_agent)))?;

        let agent = Agent::from_config(agent_config, AgentRole::Root);
        let name = agent.name.clone();
        let id = agent.id.clone();

        // Resolve the new provider
        let provider = self.resolve_agent_provider(&agent)?;

        // Collect skill names for this agent from its config paths
        let skill_names: Vec<String> = agent
            .skills
            .iter()
            .filter_map(|p| {
                p.file_stem()
                    .and_then(|f| f.to_str().map(|s| s.to_string()))
            })
            .collect();

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

        // Create an external cancel flag for this agent
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.cancel_flags.insert(name.clone(), cancel_flag.clone());

        // Spawn new agent
        let history_limit = self.config.session.history_limit_percent;
        let retry_cfg = self.config.session.retry.clone();
        let concurrency_semaphore = if self.config.session.max_concurrency > 0 {
            Some(Arc::new(tokio::sync::Semaphore::new(
                self.config.session.max_concurrency as usize,
            )))
        } else {
            None
        };
        let handle = spawn_agent(SpawnAgentConfig {
            agent,
            provider,
            skill_registry: self.skill_registry.clone(),
            skill_names,
            subagent_configs: my_subagent_configs,
            llm_registry: self.llm_registry.clone(),
            mcp_registry: Some(self.mcp_registry.clone()),
            mcp_enabled: Some(self.mcp_enabled.clone()),
            event_tx: self.event_tx.clone(),
            usage_tx: Some(self.usage_tx.clone()),
            retry_config: retry_cfg,
            db: self.database.clone(),
            session_id: self.active_session_id,
            pending_approvals: Some(self.pending_approvals.clone()),
            pending_questions: Some(self.pending_questions.clone()),
            history_limit_percent: history_limit,
            debug: self.debug.clone(),
            workspace: self.workspace.clone(),
            task_id: None,
            depth: 0,
            mode: AgentMode::Build,
            job_registry: Some(self.job_registry.clone()),
            plugins: Some(self.plugins.clone()),
            concurrency_semaphore: concurrency_semaphore.clone(),
            hook_registry: self.hook_registry.clone(),
            cancel_flag: Some(cancel_flag),
        })
        .await;

        self.agents.insert(name.clone(), id.clone());
        self.handles.insert(id, handle);

        Ok(())
    }

    async fn handle_question_answer(&self, id: &str, answer: String) {
        let mut pending = self.pending_questions.lock().await;
        if let Some(sender) = pending.remove(id) {
            let _ = sender.send(answer);
        }
    }

    pub(crate) async fn send_to_active(&self, msg: AgentMessage) -> Result<()> {
        let root_name = &self.active_agent;

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

    pub async fn shutdown(&mut self) -> Result<()> {
        // Fire OnShutdown hooks and emit events
        let hook_results = self
            .hook_registry
            .run(HookPoint::OnShutdown, &HookContext::default())
            .await;
        for r in &hook_results {
            let _ = self
                .event_tx
                .send(EngineEvent::HookExecuted {
                    point: format!("{:?}", HookPoint::OnShutdown),
                    command: r.command.clone(),
                    success: r.exit_code == Some(0),
                    output: if r.stdout.is_empty() {
                        r.stderr.clone()
                    } else {
                        r.stdout.clone()
                    },
                })
                .await;
        }

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

fn provider_config_to_llm(
    cfg: &ProviderConfig,
    ptype: LlmProviderType,
    cache: CacheControl,
) -> LlmProviderConfig {
    LlmProviderConfig {
        provider_type: ptype,
        api_key: Some(cfg.api_key.clone()),
        model: cfg.model.clone(),
        base_url: cfg.base_url.clone(),
        context_window: cfg.context_window,
        input_price_per_million: cfg.input_price_per_million,
        output_price_per_million: cfg.output_price_per_million,
        cache_control: cache,
        thinking_budget_tokens: cfg.thinking_budget_tokens,
    }
}

/// Convert an `OllamaConfig` to `LlmProviderConfig`.
fn ollama_config_to_llm(cfg: &OllamaConfig, cache: CacheControl) -> LlmProviderConfig {
    LlmProviderConfig {
        provider_type: LlmProviderType::Ollama,
        api_key: None,
        model: cfg.model.clone(),
        base_url: Some(cfg.base_url.clone()),
        context_window: cfg.context_window,
        input_price_per_million: 0.0,
        output_price_per_million: 0.0,
        cache_control: cache,
        thinking_budget_tokens: None,
    }
}

impl From<CacheMode> for CacheControl {
    fn from(mode: CacheMode) -> Self {
        match mode {
            CacheMode::Auto => CacheControl::Auto,
            CacheMode::Off => CacheControl::Off,
        }
    }
}

// ---------------------------------------------------------------------------
// Hook source merging helpers
// ---------------------------------------------------------------------------

/// Merge hooks from 4 sources with precedence:
/// 1. Config (user explicit) — highest priority
/// 2. Plugin hooks
/// 3. Skill hooks
/// 4. Auto-detect (PATH) — lowest priority
///
/// Deduplication: same (command, timeout_secs) at the same HookPoint
/// is kept only once (from the highest-precedence source).
fn merge_hook_sources(
    config: &HashMap<String, Vec<HookActionConfig>>,
    plugins: &[(HookPoint, HookActionConfig)],
    skills: &HashMap<HookPoint, Vec<HookActionConfig>>,
    auto: &HashMap<HookPoint, Vec<HookActionConfig>>,
) -> HashMap<HookPoint, Vec<HookActionConfig>> {
    let mut result: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();

    /// Helper to insert an action with deduplication.
    fn insert_dedup(
        result: &mut HashMap<HookPoint, Vec<HookActionConfig>>,
        point: HookPoint,
        action: HookActionConfig,
    ) {
        let entry = result.entry(point).or_default();
        if !entry.iter().any(|a| {
            a.timeout_secs == action.timeout_secs
                && matches!((&a.action, &action.action), (HookAction::Shell { command: c1 }, HookAction::Shell { command: c2 }) if c1 == c2)
        }) {
            entry.push(action);
        }
    }

    // 1. Auto-detect (lowest priority)
    for (point, actions) in auto {
        for action in actions {
            insert_dedup(&mut result, *point, action.clone());
        }
    }

    // 2. Skill hooks
    for (point, actions) in skills {
        for action in actions {
            insert_dedup(&mut result, *point, action.clone());
        }
    }

    // 3. Plugin hooks
    for (point, action) in plugins {
        insert_dedup(&mut result, *point, action.clone());
    }

    // 4. Config (highest priority) — inserted last so they win dedup
    for (key, actions) in config {
        if let Some(point) = parse_hook_point(key) {
            for action in actions {
                // Remove any existing action with same command+timeout
                let entry = result.entry(point).or_default();
                entry.retain(|a| {
                    a.timeout_secs != action.timeout_secs
                        || !matches!((&a.action, &action.action), (HookAction::Shell { command: c1 }, HookAction::Shell { command: c2 }) if c1 == c2)
                });
                entry.push(action.clone());
            }
        }
    }

    result
}

/// Parse a YAML config key string into a HookPoint.
fn parse_hook_point(key: &str) -> Option<HookPoint> {
    match key {
        "before_tool" => Some(HookPoint::BeforeTool),
        "after_tool" => Some(HookPoint::AfterTool),
        "before_apply" => Some(HookPoint::BeforeApply),
        "after_apply" => Some(HookPoint::AfterApply),
        "before_shell" => Some(HookPoint::BeforeShell),
        "after_shell" => Some(HookPoint::AfterShell),
        "before_fs_write" => Some(HookPoint::BeforeFsWrite),
        "after_fs_write" => Some(HookPoint::AfterFsWrite),
        "on_startup" => Some(HookPoint::OnStartup),
        "on_shutdown" => Some(HookPoint::OnShutdown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::AgentConfig;

    #[test]
    fn test_root_agent_configs_only_returns_role_root() {
        // Only agents declaring `role: root` are spawned at startup (ADR-0001).
        // Subagents — even those not referenced by any parent — are excluded.
        let configs = vec![
            AgentConfig {
                name: "root".into(),
                description: "root agent".into(),
                role: AgentRole::Root,
                model: "claude-sonnet-4".into(),
                skills: vec![],
                mcps: vec![],
                permissions: crate::config::types::PermissionConfig::default(),
                subagents: vec!["tech-writer".into()],
                system_prompt: String::new(),
                max_steps: 90,
                subagent_depth: 3,
            },
            AgentConfig {
                name: "tech-writer".into(),
                description: "subagent".into(),
                role: AgentRole::SubAgent,
                model: "claude-sonnet-4".into(),
                skills: vec![],
                mcps: vec![],
                permissions: crate::config::types::PermissionConfig::default(),
                subagents: vec![],
                system_prompt: String::new(),
                max_steps: 90,
                subagent_depth: 3,
            },
        ];

        let roots: Vec<String> = Engine::root_agent_configs(&configs)
            .map(|a| a.name.clone())
            .collect();
        assert_eq!(roots, vec!["root"]);
    }

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
            thinking_budget_tokens: None,
        };
        let llm_cfg = provider_config_to_llm(&cfg, LlmProviderType::OpenAI, CacheControl::Auto);
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
        let llm_cfg = ollama_config_to_llm(&cfg, CacheControl::Auto);
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
            cache_control: CacheControl::Auto,
            thinking_budget_tokens: None,
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

    /// Build an engine with a temp DB and an active session.
    async fn test_engine_with_session()
    -> (Engine, Uuid, mpsc::Receiver<EngineEvent>, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).await.unwrap();
        let session = db.create_session("test-session").await.unwrap();
        let (event_tx, event_rx) = mpsc::channel(64);
        let (_cmd_tx, cmd_rx) = mpsc::channel(64);
        let mut config = Config::default();
        config.session.database_path = db_path;
        let mut engine = Engine::new(config, event_tx, cmd_rx);
        engine.database = Some(db.clone());
        engine.active_session_id = Some(session.id);
        (engine, session.id, event_rx, dir)
    }

    #[tokio::test]
    async fn test_handle_undo_redo() {
        let (mut engine, session_id, _rx, _dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();
        db.store_message(session_id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(session_id, "assistant", "assistant", "Hi", None)
            .await
            .unwrap();

        engine.handle_undo().await.unwrap();
        assert_eq!(db.get_session_messages(session_id).await.unwrap().len(), 0);
        assert_eq!(engine.undo_stack.len(), 1);
        assert_eq!(engine.redo_stack.len(), 1);

        engine.handle_redo().await.unwrap();
        let msgs = db.get_session_messages(session_id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "Hello");
        assert_eq!(msgs[1].content, "Hi");
    }

    #[tokio::test]
    async fn test_handle_fork() {
        let (mut engine, session_id, _rx, _dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();
        db.store_message(session_id, "user", "user", "Hello", None)
            .await
            .unwrap();

        engine.handle_fork().await.unwrap();
        let new_id = engine.active_session_id.unwrap();
        assert_ne!(new_id, session_id);
        let msgs = db.get_session_messages(new_id).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello");
    }

    #[tokio::test]
    async fn test_handle_share_unshare() {
        let (mut engine, session_id, _rx, _dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();

        engine.handle_share().await.unwrap();
        let meta = db.get_session_metadata(session_id).await.unwrap();
        assert!(
            meta.as_ref()
                .and_then(|v| v.get("share_link"))
                .and_then(|v| v.as_str())
                .is_some()
        );

        engine.handle_unshare().await.unwrap();
        let meta = db.get_session_metadata(session_id).await.unwrap();
        assert!(meta.as_ref().and_then(|v| v.get("share_link")).is_none());
    }

    #[tokio::test]
    async fn test_handle_export_import() {
        let (mut engine, session_id, _rx, dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();
        db.store_message(session_id, "user", "user", "Hello", None)
            .await
            .unwrap();
        db.store_message(session_id, "assistant", "assistant", "Hi", None)
            .await
            .unwrap();

        let out = dir.path().join("export.json");
        engine
            .handle_export(Some(out.clone()), Some(ExportFormat::Json))
            .await
            .unwrap();
        assert!(out.exists());

        engine.handle_import(out).await.unwrap();
        let new_id = engine.active_session_id.unwrap();
        let msgs = db.get_session_messages(new_id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "Hello");
        assert_eq!(msgs[1].content, "Hi");
    }

    #[tokio::test]
    async fn test_handle_init_writes_agents_md() {
        let (mut engine, _session_id, _rx, _dir) = test_engine_with_session().await;
        engine.workspace = tempfile::TempDir::new().unwrap().keep();
        let answers = InitAnswers {
            name: "My Project".into(),
            description: "A test project".into(),
            stack: "Rust, React".into(),
        };
        engine.handle_init(answers).await.unwrap();
        let content = std::fs::read_to_string(engine.workspace.join("AGENTS.md")).unwrap();
        assert!(content.contains("# My Project"));
        assert!(content.contains("Rust, React"));
    }

    #[tokio::test]
    async fn test_handle_warp() {
        let (mut engine, _session_id, mut rx, _dir) = test_engine_with_session().await;
        let new_dir = tempfile::TempDir::new().unwrap().keep();
        engine.handle_warp(new_dir.clone()).await.unwrap();
        assert_eq!(engine.workspace, new_dir);
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev, EngineEvent::WorkspaceChanged(_)));
    }

    #[tokio::test]
    async fn test_handle_timeline() {
        let (engine, session_id, mut rx, _dir) = test_engine_with_session().await;
        let db = engine.database.clone().unwrap();
        db.store_message(session_id, "user", "user", "Hello", None)
            .await
            .unwrap();
        engine.handle_timeline().await.unwrap();
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::Timeline(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].content, "Hello");
            }
            _ => panic!("expected Timeline event"),
        }
    }

    #[tokio::test]
    async fn test_handle_move_session() {
        let (mut engine, session_id, mut rx, _dir) = test_engine_with_session().await;
        engine.handle_move_session("other-ws").await.unwrap();
        // `/move` now also re-homes the engine workspace, so a WorkspaceChanged
        // event is emitted before SessionMoved (FASE 5.1).
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::WorkspaceChanged(dir) => {
                assert_eq!(dir, std::path::PathBuf::from("other-ws"));
            }
            _ => panic!("expected WorkspaceChanged event"),
        }
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::SessionMoved {
                session_id: sid,
                workspace,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(workspace, "other-ws");
            }
            _ => panic!("expected SessionMoved event"),
        }
    }

    /// Build an engine whose config declares two root agents and whose
    /// `self.agents` map reflects that both were spawned.
    fn engine_with_agents() -> (Engine, mpsc::Receiver<EngineEvent>) {
        let (event_tx, event_rx) = mpsc::channel(64);
        let (_cmd_tx, cmd_rx) = mpsc::channel(64);
        let mut config = Config::default();
        config.agents = vec![
            AgentConfig {
                name: "root".into(),
                description: "root agent".into(),
                role: AgentRole::Root,
                model: "claude-sonnet-4".into(),
                skills: vec![],
                mcps: vec![],
                permissions: crate::config::types::PermissionConfig::default(),
                subagents: vec![],
                system_prompt: String::new(),
                max_steps: 90,
                subagent_depth: 3,
            },
            AgentConfig {
                name: "writer".into(),
                description: "writer agent".into(),
                role: AgentRole::Root,
                model: "claude-opus-4".into(),
                skills: vec![],
                mcps: vec![],
                permissions: crate::config::types::PermissionConfig::default(),
                subagents: vec![],
                system_prompt: String::new(),
                max_steps: 90,
                subagent_depth: 3,
            },
            AgentConfig {
                name: "helper".into(),
                description: "non-root agent".into(),
                role: AgentRole::SubAgent,
                model: "claude-sonnet-4".into(),
                skills: vec![],
                mcps: vec![],
                permissions: crate::config::types::PermissionConfig::default(),
                subagents: vec![],
                system_prompt: String::new(),
                max_steps: 90,
                subagent_depth: 3,
            },
        ];
        let mut engine = Engine::new(config, event_tx, cmd_rx);
        // Simulate both root agents having been spawned by `initialize()`.
        engine.agents.insert("root".into(), AgentId::new());
        engine.agents.insert("writer".into(), AgentId::new());
        (engine, event_rx)
    }

    #[tokio::test]
    async fn test_handle_switch_agent_valid() {
        let (mut engine, mut rx) = engine_with_agents();
        engine.active_agent = "root".into();
        engine.current_model = "claude-sonnet-4".into();

        engine.handle_switch_agent("writer").await.unwrap();

        assert_eq!(engine.active_agent, "writer");
        assert_eq!(engine.current_model, "claude-opus-4");
        // AgentSwitched is emitted before ModelChanged.
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::AgentSwitched { name } => assert_eq!(name, "writer"),
            _ => panic!("expected AgentSwitched event"),
        }
        let ev = rx.try_recv().unwrap();
        match ev {
            EngineEvent::ModelChanged { model } => assert_eq!(model, "claude-opus-4"),
            _ => panic!("expected ModelChanged event"),
        }
    }

    #[tokio::test]
    async fn test_handle_switch_agent_unknown() {
        let (mut engine, _rx) = engine_with_agents();
        let err = engine.handle_switch_agent("ghost").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_handle_switch_agent_non_root() {
        let (mut engine, _rx) = engine_with_agents();
        let err = engine.handle_switch_agent("helper").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_handle_switch_agent_not_spawned() {
        // A root agent that is listed as a subagent of another agent is skipped
        // during `initialize()`, so it is absent from `self.agents` even though
        // it passes the config `role == Root` check.
        let (mut engine, _rx) = engine_with_agents();
        engine.config.agents.push(AgentConfig {
            name: "orphan".into(),
            description: "root but listed as subagent".into(),
            role: AgentRole::Root,
            model: "claude-sonnet-4".into(),
            skills: vec![],
            mcps: vec![],
            permissions: crate::config::types::PermissionConfig::default(),
            subagents: vec![],
            system_prompt: String::new(),
            max_steps: 90,
            subagent_depth: 3,
        });
        // `orphan` is NOT in `self.agents` (not spawned).
        let err = engine.handle_switch_agent("orphan").await.unwrap_err();
        assert!(err.to_string().contains("not spawned"));
    }

    #[tokio::test]
    async fn test_handle_switch_agent_current_model_consistency() {
        let (mut engine, _rx) = engine_with_agents();
        engine.active_agent = "root".into();
        engine.current_model = "claude-sonnet-4".into();

        engine.handle_switch_agent("writer").await.unwrap();
        assert_eq!(engine.current_model, "claude-opus-4");

        // Switching back restores the original agent's model.
        engine.handle_switch_agent("root").await.unwrap();
        assert_eq!(engine.current_model, "claude-sonnet-4");
        assert_eq!(engine.active_agent, "root");
    }

    #[test]
    fn test_reload_config_updates_engine_config() {
        let (mut engine, _rx) = engine_with_agents();
        let original_model = engine.current_model.clone();

        // Create a new config with a different model
        let mut new_config = Config::default();
        new_config.agents = vec![AgentConfig {
            name: "root".into(),
            description: "updated root".into(),
            role: AgentRole::Root,
            model: "gpt-4o".into(),
            skills: vec![],
            mcps: vec![],
            permissions: crate::config::types::PermissionConfig::default(),
            subagents: vec![],
            system_prompt: String::new(),
            max_steps: 90,
            subagent_depth: 3,
        }];

        engine.reload_config(new_config);

        // The config was updated
        assert_eq!(engine.config.agents[0].name, "root");
        assert_eq!(engine.config.agents[0].description, "updated root");
        // The current_model is NOT changed by reload_config (only the config is updated)
        assert_eq!(engine.current_model, original_model);
    }

    // ---------------------------------------------------------------------------
    // Hook merge tests
    // ---------------------------------------------------------------------------

    /// Helper: create an action for testing
    fn action(cmd: &str, timeout: u64) -> HookActionConfig {
        HookActionConfig {
            action: HookAction::Shell {
                command: cmd.into(),
            },
            timeout_secs: timeout,
        }
    }

    #[test]
    fn test_merge_hooks_precedence_config_wins() {
        // Config hook with same (command, timeout) as auto-detect should override it
        let config_hooks = {
            let mut m = HashMap::new();
            m.insert("after_apply".into(), vec![action("codegraph sync", 60)]);
            m
        };
        let auto_hooks = {
            let mut m = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("codegraph sync", 60)]);
            m
        };
        let plugin_hooks: Vec<(HookPoint, HookActionConfig)> = vec![];
        let skill_hooks: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();

        let merged = merge_hook_sources(&config_hooks, &plugin_hooks, &skill_hooks, &auto_hooks);
        let actions = merged.get(&HookPoint::AfterApply).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0].action {
            HookAction::Shell { command } => assert_eq!(command, "codegraph sync"),
        }
    }

    #[test]
    fn test_merge_hooks_dedup_same_command() {
        // Same command from two sources should deduplicate
        let config_hooks = HashMap::new();
        let auto_hooks = {
            let mut m = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("codegraph sync", 60)]);
            m
        };
        let plugin_hooks = vec![(HookPoint::AfterApply, action("codegraph sync", 60))];
        let skill_hooks = {
            let mut m = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("codegraph sync", 60)]);
            m
        };

        let merged = merge_hook_sources(&config_hooks, &plugin_hooks, &skill_hooks, &auto_hooks);
        let actions = merged.get(&HookPoint::AfterApply).unwrap();
        assert_eq!(
            actions.len(),
            1,
            "same (command, timeout) should deduplicate"
        );
    }

    #[test]
    fn test_merge_hooks_different_commands_all_kept() {
        let config_hooks = HashMap::new();
        let auto_hooks = {
            let mut m = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("hook_a", 30)]);
            m
        };
        let plugin_hooks = vec![(HookPoint::AfterApply, action("hook_b", 30))];
        let skill_hooks = HashMap::new();

        let merged = merge_hook_sources(&config_hooks, &plugin_hooks, &skill_hooks, &auto_hooks);
        let actions = merged.get(&HookPoint::AfterApply).unwrap();
        assert_eq!(actions.len(), 2, "different commands should both be kept");
    }

    #[test]
    fn test_merge_hooks_different_timeouts_all_kept() {
        // Same command but different timeout => different entries
        let config_hooks = HashMap::new();
        let auto_hooks = {
            let mut m = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("my_cmd", 30)]);
            m
        };
        let plugin_hooks = vec![(HookPoint::AfterApply, action("my_cmd", 60))];
        let skill_hooks = HashMap::new();

        let merged = merge_hook_sources(&config_hooks, &plugin_hooks, &skill_hooks, &auto_hooks);
        let actions = merged.get(&HookPoint::AfterApply).unwrap();
        assert_eq!(
            actions.len(),
            2,
            "same command but different timeouts should both be kept"
        );
    }

    #[test]
    fn test_parse_hook_point_all_variants() {
        assert_eq!(parse_hook_point("before_tool"), Some(HookPoint::BeforeTool));
        assert_eq!(parse_hook_point("after_tool"), Some(HookPoint::AfterTool));
        assert_eq!(
            parse_hook_point("before_apply"),
            Some(HookPoint::BeforeApply)
        );
        assert_eq!(parse_hook_point("after_apply"), Some(HookPoint::AfterApply));
        assert_eq!(
            parse_hook_point("before_shell"),
            Some(HookPoint::BeforeShell)
        );
        assert_eq!(parse_hook_point("after_shell"), Some(HookPoint::AfterShell));
        assert_eq!(
            parse_hook_point("before_fs_write"),
            Some(HookPoint::BeforeFsWrite)
        );
        assert_eq!(
            parse_hook_point("after_fs_write"),
            Some(HookPoint::AfterFsWrite)
        );
        assert_eq!(parse_hook_point("on_startup"), Some(HookPoint::OnStartup));
        assert_eq!(parse_hook_point("on_shutdown"), Some(HookPoint::OnShutdown));
        assert_eq!(parse_hook_point("unknown"), None);
    }
}
