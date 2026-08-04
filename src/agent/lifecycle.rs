use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::retry::retry_with_backoff;
use crate::agent::source::load_workspace_instructions;
use crate::agent::tool_store::{ToolOutputStore, truncate_output};
use crate::agent::types::{
    Agent, AgentId, AgentMessage, AgentMode, AgentRole, AgentStatus, TaskMode,
};
use crate::config::types::AgentConfig;
use crate::config::types::RetryConfig;
use crate::db::session::Database;
use crate::engine::jobs::JobRegistry;
use crate::engine::orchestrator::{EngineEvent, UsageEvent};
use crate::error::{Error, Result};
use crate::llm::provider::{LlmProvider, LlmProviderRegistry};
use crate::llm::types::{
    LlmMessage, LlmRequest, LlmResponse, LlmStreamChunk, MessageRole, ToolCall, ToolDefinition,
};
use crate::mcp::client::McpRegistry;
use crate::permissions::checker::{
    check_command_run, check_fs_read, check_fs_write, check_mcp_use, check_net_http,
    check_skill_use,
};
use crate::permissions::types::Permissions;
use crate::skill::loader::load_agent_skills;
use crate::skill::types::Skill;
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
type PendingApprovals =
    Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>;
/// Shared state for tracking pending inline questions awaiting a user answer.
type PendingQuestions =
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
    pub skills: Vec<Skill>,
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
}

