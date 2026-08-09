use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::context::summarize_conversation;
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
use crate::config::types::AgentConfig;
use crate::config::types::RetryConfig;
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
    } = config;
    let (tx, mut rx) = mpsc::channel::<AgentMessage>(256);
    let handle = AgentHandle::new(tx);

    let agent_id = agent.id.clone();
    let agent_name = agent.name.clone();
    let model = agent.model.clone();
    let agent_permissions = agent.permissions.clone();
    let max_steps = agent.max_steps;
    let subagent_depth = agent.subagent_depth;

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
    tools.push(todo_tool_definition());
    tools.push(question_tool_definition());
    tools.push(apply_patch_tool_definition());
    tools.push(read_tool_definition());
    tools.push(grep_tool_definition());
    tools.push(glob_tool_definition());
    tools.push(webfetch_tool_definition());
    tools.push(websearch_tool_definition());
    tools.push(mcp_list_resources_tool_definition());
    tools.push(mcp_read_resource_tool_definition());
    tools.push(mcp_list_resource_templates_tool_definition());
    tools.push(lsp_query_tool_definition());
    tools.push(task_tool_definition());

    // Add custom tools registered by plugins.
    if let Some(plugins) = &plugins {
        tools.extend(plugins.custom_tools().iter().cloned());
    }

    // Render the system prompt template (supports {model}, {workspace}, {tools}).
    let system_prompt = {
        let tool_names = tools
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let mut vars = HashMap::new();
        vars.insert("model".to_string(), model.clone());
        vars.insert(
            "workspace".to_string(),
            workspace.to_string_lossy().to_string(),
        );
        vars.insert("tools".to_string(), tool_names);
        render_template(&agent.description, &vars)
    };

    // Let plugins transform the system prompt before it is sent to the model.
    let system_prompt = if let Some(plugins) = &plugins {
        plugins.on_agent_spawn(&agent_name, &system_prompt)
    } else {
        system_prompt
    };

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
                    if let Some(ref enabled_map) = mcp_enabled {
                        if !*enabled_map.lock().await.get(&server_name).unwrap_or(&true) {
                            continue;
                        }
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
                    summarize_conversation(
                        &mut conversation,
                        max_history_tokens,
                        Some(&*provider),
                        &model,
                        false,
                        Some(&tool_store),
                        Some(&retry_config),
                    )
                    .await;

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
                        summarize_conversation(
                            &mut conversation,
                            max_history_tokens,
                            Some(&*provider),
                            &model,
                            false,
                            Some(&tool_store),
                            Some(&retry_config),
                        )
                        .await;

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

                                let execute_one = |tc: ToolCall| async move {
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
                                        let hook_results =
                                            hook_registry.run(HookPoint::BeforeTool, &ctx).await;
                                        for r in &hook_results {
                                            let _ = event_tx
                                                .send(EngineEvent::HookExecuted {
                                                    point: format!("{:?}", HookPoint::BeforeTool),
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
                                        execute_task_tool(
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
                                        .await
                                        .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "todo" {
                                        execute_todo_tool(db, session_id, &tc, event_tx)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "question" {
                                        execute_question_tool(pending_questions, &tc, event_tx)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "apply_patch" {
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
                                                    point: format!("{:?}", HookPoint::BeforeApply),
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
                                                    point: format!("{:?}", HookPoint::AfterApply),
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
                                        apply_result
                                    } else if tc.function.name == "read" {
                                        execute_read_tool(workspace, agent_permissions, &tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "grep" {
                                        execute_grep_tool(workspace, agent_permissions, &tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "glob" {
                                        execute_glob_tool(workspace, agent_permissions, &tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "webfetch" {
                                        execute_webfetch_tool(agent_permissions, &tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "websearch" {
                                        execute_websearch_tool(agent_permissions, &tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "mcp_list_resources" {
                                        match mcp_registry {
                                            Some(reg) => execute_mcp_list_resources_tool(
                                                reg,
                                                agent_permissions,
                                                &tc,
                                            )
                                            .await
                                            .unwrap_or_else(|e| e),
                                            None => {
                                                "MCP registry not available for mcp_list_resources"
                                                    .to_string()
                                            }
                                        }
                                    } else if tc.function.name == "mcp_read_resource" {
                                        match mcp_registry {
                                            Some(reg) => execute_mcp_read_resource_tool(
                                                reg,
                                                agent_permissions,
                                                &tc,
                                            )
                                            .await
                                            .unwrap_or_else(|e| e),
                                            None => {
                                                "MCP registry not available for mcp_read_resource"
                                                    .to_string()
                                            }
                                        }
                                    } else if tc.function.name == "mcp_list_resource_templates" {
                                        match mcp_registry {
                                            Some(reg) => {
                                                execute_mcp_list_resource_templates_tool(
                                                    reg, agent_permissions, &tc,
                                                )
                                                .await
                                                .unwrap_or_else(|e| e)
                                            }
                                            None => {
                                                "MCP registry not available for mcp_list_resource_templates"
                                                    .to_string()
                                            }
                                        }
                                    } else if tc.function.name == "lsp_query" {
                                        execute_lsp_query_tool(agent_permissions, &tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if skill_names.contains(&tc.function.name) {
                                        let reg = skill_registry.read().await;
                                        execute_skill_tool(
                                            &reg,
                                            agent_name,
                                            &tc,
                                            event_tx,
                                            agent_id,
                                            Some(hook_registry),
                                        )
                                        .await
                                        .unwrap_or_else(|e| e)
                                    } else if let Some(config) =
                                        subagent_configs.iter().find(|c| c.name == tc.function.name)
                                    {
                                        // Extract task from tool call arguments
                                        let args: serde_json::Value =
                                            serde_json::from_str(&tc.function.arguments)
                                                .unwrap_or_default();
                                        let task =
                                            args.get("task").and_then(|v| v.as_str()).unwrap_or("");

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
                                            Ok(response) => response,
                                            Err(e) => format!("Subagent error: {e}"),
                                        }
                                    } else if let Some((server_name, original_name)) =
                                        mcp_tool_map.get(&tc.function.name)
                                    {
                                        // Execute MCP tool with retries
                                        let args: serde_json::Value =
                                            serde_json::from_str(&tc.function.arguments)
                                                .unwrap_or_default();
                                        if let Some(mcp_reg) = mcp_registry {
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
                                        }
                                    } else if let Some(handler) = plugins
                                        .as_ref()
                                        .and_then(|p| p.custom_tool_handler(&tc.function.name))
                                    {
                                        handler(&tc)
                                    } else {
                                        format!("Unknown tool or subagent: {}", tc.function.name)
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
                                                    point: format!("{:?}", HookPoint::AfterTool),
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
                    let _ = event_tx
                        .send(EngineEvent::ConversationCompacted {
                            agent_id: agent_id.clone(),
                            agent_name: agent_name.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::AgentRole;

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
            role: AgentRole::SubAgent,
            model: "claude-sonnet-4".into(),
            skills: vec![],
            mcps: vec![],
            permissions: Default::default(),
            subagents: vec![],
            system_prompt: "You are a test agent.".into(),
            max_steps: 60,
            subagent_depth: 3,
        };
        let root = Agent::from_config(&config, AgentRole::Root);
        assert!(root.is_root());
        assert!(!root.is_subagent());

        let sub = Agent::from_config(&config, AgentRole::SubAgent);
        assert!(!sub.is_root());
        assert!(sub.is_subagent());
    }
}
