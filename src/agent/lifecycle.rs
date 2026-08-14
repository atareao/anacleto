use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::context::{conversation_tokens, summarize_conversation};
use crate::agent::retry::retry_with_backoff;
use crate::agent::source::load_workspace_instructions;
use crate::agent::tool_store::{ToolOutputStore, truncate_output};
use crate::agent::tools::{
    SpawnSubagentConfig, apply_patch_tool_definition, check_tool_permission,
    execute_apply_patch_tool, execute_question_tool, execute_skill_tool, execute_task_tool,
    execute_todo_tool, plan_mode_blocked, question_tool_definition, skill_to_tool_definition,
    spawn_subagent_and_delegate, subagent_config_to_tool_definition, task_tool_definition,
    todo_tool_definition,
};
use crate::agent::types::{Agent, AgentMessage, AgentMode, AgentStatus, TaskMode};
use crate::config::types::{AgentConfig, RetryConfig, ToolSettings};
use crate::db::session::Database;
use crate::engine::jobs::JobRegistry;
use crate::engine::orchestrator::{EngineEvent, UsageEvent};
use crate::error::{Error, Result};
use crate::hook::{HookContext, HookPoint, HookRegistry};
use crate::llm::provider::{LlmProvider, LlmProviderRegistry};
use crate::llm::template::render_template;
use crate::llm::types::{
    LlmMessage, LlmRequest, LlmResponse, LlmStreamChunk, MessageRole, ToolCall, ToolDefinition,
};
use crate::mcp::client::McpRegistry;
use crate::plugin::PluginRegistry;
use crate::skill::registry::SharedSkillRegistry;
use crate::tools::glob::{execute_glob_tool, glob_tool_definition};
use crate::tools::grep::{execute_grep_tool, grep_tool_definition};
use crate::tools::lsp::{execute_lsp_query_tool, lsp_query_tool_definition};
use crate::tools::mcp::{
    execute_mcp_list_resource_templates_tool, execute_mcp_list_resources_tool,
    execute_mcp_read_resource_tool, mcp_list_resource_templates_tool_definition,
    mcp_list_resources_tool_definition, mcp_read_resource_tool_definition,
};
use crate::tools::read::{execute_read_tool, read_tool_definition};
use crate::tools::web::{
    execute_webfetch_tool, execute_websearch_tool, webfetch_tool_definition,
    websearch_tool_definition,
};

/// Maximum number of characters of a tool result passed to the LLM.
///
/// The full output is retained in the [`ToolOutputStore`]; only this many
/// characters are sent to the model to keep the context window bounded.
pub const TOOL_RESULT_MAX_CHARS: usize = 4000;

/// Shared state for tracking pending human approvals.
pub(crate) type PendingApprovals =
    Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>;
/// Shared state for tracking pending inline questions awaiting a user answer.
pub(crate) type PendingQuestions =
    Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>>;

/// Handle for communicating with a running agent task.
#[derive(Debug)]
pub struct AgentHandle {
    /// Channel to send messages to the agent.
    pub sender: mpsc::Sender<AgentMessage>,
    /// The agent's current status.
    pub status: AgentStatus,
}

impl AgentHandle {
    pub fn new(sender: mpsc::Sender<AgentMessage>) -> Self {
        Self {
            sender,
            status: AgentStatus::Idle,
        }
    }

    /// Send a message to the agent.
    pub async fn send(&self, msg: AgentMessage) -> Result<()> {
        self.sender
            .send(msg)
            .await
            .map_err(|_| Error::ChannelClosed("Agent channel closed".into()))
    }
}

/// Configuration for spawning a new agent.
pub struct SpawnAgentConfig {
    pub agent: Agent,
    pub provider: Arc<dyn LlmProvider>,
    /// Reference to the central skill registry.
    pub skill_registry: SharedSkillRegistry,
    /// Names of skills this agent has access to.
    pub skill_names: Vec<String>,
    pub subagent_configs: Vec<AgentConfig>,
    pub llm_registry: LlmProviderRegistry,
    pub mcp_registry: Option<Arc<tokio::sync::Mutex<McpRegistry>>>,
    pub mcp_enabled: Option<Arc<tokio::sync::Mutex<HashMap<String, bool>>>>,
    pub event_tx: mpsc::Sender<EngineEvent>,
    pub usage_tx: Option<mpsc::Sender<UsageEvent>>,
    pub retry_config: RetryConfig,
    pub db: Option<Database>,
    pub session_id: Option<Uuid>,
    pub pending_approvals: Option<PendingApprovals>,
    pub pending_questions: Option<PendingQuestions>,
    pub history_limit_percent: f64,
    /// The workspace directory that `apply_patch` operates on.
    pub workspace: PathBuf,
    /// If true, emit LlmRequestDebug/LlmResponseDebug events with serialized JSON.
    /// Shared so the `/debug` toggle takes effect on running agents immediately.
    pub debug: Arc<AtomicBool>,
    /// Optional id of the task that spawned this agent (via the `task` tool).
    pub task_id: Option<String>,
    /// Current delegation depth (0 for root agents). Used to enforce
    /// `subagent_depth` on dynamic `task` tool delegation.
    pub depth: u32,
    /// Operational mode (Plan = read-only, Build = full access).
    pub mode: AgentMode,
    /// Shared registry of background jobs, used to track `task` tool
    /// background delegations.
    pub job_registry: Option<Arc<tokio::sync::Mutex<JobRegistry>>>,
    /// Loaded plugins and their custom tools.
    pub plugins: Option<Arc<PluginRegistry>>,
    /// Semaphore to limit concurrent subagent spawns.
    pub concurrency_semaphore: Option<Arc<tokio::sync::Semaphore>>,
    /// Hook registry for pre/post execution hooks.
    pub hook_registry: HookRegistry,
    /// Optional external cancel flag. If provided, used instead of creating one internally.
    pub cancel_flag: Option<Arc<AtomicBool>>,

    /// Default tool display properties from config.yaml.
    /// Used to populate display settings for tools the agent declares.
    pub tool_defaults: HashMap<String, crate::config::ToolDefaults>,
}