/// Spawn a new agent task and return a handle to it.
///
/// The spawned task receives messages from its channel, calls the LLM
/// provider with loaded skills as tools, and handles tool call loops.
/// If `subagent_configs` is non-empty, those subagents are exposed as
/// tools the LLM can invoke for task delegation.
pub fn spawn_agent(config: SpawnAgentConfig) -> AgentHandle {
    let SpawnAgentConfig {
        agent,
        provider,
        skills,
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
    } = config;
    let (tx, mut rx) = mpsc::channel::<AgentMessage>(256);
    let handle = AgentHandle::new(tx);

    let agent_id = agent.id.clone();
    let agent_name = agent.name.clone();
    let model = agent.model.clone();
    let agent_permissions = agent.permissions.clone();
    let max_steps = agent.max_steps;
    let subagent_depth = agent.subagent_depth;

    // Load agent description as system prompt
    let system_prompt = agent.description.clone();

    // Build tool list: skills + subagents + built-in todo tool
    let mut tools: Vec<ToolDefinition> = skills.iter().map(skill_to_tool_definition).collect();
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

    // Clone what the task needs
    let agent_mcp_names = agent.mcps.clone();
    let debug_mode = debug;

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
                    )
                    .await;

                    // Inner loop: LLM may call tools, we execute and loop back
                    let mut step_count: u32 = 0;
                    'tool_loop: loop {
                        step_count += 1;
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
                        )
                        .await;

                        let request = LlmRequest {
                            model: model.clone(),
                            messages: conversation.clone(),
                            tools: tools.clone(),
                            max_tokens: None,
                            temperature: None,
                            stream: true,
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
                                    let _ = event_tx
                                        .send(EngineEvent::AgentOutput {
                                            agent_id: agent_id.clone(),
                                            agent_name: agent_name.clone(),
                                            content: full_response,
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

                                // Execute each tool call — try skills first, then subagents, then MCP
                                for tc in &tool_calls {
                                    // Check permissions before executing
                                    let permission_ok = check_tool_permission(
                                        tc,
                                        &agent_permissions,
                                        &pending_approvals,
                                        &event_tx,
                                        &agent_name,
                                    )
                                    .await;

                                    // In Plan mode, block write tools (read-only).
                                    let plan_blocked = plan_mode_blocked(
                                        &mode,
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
                                    } else if tc.function.name == "task" {
                                        execute_task_tool(
                                            tc,
                                            &agent_permissions,
                                            &llm_registry,
                                            &skills,
                                            &event_tx,
                                            &usage_tx,
                                            &db,
                                            session_id,
                                            history_limit_percent,
                                            &retry_config,
                                            &debug_mode,
                                            depth,
                                            subagent_depth,
                                            &agent_name,
                                            &agent_id,
                                            &model,
                                            &job_registry,
                                        )
                                        .await
                                        .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "todo" {
                                        execute_todo_tool(&db, session_id, tc, &event_tx)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "question" {
                                        execute_question_tool(&pending_questions, tc, &event_tx)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "apply_patch" {
                                        execute_apply_patch_tool(
                                            &workspace,
                                            &agent_permissions,
                                            &pending_approvals,
                                            &event_tx,
                                            &agent_name,
                                            tc,
                                        )
                                        .await
                                        .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "read" {
                                        execute_read_tool(&workspace, &agent_permissions, tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "grep" {
                                        execute_grep_tool(&workspace, &agent_permissions, tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "glob" {
                                        execute_glob_tool(&workspace, &agent_permissions, tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "webfetch" {
                                        execute_webfetch_tool(&agent_permissions, tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "websearch" {
                                        execute_websearch_tool(&agent_permissions, tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if tc.function.name == "mcp_list_resources" {
                                        match mcp_registry {
                                            Some(ref reg) => execute_mcp_list_resources_tool(
                                                reg,
                                                &agent_permissions,
                                                tc,
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
                                            Some(ref reg) => execute_mcp_read_resource_tool(
                                                reg,
                                                &agent_permissions,
                                                tc,
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
                                            Some(ref reg) => {
                                                execute_mcp_list_resource_templates_tool(
                                                    reg, &agent_permissions, tc,
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
                                        execute_lsp_query_tool(&agent_permissions, tc)
                                            .await
                                            .unwrap_or_else(|e| e)
                                    } else if let Some(_skill) =
                                        skills.iter().find(|s| s.name == tc.function.name)
                                    {
                                        execute_skill_tool(
                                            &skills,
                                            &agent_name,
                                            tc,
                                            &event_tx,
                                            &agent_id,
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
                                            skills_override: None,
                                            permissions_override: None,
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
                                        if let Some(ref mcp_reg) = mcp_registry {
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
                                    } else {
                                        format!("Unknown tool or subagent: {}", tc.function.name)
                                    };

                                    // Store the full tool output for later
                                    // re-query, and pass a truncated version
                                    // to the LLM to keep the context bounded.
                                    tool_store.insert(tc.id.clone(), result.clone());
                                    let llm_result =
                                        truncate_output(&result, TOOL_RESULT_MAX_CHARS);

                                    conversation.push(LlmMessage {
                                        role: MessageRole::Tool,
                                        content: llm_result,
                                        tool_calls: None,
                                        tool_call_id: Some(tc.id.clone()),
                                    });
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

// ---------------------------------------------------------------------------
// Permission checking with human-in-the-loop approval
// ---------------------------------------------------------------------------

/// Check if a tool call is permitted, requesting human approval if needed.
/// Returns true if the operation should proceed, false if denied.
async fn check_tool_permission(
    tool_call: &ToolCall,
    permissions: &Permissions,
    pending_approvals: &Option<PendingApprovals>,
    event_tx: &mpsc::Sender<EngineEvent>,
    agent_name: &str,
) -> bool {
    // Determine the permission type and operation description
    let (perm_type, operation_desc) = classify_tool_operation(tool_call);

    // Check if explicitly denied
    let denied = match perm_type.as_str() {
        "command.run" => check_command_run(permissions).is_err(),
        "fs.write" => check_fs_write(permissions).is_err(),
        "fs.read" => check_fs_read(permissions).is_err(),
        "net.http" => check_net_http(permissions).is_err(),
        "mcp.use" => check_mcp_use(permissions).is_err(),
        "skill.use" => check_skill_use(permissions).is_err(),
        _ => false, // unknown types are allowed by default
    };

    if denied {
        let _ = event_tx
            .send(EngineEvent::Error {
                agent_id: None,
                message: format!(
                    "Agent '{}' attempted denied operation: {} ({})",
                    agent_name, tool_call.function.name, operation_desc
                ),
            })
            .await;
        return false;
    }

    // Check if operation is sensitive and requires human approval
    if is_sensitive_operation(&perm_type, &operation_desc) {
        if let Some(approvals) = pending_approvals {
            let id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel::<bool>();

            // Store the sender
            {
                let mut map = approvals.lock().await;
                map.insert(id.clone(), tx);
            }

            // Emit approval required event
            let _ = event_tx
                .send(EngineEvent::ApprovalRequired {
                    id: id.clone(),
                    operation: format!(
                        "Agent '{}' wants to run: {} ({})",
                        agent_name, tool_call.function.name, operation_desc
                    ),
                })
                .await;

            // Wait for user response (with timeout to avoid hanging)
            match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
                Ok(Ok(true)) => {
                    let _ = event_tx
                        .send(EngineEvent::AgentMessage {
                            agent_id: AgentId::new(),
                            agent_name: agent_name.to_string(),
                            message: format!("✓ Approved: {}", operation_desc),
                        })
                        .await;
                    true
                }
                Ok(Ok(false)) => {
                    let _ = event_tx
                        .send(EngineEvent::AgentMessage {
                            agent_id: AgentId::new(),
                            agent_name: agent_name.to_string(),
                            message: format!("✗ Denied: {}", operation_desc),
                        })
                        .await;
                    false
                }
                _ => {
                    // Timeout or channel closed — deny
                    let _ = event_tx
                        .send(EngineEvent::Error {
                            agent_id: None,
                            message: format!(
                                "Approval timed out for: {} ({})",
                                tool_call.function.name, operation_desc
                            ),
                        })
                        .await;
                    false
                }
            }
        } else {
            // No approval mechanism available — deny sensitive operations
            false
        }
    } else {
        // Non-sensitive operation, allowed by default
        true
    }
}

/// Classify a tool call into a permission type and human-readable description.
fn classify_tool_operation(tool_call: &ToolCall) -> (String, String) {
    let name = &tool_call.function.name;
    let args: serde_json::Value =
        serde_json::from_str(&tool_call.function.arguments).unwrap_or_default();
    let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");

    match name {
        n if n == "shell" || n.starts_with("shell_") => {
            ("command.run".into(), format!("shell: {}", task))
        }
        n if n == "read" || n == "grep" || n == "glob" => {
            ("fs.read".into(), format!("filesystem: {}", task))
        }
        n if n == "webfetch" || n == "websearch" => {
            ("net.http".into(), format!("network: {}", task))
        }
        n if n.starts_with("mcp_") => ("mcp.use".into(), format!("mcp: {}", task)),
        n if n == "lsp_query" => ("command.run".into(), format!("lsp: {}", task)),
        n if n.contains("write") || n.contains("create") || n.contains("delete") => {
            ("fs.write".into(), format!("filesystem: {}", task))
        }
        n if n.contains("http") || n.contains("fetch") || n.contains("web") => {
            ("net.http".into(), format!("network: {}", task))
        }
        n if n == "filesystem" => {
            // Inspect the operation to classify read/list (safe) vs write/edit/delete.
            match crate::filesystem::parse_request(task) {
                Ok(req) if crate::filesystem::is_write_op(&req.op) => {
                    ("fs.write".into(), format!("filesystem: {}", task))
                }
                _ => ("skill.use".into(), format!("filesystem: {}", task)),
            }
        }
        _ => ("skill.use".into(), format!("{}: {}", name, task)),
    }
}

/// Determine if an operation is sensitive enough to require human approval.
fn is_sensitive_operation(perm_type: &str, operation_desc: &str) -> bool {
    match perm_type {
        // Most shell commands are safe (ls, find, grep, etc.); only
        // flag destructive/privileged ones for human approval.
        "command.run" => {
            let lower = operation_desc.to_lowercase();
            lower.contains("sudo")
                || lower.contains("rm -rf")
                || lower.contains("chmod")
                || lower.contains("chown")
                || lower.contains("mkfs")
                || lower.contains("dd if=")
                || lower.contains(" >/dev/")
                || lower.contains("shutdown")
                || lower.contains("reboot")
                || lower.contains("poweroff")
                || lower.contains("passwd")
        }
        // Filesystem writes to system paths require approval
        "fs.write" => {
            operation_desc.contains("/etc/")
                || operation_desc.contains("/usr/")
                || operation_desc.contains("/boot/")
                || operation_desc.contains("sudo")
                || operation_desc.contains("rm -rf")
                || operation_desc.contains("chmod")
        }
        // Network operations are not sensitive by default
        "net.http" => false,
        // Skill usage is not sensitive
        "skill.use" => false,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Skill-to-tool conversion and execution
// ---------------------------------------------------------------------------

/// Convert a `Skill` to a `ToolDefinition` that can be passed to an LLM.
fn skill_to_tool_definition(skill: &Skill) -> ToolDefinition {
    // All skills take a "task" string describing what to do
    let input_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "task": {
                "type": "string",
                "description": format!(
                    "The specific task to perform using the '{}' skill. Skill instructions: {}",
                    skill.name, skill.description
                )
            }
        },
        "required": ["task"]
    });

    let mut tool = ToolDefinition {
        name: skill.name.clone(),
        description: skill.description.clone(),
        input_schema,
    };

    // For the shell skill, append the tool inventory so the agent knows which
    // modern tools are available without having to ask repeatedly.
    if skill.name.to_lowercase() == "shell" {
        tool.description = format!(
            "{}\n\n{}",
            skill.description,
            crate::shell::inventory().to_prompt()
        );
    }

    // For the filesystem skill, append the JSON task format so the agent knows
    // exactly how to structure its requests.
    if skill.name.to_lowercase() == "filesystem" {
        tool.description = format!("{}\n\n{}", skill.description, FILESYSTEM_TASK_DOC);
    }

    tool
}

/// Documentation of the JSON task format for the `filesystem` skill.
const FILESYSTEM_TASK_DOC: &str = r#"The `task` argument must be a JSON object string describing one of these operations:

- read:   {"op":"read","path":"..."}
- write:  {"op":"write","path":"...","content":"..."}
- edit:   {"op":"edit","path":"...","old":"...","new":"..."}
- list:   {"op":"list","path":"..."}
- delete: {"op":"delete","path":"..."}

Rules:
- Always provide the `task` argument as a JSON object string.
- Use read before edit to confirm the file's current contents.
- edit replaces ALL occurrences of `old` with `new`."#;

/// Built-in `todo` tool definition: lets the model manage a persisted task list.
fn todo_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "todo".to_string(),
        description: "Manage a persistent task list for the current session. \
                       Actions: add (create a task), update (change status/priority/content), \
                       delete (remove a task), list (show all tasks). \
                       Status values: pending, in_progress, completed, cancelled."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "update", "delete", "list"],
                    "description": "The todo operation to perform."
                },
                "content": {
                    "type": "string",
                    "description": "Task text (required for add, optional for update)."
                },
                "id": {
                    "type": "string",
                    "description": "Task id (required for update/delete)."
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "cancelled"],
                    "description": "New status (optional for update)."
                },
                "priority": {
                    "type": "string",
                    "description": "Optional priority label (e.g. high, medium, low)."
                }
            },
            "required": ["action"]
        }),
    }
}

/// Execute a `todo` tool call against the database.
async fn execute_todo_tool(
    db: &Option<crate::db::Database>,
    session_id: Option<Uuid>,
    tool_call: &ToolCall,
    event_tx: &mpsc::Sender<EngineEvent>,
) -> std::result::Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse todo arguments: {e}"))?;
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");

    let Some(db) = db else {
        return Err("No database available for todo tool".to_string());
    };
    let Some(session_id) = session_id else {
        return Err("No active session for todo tool".to_string());
    };

    let result = match action {
        "add" => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "todo add requires 'content'".to_string())?;
            let priority = args.get("priority").and_then(|v| v.as_str());
            let todo = db
                .add_todo(session_id, content, "pending", priority)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("Added todo [{}]: {}", todo.id, todo.content))
        }
        "update" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "todo update requires 'id'".to_string())?;
            let id = Uuid::parse_str(id).map_err(|e| format!("Invalid todo id: {e}"))?;
            let content = args.get("content").and_then(|v| v.as_str());
            let status = args.get("status").and_then(|v| v.as_str());
            let priority = args.get("priority").and_then(|v| v.as_str());
            db.update_todo(id, content, status, priority)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("Updated todo {id}"))
        }
        "delete" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "todo delete requires 'id'".to_string())?;
            let id = Uuid::parse_str(id).map_err(|e| format!("Invalid todo id: {e}"))?;
            db.delete_todo(id).await.map_err(|e| e.to_string())?;
            Ok(format!("Deleted todo {id}"))
        }
        "list" => {
            let todos = db.list_todos(session_id).await.map_err(|e| e.to_string())?;
            if todos.is_empty() {
                Ok("No todos for this session.".to_string())
            } else {
                let mut out = String::from("Todos:\n");
                for t in &todos {
                    let prio = t.priority.as_deref().unwrap_or("-");
                    out.push_str(&format!(
                        "- [{}] ({}) {} — {}\n",
                        t.status, prio, t.id, t.content
                    ));
                }
                Ok(out)
            }
        }
        _ => Err(format!("Unknown todo action: {action}")),
    };

    // Emit the updated todo list so the TUI can refresh its sidebar.
    if let Ok(list) = db.list_todos(session_id).await {
        let _ = event_tx.send(EngineEvent::TodosUpdated(list)).await;
    }

    result
}

/// Tool definition for the inline `question` tool.
fn question_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "question".to_string(),
        description: "Ask the user a structured question mid-turn to resolve ambiguity. \
                       Provide a clear question, an optional list of options, and an optional \
                       recommended default. The user's answer is returned as the tool result."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional multiple-choice options."
                },
                "recommended": {
                    "type": "string",
                    "description": "Optional recommended default answer."
                }
            },
            "required": ["question"]
        }),
    }
}

/// Execute a `question` tool call: register a pending question, emit an event
/// for the TUI to display, and await the user's answer.
async fn execute_question_tool(
    pending_questions: &Option<PendingQuestions>,
    tool_call: &ToolCall,
    event_tx: &mpsc::Sender<EngineEvent>,
) -> std::result::Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse question arguments: {e}"))?;
    let question = args
        .get("question")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "question requires 'question'".to_string())?;
    let options: Vec<String> = args
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let recommended = args
        .get("recommended")
        .and_then(|v| v.as_str())
        .map(String::from);

    let Some(pending) = pending_questions else {
        return Err("No question handler available".to_string());
    };

    let id = Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    pending.lock().await.insert(id.clone(), tx);

    let _ = event_tx
        .send(EngineEvent::Question {
            id: id.clone(),
            question: question.to_string(),
            options,
            recommended,
        })
        .await;

    // Await the user's answer (with a generous timeout so the turn can resume).
    match tokio::time::timeout(std::time::Duration::from_secs(600), rx).await {
        Ok(Ok(answer)) => Ok(format!("User answered: {answer}")),
        Ok(Err(_)) => Err("Question channel closed without an answer".to_string()),
        Err(_) => Err("Question timed out waiting for user answer".to_string()),
    }
}

/// Tool definition for the `apply_patch` tool: applies a batch of file
/// operations (add/update/delete) with a single approval.
fn apply_patch_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "apply_patch".to_string(),
        description: "Apply a batch of file changes (add/update/delete) to the workspace \
                       in one operation. All changes are applied together after a single \
                       approval. Paths are relative to the workspace. Existing files keep \
                       their original encoding (UTF-8 BOM and CRLF line endings)."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "operations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {
                                "type": "string",
                                "enum": ["add", "update", "delete"],
                                "description": "add creates a new file, update replaces an \
                                               existing file's contents, delete removes a file."
                            },
                            "path": {
                                "type": "string",
                                "description": "File path relative to the workspace."
                            },
                            "content": {
                                "type": "string",
                                "description": "File contents (required for add/update)."
                            }
                        },
                        "required": ["op", "path"]
                    }
                }
            },
            "required": ["operations"]
        }),
    }
}

/// Request a single human approval for an entire patch batch.
///
/// Reuses the same `pending_approvals` / `EngineEvent::ApprovalRequired`
/// mechanism as single-tool approvals. Returns `true` if approved.
async fn request_batch_approval(
    pending_approvals: &Option<PendingApprovals>,
    event_tx: &mpsc::Sender<EngineEvent>,
    agent_name: &str,
    operation_desc: String,
) -> bool {
    let Some(approvals) = pending_approvals else {
        return false;
    };

    let id = Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();

    {
        let mut map = approvals.lock().await;
        map.insert(id.clone(), tx);
    }

    let _ = event_tx
        .send(EngineEvent::ApprovalRequired {
            id: id.clone(),
            operation: format!("Agent '{agent_name}' wants to apply patch: {operation_desc}"),
        })
        .await;

    match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
        Ok(Ok(true)) => true,
        Ok(Ok(false)) => false,
        _ => false,
    }
}

/// Execute an `apply_patch` tool call.
///
/// Parses the batch, validates every path (rejecting traversal), requests a
/// single approval for the whole batch, and only then applies the changes.
async fn execute_apply_patch_tool(
    workspace: &Path,
    permissions: &Permissions,
    pending_approvals: &Option<PendingApprovals>,
    event_tx: &mpsc::Sender<EngineEvent>,
    agent_name: &str,
    tool_call: &ToolCall,
) -> std::result::Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse apply_patch arguments: {e}"))?;

    let json = args
        .get("operations")
        .map(|v| v.to_string())
        .unwrap_or_else(|| tool_call.function.arguments.clone());

    let batch = crate::engine::apply_patch::parse_patch_batch(&json)?;

    // Validate every path before requesting approval or touching the filesystem.
    for op in &batch.operations {
        crate::engine::apply_patch::resolve_within_workspace(workspace, &op.path)?;
    }

    // apply_patch only performs filesystem writes.
    check_fs_write(permissions).map_err(|e| format!("Permission denied: {e}"))?;
    // Writes outside the workspace additionally require fs.external.
    let allow_external = crate::permissions::checker::check_fs_external(permissions).is_ok();

    // Build a human-readable summary of the batch for the approval prompt.
    let summary = batch
        .operations
        .iter()
        .map(|op| format!("{:?} {}", op.op, op.path))
        .collect::<Vec<_>>()
        .join(", ");

    // Request ONE approval for the entire batch. If denied, apply nothing.
    if !request_batch_approval(pending_approvals, event_tx, agent_name, summary).await {
        return Err("apply_patch was denied by the user; no changes were applied.".to_string());
    }

    let results = crate::engine::apply_patch::apply_patch_batch(workspace, &batch, allow_external)?;

    // Emit a unified diff for the TUI diff viewer.
    let diff_text = crate::engine::apply_patch::batch_to_unified_diff(&batch);
    let _ = event_tx
        .send(EngineEvent::DiffAvailable {
            text: diff_text,
            title: format!("apply_patch — {}", agent_name),
        })
        .await;

    Ok(results.join("\n"))
}

/// Execute a tool call against a matching skill and return the result as a string.
async fn execute_skill_tool(
    skills: &[Skill],
    agent_name: &str,
    tool_call: &ToolCall,
    event_tx: &mpsc::Sender<EngineEvent>,
    agent_id: &crate::agent::types::AgentId,
) -> std::result::Result<String, String> {
    // Find the skill by name
    let skill = skills
        .iter()
        .find(|s| s.name == tool_call.function.name)
        .ok_or_else(|| {
            format!(
                "Skill '{}' not found in agent's skills",
                tool_call.function.name
            )
        })?;

    // Extract the task argument from the JSON string
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse tool call arguments: {e}"))?;
    let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");

    // Emit tool execution tracing event
    let _ = event_tx
        .send(EngineEvent::ToolExecution {
            agent_id: agent_id.clone(),
            agent_name: agent_name.to_string(),
            tool_name: skill.name.clone(),
            task: task.to_string(),
        })
        .await;

    // Execute the tool and capture result
    let skill_name_lower = skill.name.to_lowercase();
    let result = if skill_name_lower == "shell" {
        let result = execute_shell_command(task).await;
        let prompt = crate::shell::inventory().to_prompt();
        result.map(|r| format!("{prompt}\n\n{r}"))
    } else if skill_name_lower.contains("web") || skill_name_lower.contains("research") {
        execute_web_fetch(task).await
    } else if skill_name_lower == "filesystem" {
        execute_filesystem_operation(task).await
    } else {
        Ok(format!(
            r#"Executed skill "{}". Here are the skill instructions:

{}

The task requested was: {}"#,
            skill.name, skill.instructions, task
        ))
    };

    // Emit tool result tracing event
    let summary = match &result {
        Ok(r) => truncate_output(r, 120),
        Err(e) => e.clone(),
    };
    let _ = event_tx
        .send(EngineEvent::ToolResult {
            agent_id: agent_id.clone(),
            agent_name: agent_name.to_string(),
            tool_name: skill.name.clone(),
            success: result.is_ok(),
            summary,
        })
        .await;

    result
}

/// Execute a shell command via `sh -c` with a 60-second timeout.
///
/// The `task` is a natural-language description from the LLM (e.g.
/// "Run the tests: cargo test"). We extract the actual command(s) from it so
/// that prose containing apostrophes (e.g. "we're") is never passed verbatim
/// to `sh -c`, which would otherwise fail with an "unexpected EOF" error.
async fn execute_shell_command(task: &str) -> std::result::Result<String, String> {
    let command = extract_shell_command(task);
    if command.is_empty() {
        return Err(
            "No shell command found in the task. Please provide a shell command to run.".into(),
        );
    }

    let shell = crate::shell::inventory().shell.path.clone();
    let child = tokio::process::Command::new(&shell)
        .arg("-c")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn shell command: {e}"))?;

    let output = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| "Command timed out after 60 seconds".to_string())?
        .map_err(|e| format!("Failed to wait for command: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    // grep (and similar search tools) exit with code 1 when no lines match.
    // That is a legitimate "no results" outcome, not a failure.
    let is_search =
        command.contains("grep") || command.contains("rg ") || command.contains("find ");
    if output.status.code() == Some(1) && is_search {
        return Ok("No results found.".to_string());
    }

    if output.status.success() {
        let mut result = String::new();
        if !stdout.is_empty() {
            result.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&stderr);
        }
        if result.is_empty() {
            result.push_str("Command completed successfully (no output).");
        }
        // Ensure trailing newline for clean display
        if !result.ends_with('\n') {
            result.push('\n');
        }
        Ok(result)
    } else {
        let msg = if !stderr.is_empty() {
            stderr.trim().to_string()
        } else if !stdout.is_empty() {
            stdout.trim().to_string()
        } else {
            format!("Command failed with exit code: {:?}", output.status.code())
        };
        Err(msg)
    }
}