/// Spawn a new agent task and return a handle to it.
///
/// The spawned task receives messages from its channel, calls the LLM
/// provider with loaded skills as tools, and handles tool call loops.
/// If `subagent_configs` is non-empty, those subagents are exposed as
/// tools the LLM can invoke for task delegation.
pub async fn spawn_agent(config: SpawnAgentConfig) -> AgentHandle {
    let SpawnAgentConfig {
        agent,
        provider,
        skill_registry,
        skill_names,
        subagent_configs,
        llm_registry,
        mcp_registry,
        mcp_enabled,
        event_tx,
        usage_tx,
        retry_config,
        db,
        session_id,
        pending_approvals,
        pending_questions,
        history_limit_percent,
        debug,
        workspace,
        task_id: _task_id,
        depth,
        mode,
        job_registry,
        plugins,
        concurrency_semaphore,
        hook_registry,
        cancel_flag,
        tool_defaults,
    } = config;
    let (tx, mut rx) = mpsc::channel::<AgentMessage>(256);
    let handle = AgentHandle::new(tx);

    let agent_id = agent.id.clone();
    let agent_name = agent.name.clone();
    let model = agent.model.clone();
    let agent_permissions = agent.permissions.clone();
    let max_steps = agent.max_steps;
    let subagent_depth = agent.subagent_depth;
    let tool_settings = Arc::new(agent.tool_settings);
    let tool_settings_clone = (*tool_settings).clone();

    // Load agent description as system prompt (rendered as a template below)

    // Build tool list from registry: skills + subagents + built-in tools
    let mut tools: Vec<ToolDefinition> = {
        let reg = skill_registry.read().await;
        skill_names
            .iter()
            .filter_map(|name| reg.get(name))
            .map(skill_to_tool_definition)
            .collect()
    };
    for sc in &subagent_configs {
        tools.push(subagent_config_to_tool_definition(sc));
    }

    // Add built-in tools based on agent's tool declarations.
    // Only tools listed in the agent's `tools:` frontmatter are included.
    // Display properties are merged from config.yaml defaults + agent overrides.
    let builtin_tools = builtin_tool_definitions();
    for (tool_name, agent_settings) in &tool_settings_clone {
        if !agent_settings.enabled {
            continue;
        }
        if let Some(mut def) = builtin_tools.get(tool_name).cloned() {
            // Merge description from config.yaml defaults
            if let Some(defaults) = tool_defaults.get(tool_name)
                && !defaults.description.is_empty()
            {
                def.description = defaults.description.clone();
            }
            tools.push(def);
        }
    }

    // Add custom tools registered by plugins.
    if let Some(plugins) = &plugins {
        tools.extend(plugins.custom_tools().iter().cloned());
    }

    // Render the system prompt template (supports {model}, {workspace}, {tools}, {subagents}).
    let system_prompt = {
        let tool_names = tools
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        // Build subagents block for the {subagents} template variable
        let subagents_template = if subagent_configs.is_empty() {
            String::new()
        } else {
            let mut block = String::from("\n");
            for sc in &subagent_configs {
                block.push_str(&format!("- **{}** — {}", sc.name, sc.description));
                if !sc.when_to_use.is_empty() {
                    block.push_str(&format!(" ({})", sc.when_to_use));
                }
                block.push('\n');
            }
            block
        };
        let mut vars = HashMap::new();
        vars.insert("model".to_string(), model.clone());
        vars.insert(
            "workspace".to_string(),
            workspace.to_string_lossy().to_string(),
        );
        vars.insert("tools".to_string(), tool_names);
        vars.insert("subagents".to_string(), subagents_template);
        render_template(&agent.description, &vars)
    };

    // Auto-inject subagents block: the parent discovers what each subagent
    // does and when to use it, without editing the parent's Markdown.
    // Skip if the template already uses {subagents} to avoid duplication.
    let mut system_prompt = system_prompt;
    if !subagent_configs.is_empty() && !agent.description.contains("{subagents}") {
        let mut subagents_block = String::from("\n\n--- Available subagents ---\n");
        for sc in &subagent_configs {
            subagents_block.push_str(&format!("• **{}** — {}\n", sc.name, sc.description));
            if !sc.when_to_use.is_empty() {
                subagents_block.push_str(&format!("  *When to use*: {}\n", sc.when_to_use));
            }
        }
        system_prompt.push_str(&subagents_block);
    }

    // Let plugins transform the system prompt before it is sent to the model.
    let system_prompt = if let Some(plugins) = &plugins {
        plugins.on_agent_spawn(&agent_name, &system_prompt)
    } else {
        system_prompt
    };

    // Send tool settings to the TUI for display customization.
    let _ = event_tx
        .send(EngineEvent::ToolSettingsUpdated(tool_settings_clone))
        .await;

    // Clone what the task needs
    let agent_mcp_names = agent.mcps.clone();
    let debug_mode = debug;
    let cancel_flag = cancel_flag.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));

    tokio::spawn(async move {
        let mut conversation: Vec<LlmMessage> = Vec::new();

        // Store of full tool outputs keyed by tool_call_id. The LLM receives a
        // truncated version; the full output is retained here for re-query.
        let mut tool_store = ToolOutputStore::new();

        // Context window budget
        let max_history_tokens =
            (provider.context_window() as f64 * history_limit_percent / 100.0) as usize;

        // Collect MCP tools inside the async task
        let mcp_tool_map: std::collections::HashMap<String, (String, String)> = {
            let mut map = std::collections::HashMap::new();
            if let Some(ref mcp_reg) = mcp_registry {
                let reg = mcp_reg.lock().await;
                let collected = reg.collect_tools(&agent_mcp_names).await;
                for (server_name, original_name, tool_def) in collected {
                    // Skip servers that have been disabled via `/mcps`.
                    if let Some(ref enabled_map) = mcp_enabled
                        && !*enabled_map.lock().await.get(&server_name).unwrap_or(&true)
                    {
                        continue;
                    }
                    let prefixed_name = tool_def.name.clone();
                    map.insert(prefixed_name, (server_name, original_name));
                    tools.push(tool_def);
                }
            }
            map
        };

        // Prepend system prompt if available
        if !system_prompt.is_empty() {
            conversation.push(LlmMessage {
                role: MessageRole::System,
                content: system_prompt.clone(),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Inject workspace instruction files (AGENTS.md, CLAUDE.md, CONTEXT.md)
        // as initial System context when they exist in the workspace.
        if workspace.is_dir() {
            for (name, content) in load_workspace_instructions(&workspace) {
                conversation.push(LlmMessage {
                    role: MessageRole::System,
                    content: format!("[Instrucciones del workspace: {name}]\n{content}"),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        while let Some(msg) = rx.recv().await {
            match msg {
                AgentMessage::UserInput { content } => {
                    // Emit status: Working
                    let _ = event_tx
                        .send(EngineEvent::AgentStatusChanged {
                            agent_id: agent_id.clone(),
                            agent_name: agent_name.clone(),
                            status: AgentStatus::Working,
                        })
                        .await;

                    // Emit that the agent received a message
                    let _ = event_tx
                        .send(EngineEvent::AgentMessage {
                            agent_id: agent_id.clone(),
                            agent_name: agent_name.clone(),
                            message: content.clone(),
                        })
                        .await;

                    // Persist user message
                    if let (Some(db), Some(sid)) = (db.as_ref(), session_id) {
                        let _ = db
                            .store_message(sid, &agent_name, "user", &content, None)
                            .await;
                    }

                    // Add user message to conversation history
                    conversation.push(LlmMessage {
                        role: MessageRole::User,
                        content: content.clone(),
                        tool_calls: None,
                        tool_call_id: None,
                    });

                    // Trim conversation to fit context window
                    let compacted = summarize_conversation(
                        &mut conversation,
                        max_history_tokens,
                        Some(&*provider),
                        &model,
                        false,
                        Some(&tool_store),
                        Some(&retry_config),
                    )
                    .await;
                    if compacted {
                        let tokens = conversation_tokens(&conversation) as u32;
                        let _ = event_tx
                            .send(EngineEvent::ConversationCompacted {
                                agent_id: agent_id.clone(),
                                agent_name: agent_name.clone(),
                                tokens,
                            })
                            .await;
                        let _ = event_tx
                            .send(EngineEvent::LocalTokenEstimate {
                                tokens: tokens as usize,
                            })
                            .await;
                    }

                    // Inner loop: LLM may call tools, we execute and loop back
                    let mut step_count: u32 = 0;
                    'tool_loop: loop {
                        step_count += 1;
                        if cancel_flag.load(Ordering::Relaxed) {
                            cancel_flag.store(false, Ordering::Relaxed);
                            let _ = event_tx
                                .send(EngineEvent::AgentStatusChanged {
                                    agent_id: agent_id.clone(),
                                    agent_name: agent_name.clone(),
                                    status: AgentStatus::Idle,
                                })
                                .await;
                            break 'tool_loop;
                        }
                        if step_count > max_steps {
                            // Mark the task as incomplete: emit a clear output and go idle.
                            let _ = event_tx
                                .send(EngineEvent::AgentOutput {
                                    agent_id: agent_id.clone(),
                                    agent_name: agent_name.clone(),
                                    content: format!(
                                        "[Tarea incompleta] Se alcanzó el límite de {max_steps} pasos (turnos) sin completar la tarea."
                                    ),
                                })
                                .await;
                            let _ = event_tx
                                .send(EngineEvent::AgentStatusChanged {
                                    agent_id: agent_id.clone(),
                                    agent_name: agent_name.clone(),
                                    status: AgentStatus::Idle,
                                })
                                .await;
                            break 'tool_loop;
                        }
                        // Trim before each LLM call (tool results may have grown conversation)
                        let compacted = summarize_conversation(
                            &mut conversation,
                            max_history_tokens,
                            Some(&*provider),
                            &model,
                            false,
                            Some(&tool_store),
                            Some(&retry_config),
                        )
                        .await;
                        if compacted {
                            let tokens = conversation_tokens(&conversation) as u32;
                            let _ = event_tx
                                .send(EngineEvent::ConversationCompacted {
                                    agent_id: agent_id.clone(),
                                    agent_name: agent_name.clone(),
                                    tokens,
                                })
                                .await;
                            let _ = event_tx
                                .send(EngineEvent::LocalTokenEstimate {
                                    tokens: tokens as usize,
                                })
                                .await;
                        }

                        let request = LlmRequest {
                            model: model.clone(),
                            messages: conversation.clone(),
                            tools: tools.clone(),
                            max_tokens: None,
                            temperature: None,
                            stream: true,
                            cache_control: None,
                        };

                        // Emit debug event for LLM request if debug mode is on
                        if debug_mode.load(Ordering::Relaxed) {
                            let payload = serde_json::to_string_pretty(&request)
                                .unwrap_or_else(|_| "{}".to_string());
                            let _ = event_tx
                                .send(EngineEvent::LlmRequestDebug {
                                    agent_name: agent_name.clone(),
                                    model: model.clone(),
                                    payload,
                                })
                                .await;
                        }

                        // Wrap LLM call with retries
                        let retry_cfg = retry_config.clone();
                        let agent_name_clone = agent_name.clone();
                        let llm_result = retry_with_backoff(
                            |_attempt| {
                                let req = request.clone();
                                let prov = provider.clone();
                                async move { prov.complete_stream(req).await }
                            },
                            &retry_cfg,
                            &format!("LLM stream call for '{}'", agent_name_clone),
                        )
                        .await;

                        match llm_result {
                            Ok(mut stream_rx) => {
                                let mut full_response = String::new();
                                let mut tool_calls: Vec<ToolCall> = Vec::new();
                                let mut stream_error: Option<String> = None;

                                // Collect all chunks from the stream
                                while let Some(chunk) = stream_rx.recv().await {
                                    match chunk {
                                        Ok(LlmStreamChunk::Thinking(text)) => {
                                            let _ = event_tx
                                                .send(EngineEvent::AgentThinkingChunk {
                                                    agent_id: agent_id.clone(),
                                                    agent_name: agent_name.clone(),
                                                    content: text,
                                                })
                                                .await;
                                        }
                                        Ok(LlmStreamChunk::Content(text)) => {
                                            full_response.push_str(&text);
                                            let _ = event_tx
                                                .send(EngineEvent::AgentStreamChunk {
                                                    agent_id: agent_id.clone(),
                                                    agent_name: agent_name.clone(),
                                                    content: text,
                                                })
                                                .await;
                                        }
                                        Ok(LlmStreamChunk::ToolCall(tc)) => {
                                            tool_calls.push(tc);
                                        }
                                        Ok(LlmStreamChunk::Done(usage)) => {
                                            let cost = (usage.prompt_tokens as f64
                                                * provider.input_price_per_million()
                                                + usage.completion_tokens as f64
                                                    * provider.output_price_per_million())
                                                / 1_000_000.0;
                                            let _ = event_tx
                                                .send(EngineEvent::TokenUsage {
                                                    agent_id: agent_id.clone(),
                                                    agent_name: agent_name.clone(),
                                                    total_tokens: usage.total_tokens,
                                                    prompt_tokens: usage.prompt_tokens,
                                                    context_window: provider.context_window()
                                                        as u32,
                                                    cost,
                                                })
                                                .await;
                                            if let Some(ref utx) = usage_tx {
                                                let _ = utx
                                                    .send(UsageEvent {
                                                        total_tokens: usage.total_tokens,
                                                        cost,
                                                    })
                                                    .await;
                                            }
                                            break;
                                        }
                                        Ok(LlmStreamChunk::Error(e)) => {
                                            stream_error = Some(e);
                                            break;
                                        }
                                        Err(e) => {
                                            stream_error = Some(format!("LLM stream error: {e}"));
                                            break;
                                        }
                                    }
                                }

                                // Emit debug event for completed LLM response
                                if debug_mode.load(Ordering::Relaxed) {
                                    let response = LlmResponse {
                                        content: full_response.clone(),
                                        tool_calls: tool_calls.clone(),
                                        finish_reason: if tool_calls.is_empty() {
                                            "stop".to_string()
                                        } else {
                                            "tool_use".to_string()
                                        },
                                        usage: None,
                                        thinking: None,
                                    };
                                    let payload = serde_json::to_string_pretty(&response)
                                        .unwrap_or_else(|_| "{}".to_string());
                                    let _ = event_tx
                                        .send(EngineEvent::LlmResponseDebug {
                                            agent_name: agent_name.clone(),
                                            model: model.clone(),
                                            payload,
                                        })
                                        .await;
                                }

                                // Handle stream error
                                if let Some(e) = stream_error {
                                    let _ = event_tx
                                        .send(EngineEvent::Error {
                                            agent_id: Some(agent_id.clone()),
                                            message: e,
                                        })
                                        .await;
                                    // Emit status: Idle on error
                                    let _ = event_tx
                                        .send(EngineEvent::AgentStatusChanged {
                                            agent_id: agent_id.clone(),
                                            agent_name: agent_name.clone(),
                                            status: AgentStatus::Idle,
                                        })
                                        .await;
                                    break 'tool_loop;
                                }

                                // Handle Done — decide based on tool calls
                                if tool_calls.is_empty() {
                                    // Persist assistant response
                                    if let (Some(db), Some(sid)) = (db.as_ref(), session_id) {
                                        let _ = db
                                            .store_message(
                                                sid,
                                                &agent_name,
                                                "assistant",
                                                &full_response,
                                                None,
                                            )
                                            .await;
                                    }

                                    // No tool calls — finalize response
                                    if !full_response.is_empty() {
                                        conversation.push(LlmMessage {
                                            role: MessageRole::Assistant,
                                            content: full_response.clone(),
                                            tool_calls: None,
                                            tool_call_id: None,
                                        });
                                    }
                                    // The response was already streamed chunk-by-chunk via
                                    // AgentStreamChunk events. Send empty content so the TUI
                                    // does not duplicate the text (commit_stream + push_msg).
                                    let _ = event_tx
                                        .send(EngineEvent::AgentOutput {
                                            agent_id: agent_id.clone(),
                                            agent_name: agent_name.clone(),
                                            content: String::new(),
                                        })
                                        .await;
                                    // Emit status: Idle
                                    let _ = event_tx
                                        .send(EngineEvent::AgentStatusChanged {
                                            agent_id: agent_id.clone(),
                                            agent_name: agent_name.clone(),
                                            status: AgentStatus::Idle,
                                        })
                                        .await;
                                    break 'tool_loop;
                                }

                                // Tool calls present — save assistant message and execute
                                conversation.push(LlmMessage {
                                    role: MessageRole::Assistant,
                                    content: full_response,
                                    tool_calls: Some(tool_calls.clone()),
                                    tool_call_id: None,
                                });

                                // Execute each tool call — try skills first, then subagents, then MCP.
                                //
                                // `execute_one` runs a single tool call and returns
                                // `(tool_call_id, result_string)`. It captures the shared
                                // state by reference so it can be invoked concurrently for
                                // multiple `task` calls in the same batch.
                                //
                                // Bind shared state as references (Copy) so the `async move`
                                // closure captures only references and remains `Fn` (callable
                                // multiple times, including concurrently).
                                let agent_permissions = &agent_permissions;
                                let llm_registry = &llm_registry;
                                let skill_registry = &skill_registry;
                                let skill_names = &skill_names;
                                let event_tx = &event_tx;
                                let usage_tx = &usage_tx;
                                let db = &db;
                                let retry_config = &retry_config;
                                let debug_mode = &debug_mode;
                                let cancel_flag = &cancel_flag;
                                let agent_name = &agent_name;
                                let agent_id = &agent_id;
                                let model = &model;
                                let job_registry = &job_registry;
                                let subagent_configs = &subagent_configs;
                                let workspace = &workspace;
                                let pending_approvals = &pending_approvals;
                                let pending_questions = &pending_questions;
                                let plugins = &plugins;
                                let mcp_registry = &mcp_registry;
                                let mcp_tool_map = &mcp_tool_map;
                                let mode = &mode;
                                let concurrency_semaphore = &concurrency_semaphore;
                                let hook_registry = &hook_registry;
                                let ts = Arc::clone(&tool_settings);

                                // Shared flag: set to true when a subagent runs out of
                                // steps, signalling the tool loop to stop.
                                let subagent_stopped = Arc::new(AtomicBool::new(false));
                                let subagent_stopped = &subagent_stopped;

                                let execute_one = |tc: ToolCall| {
                                    let ts = ts.clone();
                                    async move {
                                        if cancel_flag.load(Ordering::Relaxed) {
                                            return (
                                                tc.id.clone(),
                                                "[Cancelled] Operation stopped by user".to_string(),
                                            );
                                        }
                                        // Fire BeforeTool hook
                                        {
                                            let ctx = HookContext {
                                                tool_name: Some(tc.function.name.clone()),
                                                agent_name: Some(agent_name.clone()),
                                                ..Default::default()
                                            };
                                            let hook_results = hook_registry
                                                .run(HookPoint::BeforeTool, &ctx)
                                                .await;
                                            for r in &hook_results {
                                                let _ = event_tx
                                                    .send(EngineEvent::HookExecuted {
                                                        point: format!(
                                                            "{:?}",
                                                            HookPoint::BeforeTool
                                                        ),
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
                                        }

                                        // Check permissions before executing
                                        let permission_ok = check_tool_permission(
                                            &tc,
                                            agent_permissions,
                                            pending_approvals,
                                            event_tx,
                                            agent_name,
                                        )
                                        .await;

                                        // In Plan mode, block write tools (read-only).
                                        let plan_blocked = plan_mode_blocked(
                                            mode,
                                            &tc.function.name,
                                            &tc.function.arguments,
                                        );

                                        let result = if let Some(msg) = plan_blocked {
                                            msg
                                        } else if !permission_ok {
                                            format!(
                                                "Operation '{}' was denied by user or permissions.",
                                                tc.function.name
                                            )
                                        } else if let Some(hook_result) =
                                            plugins.as_ref().and_then(|p| p.on_tool_call(&tc))
                                        {
                                            // A plugin short-circuited this tool call.
                                            hook_result
                                        } else if tc.function.name == "task" {
                                            // Acquire concurrency permit if semaphore is configured
                                            let _permit = if let Some(sem) = concurrency_semaphore {
                                                match sem.acquire().await {
                                                    Ok(permit) => Some(permit),
                                                    Err(e) => {
                                                        return (
                                                            tc.id.clone(),
                                                            format!("Semaphore error: {e}"),
                                                        );
                                                    }
                                                }
                                            } else {
                                                None
                                            };
                                            let show_task = should_emit_tool(ts.as_ref(), "task");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "task",
                                                &tc.function.arguments,
                                            );
                                            if show_task {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "task".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let task_result = execute_task_tool(
                                                &tc,
                                                agent_permissions,
                                                llm_registry,
                                                skill_registry,
                                                skill_names,
                                                event_tx,
                                                usage_tx,
                                                db,
                                                session_id,
                                                history_limit_percent,
                                                retry_config,
                                                debug_mode,
                                                depth,
                                                subagent_depth,
                                                agent_name,
                                                agent_id,
                                                model,
                                                job_registry,
                                                subagent_configs,
                                            )
                                            .await;
                                            let (is_ok, output) = match &task_result {
                                                Ok(o) => (true, o.clone()),
                                                Err(e) => (false, e.clone()),
                                            };
                                            if show_task {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "task".to_string(),
                                                        success: is_ok,
                                                        summary: truncate_output(&output, 5000),
                                                    })
                                                    .await;
                                            }
                                            task_result.unwrap_or_else(|e| e)
                                        } else if tc.function.name == "todo" {
                                            let show_todo = should_emit_tool(ts.as_ref(), "todo");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "todo",
                                                &tc.function.arguments,
                                            );
                                            if show_todo {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "todo".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let result =
                                                execute_todo_tool(db, session_id, &tc, event_tx)
                                                    .await;
                                            if show_todo {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "todo".to_string(),
                                                        success: result.is_ok(),
                                                        summary: match &result {
                                                            Ok(s) => truncate_output(s, 5000),
                                                            Err(e) => e.clone(),
                                                        },
                                                    })
                                                    .await;
                                            }
                                            result.unwrap_or_else(|e| e)
                                        } else if tc.function.name == "question" {
                                            let show_q = should_emit_tool(ts.as_ref(), "question");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "question",
                                                &tc.function.arguments,
                                            );
                                            if show_q {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "question".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let result = execute_question_tool(
                                                pending_questions,
                                                &tc,
                                                event_tx,
                                            )
                                            .await;
                                            if show_q {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "question".to_string(),
                                                        success: result.is_ok(),
                                                        summary: match &result {
                                                            Ok(s) => truncate_output(s, 5000),
                                                            Err(e) => e.clone(),
                                                        },
                                                    })
                                                    .await;
                                            }
                                            result.unwrap_or_else(|e| e)
                                        } else if tc.function.name == "apply_patch" {
                                            let show_ap =
                                                should_emit_tool(ts.as_ref(), "apply_patch");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "apply_patch",
                                                &tc.function.arguments,
                                            );
                                            if show_ap {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "apply_patch".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let apply_ctx = HookContext {
                                                tool_name: Some(tc.function.name.clone()),
                                                agent_name: Some(agent_name.clone()),
                                                ..Default::default()
                                            };
                                            let hook_results = hook_registry
                                                .run(HookPoint::BeforeApply, &apply_ctx)
                                                .await;
                                            for r in &hook_results {
                                                let _ = event_tx
                                                    .send(EngineEvent::HookExecuted {
                                                        point: format!(
                                                            "{:?}",
                                                            HookPoint::BeforeApply
                                                        ),
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
                                            let apply_result = execute_apply_patch_tool(
                                                workspace,
                                                agent_permissions,
                                                pending_approvals,
                                                event_tx,
                                                agent_name,
                                                &tc,
                                            )
                                            .await
                                            .unwrap_or_else(|e| e);
                                            let hook_results = hook_registry
                                                .run(HookPoint::AfterApply, &apply_ctx)
                                                .await;
                                            for r in &hook_results {
                                                let _ = event_tx
                                                    .send(EngineEvent::HookExecuted {
                                                        point: format!(
                                                            "{:?}",
                                                            HookPoint::AfterApply
                                                        ),
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
                                            if show_ap {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "apply_patch".to_string(),
                                                        success: true,
                                                        summary: truncate_output(
                                                            &apply_result,
                                                            5000,
                                                        ),
                                                    })
                                                    .await;
                                            }
                                            apply_result
                                        } else if tc.function.name == "read" {
                                            let show = should_emit_tool(ts.as_ref(), "read");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "read",
                                                &tc.function.arguments,
                                            );
                                            if show {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "read".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let result = execute_read_tool(
                                                workspace,
                                                agent_permissions,
                                                &tc,
                                            )
                                            .await;
                                            if show {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "read".to_string(),
                                                        success: result.is_ok(),
                                                        summary: match &result {
                                                            Ok(s) => truncate_output(s, 5000),
                                                            Err(e) => e.clone(),
                                                        },
                                                    })
                                                    .await;
                                            }
                                            result.unwrap_or_else(|e| e)
                                        } else if tc.function.name == "grep" {
                                            let show_grep = should_emit_tool(ts.as_ref(), "grep");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "grep",
                                                &tc.function.arguments,
                                            );
                                            if show_grep {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "grep".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let result = execute_grep_tool(
                                                workspace,
                                                agent_permissions,
                                                &tc,
                                            )
                                            .await;
                                            if show_grep {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "grep".to_string(),
                                                        success: result.is_ok(),
                                                        summary: match &result {
                                                            Ok(s) => truncate_output(s, 5000),
                                                            Err(e) => e.clone(),
                                                        },
                                                    })
                                                    .await;
                                            }
                                            result.unwrap_or_else(|e| e)
                                        } else if tc.function.name == "glob" {
                                            let show_glob = should_emit_tool(ts.as_ref(), "glob");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "glob",
                                                &tc.function.arguments,
                                            );
                                            if show_glob {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "glob".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let result = execute_glob_tool(
                                                workspace,
                                                agent_permissions,
                                                &tc,
                                            )
                                            .await;
                                            if show_glob {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "glob".to_string(),
                                                        success: result.is_ok(),
                                                        summary: match &result {
                                                            Ok(s) => truncate_output(s, 5000),
                                                            Err(e) => e.clone(),
                                                        },
                                                    })
                                                    .await;
                                            }
                                            result.unwrap_or_else(|e| e)
                                        } else if tc.function.name == "webfetch" {
                                            let show_wf = should_emit_tool(ts.as_ref(), "webfetch");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "webfetch",
                                                &tc.function.arguments,
                                            );
                                            if show_wf {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "webfetch".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let result =
                                                execute_webfetch_tool(agent_permissions, &tc).await;
                                            if show_wf {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "webfetch".to_string(),
                                                        success: result.is_ok(),
                                                        summary: match &result {
                                                            Ok(s) => truncate_output(s, 5000),
                                                            Err(e) => e.clone(),
                                                        },
                                                    })
                                                    .await;
                                            }
                                            result.unwrap_or_else(|e| e)
                                        } else if tc.function.name == "websearch" {
                                            let show_ws =
                                                should_emit_tool(ts.as_ref(), "websearch");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "websearch",
                                                &tc.function.arguments,
                                            );
                                            if show_ws {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "websearch".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let result =
                                                execute_websearch_tool(agent_permissions, &tc)
                                                    .await;
                                            if show_ws {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "websearch".to_string(),
                                                        success: result.is_ok(),
                                                        summary: match &result {
                                                            Ok(s) => truncate_output(s, 5000),
                                                            Err(e) => e.clone(),
                                                        },
                                                    })
                                                    .await;
                                            }
                                            result.unwrap_or_else(|e| e)
                                        } else if tc.function.name == "mcp_list_resources" {
                                            let show_mcp_lr =
                                                should_emit_tool(ts.as_ref(), "mcp_list_resources");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "mcp_list_resources",
                                                &tc.function.arguments,
                                            );
                                            if show_mcp_lr {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "mcp_list_resources".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            match mcp_registry {
                                                Some(reg) => match execute_mcp_list_resources_tool(
                                                    reg,
                                                    agent_permissions,
                                                    &tc,
                                                )
                                                .await
                                                {
                                                    Ok(output) => {
                                                        if show_mcp_lr {
                                                            let _ = event_tx
                                                                .send(EngineEvent::ToolResult {
                                                                    agent_id: agent_id.clone(),
                                                                    agent_name: agent_name.clone(),
                                                                    tool_name: "mcp_list_resources"
                                                                        .to_string(),
                                                                    success: true,
                                                                    summary: truncate_output(
                                                                        &output, 5000,
                                                                    ),
                                                                })
                                                                .await;
                                                        }
                                                        output
                                                    }
                                                    Err(e) => {
                                                        if show_mcp_lr {
                                                            let _ = event_tx
                                                                .send(EngineEvent::ToolResult {
                                                                    agent_id: agent_id.clone(),
                                                                    agent_name: agent_name.clone(),
                                                                    tool_name: "mcp_list_resources"
                                                                        .to_string(),
                                                                    success: false,
                                                                    summary: e.clone(),
                                                                })
                                                                .await;
                                                        }
                                                        e
                                                    }
                                                },
                                                None => {
                                                    let msg =
                                                    "MCP registry not available for mcp_list_resources"
                                                        .to_string();
                                                    if show_mcp_lr {
                                                        let _ = event_tx
                                                            .send(EngineEvent::ToolResult {
                                                                agent_id: agent_id.clone(),
                                                                agent_name: agent_name.clone(),
                                                                tool_name: "mcp_list_resources"
                                                                    .to_string(),
                                                                success: false,
                                                                summary: msg.clone(),
                                                            })
                                                            .await;
                                                    }
                                                    msg
                                                }
                                            }
                                        } else if tc.function.name == "mcp_read_resource" {
                                            let show_mcp_rr =
                                                should_emit_tool(ts.as_ref(), "mcp_read_resource");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "mcp_read_resource",
                                                &tc.function.arguments,
                                            );
                                            if show_mcp_rr {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "mcp_read_resource".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            match mcp_registry {
                                                Some(reg) => {
                                                    match execute_mcp_read_resource_tool(
                                                        reg,
                                                        agent_permissions,
                                                        &tc,
                                                    )
                                                    .await
                                                    {
                                                        Ok(output) => {
                                                            if show_mcp_rr {
                                                                let _ = event_tx
                                                                    .send(EngineEvent::ToolResult {
                                                                        agent_id: agent_id.clone(),
                                                                        agent_name: agent_name
                                                                            .clone(),
                                                                        tool_name:
                                                                            "mcp_read_resource"
                                                                                .to_string(),
                                                                        success: true,
                                                                        summary: truncate_output(
                                                                            &output, 5000,
                                                                        ),
                                                                    })
                                                                    .await;
                                                            }
                                                            output
                                                        }
                                                        Err(e) => {
                                                            if show_mcp_rr {
                                                                let _ = event_tx
                                                                    .send(EngineEvent::ToolResult {
                                                                        agent_id: agent_id.clone(),
                                                                        agent_name: agent_name
                                                                            .clone(),
                                                                        tool_name:
                                                                            "mcp_read_resource"
                                                                                .to_string(),
                                                                        success: false,
                                                                        summary: e.clone(),
                                                                    })
                                                                    .await;
                                                            }
                                                            e
                                                        }
                                                    }
                                                }
                                                None => {
                                                    let msg =
                                                    "MCP registry not available for mcp_read_resource"
                                                        .to_string();
                                                    if show_mcp_rr {
                                                        let _ = event_tx
                                                            .send(EngineEvent::ToolResult {
                                                                agent_id: agent_id.clone(),
                                                                agent_name: agent_name.clone(),
                                                                tool_name: "mcp_read_resource"
                                                                    .to_string(),
                                                                success: false,
                                                                summary: msg.clone(),
                                                            })
                                                            .await;
                                                    }
                                                    msg
                                                }
                                            }
                                        } else if tc.function.name == "mcp_list_resource_templates"
                                        {
                                            let show_mcp_lrt = should_emit_tool(
                                                ts.as_ref(),
                                                "mcp_list_resource_templates",
                                            );
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "mcp_list_resource_templates",
                                                &tc.function.arguments,
                                            );
                                            if show_mcp_lrt {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "mcp_list_resource_templates"
                                                            .to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            match mcp_registry {
                                                Some(reg) => {
                                                    match execute_mcp_list_resource_templates_tool(
                                                        reg,
                                                        agent_permissions,
                                                        &tc,
                                                    )
                                                    .await
                                                    {
                                                        Ok(output) => {
                                                            if show_mcp_lrt {
                                                                let _ = event_tx
                                                                .send(EngineEvent::ToolResult {
                                                                    agent_id: agent_id.clone(),
                                                                    agent_name: agent_name.clone(),
                                                                    tool_name:
                                                                        "mcp_list_resource_templates"
                                                                            .to_string(),
                                                                    success: true,
                                                                    summary: truncate_output(
                                                                        &output, 5000,
                                                                    ),
                                                                })
                                                                .await;
                                                            }
                                                            output
                                                        }
                                                        Err(e) => {
                                                            if show_mcp_lrt {
                                                                let _ = event_tx
                                                                .send(EngineEvent::ToolResult {
                                                                    agent_id: agent_id.clone(),
                                                                    agent_name: agent_name.clone(),
                                                                    tool_name:
                                                                        "mcp_list_resource_templates"
                                                                            .to_string(),
                                                                    success: false,
                                                                    summary: e.clone(),
                                                                })
                                                                .await;
                                                            }
                                                            e
                                                        }
                                                    }
                                                }
                                                None => {
                                                    let msg = "MCP registry not available for mcp_list_resource_templates"
                                                    .to_string();
                                                    if show_mcp_lrt {
                                                        let _ = event_tx
                                                            .send(EngineEvent::ToolResult {
                                                                agent_id: agent_id.clone(),
                                                                agent_name: agent_name.clone(),
                                                                tool_name:
                                                                    "mcp_list_resource_templates"
                                                                        .to_string(),
                                                                success: false,
                                                                summary: msg.clone(),
                                                            })
                                                            .await;
                                                    }
                                                    msg
                                                }
                                            }
                                        } else if tc.function.name == "lsp_query" {
                                            let show_lsp =
                                                should_emit_tool(ts.as_ref(), "lsp_query");
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                "lsp_query",
                                                &tc.function.arguments,
                                            );
                                            if show_lsp {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "lsp_query".to_string(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            let result =
                                                execute_lsp_query_tool(agent_permissions, &tc)
                                                    .await;
                                            if show_lsp {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: "lsp_query".to_string(),
                                                        success: result.is_ok(),
                                                        summary: match &result {
                                                            Ok(s) => truncate_output(s, 5000),
                                                            Err(e) => e.clone(),
                                                        },
                                                    })
                                                    .await;
                                            }
                                            result.unwrap_or_else(|e| e)
                                        } else if skill_names.contains(&tc.function.name) {
                                            let show_skill =
                                                should_emit_tool(ts.as_ref(), &tc.function.name);
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                &tc.function.name,
                                                &tc.function.arguments,
                                            );
                                            let reg = skill_registry.read().await;
                                            execute_skill_tool(
                                                &reg,
                                                agent_name,
                                                &tc,
                                                event_tx,
                                                agent_id,
                                                Some(hook_registry),
                                                show_skill,
                                                &task_preview,
                                            )
                                            .await
                                            .unwrap_or_else(|e| e)
                                        } else if let Some(config) = subagent_configs
                                            .iter()
                                            .find(|c| c.name == tc.function.name)
                                        {
                                            // Extract task from tool call arguments
                                            let args: serde_json::Value =
                                                serde_json::from_str(&tc.function.arguments)
                                                    .unwrap_or_default();
                                            let task = args
                                                .get("task")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");

                                            // Emit status: WaitingForSubAgent
                                            let _ = event_tx
                                                .send(EngineEvent::AgentStatusChanged {
                                                    agent_id: agent_id.clone(),
                                                    agent_name: agent_name.clone(),
                                                    status: AgentStatus::WaitingForSubAgent,
                                                })
                                                .await;

                                            match spawn_subagent_and_delegate(SpawnSubagentConfig {
                                                config: config.clone(),
                                                parent_id: agent_id.clone(),
                                                llm_registry: llm_registry.clone(),
                                                task: task.to_string(),
                                                db: db.clone(),
                                                session_id,
                                                event_tx: event_tx.clone(),
                                                usage_tx: usage_tx.clone(),
                                                history_limit_percent,
                                                retry_config: retry_config.clone(),
                                                debug: debug_mode.clone(),
                                                max_steps: config.max_steps,
                                                depth: depth + 1,
                                                skill_registry: skill_registry.clone(),
                                                skill_names: skill_names.clone(),
                                                permissions_override: None,
                                                agent_type: Some(config.name.clone()),
                                                mode: TaskMode::Foreground,
                                            })
                                            .await
                                            {
                                                Ok(outcome) => {
                                                    if outcome.is_out_of_steps() {
                                                        // Subagent ran out of steps — set the
                                                        // shared flag so the tool loop stops.
                                                        subagent_stopped
                                                            .store(true, Ordering::Relaxed);
                                                        let msg = outcome.into_content();
                                                        let _ = event_tx
                                                            .send(EngineEvent::AgentOutput {
                                                                agent_id: agent_id.clone(),
                                                                agent_name: agent_name.clone(),
                                                                content: format!(
                                                                    "[Subagente sin pasos] {msg}"
                                                                ),
                                                            })
                                                            .await;
                                                        format!("[Subagente sin pasos] {msg}")
                                                    } else {
                                                        outcome.into_content()
                                                    }
                                                }
                                                Err(e) => format!("Subagent error: {e}"),
                                            }
                                        } else if let Some((server_name, original_name)) =
                                            mcp_tool_map.get(&tc.function.name)
                                        {
                                            let show_mcp_tool =
                                                should_emit_tool(ts.as_ref(), &tc.function.name);
                                            let task_preview = resolve_tool_preview(
                                                ts.as_ref(),
                                                &tc.function.name,
                                                &tc.function.arguments,
                                            );
                                            if show_mcp_tool {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolExecution {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: tc.function.name.clone(),
                                                        task: task_preview,
                                                    })
                                                    .await;
                                            }
                                            // Execute MCP tool with retries
                                            let args: serde_json::Value =
                                                serde_json::from_str(&tc.function.arguments)
                                                    .unwrap_or_default();
                                            let mcp_result_str = if let Some(mcp_reg) = mcp_registry
                                            {
                                                let mcp_retry_cfg = retry_config.clone();
                                                let mcp_server = server_name.clone();
                                                let mcp_tool = original_name.clone();
                                                let mcp_args = args.clone();
                                                let mcp_clone = mcp_reg.clone();
                                                let mcp_result = retry_with_backoff(
                                                    |_attempt| {
                                                        let srv = mcp_server.clone();
                                                        let t = mcp_tool.clone();
                                                        let a = mcp_args.clone();
                                                        let reg = mcp_clone.clone();
                                                        async move {
                                                            reg.lock()
                                                                .await
                                                                .call_tool(&srv, &t, a)
                                                                .await
                                                        }
                                                    },
                                                    &mcp_retry_cfg,
                                                    &format!(
                                                        "MCP tool '{}_{}'",
                                                        server_name, original_name
                                                    ),
                                                )
                                                .await;
                                                match mcp_result {
                                                    Ok(result) => result,
                                                    Err(e) => {
                                                        format!("MCP tool error after retries: {e}")
                                                    }
                                                }
                                            } else {
                                                format!(
                                                    "MCP registry not available for tool: {}",
                                                    tc.function.name
                                                )
                                            };
                                            // Emit tool result event
                                            let success = !mcp_result_str
                                                .starts_with("MCP tool error")
                                                && !mcp_result_str
                                                    .starts_with("MCP registry not available");
                                            if show_mcp_tool {
                                                let _ = event_tx
                                                    .send(EngineEvent::ToolResult {
                                                        agent_id: agent_id.clone(),
                                                        agent_name: agent_name.clone(),
                                                        tool_name: tc.function.name.clone(),
                                                        success,
                                                        summary: truncate_output(
                                                            &mcp_result_str,
                                                            5000,
                                                        ),
                                                    })
                                                    .await;
                                            }
                                            mcp_result_str
                                        } else if let Some(handler) = plugins
                                            .as_ref()
                                            .and_then(|p| p.custom_tool_handler(&tc.function.name))
                                        {
                                            handler(&tc)
                                        } else {
                                            format!(
                                                "Unknown tool or subagent: {}",
                                                tc.function.name
                                            )
                                        };

                                        // Fire AfterTool hook
                                        {
                                            let ctx = HookContext {
                                                tool_name: Some(tc.function.name.clone()),
                                                agent_name: Some(agent_name.clone()),
                                                ..Default::default()
                                            };
                                            let hook_results =
                                                hook_registry.run(HookPoint::AfterTool, &ctx).await;
                                            for r in &hook_results {
                                                let _ = event_tx
                                                    .send(EngineEvent::HookExecuted {
                                                        point: format!(
                                                            "{:?}",
                                                            HookPoint::AfterTool
                                                        ),
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
                                        }

                                        (tc.id.clone(), result)
                                    }
                                };

                                // Count how many `task` tool calls are in this batch.
                                let task_call_count = tool_calls
                                    .iter()
                                    .filter(|tc| tc.function.name == "task")
                                    .count();

                                if task_call_count > 1 {
                                    // Multiple `task` calls: run ALL tool calls in the batch
                                    // concurrently, then record results in the ORIGINAL order
                                    // to preserve the tool_call_id → result mapping and the
                                    // conversation ordering.
                                    let results = futures::future::join_all(
                                        tool_calls.into_iter().map(&execute_one),
                                    )
                                    .await;
                                    for (tool_call_id, result) in results {
                                        // Store the full tool output for later re-query, and
                                        // pass a truncated version to the LLM to keep the
                                        // context bounded.
                                        tool_store.insert(tool_call_id.clone(), result.clone());
                                        let llm_result =
                                            truncate_output(&result, TOOL_RESULT_MAX_CHARS);

                                        conversation.push(LlmMessage {
                                            role: MessageRole::Tool,
                                            content: llm_result,
                                            tool_calls: None,
                                            tool_call_id: Some(tool_call_id),
                                        });
                                    }
                                    let _ = event_tx
                                        .send(EngineEvent::LocalTokenEstimate {
                                            tokens: conversation_tokens(&conversation),
                                        })
                                        .await;
                                } else {
                                    // Sequential execution (0 or 1 `task` calls): preserve the
                                    // exact original behavior.
                                    for tc in tool_calls {
                                        let (tool_call_id, result) = execute_one(tc).await;

                                        // Store the full tool output for later re-query, and
                                        // pass a truncated version to the LLM to keep the
                                        // context bounded.
                                        tool_store.insert(tool_call_id.clone(), result.clone());
                                        let llm_result =
                                            truncate_output(&result, TOOL_RESULT_MAX_CHARS);

                                        conversation.push(LlmMessage {
                                            role: MessageRole::Tool,
                                            content: llm_result,
                                            tool_calls: None,
                                            tool_call_id: Some(tool_call_id),
                                        });
                                    }
                                    let _ = event_tx
                                        .send(EngineEvent::LocalTokenEstimate {
                                            tokens: conversation_tokens(&conversation),
                                        })
                                        .await;
                                }

                                // If a subagent ran out of steps, stop the tool loop.
                                if subagent_stopped.load(Ordering::Relaxed) {
                                    let _ = event_tx
                                        .send(EngineEvent::AgentStatusChanged {
                                            agent_id: agent_id.clone(),
                                            agent_name: agent_name.clone(),
                                            status: AgentStatus::Idle,
                                        })
                                        .await;
                                    break 'tool_loop;
                                }

                                // Loop back: LLM now has tool results
                            }
                            Err(e) => {
                                let _ = event_tx
                                    .send(EngineEvent::Error {
                                        agent_id: Some(agent_id.clone()),
                                        message: format!("LLM error after retries: {e}"),
                                    })
                                    .await;
                                break 'tool_loop;
                            }
                        }
                    }
                }
                AgentMessage::Cancel => {
                    cancel_flag.store(true, Ordering::Relaxed);
                    // Emit status: Idle so the TUI knows we stopped
                    let _ = event_tx
                        .send(EngineEvent::AgentStatusChanged {
                            agent_id: agent_id.clone(),
                            agent_name: agent_name.clone(),
                            status: AgentStatus::Idle,
                        })
                        .await;
                }
                AgentMessage::Shutdown => break,
                AgentMessage::LoadHistory(history) => {
                    conversation = history;
                }
                AgentMessage::ClearHistory => {
                    conversation.clear();
                }
                AgentMessage::Compact => {
                    // Force compaction of the conversation context, even if
                    // under the token budget (invoked via `/compact`).
                    summarize_conversation(
                        &mut conversation,
                        max_history_tokens,
                        Some(&*provider),
                        &model,
                        true,
                        Some(&tool_store),
                        Some(&retry_config),
                    )
                    .await;
                    let tokens = conversation_tokens(&conversation) as u32;
                    let _ = event_tx
                        .send(EngineEvent::ConversationCompacted {
                            agent_id: agent_id.clone(),
                            agent_name: agent_name.clone(),
                            tokens,
                        })
                        .await;
                    let _ = event_tx
                        .send(EngineEvent::LocalTokenEstimate {
                            tokens: tokens as usize,
                        })
                        .await;
                }
                _ => {
                    // Delegate, Response, System, ToolCall, ToolResult
                    // are no-ops for the initial wiring
                }
            }
        }
    });

    handle
}