/// Extract the actual shell command(s) from a natural-language task string.
///
/// The LLM typically produces a task like "Run the tests: cargo test" or a
/// multi-line description followed by indented commands. We keep the command
/// part (after a colon, or lines that look like shell commands) and drop the
/// surrounding prose, so natural language is never executed by the shell.
fn extract_shell_command(task: &str) -> String {
    let mut commands: Vec<String> = Vec::new();
    for raw_line in task.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // "description: command" -> take the part after the last colon.
        let candidate = if let Some(idx) = line.rfind(':') {
            let after = line[idx + 1..].trim();
            if after.is_empty() { line } else { after }
        } else {
            line
        };
        if looks_like_shell_command(candidate) {
            commands.push(candidate.to_string());
        }
    }
    commands.join("\n")
}

/// Heuristic: does this line look like a shell command rather than prose?
fn looks_like_shell_command(line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() {
        return false;
    }
    // Shell operators strongly indicate a command.
    if line.contains('|')
        || line.contains('>')
        || line.contains('<')
        || line.contains("&&")
        || line.contains(';')
    {
        return true;
    }
    // Starts with a common command word.
    const COMMANDS: &[&str] = &[
        "ls",
        "cd",
        "pwd",
        "find",
        "grep",
        "cat",
        "echo",
        "cargo",
        "git",
        "npm",
        "make",
        "python",
        "python3",
        "node",
        "mkdir",
        "rm",
        "cp",
        "mv",
        "touch",
        "head",
        "tail",
        "wc",
        "sort",
        "uniq",
        "awk",
        "sed",
        "curl",
        "wget",
        "tar",
        "chmod",
        "chown",
        "source",
        "export",
        "exit",
        "sh",
        "bash",
        "test",
        "true",
        "false",
        "env",
        "which",
        "type",
        "file",
        "stat",
        "du",
        "df",
        "ps",
        "kill",
        "xargs",
        "tee",
        "printf",
        "read",
        "sleep",
        "time",
        "date",
        "whoami",
        "id",
        // Modern Rust CLI tools (replacements for legacy Unix commands)
        "rg",
        "fd",
        "bat",
        "eza",
        "lsd",
        "procs",
        "sd",
        "duf",
        "dust",
        "jq",
        "yq",
        "fzf",
        "hyperfine",
        "watchexec",
        "tldr",
        "zoxide",
        "delta",
        "tokei",
        "broot",
        "gitui",
        "ripgrep",
        "dog",
        "xh",
        "choose",
        "pastel",
        "bandwhich",
        "bottom",
        "btm",
        "gping",
        "hexyl",
        "cargo-binstall",
        "cargo-update",
        "cargo-audit",
        "cargo-expand",
        "cargo-fuzz",
    ];
    let first = line.split_whitespace().next().unwrap_or("");
    COMMANDS.contains(&first)
}

/// Fetch a URL and return the response body text with a 30-second timeout.
async fn execute_web_fetch(task: &str) -> std::result::Result<String, String> {
    let trimmed = task.trim();

    // Find a URL in the task string (prefer https://, fall back to http://)
    let url = if let Some(start) = trimmed.find("https://") {
        let rest = &trimmed[start..];
        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        rest[..end].to_string()
    } else if let Some(start) = trimmed.find("http://") {
        let rest = &trimmed[start..];
        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        rest[..end].to_string()
    } else {
        return Err(
            "No URL found in the task. Please provide a valid URL (starting with http:// or https://) to fetch."
                .into(),
        );
    };

    let response = tokio::time::timeout(std::time::Duration::from_secs(30), reqwest::get(&url))
        .await
        .map_err(|_| format!("Request timed out after 30 seconds fetching: {url}"))?
        .map_err(|e| format!("HTTP request failed for {url}: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {} fetching {url}", status.as_u16()));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    let mut result = format!("Content from {url}:\n\n");
    result.push_str(&truncate_output(&body, 10_000));

    Ok(result)
}

/// Execute a filesystem operation by parsing the JSON task and running it.
async fn execute_filesystem_operation(task: &str) -> std::result::Result<String, String> {
    let req = crate::filesystem::parse_request(task)?;
    crate::filesystem::execute(req).await
}

/// Convert an `AgentConfig` to a `ToolDefinition` so the LLM can invoke a subagent.
fn subagent_config_to_tool_definition(config: &AgentConfig) -> ToolDefinition {
    let input_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "task": {
                "type": "string",
                "description": format!(
                    "The task to delegate to the '{}' subagent",
                    config.name
                )
            }
        },
        "required": ["task"]
    });

    ToolDefinition {
        name: config.name.clone(),
        description: format!(
            "Delegate a task to the '{}' subagent for specialized work",
            config.name
        ),
        input_schema,
    }
}

/// Parsed arguments for the dynamic `task` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskToolArgs {
    task_id: String,
    description: String,
    mode: TaskMode,
    model: Option<String>,
    tools: Vec<String>,
}

impl TaskToolArgs {
    /// Parse the `task` tool arguments from the LLM's JSON string.
    fn parse(arguments: &str) -> std::result::Result<Self, String> {
        let args: serde_json::Value = serde_json::from_str(arguments)
            .map_err(|e| format!("Failed to parse task arguments: {e}"))?;
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "task requires 'description'".to_string())?
            .to_string();
        let mode = match args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("foreground")
        {
            "background" => TaskMode::Background,
            _ => TaskMode::Foreground,
        };
        let model = args.get("model").and_then(|v| v.as_str()).map(String::from);
        let tools = args
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            task_id,
            description,
            mode,
            model,
            tools,
        })
    }
}

/// Tool definition for the dynamic `task` tool.
///
/// SEMANTICS of the `tools` argument: it is an allow-list of *skill* names
/// granted to the spawned subagent. It does NOT restrict the subagent's
/// permissions — the subagent's effective permissions are always
/// `parent ∩ child` (the parent's permissions intersected with the child's
/// own). To restrict what a subagent may do, configure the parent's
/// permissions (deny rules propagate down to children).
fn task_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "task".to_string(),
        description: "Dynamically delegate a task to a fresh subagent. \
                       Provide a task_id, a description of the work, and a mode \
                       ('foreground' to wait for the result, 'background' to run \
                       asynchronously and return immediately). Optionally specify \
                       a model and a list of tool/skill names to grant. \
                       NOTE: the 'tools' list only filters which skills the \
                       subagent may use; it does not restrict permissions (the \
                       subagent inherits the parent's permissions)."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "A unique identifier for this task."
                },
                "description": {
                    "type": "string",
                    "description": "The task to delegate to the subagent."
                },
                "mode": {
                    "type": "string",
                    "enum": ["foreground", "background"],
                    "description": "foreground waits for the result; background returns immediately."
                },
                "model": {
                    "type": "string",
                    "description": "Optional model for the subagent."
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of tool/skill names to grant the subagent."
                }
            },
            "required": ["task_id", "description"]
        }),
    }
}

/// In Plan (read-only) mode, block write tools and return an error message.
/// Returns `None` when the tool is not a write operation or mode is Build.
fn plan_mode_blocked(mode: &AgentMode, tool_name: &str, arguments: &str) -> Option<String> {
    if *mode != AgentMode::Plan {
        return None;
    }
    let is_write = match tool_name {
        "apply_patch" => true,
        "filesystem" => {
            let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
            let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
            crate::filesystem::parse_request(task)
                .map(|r| crate::filesystem::is_write_op(&r.op))
                .unwrap_or(false)
        }
        n if n.contains("write")
            || n.contains("create")
            || n.contains("delete")
            || n.contains("edit") =>
        {
            true
        }
        _ => false,
    };
    if is_write {
        Some(format!(
            "read-only plan mode: tool '{tool_name}' is disabled"
        ))
    } else {
        None
    }
}