/// Extract a human-readable task preview from tool call arguments JSON.
fn extract_task_preview(tool_name: &str, args: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
    let preview: Option<String> = match tool_name {
        "read" => v
            .get("filePath")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "grep" => v
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "glob" => v
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "webfetch" => v.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()),
        "websearch" => v
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "todo" => {
            let action = v.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let content = v.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if content.is_empty() {
                Some(action.to_string())
            } else {
                Some(format!("{action}: {content}"))
            }
        }
        "question" => v
            .get("question")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "apply_patch" => v
            .get("operations")
            .and_then(|v| v.as_array())
            .map(|a| format!("{} operaciones", a.len())),
        "lsp_query" => v
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "task" => v
            .get("task")
            .or_else(|| v.get("description"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    };
    match preview {
        Some(s) if s.len() > 120 => format!("{}…", &s[..120]),
        Some(s) => s,
        None => {
            if args.len() > 120 {
                format!("{}…", &args[..120])
            } else {
                args.to_string()
            }
        }
    }
}

/// Returns all built-in tool definitions keyed by tool name.
/// These are the tools that agents can declare in their `tools:` frontmatter.
pub fn builtin_tool_definitions() -> HashMap<String, ToolDefinition> {
    let mut map = HashMap::new();
    for def in [
        todo_tool_definition(),
        question_tool_definition(),
        apply_patch_tool_definition(),
        read_tool_definition(),
        grep_tool_definition(),
        glob_tool_definition(),
        webfetch_tool_definition(),
        websearch_tool_definition(),
        mcp_list_resources_tool_definition(),
        mcp_read_resource_tool_definition(),
        mcp_list_resource_templates_tool_definition(),
        lsp_query_tool_definition(),
        task_tool_definition(),
    ] {
        map.insert(def.name.clone(), def);
    }
    map
}

/// Check whether tool execution should be shown in the chat based on `show` setting.
fn should_emit_tool(tool_settings: &HashMap<String, ToolSettings>, tool_name: &str) -> bool {
    tool_settings.get(tool_name).is_none_or(|s| s.show)
}

/// Resolve display template from tool settings; fall back to `extract_task_preview`.
fn resolve_tool_preview(
    tool_settings: &HashMap<String, ToolSettings>,
    tool_name: &str,
    args: &str,
) -> String {
    if let Some(template) = tool_settings
        .get(tool_name)
        .and_then(|s| s.display.as_deref())
    {
        let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
        let mut result = template.to_string();
        // Replace {param} placeholders with actual values from args
        if let Some(obj) = v.as_object() {
            for (key, val) in obj {
                if let Some(s) = val.as_str() {
                    result = result.replace(&format!("{{{key}}}"), s);
                }
            }
        }
        result
    } else {
        extract_task_preview(tool_name, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::AgentRole;
    use crate::skill::types::Skill;
    use crate::tools::glob::glob_tool_definition;
    use crate::tools::grep::grep_tool_definition;
    use crate::tools::lsp::lsp_query_tool_definition;
    use crate::tools::mcp::{
        mcp_list_resource_templates_tool_definition, mcp_list_resources_tool_definition,
        mcp_read_resource_tool_definition,
    };
    use crate::tools::read::read_tool_definition;
    use crate::tools::web::{webfetch_tool_definition, websearch_tool_definition};

    #[test]
    fn test_chat_agent_payload_size() {
        // Simulate the 3 skills of the chat agent: weather, web-research, shell
        let weather_skill = Skill {
            name: "weather".into(),
            description: "Consulta meteorológica para cualquier localidad del mundo. Proporciona temperatura actual, sensación térmica, humedad, viento, probabilidad de lluvia, estado del cielo, amanecer/anochecer y previsión por horas/días.".into(),
            instructions: String::new(),
            metadata: Default::default(),
            hooks: Default::default(),
        };
        let web_research_skill = Skill {
            name: "web-research".into(),
            description: "Investiga cualquier tema combinando búsqueda web con SearXNG y fetch de URLs — encuentra fuentes, las analiza y sintetiza un informe estructurado".into(),
            instructions: String::new(),
            metadata: Default::default(),
            hooks: Default::default(),
        };
        let shell_skill = Skill {
            name: "shell".into(),
            description: "Execute shell commands in the workspace environment".into(),
            instructions: String::new(),
            metadata: Default::default(),
            hooks: Default::default(),
        };

        // Build the exact same tool list as spawn_agent() does
        let tools: Vec<ToolDefinition> = vec![
            skill_to_tool_definition(&weather_skill),
            skill_to_tool_definition(&web_research_skill),
            skill_to_tool_definition(&shell_skill),
            todo_tool_definition(),
            question_tool_definition(),
            apply_patch_tool_definition(),
            read_tool_definition(),
            grep_tool_definition(),
            glob_tool_definition(),
            webfetch_tool_definition(),
            websearch_tool_definition(),
            mcp_list_resources_tool_definition(),
            mcp_read_resource_tool_definition(),
            mcp_list_resource_templates_tool_definition(),
            lsp_query_tool_definition(),
            task_tool_definition(),
        ];

        // Construct realistic messages (system prompt + "Hola")
        let messages = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "You are **Chat**, a friendly and helpful conversational agent within the Anacleto orchestration engine. Your purpose is to have natural, engaging conversations with users while being especially good at checking the weather.\n\n## Personality\n\n- **Warm and approachable** — You greet users with enthusiasm and maintain a friendly tone throughout the conversation.\n- **Conversational** — You can chat about almost anything: daily life, tech, recommendations, casual topics. You're like a knowledgeable friend.\n- **Concise but complete** — You give clear, useful answers without being overly verbose unless the user asks for details.\n- **Proactive** — If someone mentions travel, outdoor plans, or events, you offer to check the weather for them.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "> Hola".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];

        // Serialize the full LlmRequest to JSON (this is what gets sent to the API)
        let request = LlmRequest {
            model: "deepseek/deepseek-v4-flash".into(),
            messages,
            tools,
            max_tokens: None,
            temperature: None,
            stream: true,
            cache_control: None,
        };

        let json = serde_json::to_string_pretty(&request).unwrap();
        let char_count = json.len();

        // Estimate tokens: most models use ~4 chars per token for JSON
        let estimated_tokens = char_count / 4;

        println!("========================================");
        println!("CHAT AGENT LLM REQUEST PAYLOAD");
        println!("========================================");
        println!("JSON characters: {}", char_count);
        println!("Estimated tokens (4 chars/token): {}", estimated_tokens);
        println!("Number of tool definitions: {}", request.tools.len());

        // Also measure each component separately
        let tools_json = serde_json::to_string_pretty(&request.tools).unwrap();
        let messages_json = serde_json::to_string_pretty(&request.messages).unwrap();
        println!("Tools section chars: {}", tools_json.len());
        println!("Tools estimated tokens: {}", tools_json.len() / 4);
        println!("Messages section chars: {}", messages_json.len());
        println!("Messages estimated tokens: {}", messages_json.len() / 4);
        println!("========================================");

        // Sanity check: the payload must be non-trivial
        assert!(char_count > 1000, "Payload too small: {} chars", char_count);
    }

    #[test]
    fn test_agent_id_unique() {
        let id1 = crate::agent::types::AgentId::new();
        let id2 = crate::agent::types::AgentId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_agent_role() {
        let config = crate::config::AgentConfig {
            name: "test".into(),
            description: "A test agent".into(),
            when_to_use: String::new(),
            role: AgentRole::SubAgent,
            model: "claude-sonnet-4".into(),
            skills: vec![],
            mcps: vec![],
            permissions: Default::default(),
            subagents: vec![],
            system_prompt: "You are a test agent.".into(),
            max_steps: 60,
            subagent_depth: 3,
            tools: HashMap::new(),
        };
        let root = Agent::from_config(&config, AgentRole::Root);
        assert!(root.is_root());
        assert!(!root.is_subagent());

        let sub = Agent::from_config(&config, AgentRole::SubAgent);
        assert!(!sub.is_root());
        assert!(sub.is_subagent());
    }
}