/// Execute a dynamic `task` tool call: delegate to a fresh subagent.
///
/// In `Foreground` mode the subagent is spawned and awaited inline. In
/// `Background` mode the subagent runs in its own tokio task, is registered
/// in the shared `JobRegistry`, and emits `EngineEvent::SubagentFinished`
/// when it completes.
#[allow(clippy::too_many_arguments)]
async fn execute_task_tool(
    tool_call: &ToolCall,
    parent_permissions: &Permissions,
    llm_registry: &LlmProviderRegistry,
    parent_skills: &[Skill],
    event_tx: &mpsc::Sender<EngineEvent>,
    usage_tx: &Option<mpsc::Sender<UsageEvent>>,
    db: &Option<Database>,
    session_id: Option<Uuid>,
    history_limit_percent: f64,
    retry_config: &RetryConfig,
    debug: &Arc<AtomicBool>,
    depth: u32,
    subagent_depth: u32,
    parent_name: &str,
    parent_id: &AgentId,
    parent_model: &str,
    job_registry: &Option<Arc<tokio::sync::Mutex<JobRegistry>>>,
) -> std::result::Result<String, String> {
    let args = TaskToolArgs::parse(&tool_call.function.arguments)?;

    // Enforce the delegation depth limit.
    if depth >= subagent_depth {
        return Err(format!(
            "subagent depth limit reached ({depth} >= {subagent_depth}): cannot delegate further"
        ));
    }

    // Derive the child's permissions: parent ∩ child (deny propagates down).
    //
    // NOTE: The `tools` argument does NOT restrict the child's permissions.
    // The child's effective permissions are always `parent ∩ child` — the
    // parent's permissions intersected with the child's own (here the default,
    // i.e. allow-by-default). The `tools` list only filters which *skills* the
    // child is granted (see `skills_override` below); it is a capability
    // allow-list for skills, not a permission restriction. Reimplementing
    // per-tool permission filtering is intentionally out of scope here.
    let child_permissions = parent_permissions.intersection(&Permissions::default());

    // Filter the parent's skills by the requested tool names (or grant all).
    // This is the ONLY effect of the `tools` list: it narrows the set of
    // skills the child can invoke. It does not alter the child's permissions.
    let skills_override: Vec<Skill> = if args.tools.is_empty() {
        parent_skills.to_vec()
    } else {
        parent_skills
            .iter()
            .filter(|s| args.tools.iter().any(|t| t == &s.name))
            .cloned()
            .collect()
    };

    let model = args
        .model
        .clone()
        .unwrap_or_else(|| parent_model.to_string());

    let config = AgentConfig {
        name: args.task_id.clone(),
        description: args.description.clone(),
        role: AgentRole::SubAgent,
        model,
        skills: Vec::new(), // skills provided via `skills_override`
        mcps: Vec::new(),
        permissions: crate::config::PermissionConfig::default(),
        subagents: Vec::new(),
        system_prompt: args.description.clone(),
        max_steps: 90,
        subagent_depth,
    };

    let sub_cfg = SpawnSubagentConfig {
        config,
        parent_id: parent_id.clone(),
        llm_registry: llm_registry.clone(),
        task: args.description.clone(),
        db: db.clone(),
        session_id,
        event_tx: event_tx.clone(),
        usage_tx: usage_tx.clone(),
        history_limit_percent,
        retry_config: retry_config.clone(),
        debug: debug.clone(),
        max_steps: 90,
        depth: depth + 1,
        skills_override: Some(skills_override),
        permissions_override: Some(child_permissions),
    };

    match args.mode {
        TaskMode::Foreground => {
            let _ = event_tx
                .send(EngineEvent::AgentStatusChanged {
                    agent_id: parent_id.clone(),
                    agent_name: parent_name.to_string(),
                    status: AgentStatus::WaitingForSubAgent,
                })
                .await;
            let result = spawn_subagent_and_delegate(sub_cfg).await;
            let _ = event_tx
                .send(EngineEvent::AgentStatusChanged {
                    agent_id: parent_id.clone(),
                    agent_name: parent_name.to_string(),
                    status: AgentStatus::Working,
                })
                .await;
            match result {
                Ok(r) => Ok(r),
                Err(e) => Err(format!("Subagent error: {e}")),
            }
        }
        TaskMode::Background => {
            let task_id = args.task_id.clone();
            let task_id_for_register = task_id.clone();
            let job_registry_owned = job_registry.clone();
            let event_tx_owned = event_tx.clone();
            let handle = tokio::spawn(async move {
                let summary = match spawn_subagent_and_delegate(sub_cfg).await {
                    Ok(r) => r,
                    Err(e) => format!("Subagent error: {e}"),
                };
                let _ = event_tx_owned
                    .send(EngineEvent::SubagentFinished {
                        task_id: task_id.clone(),
                        summary,
                    })
                    .await;
                if let Some(reg) = job_registry_owned {
                    reg.lock().await.remove(&task_id);
                }
            });
            if let Some(reg) = job_registry {
                reg.lock()
                    .await
                    .register(task_id_for_register.clone(), handle);
            }
            // NOTE: When `job_registry` is `None` (e.g. headless tests or a
            // caller that did not provide a registry), the `JoinHandle` is
            // dropped here and the background task is detached: it keeps
            // running and emits `SubagentFinished`, but it is not tracked by
            // any `JobRegistry` and therefore never appears in `/jobs`. In the
            // engine this is always `Some`, so background jobs are tracked in
            // practice; the `None` case is only a documented fallback.
            Ok(format!(
                "Background task '{task_id_for_register}' launched. It will run asynchronously."
            ))
        }
    }
}

/// Resolve an LLM provider for a given model name using the registry.
fn resolve_provider_for_model(
    model: &str,
    registry: &LlmProviderRegistry,
) -> std::result::Result<Arc<dyn LlmProvider>, String> {
    let provider_name = if model.contains('/') {
        "openrouter"
    } else if model.starts_with("claude") {
        "anthropic"
    } else if model.starts_with("gpt") || model.starts_with("o1") || model.starts_with("o3") {
        "openai"
    } else {
        "ollama"
    };

    registry.get(provider_name).ok_or_else(|| {
        format!("No provider configured for model '{model}'. Tried provider: {provider_name}")
    })
}

/// Configuration for spawning a subagent on-demand.
pub struct SpawnSubagentConfig {
    pub config: AgentConfig,
    pub parent_id: AgentId,
    pub llm_registry: LlmProviderRegistry,
    pub task: String,
    pub db: Option<Database>,
    pub session_id: Option<Uuid>,
    pub event_tx: mpsc::Sender<EngineEvent>,
    pub usage_tx: Option<mpsc::Sender<UsageEvent>>,
    pub history_limit_percent: f64,
    pub retry_config: RetryConfig,
    pub debug: Arc<AtomicBool>,
    pub max_steps: u32,
    /// Delegation depth of the spawned subagent (parent depth + 1).
    pub depth: u32,
    /// Pre-loaded skills to grant the subagent. If `None`, skills are loaded
    /// from the config's skill paths.
    pub skills_override: Option<Vec<Skill>>,
    /// Effective permissions for the subagent (parent ∩ child). If `Some`,
    /// overrides the permissions derived from `config.permissions`.
    pub permissions_override: Option<Permissions>,
}

/// Spawn a subagent on-demand, delegate a task to it, and wait for the response.
///
/// The subagent is created fresh with its own LLM provider and skills.
/// It processes the task using its own model (which may differ from the parent's)
/// and returns the text response. The subagent task is destroyed after responding.
async fn spawn_subagent_and_delegate(cfg: SpawnSubagentConfig) -> Result<String> {
    let SpawnSubagentConfig {
        config,
        parent_id,
        llm_registry,
        task,
        db: _db,
        session_id: _session_id,
        event_tx,
        usage_tx,
        history_limit_percent,
        retry_config,
        debug,
        max_steps,
        depth: _depth,
        skills_override,
        permissions_override,
    } = cfg;
    // Create agent with SubAgent role
    let mut agent = Agent::from_config(&config, AgentRole::SubAgent);
    if let Some(perms) = permissions_override {
        agent.permissions = perms;
    }
    let agent_id = agent.id.clone();
    let agent_name = agent.name.clone();
    let model = agent.model.clone();

    // Emit subagent created event
    let _ = event_tx
        .send(EngineEvent::SubagentCreated {
            parent_id: parent_id.clone(),
            subagent_id: agent_id.clone(),
            subagent_name: agent_name.clone(),
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
        .await;

    // Resolve provider
    let provider = resolve_provider_for_model(&model, &llm_registry).map_err(Error::Provider)?;

    // Load subagent's own skills (or use pre-loaded skills from `skills_override`).
    let skills = match skills_override {
        Some(s) => s,
        None => load_agent_skills(&agent.skills),
    };
    let subagent_tools: Vec<ToolDefinition> = skills.iter().map(skill_to_tool_definition).collect();

    // Load subagent description as system prompt
    let system_prompt = agent.description.clone();

    // Owned copies for the spawned task
    let task_owned = task.to_string();

    // Create oneshot channel for the response
    let (response_tx, response_rx) = tokio::sync::oneshot::channel::<String>();

    // Spawn subagent task
    let subagent_history_limit = history_limit_percent;
    let subagent_retry_config = retry_config.clone();
    let debug_mode = debug;
    tokio::spawn(async move {
        let mut conversation: Vec<LlmMessage> = Vec::new();

        // Context window budget for subagent
        let max_history_tokens =
            (provider.context_window() as f64 * subagent_history_limit / 100.0) as usize;

        // Prepend system prompt if available
        if !system_prompt.is_empty() {
            conversation.push(LlmMessage {
                role: MessageRole::System,
                content: system_prompt.clone(),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add the user task
        conversation.push(LlmMessage {
            role: MessageRole::User,
            content: task_owned,
            tool_calls: None,
            tool_call_id: None,
        });

        // Tool loop for subagent (skills only, no nested subagents)
        let mut step_count: u32 = 0;
        'tool_loop: loop {
            step_count += 1;
            if step_count > max_steps {
                let _ = response_tx.send(format!(
                    "[Tarea incompleta] Se alcanzó el límite de {max_steps} pasos (turnos) sin completar la tarea."
                ));
                break 'tool_loop;
            }
            // Trim conversation to fit context window
            summarize_conversation(
                &mut conversation,
                max_history_tokens,
                Some(&*provider),
                &model,
                false,
                None, // subagents have no access to the parent's tool store
            )
            .await;

            let request = LlmRequest {
                model: model.clone(),
                messages: conversation.clone(),
                tools: subagent_tools.clone(),
                max_tokens: None,
                temperature: None,
                stream: false, // non-streaming for subagents
            };

            // Emit debug event for subagent LLM request if debug mode is on
            if debug_mode.load(Ordering::Relaxed) {
                let payload =
                    serde_json::to_string_pretty(&request).unwrap_or_else(|_| "{}".to_string());
                let _ = event_tx
                    .send(EngineEvent::LlmRequestDebug {
                        agent_name: agent_name.clone(),
                        model: model.clone(),
                        payload,
                    })
                    .await;
            }

            // Wrap subagent LLM call with retries
            let sub_retry_cfg = subagent_retry_config.clone();
            let sub_agent_name = agent_name.clone();
            let complete_result = retry_with_backoff(
                |_attempt| {
                    let req = request.clone();
                    let prov = provider.clone();
                    async move { prov.complete(req).await }
                },
                &sub_retry_cfg,
                &format!("Subagent LLM call for '{}'", sub_agent_name),
            )
            .await;

            match complete_result {
                Ok(response) => {
                    // Emit token usage if available
                    if let Some(ref usage) = response.usage {
                        let cost = (usage.prompt_tokens as f64
                            * provider.input_price_per_million()
                            + usage.completion_tokens as f64 * provider.output_price_per_million())
                            / 1_000_000.0;
                        let _ = event_tx
                            .send(EngineEvent::TokenUsage {
                                agent_id: agent_id.clone(),
                                agent_name: agent_name.clone(),
                                total_tokens: usage.total_tokens,
                                context_window: provider.context_window() as u32,
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
                    }

                    // Emit debug event for subagent LLM response
                    if debug_mode.load(Ordering::Relaxed) {
                        let resp = LlmResponse {
                            content: response.content.clone(),
                            tool_calls: response.tool_calls.clone(),
                            finish_reason: if response.tool_calls.is_empty() {
                                "stop".to_string()
                            } else {
                                "tool_use".to_string()
                            },
                            usage: response.usage,
                        };
                        let payload = serde_json::to_string_pretty(&resp)
                            .unwrap_or_else(|_| "{}".to_string());
                        let _ = event_tx
                            .send(EngineEvent::LlmResponseDebug {
                                agent_name: agent_name.clone(),
                                model: model.clone(),
                                payload,
                            })
                            .await;
                    }

                    if response.tool_calls.is_empty() {
                        // No tool calls — final response
                        let _ = response_tx.send(response.content);
                        break 'tool_loop;
                    }

                    // Save assistant message with tool calls
                    conversation.push(LlmMessage {
                        role: MessageRole::Assistant,
                        content: response.content,
                        tool_calls: Some(response.tool_calls.clone()),
                        tool_call_id: None,
                    });

                    // Execute each tool call
                    for tc in &response.tool_calls {
                        let result =
                            execute_skill_tool(&skills, &agent_name, tc, &event_tx, &agent_id)
                                .await
                                .unwrap_or_else(|e| e);

                        conversation.push(LlmMessage {
                            role: MessageRole::Tool,
                            content: result,
                            tool_calls: None,
                            tool_call_id: Some(tc.id.clone()),
                        });
                    }
                    // Loop back: LLM now has tool results
                }
                Err(e) => {
                    let _ = response_tx.send(format!("Subagent error: {e}"));
                    break 'tool_loop;
                }
            }
        }

        // Emit subagent completed event
        let _ = event_tx
            .send(EngineEvent::SubagentCompleted {
                subagent_id: agent_id,
                subagent_name: agent_name,
                result: "completed".into(),
            })
            .await;
    });

    // Wait for the response
    let response = response_rx
        .await
        .map_err(|_| Error::Agent("Subagent response channel closed".into()))?;

    Ok(response)
}

/// Roughly estimate the number of tokens in a text string.
/// Uses the rule of thumb: ~4 characters per token for English text.
fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.len().div_ceil(4)
    }
}

/// Fraction of the token budget at which compaction is triggered automatically.
///
/// When the conversation's estimated token count exceeds
/// `max_tokens * COMPACTION_THRESHOLD_RATIO`, compaction runs even without an
/// explicit `/compact` command.
pub const COMPACTION_THRESHOLD_RATIO: f64 = 0.8;

/// Whether the conversation should be compacted based on the token threshold.
///
/// Returns `true` when `total_tokens` exceeds `max_tokens * ratio`. A `ratio`
/// of `1.0` (or greater) effectively disables threshold-based compaction.
pub fn should_compact(total_tokens: usize, max_tokens: usize, ratio: f64) -> bool {
    if max_tokens == 0 {
        return false;
    }
    let threshold = (max_tokens as f64 * ratio) as usize;
    total_tokens > threshold
}

/// The anchored marker prepended to a structured summary injected as a System
/// message after compaction.
pub const SUMMARY_ANCHOR_MARKER: &str = "[Resumen anclado de conversación anterior]";

/// The structured summary template used to guide the LLM during compaction.
///
/// The LLM is asked to fill only these sections, in order, producing an
/// anchored, structured summary rather than a free-form blob of text.
pub const SUMMARY_TEMPLATE: &str = "\
Resumen estructurado de la conversación. Rellena SOLO estas secciones, en este orden, sin añadir nada más:
## Objetivo
## Decisiones tomadas
## Hechos y contexto clave
## Código/patrones relevantes
## Tareas pendientes
## Riesgos o bloqueos";

/// Build the summarization prompt for a set of messages to summarize.
///
/// The prompt instructs the LLM to produce a structured, anchored summary
/// following [`SUMMARY_TEMPLATE`], preserving key facts, decisions, code
/// patterns and context.
///
/// When `tool_store` is provided, Tool messages whose `tool_call_id` is present
/// in the store use the FULL tool output (retained by the [`ToolOutputStore`])
/// instead of the truncated version the LLM originally received, so the summary
/// is built from complete information rather than the shortened context.
fn build_summary_prompt(messages: &[LlmMessage], tool_store: Option<&ToolOutputStore>) -> String {
    let summary_text: String = messages
        .iter()
        .map(|m| {
            let role_label = match m.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::Tool => "Tool",
                MessageRole::System => "System",
            };
            // For Tool messages, prefer the full output retained in the store
            // over the truncated version the LLM originally received.
            let content = if m.role == MessageRole::Tool {
                if let (Some(id), Some(store)) = (&m.tool_call_id, tool_store) {
                    store.get(id).map(|s| s.as_str()).unwrap_or(&m.content)
                } else {
                    &m.content
                }
            } else {
                &m.content
            };
            format!("{}: {}", role_label, content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!("{SUMMARY_TEMPLATE}\n\nConversación a resumir:\n\n{summary_text}")
}

/// Trim conversation history to fit within a token budget.
///
/// Always preserves:
/// - System messages (role == System)
/// - The most recent non-system messages (at least the last exchange)
///
/// Removes oldest non-system messages first when over budget.
fn trim_conversation(messages: &mut Vec<LlmMessage>, max_tokens: usize) {
    if messages.is_empty() || max_tokens == 0 {
        return;
    }

    // Quick check: if already under budget, do nothing
    let total: usize = messages.iter().map(|m| estimate_tokens(&m.content)).sum();
    if total <= max_tokens {
        return;
    }

    // Separate system messages from the rest
    let mut system_msgs: Vec<LlmMessage> = Vec::new();
    let mut non_system_msgs: Vec<LlmMessage> = Vec::new();

    for msg in messages.drain(..) {
        if msg.role == MessageRole::System {
            system_msgs.push(msg);
        } else {
            non_system_msgs.push(msg);
        }
    }

    // Remove oldest non-system messages until under budget
    // Always keep at least the last 2 messages (one exchange)
    while non_system_msgs.len() > 2 {
        // Build a temporary reference list to check token count
        let test_total: usize = system_msgs
            .iter()
            .chain(non_system_msgs.iter())
            .map(|m| estimate_tokens(&m.content))
            .sum();

        if test_total <= max_tokens {
            break;
        }
        non_system_msgs.remove(0);
    }

    // Restore: system first, then remaining non-system
    *messages = system_msgs;
    messages.extend(non_system_msgs);
}

/// Try to summarize old conversation messages using the LLM.
///
/// When the conversation exceeds the token budget, this function attempts to
/// summarize older messages (keeping the latest exchange intact) by calling
/// the LLM with a summarization prompt. The summary is injected as a System
/// message. Falls back to `trim_conversation` if the LLM call fails or if
/// there aren't enough messages to make summarization worthwhile.
///
/// When `tool_store` is provided, Tool messages being summarized use the full
/// output retained in the store (see [`build_summary_prompt`]).
///
/// When `force` is `true`, the summarization is attempted even if the
/// conversation is under the token budget (used by the `/compact` command).
/// The rest of the logic (needs a provider, needs at least 4 non-system
/// messages, preserves system + latest exchange) is still respected.
async fn summarize_conversation(
    conversation: &mut Vec<LlmMessage>,
    max_tokens: usize,
    provider: Option<&dyn LlmProvider>,
    model: &str,
    force: bool,
    tool_store: Option<&ToolOutputStore>,
) {
    // Quick check: if already under the compaction threshold, do nothing
    // (unless forced). The threshold is `max_tokens * COMPACTION_THRESHOLD_RATIO`.
    let total: usize = conversation
        .iter()
        .map(|m| estimate_tokens(&m.content))
        .sum();
    if !force && !should_compact(total, max_tokens, COMPACTION_THRESHOLD_RATIO) {
        return;
    }

    // Need a provider for summarization
    let Some(prov) = provider else {
        trim_conversation(conversation, max_tokens);
        return;
    };

    // Separate system from non-system
    let mut system_msgs: Vec<LlmMessage> = Vec::new();
    let mut non_system_msgs: Vec<LlmMessage> = Vec::new();

    for msg in conversation.drain(..) {
        if msg.role == MessageRole::System {
            system_msgs.push(msg);
        } else {
            non_system_msgs.push(msg);
        }
    }

    // Need at least 4 non-system messages to make summarization worthwhile
    if non_system_msgs.len() < 4 {
        system_msgs.extend(non_system_msgs);
        *conversation = system_msgs;
        trim_conversation(conversation, max_tokens);
        return;
    }

    // Keep the last 2 messages (latest exchange), summarize the rest
    let keep = 2;
    let split_at = non_system_msgs.len() - keep;
    let to_summarize: Vec<LlmMessage> = non_system_msgs.drain(..split_at).collect();
    let recent = non_system_msgs;

    // Build summarization prompt using the structured, anchored template.
    let summary_prompt = build_summary_prompt(&to_summarize, tool_store);

    // Call LLM for summarization
    let request = LlmRequest {
        model: model.to_string(),
        messages: vec![LlmMessage {
            role: MessageRole::User,
            content: summary_prompt,
            tool_calls: None,
            tool_call_id: None,
        }],
        tools: vec![],
        max_tokens: Some(1024),
        temperature: Some(0.3),
        stream: false,
    };

    match prov.complete(request).await {
        Ok(response) => {
            // Replace summarized messages with an anchored summary system message
            system_msgs.push(LlmMessage {
                role: MessageRole::System,
                content: format!("{SUMMARY_ANCHOR_MARKER}\n{}", response.content.trim()),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        Err(_) => {
            // Fallback: restore and truncate
            system_msgs.extend(to_summarize);
            system_msgs.extend(recent);
            *conversation = system_msgs;
            trim_conversation(conversation, max_tokens);
            return;
        }
    }

    // Rebuild conversation
    *conversation = system_msgs;
    conversation.extend(recent);

    // Final trim check (summary itself might still be over budget)
    trim_conversation(conversation, max_tokens);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::AgentRole;
    use crate::llm::types::LlmUsage;

    #[test]
    fn test_build_summary_prompt_contains_anchored_sections() {
        let messages = vec![LlmMessage {
            role: MessageRole::User,
            content: "Implement auth".into(),
            tool_calls: None,
            tool_call_id: None,
        }];
        let prompt = build_summary_prompt(&messages, None);
        for section in [
            "## Objetivo",
            "## Decisiones tomadas",
            "## Hechos y contexto clave",
            "## Código/patrones relevantes",
            "## Tareas pendientes",
            "## Riesgos o bloqueos",
        ] {
            assert!(
                prompt.contains(section),
                "prompt should contain anchored section '{section}'"
            );
        }
        // The conversation content should also be included.
        assert!(prompt.contains("Implement auth"));
    }

    #[test]
    fn test_build_summary_prompt_uses_full_tool_output_from_store() {
        // A Tool message whose content was truncated for the LLM, but whose
        // full output is retained in the store.
        let messages = vec![LlmMessage {
            role: MessageRole::Tool,
            content: "truncated output...".into(),
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
        }];

        // Without a store, the truncated content is used.
        let prompt_no_store = build_summary_prompt(&messages, None);
        assert!(prompt_no_store.contains("truncated output..."));
        assert!(!prompt_no_store.contains("FULL OUTPUT"));

        // With a store containing the full output, the full output is used.
        let mut store = ToolOutputStore::new();
        store.insert("call_1", "FULL OUTPUT of the tool call");
        let prompt_with_store = build_summary_prompt(&messages, Some(&store));
        assert!(prompt_with_store.contains("FULL OUTPUT of the tool call"));
        assert!(!prompt_with_store.contains("truncated output..."));
    }

    #[test]
    fn test_build_summary_prompt_falls_back_to_truncated_when_id_missing() {
        // A Tool message whose id is NOT in the store falls back to its own
        // (truncated) content.
        let messages = vec![LlmMessage {
            role: MessageRole::Tool,
            content: "truncated output...".into(),
            tool_calls: None,
            tool_call_id: Some("call_missing".into()),
        }];
        let store = ToolOutputStore::new();
        let prompt = build_summary_prompt(&messages, Some(&store));
        assert!(prompt.contains("truncated output..."));
    }

    #[test]
    fn test_should_compact_threshold_logic() {
        let max_tokens = 1000;
        // Below the 0.8 ratio -> no compaction.
        assert!(!should_compact(799, max_tokens, COMPACTION_THRESHOLD_RATIO));
        // Exactly at the threshold (800) -> still not over it.
        assert!(!should_compact(800, max_tokens, COMPACTION_THRESHOLD_RATIO));
        // Over the threshold -> compact.
        assert!(should_compact(801, max_tokens, COMPACTION_THRESHOLD_RATIO));
        // A ratio of 1.0 effectively disables threshold-based compaction.
        assert!(!should_compact(1000, max_tokens, 1.0));
        // Zero max_tokens never compacts.
        assert!(!should_compact(100, 0, COMPACTION_THRESHOLD_RATIO));
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

    #[test]
    fn test_skill_to_tool_definition() {
        let skill = Skill {
            name: "code-review".into(),
            description: "Reviews code for quality".into(),
            instructions: "Check for correctness, performance, style.".into(),
            metadata: Default::default(),
        };
        let tool = skill_to_tool_definition(&skill);
        assert_eq!(tool.name, "code-review");
        assert!(tool.description.contains("Reviews code"));
        assert_eq!(tool.input_schema["type"], "object");
        assert!(
            tool.input_schema["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("task"))
        );
    }

    #[tokio::test]
    async fn test_execute_skill_tool_found() {
        let skills = vec![Skill {
            name: "test-skill".into(),
            description: "A test skill".into(),
            instructions: "Do the thing.".into(),
            metadata: Default::default(),
        }];
        let tool_call = ToolCall {
            id: "call_1".into(),
            call_type: "function".into(),
            function: crate::llm::types::ToolFunction {
                name: "test-skill".into(),
                arguments: r#"{"task": "test the thing"}"#.into(),
            },
        };
        let (tx, _) = mpsc::channel(64);
        let id = crate::agent::types::AgentId::new();
        let result = execute_skill_tool(&skills, "agent", &tool_call, &tx, &id).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("test-skill"));
        assert!(output.contains("Do the thing."));
        assert!(output.contains("test the thing"));
    }

    #[tokio::test]
    async fn test_execute_skill_tool_shell_reality() {
        let skills = vec![Skill {
            name: "shell".into(),
            description: "Run shell commands".into(),
            instructions: "Execute commands via sh -c".into(),
            metadata: Default::default(),
        }];
        let tool_call = ToolCall {
            id: "call_shell".into(),
            call_type: "function".into(),
            function: crate::llm::types::ToolFunction {
                name: "shell".into(),
                arguments: r#"{"task": "echo 'hello from anacleto'"}"#.into(),
            },
        };
        let (tx, _) = mpsc::channel(64);
        let id = crate::agent::types::AgentId::new();
        let result = execute_skill_tool(&skills, "agent", &tool_call, &tx, &id).await;
        assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result);
        let output = result.unwrap();
        assert!(
            output.contains("hello from anacleto"),
            "Expected 'hello from anacleto' in output, got: {output:?}"
        );
    }

    #[tokio::test]
    async fn test_execute_skill_tool_shell_error() {
        let skills = vec![Skill {
            name: "shell".into(),
            description: "Run shell commands".into(),
            instructions: "Execute commands via sh -c".into(),
            metadata: Default::default(),
        }];
        let tool_call = ToolCall {
            id: "call_shell_err".into(),
            call_type: "function".into(),
            function: crate::llm::types::ToolFunction {
                name: "shell".into(),
                arguments: r#"{"task": "exit 1"}"#.into(),
            },
        };
        let (tx, _) = mpsc::channel(64);
        let id = crate::agent::types::AgentId::new();
        let result = execute_skill_tool(&skills, "agent", &tool_call, &tx, &id).await;
        assert!(result.is_err(), "Expected Err for exit 1, got Ok");
    }

    #[test]
    fn test_extract_shell_command_from_prose() {
        // Prose with an apostrophe must NOT be passed to sh -c (would cause EOF).
        let task = "Find the project structure to understand what we're working with";
        assert_eq!(extract_shell_command(task), "");
    }

    #[test]
    fn test_extract_shell_command_description_colon() {
        let task = "Run the test suite: cargo test";
        assert_eq!(extract_shell_command(task), "cargo test");
    }

    #[test]
    fn test_extract_shell_command_multiline() {
        let task = "Check git status:\n  git status --short\n  git status";
        assert_eq!(
            extract_shell_command(task),
            "git status --short\ngit status"
        );
    }

    #[test]
    fn test_extract_shell_command_direct() {
        assert_eq!(
            extract_shell_command("echo 'hello from anacleto'"),
            "echo 'hello from anacleto'"
        );
    }

    #[tokio::test]
    async fn test_execute_shell_command_prose_returns_helpful_error() {
        let result = execute_shell_command(
            "Find the project structure to understand what we're working with",
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("No shell command found"),
            "Expected helpful error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_execute_shell_command_grep_no_results_is_ok() {
        // grep on an empty file exits 1 (no matches); should be reported as
        // "no results", not an error.
        let result = execute_shell_command("grep nonexistent_zzz /dev/null").await;
        assert!(
            result.is_ok(),
            "Expected Ok for grep no-results, got Err: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn test_summarize_under_budget_no_change() {
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::User,
                content: "hello".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "world".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let original_len = msgs.len();
        summarize_conversation(&mut msgs, 9999, None, "test", false, None).await;
        assert_eq!(msgs.len(), original_len);
        assert_eq!(msgs[0].content, "hello");
    }

    #[tokio::test]
    async fn test_summarize_force_true_under_budget_trims() {
        // With `force = true`, summarization is attempted even under budget.
        // A provider is available, so the old messages are replaced by a
        // summary System message (the conversation shrinks).
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "System prompt".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let original_len = msgs.len();
        let provider = MockProvider;
        // Generous budget: without `force` nothing would change.
        summarize_conversation(&mut msgs, 9999, Some(&provider), "test", true, None).await;
        // System preserved, oldest non-system replaced by a summary, last exchange kept.
        assert_eq!(msgs[0].role, MessageRole::System);
        assert!(msgs.len() < original_len);
        assert!(msgs.iter().any(|m| m.content.contains("SUMMARY")));
        assert_eq!(msgs[msgs.len() - 2].content, "msg2");
        assert_eq!(msgs[msgs.len() - 1].content, "resp2");
    }

    #[tokio::test]
    async fn test_summarize_force_false_under_budget_no_change() {
        // With `force = false` and a generous budget, nothing is trimmed.
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "System prompt".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let original_len = msgs.len();
        summarize_conversation(&mut msgs, 9999, None, "test", false, None).await;
        assert_eq!(msgs.len(), original_len);
    }

    #[tokio::test]
    async fn test_summarize_over_budget_no_provider_falls_back_to_trim() {
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "You are a helpful assistant.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "What is Rust?".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "Rust is a systems programming language.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "Tell me more about ownership.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "Ownership is Rust's core feature.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        // Very tight budget: only system + last 2 messages fit
        summarize_conversation(&mut msgs, 30, None, "test", false, None).await;
        // System should be preserved, oldest non-system dropped
        assert_eq!(msgs[0].role, MessageRole::System);
        assert!(msgs.len() >= 2); // at least system + last exchange
    }

    #[tokio::test]
    async fn test_summarize_few_messages_falls_back_to_trim() {
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::User,
                content: "hello".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "hi".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "how are you?".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        // Only 3 non-system messages (< 4), should fall back to trim
        summarize_conversation(&mut msgs, 5, None, "test", false, None).await;
        // Should still have at least 2 messages (last exchange)
        assert!(msgs.len() >= 2);
    }

    #[tokio::test]
    async fn test_trim_conversation_preserves_system_and_recent() {
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "System prompt".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        trim_conversation(&mut msgs, 10);
        assert_eq!(msgs[0].role, MessageRole::System);
        assert_eq!(msgs[0].content, "System prompt");
        // Last exchange preserved
        assert_eq!(msgs[msgs.len() - 2].content, "msg2");
        assert_eq!(msgs[msgs.len() - 1].content, "resp2");
    }
    #[tokio::test]
    async fn test_execute_skill_tool_not_found() {
        let skills = vec![];
        let tool_call = ToolCall {
            id: "call_1".into(),
            call_type: "function".into(),
            function: crate::llm::types::ToolFunction {
                name: "nonexistent".into(),
                arguments: r#"{"task": "test"}"#.into(),
            },
        };
        let (tx, _) = mpsc::channel(64);
        let id = crate::agent::types::AgentId::new();
        let result = execute_skill_tool(&skills, "agent", &tool_call, &tx, &id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("nonexistent"));
    }

    /// Minimal `LlmProvider` stub that returns a fixed summary, used to test
    /// `summarize_conversation` without hitting a real LLM API.
    struct MockProvider;

    #[async_trait::async_trait]
    impl crate::llm::provider::LlmProvider for MockProvider {
        async fn complete(&self, _request: LlmRequest) -> crate::error::Result<LlmResponse> {
            Ok(LlmResponse {
                content: "SUMMARY of earlier conversation".into(),
                tool_calls: vec![],
                finish_reason: "stop".into(),
                usage: None,
            })
        }

        async fn complete_stream(
            &self,
            _request: LlmRequest,
        ) -> crate::error::Result<tokio::sync::mpsc::Receiver<Result<LlmStreamChunk>>> {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let _ = tx
                .send(Ok(LlmStreamChunk::Done(LlmUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                })))
                .await;
            Ok(rx)
        }

        fn context_window(&self) -> usize {
            8192
        }

        async fn fetch_context_window(&self) -> crate::error::Result<usize> {
            Ok(8192)
        }

        fn set_context_window(&self, _value: usize) {}

        fn input_price_per_million(&self) -> f64 {
            0.0
        }

        fn output_price_per_million(&self) -> f64 {
            0.0
        }
    }

    #[test]
    fn test_task_tool_args_parse_foreground() {
        let args = TaskToolArgs::parse(
            r#"{"task_id":"t1","description":"do the thing","mode":"foreground"}"#,
        )
        .unwrap();
        assert_eq!(args.task_id, "t1");
        assert_eq!(args.description, "do the thing");
        assert_eq!(args.mode, TaskMode::Foreground);
        assert_eq!(args.model, None);
        assert!(args.tools.is_empty());
    }

    #[test]
    fn test_task_tool_args_parse_background_with_model_and_tools() {
        let args = TaskToolArgs::parse(
            r#"{"task_id":"t2","description":"research","mode":"background","model":"claude-opus-4","tools":["shell","read"]}"#,
        )
        .unwrap();
        assert_eq!(args.task_id, "t2");
        assert_eq!(args.mode, TaskMode::Background);
        assert_eq!(args.model.as_deref(), Some("claude-opus-4"));
        assert_eq!(args.tools, vec!["shell".to_string(), "read".to_string()]);
    }

    #[test]
    fn test_task_tool_args_defaults_mode_and_task_id() {
        // Missing mode defaults to foreground; missing task_id is generated.
        let args = TaskToolArgs::parse(r#"{"description":"just a task"}"#).unwrap();
        assert_eq!(args.mode, TaskMode::Foreground);
        assert!(!args.task_id.is_empty());
    }

    #[test]
    fn test_task_tool_args_requires_description() {
        let err = TaskToolArgs::parse(r#"{"task_id":"t3"}"#).unwrap_err();
        assert!(err.contains("description"));
    }

    #[test]
    fn test_task_tool_args_invalid_json() {
        assert!(TaskToolArgs::parse("not json").is_err());
    }

    #[test]
    fn test_plan_mode_blocks_apply_patch() {
        let blocked = plan_mode_blocked(&AgentMode::Plan, "apply_patch", "{}");
        assert!(blocked.is_some());
        assert!(blocked.unwrap().contains("plan mode"));
    }

    #[test]
    fn test_plan_mode_blocks_filesystem_write() {
        let blocked = plan_mode_blocked(
            &AgentMode::Plan,
            "filesystem",
            r#"{"task":"{\"op\":\"write\",\"path\":\"a.txt\",\"content\":\"x\"}"}"#,
        );
        assert!(blocked.is_some());
    }

    #[test]
    fn test_plan_mode_allows_reads() {
        assert!(plan_mode_blocked(&AgentMode::Plan, "read", "{}").is_none());
        assert!(plan_mode_blocked(&AgentMode::Plan, "grep", "{}").is_none());
        assert!(
            plan_mode_blocked(
                &AgentMode::Plan,
                "filesystem",
                r#"{"task":"{\"op\":\"read\",\"path\":\"a.txt\"}"}"#,
            )
            .is_none()
        );
    }

    #[test]
    fn test_build_mode_allows_writes() {
        assert!(plan_mode_blocked(&AgentMode::Build, "apply_patch", "{}").is_none());
    }
}
