use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

use crate::agent::context::{conversation_tokens, summarize_conversation};
use crate::agent::retry::retry_with_backoff;
use crate::agent::source::load_workspace_instructions;
use crate::agent::tool_store::{ToolOutputStore, summarize_tool_result};
use crate::agent::tools::{
    builtin_tool_definitions, skill_to_tool_definition, subagent_tool_definitions,
};
use crate::agent::types::PendingQuestions;
use crate::agent::types::{Agent, AgentId, AgentMode, AgentStatus, BackgroundTaskManager};
use crate::config::types::AgentConfig;
use crate::config::types::RetryConfig;
use crate::db::session::Database;
use crate::engine::orchestrator::{EngineEvent, TaskStatus, UsageEvent};
use crate::error::{Error, Result};
use crate::hook::{HookContext, HookPoint, HookRegistry};
use crate::llm::provider::LlmProvider;
use crate::llm::provider::LlmProviderRegistry;
use crate::llm::template::render_template;
use crate::llm::types::{
    LlmMessage, LlmRequest, LlmResponse, LlmStreamChunk, LlmUsage, MessageRole, ToolCall,
    ToolDefinition,
};
use crate::mcp::client::McpRegistry;
use crate::plugin::PluginRegistry;
use crate::skill::registry::SharedSkillRegistry;
use crate::tools::delete::execute_delete_tool;
use crate::tools::execute::execute_execute_tool;
use crate::tools::format::execute_format_document_tool;
use crate::tools::glob::execute_glob_tool;
use crate::tools::grep::execute_grep_tool;
use crate::tools::insert::execute_insert_tool;
use crate::tools::list::execute_list_tool;
use crate::tools::lsp::execute_lsp_query_tool;
use crate::tools::mcp::{
    execute_mcp_list_resource_templates_tool, execute_mcp_list_resources_tool,
    execute_mcp_read_resource_tool,
};
use crate::tools::read::execute_read_tool;
use crate::tools::replace::execute_replace_tool;
use crate::tools::web::{execute_webfetch_tool, execute_websearch_tool};
use crate::tools::write::execute_write_tool;

/// Outcome of a single call to [`AgentSession::process`].
#[derive(Debug, Clone)]
pub enum AgentOutcome {
    /// The agent finished its task normally.
    Completed(String),
    /// The agent paused itself and is waiting for an answer from the user.
    NeedsAnswer { question: String },
    /// The agent ran out of steps before completing the task.
    OutOfSteps { partial: String },
    /// The agent was cancelled (via cancel flag).
    Cancelled,
}

/// Read-only state shared across all agent sessions in a session tree.
pub struct AgentSharedState {
    pub event_tx: tokio::sync::mpsc::Sender<EngineEvent>,
    pub llm_registry: LlmProviderRegistry,
    pub usage_tx: Option<tokio::sync::mpsc::Sender<UsageEvent>>,
    pub skill_registry: SharedSkillRegistry,
    pub retry_config: RetryConfig,
    pub pending_questions: Option<PendingQuestions>,
    pub debug: Arc<AtomicBool>,
    pub history_limit_percent: f64,
    pub session_id: Option<Uuid>,
    pub db: Option<Database>,
    pub workspace: PathBuf,
    pub plugins: Option<Arc<PluginRegistry>>,
    pub hook_registry: HookRegistry,
    pub compact_requested: Arc<AtomicBool>,
    pub background_tasks: Arc<tokio::sync::Mutex<BackgroundTaskManager>>,
}

/// A single agent session — the state and logic for one agent invocation.
///
/// Unlike the old `spawn_agent` model, an `AgentSession` is **not** a
/// background task.  It is a plain struct whose [`process`] method runs
/// the LLM→tools→LLM loop synchronously (in async terms).  Callers
/// decide whether to run it sequentially, in a `JoinSet`, or in a
/// `tokio::spawn`.
pub struct AgentSession {
    pub agent: Agent,
    pub shared: Arc<AgentSharedState>,
    pub subagent_configs: Vec<AgentConfig>,
    skill_names: Vec<String>,
    pub conversation: Vec<LlmMessage>,
    pub tool_store: ToolOutputStore,
    tools: Vec<ToolDefinition>,
    mcp_tool_map: HashMap<String, (String, String)>,
    mcp_registry: Option<Arc<tokio::sync::Mutex<McpRegistry>>>,
    max_history_tokens: usize,
    steps_used: usize,
    pub max_steps: u32,
    mode: AgentMode,
    writable_paths: Vec<PathBuf>,
    loaded_skills: HashSet<String>,
    cancel_flag: Arc<AtomicBool>,
    debug: bool,
}

impl AgentSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent: Agent,
        shared: Arc<AgentSharedState>,
        subagent_configs: Vec<AgentConfig>,
        skill_names: Vec<String>,
        cancel_flag: Option<Arc<AtomicBool>>,
        mode: AgentMode,
    ) -> Self {
        let cancel_flag = cancel_flag.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let debug = shared.debug.load(Ordering::Relaxed);
        let max_steps = agent.max_steps;

        Self {
            agent,
            shared,
            subagent_configs,
            skill_names,
            conversation: Vec::new(),
            tool_store: ToolOutputStore::new(),
            tools: Vec::new(),
            mcp_tool_map: HashMap::new(),
            mcp_registry: None,
            max_history_tokens: 0,
            steps_used: 0,
            max_steps,
            mode,
            writable_paths: Vec::new(),
            loaded_skills: HashSet::new(),
            cancel_flag,
            debug,
        }
    }

    pub async fn initialize(
        &mut self,
        provider: &Arc<dyn LlmProvider>,
        mcp_registry: Option<Arc<tokio::sync::Mutex<McpRegistry>>>,
        mcp_enabled: Option<Arc<tokio::sync::Mutex<HashMap<String, bool>>>>,
        plugins: Option<&Arc<PluginRegistry>>,
        workspace: &Path,
    ) -> Result<()> {
        self.mcp_registry = mcp_registry.clone();
        self.max_history_tokens =
            (provider.context_window() as f64 * self.shared.history_limit_percent / 100.0) as usize;

        let mut tools: Vec<ToolDefinition> = {
            let reg = self.shared.skill_registry.read().await;
            self.skill_names
                .iter()
                .filter_map(|name| reg.get(name))
                .map(skill_to_tool_definition)
                .collect()
        };
        if !self.subagent_configs.is_empty() {
            tools.extend(subagent_tool_definitions(&self.subagent_configs));
        }

        let builtin_tools = builtin_tool_definitions();
        for tool_name in &self.agent.tools {
            if let Some(def) = builtin_tools.get(tool_name).cloned() {
                tools.push(def);
            }
        }

        if let Some(plugins) = plugins {
            tools.extend(plugins.custom_tools().iter().cloned());
        }

        let mcp_tool_map: HashMap<String, (String, String)> = {
            let mut map = HashMap::new();
            if let Some(ref mcp_reg) = mcp_registry {
                let reg = mcp_reg.lock().await;
                let collected = reg.collect_tools(&self.agent.mcps).await;
                for (server_name, original_name, tool_def) in collected {
                    if let Some(ref enabled_map) = mcp_enabled
                        && !*enabled_map.lock().await.get(&server_name).unwrap_or(&true)
                    {
                        continue;
                    }
                    let prefixed_name = tool_def.name.clone();
                    map.insert(prefixed_name, (server_name.clone(), original_name.clone()));
                    // Also insert an unprefixed alias so the LLM can call tools
                    // by their natural name (e.g. "codegraph_files" instead of
                    // "codegraph_codegraph_files").
                    map.insert(original_name.clone(), (server_name, original_name));
                    tools.push(tool_def);
                }
            }
            map
        };

        self.tools = tools;
        self.mcp_tool_map = mcp_tool_map;
        self.writable_paths = self.agent.writable_paths.clone();

        let system_prompt = self.render_system_prompt(workspace).await?;
        if !system_prompt.is_empty() {
            self.conversation.push(LlmMessage {
                role: MessageRole::System,
                content: system_prompt,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        if workspace.is_dir() {
            for (name, content) in load_workspace_instructions(workspace) {
                self.conversation.push(LlmMessage {
                    role: MessageRole::System,
                    content: format!("[Instrucciones del workspace: {name}]\n{content}"),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        Ok(())
    }

    async fn render_system_prompt(&self, workspace: &Path) -> Result<String> {
        let tool_names = self
            .tools
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<_>>()
            .join(", ");

        // Build {lifecycle} template variable
        let lifecycle = format!(
            "You have a maximum of {} steps (LLM + tool iterations) to complete your task. \
             After that, you will be stopped and the task will be marked as incomplete.\n\
             Subagents are disposable: they are created for a single task, work independently, \
             and are destroyed after returning their result. Subagents do NOT inherit any \
             tools, skills, MCPs, or configuration from the parent agent.\n\
             Use the 'delegate' tool to assign work to a subagent. Use 'spawn_background' \
             for tasks that should continue running while you do other work.",
            self.max_steps
        );

        // Build {subagents} template variable with full capabilities
        let subagents_template = if self.subagent_configs.is_empty() {
            String::new()
        } else {
            let mut block = String::from("\n");
            for sc in &self.subagent_configs {
                block.push_str(&format!("- **{}** — {}", sc.name, sc.description));
                if !sc.when_to_use.is_empty() {
                    block.push_str(&format!(" ({})", sc.when_to_use));
                }
                block.push('\n');
                // Add capabilities
                let tools_list = if sc.tools.is_empty() {
                    "none".to_string()
                } else {
                    sc.tools.join(", ")
                };
                let skills_list = if sc.skills.is_empty() {
                    "none".to_string()
                } else {
                    sc.skills
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let mcps_list = if sc.mcps.is_empty() {
                    "none".to_string()
                } else {
                    sc.mcps.join(", ")
                };
                block.push_str(&format!(
                    "  - Tools: {tools_list}\n  - Skills: {skills_list}\n  - MCPs: {mcps_list}\n  - Max steps: {}\n  - Model: {}\n",
                    sc.max_steps, sc.model
                ));
            }
            block
        };

        let mut vars = HashMap::new();
        vars.insert("model".to_string(), self.agent.model.clone());
        vars.insert(
            "workspace".to_string(),
            workspace.to_string_lossy().to_string(),
        );
        vars.insert("tools".to_string(), tool_names);
        vars.insert("subagents".to_string(), subagents_template);
        vars.insert("lifecycle".to_string(), lifecycle.clone());
        let mut system_prompt = render_template(&self.agent.description, &vars);

        // Auto-append {lifecycle} if not used in template
        if !self.agent.description.contains("{lifecycle}") {
            system_prompt.push_str("\n\n--- Lifecycle ---\n");
            system_prompt.push_str(&lifecycle);
        }

        // Always inject the workspace path so the agent knows where files are.
        system_prompt.push_str(&format!(
            "\n\n--- Workspace ---\nYour workspace root is: {}\n",
            workspace.display()
        ));

        // Auto-append {subagents} if not used in template and subagents exist
        if !self.subagent_configs.is_empty() && !self.agent.description.contains("{subagents}") {
            let mut subagents_block = String::from("\n\n--- Available subagents ---\n");
            for sc in &self.subagent_configs {
                subagents_block.push_str(&format!("• **{}** — {}\n", sc.name, sc.description));
                if !sc.when_to_use.is_empty() {
                    subagents_block.push_str(&format!("  *When to use*: {}\n", sc.when_to_use));
                }
                let tools_list = if sc.tools.is_empty() {
                    "none".to_string()
                } else {
                    sc.tools.join(", ")
                };
                let skills_list = if sc.skills.is_empty() {
                    "none".to_string()
                } else {
                    sc.skills
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let mcps_list = if sc.mcps.is_empty() {
                    "none".to_string()
                } else {
                    sc.mcps.join(", ")
                };
                subagents_block.push_str(&format!(
                    "  - Tools: {tools_list}\n  - Skills: {skills_list}\n  - MCPs: {mcps_list}\n  - Max steps: {}\n  - Model: {}\n",
                    sc.max_steps, sc.model
                ));
            }
            system_prompt.push_str(&subagents_block);
        }

        // --- Tool Output Store instructions ---
        system_prompt.push_str(
            "\n\
             --- Tool Output Store ---\n\
             Tool results larger than about 2700 characters are SUMMARIZED in the \n\
             conversation: you will see the first ~2000 characters and the last ~500 \n\
             characters, plus a note about the total size and a reference ID \
             (tool_call_id).\n\
             \n\
             The FULL tool output is always stored in the ToolOutputStore. Use \
             `get_tool_result(tool_call_id=\"...\")` to retrieve the complete \
             content when you need more detail than the summary provides. This \
             keeps the conversation lean and your reasoning in context.\n",
        );

        if let Some(plugins) = &self.shared.plugins {
            system_prompt = plugins.on_agent_spawn(&self.agent.name, &system_prompt);
        }

        Ok(system_prompt)
    }

    /// Emit final events and return the appropriate `AgentOutcome` when the
    /// agent finishes its lifecycle. This is called by every exit path in
    /// [`process()`] so that `EngineEvent::TaskComplete` is always emitted
    /// regardless of how the agent terminated (success, error, max steps, or
    /// cancelled).
    async fn finalize(&mut self, status: TaskStatus, result: String) -> Result<AgentOutcome> {
        self.emit_event(EngineEvent::AgentOutput {
            agent_id: self.agent.id.clone(),
            agent_name: self.agent.name.clone(),
            content: result.clone(),
        })
        .await;
        self.emit_event(EngineEvent::AgentStatusChanged {
            agent_id: self.agent.id.clone(),
            agent_name: self.agent.name.clone(),
            status: AgentStatus::Idle,
        })
        .await;
        self.emit_event(EngineEvent::TaskComplete {
            agent_id: self.agent.id.clone(),
            agent_name: self.agent.name.clone(),
            status,
            result: result.clone(),
        })
        .await;

        Ok(match status {
            TaskStatus::Success => AgentOutcome::Completed(result),
            TaskStatus::Error => AgentOutcome::Completed(result),
            TaskStatus::MaxStepsReached => AgentOutcome::OutOfSteps { partial: result },
            TaskStatus::Cancelled => AgentOutcome::Cancelled,
        })
    }

    /// Run the agent loop: LLM → tools → LLM → … until done, blocked, or cancelled.
    pub async fn process(&mut self, input: &str) -> Result<AgentOutcome> {
        self.steps_used = 0;

        tracing::info!(
            agent = %self.agent.name,
            input_len = %input.len(),
            max_steps = %self.max_steps,
            "Agent process started"
        );

        self.emit_event(EngineEvent::AgentStatusChanged {
            agent_id: self.agent.id.clone(),
            agent_name: self.agent.name.clone(),
            status: AgentStatus::Working,
        })
        .await;

        self.emit_event(EngineEvent::AgentMessage {
            agent_id: self.agent.id.clone(),
            agent_name: self.agent.name.clone(),
            message: input.to_string(),
        })
        .await;

        if let (Some(db), Some(sid)) = (self.shared.db.as_ref(), self.shared.session_id) {
            let _ = db
                .store_message(sid, &self.agent.name, "user", input, None)
                .await;
        }

        self.conversation.push(LlmMessage {
            role: MessageRole::User,
            content: input.to_string(),
            tool_calls: None,
            tool_call_id: None,
        });

        self.compact_conversation().await;

        // Check if compact was requested (from TUI /compact command)
        if self.shared.compact_requested.swap(false, Ordering::Relaxed) {
            self.compact_conversation().await;
        }

        let provider = self.resolve_provider()?;

        loop {
            self.steps_used += 1;

            tracing::debug!(
                target: "anacleto::agent::session",
                agent = %self.agent.name,
                step = %self.steps_used,
                conversation_len = %self.conversation.len(),
                "Agent loop iteration"
            );

            if self.cancel_flag.load(Ordering::Relaxed) {
                self.cancel_flag.store(false, Ordering::Relaxed);
                self.emit_event(EngineEvent::AgentStatusChanged {
                    agent_id: self.agent.id.clone(),
                    agent_name: self.agent.name.clone(),
                    status: AgentStatus::Idle,
                })
                .await;
                self.emit_event(EngineEvent::TaskComplete {
                    agent_id: self.agent.id.clone(),
                    agent_name: self.agent.name.clone(),
                    status: TaskStatus::Cancelled,
                    result: String::new(),
                })
                .await;
                return Ok(AgentOutcome::Cancelled);
            }

            if self.steps_used > self.max_steps as usize {
                let partial = format!(
                    "[Incomplete task] Reached the limit of {} steps without completing the task.",
                    self.max_steps
                );
                tracing::warn!(
                    agent = %self.agent.name,
                    max_steps = %self.max_steps,
                    "Agent ran out of steps"
                );
                return self.finalize(TaskStatus::MaxStepsReached, partial).await;
            }

            self.emit_event(EngineEvent::LocalTokenEstimate {
                tokens: conversation_tokens(&self.conversation),
            })
            .await;

            // LLM call — get streaming response
            let request = LlmRequest {
                model: self.agent.model.clone(),
                messages: self.conversation.clone(),
                tools: self.tools.clone(),
                max_tokens: self.agent.max_tokens,
                temperature: self.agent.temperature,
                top_p: self.agent.top_p,
                stream: true,
                cache_control: None,
            };

            tracing::debug!(
                target: "anacleto::agent::session",
                agent = %self.agent.name,
                step = %self.steps_used,
                messages = %request.messages.len(),
                tools = %request.tools.len(),
                "LLM request built"
            );

            tracing::info!(
                agent = %self.agent.name,
                model = %request.model,
                step = %self.steps_used,
                messages = %request.messages.len(),
                tools = %request.tools.len(),
                "LLM request"
            );

            // Debug event
            if self.debug {
                self.emit_event(EngineEvent::LlmRequestDebug {
                    agent_name: self.agent.name.clone(),
                    model: self.agent.model.clone(),
                    payload: serde_json::to_string(&request).unwrap_or_default(),
                })
                .await;
            }

            // Retry wrapper
            let agent_id = self.agent.id.clone();
            let agent_name = self.agent.name.clone();
            let debug = self.debug;
            let event_tx = self.shared.event_tx.clone();
            let usage_tx = self.shared.usage_tx.clone();
            let provider = Arc::clone(&provider);

            let llm_result = retry_with_backoff(
                move |_attempt| {
                    let request = request.clone();
                    let provider = Arc::clone(&provider);
                    let event_tx = event_tx.clone();
                    let usage_tx = usage_tx.clone();
                    let agent_id = agent_id.clone();
                    let agent_name = agent_name.clone();
                    let debug = debug;
                    async move {
                        let mut stream_rx = provider.complete_stream(request.clone()).await?;
                        let mut full_content = String::new();
                        let mut tool_calls: Vec<ToolCall> = Vec::new();
                        let mut usage: Option<LlmUsage> = None;

                        while let Some(chunk) = stream_rx.recv().await {
                            match chunk {
                                Ok(LlmStreamChunk::Content(text)) => {
                                    full_content.push_str(&text);
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
                                Ok(LlmStreamChunk::Thinking(text)) => {
                                    let _ = event_tx
                                        .send(EngineEvent::AgentThinkingChunk {
                                            agent_id: agent_id.clone(),
                                            agent_name: agent_name.clone(),
                                            content: text,
                                        })
                                        .await;
                                }
                                Ok(LlmStreamChunk::Done(u)) => {
                                    usage = Some(u);
                                }
                                Ok(LlmStreamChunk::Error(e)) => {
                                    return Err(Error::Llm(e));
                                }
                                Err(e) => {
                                    return Err(e);
                                }
                            }
                        }

                        // Emit usage if available
                        if let Some(u) = usage {
                            let cost = u.cost.unwrap_or_else(|| {
                                (u.prompt_tokens as f64 * provider.input_price_per_million()
                                    + u.completion_tokens as f64
                                        * provider.output_price_per_million())
                                    / 1_000_000.0
                            });
                            let _ = event_tx
                                .send(EngineEvent::TokenUsage {
                                    agent_id: agent_id.clone(),
                                    agent_name: agent_name.clone(),
                                    total_tokens: u.total_tokens,
                                    prompt_tokens: u.prompt_tokens,
                                    context_window: provider.context_window() as u32,
                                    cost,
                                })
                                .await;
                            if let Some(ref utx) = usage_tx {
                                let _ = utx
                                    .send(UsageEvent {
                                        total_tokens: u.total_tokens,
                                        cost,
                                    })
                                    .await;
                            }
                        }

                        // Debug event
                        if debug {
                            let response = LlmResponse {
                                content: full_content.clone(),
                                tool_calls: tool_calls.clone(),
                                finish_reason: if tool_calls.is_empty() {
                                    "stop".to_string()
                                } else {
                                    "tool_calls".to_string()
                                },
                                usage: None,
                                thinking: None,
                            };
                            let _ = event_tx
                                .send(EngineEvent::LlmResponseDebug {
                                    agent_name: agent_name.clone(),
                                    model: request.model.clone(),
                                    payload: serde_json::to_string(&response).unwrap_or_default(),
                                })
                                .await;
                        }

                        let finish_reason = if tool_calls.is_empty() {
                            "stop".to_string()
                        } else {
                            "tool_calls".to_string()
                        };

                        Ok(LlmResponse {
                            content: full_content,
                            tool_calls,
                            finish_reason,
                            usage: None,
                            thinking: None,
                        })
                    }
                },
                &self.shared.retry_config,
                &format!("LLM stream call for '{}'", self.agent.name),
            )
            .await;

            let response = match llm_result {
                Ok(r) => {
                    tracing::info!(
                        agent = %self.agent.name,
                        step = %self.steps_used,
                        tool_calls = %r.tool_calls.len(),
                        content_len = %r.content.len(),
                        finish_reason = %r.finish_reason,
                        "LLM response received"
                    );
                    r
                }
                Err(e) => {
                    tracing::error!(
                        agent = %self.agent.name,
                        step = %self.steps_used,
                        error = %e,
                        "LLM request failed"
                    );
                    let result = format!("[Error en LLM] {e}");
                    self.emit_event(EngineEvent::AgentStatusChanged {
                        agent_id: self.agent.id.clone(),
                        agent_name: self.agent.name.clone(),
                        status: AgentStatus::Idle,
                    })
                    .await;
                    self.emit_event(EngineEvent::TaskComplete {
                        agent_id: self.agent.id.clone(),
                        agent_name: self.agent.name.clone(),
                        status: TaskStatus::Error,
                        result: result.clone(),
                    })
                    .await;
                    return Ok(AgentOutcome::Completed(result));
                }
            };

            // Process tool calls
            if response.tool_calls.is_empty() {
                let output = response.content;
                self.conversation.push(LlmMessage {
                    role: MessageRole::Assistant,
                    content: output.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                });
                tracing::info!(
                    agent = %self.agent.name,
                    output_len = %output.len(),
                    "Agent completed successfully"
                );
                return self.finalize(TaskStatus::Success, output).await;
            }

            // Execute tool calls: delegate calls (prefixed with `delegate_to_`)
            // run concurrently in separate OS threads with their own Tokio runtimes.
            // Non-delegate calls (skills, MCP, built-in) run sequentially.
            //
            // Both paths feed into the conversation in the same way — the only
            // difference is concurrency model.
            /// Result type for a parallel future (delegate or other concurrent tool).
            type ParallelFuture =
                std::pin::Pin<Box<dyn futures::Future<Output = (String, String)> + Send>>;

            let mut parallel_futures: FuturesUnordered<ParallelFuture> = FuturesUnordered::new();

            for tc in &response.tool_calls {
                let tool_name = &tc.function.name;
                if let Some(subagent_name) = tool_name.strip_prefix("delegate_to_") {
                    // Spawn delegate in a dedicated thread with its own runtime.
                    // Results are piped back via a oneshot channel (which is Send
                    // and implements Future) so they can be polled in completion
                    // order through the FuturesUnordered.
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let subagent_name = subagent_name.to_string();
                    let arguments = tc.function.arguments.clone();
                    let tool_call_id = tc.id.clone();
                    let subagent_configs = self.subagent_configs.clone();
                    let shared = self.shared.clone();
                    let mode = self.mode.clone();
                    let parent_id = self.agent.id.clone();
                    let parent_name = self.agent.name.clone();
                    let event_tx = self.shared.event_tx.clone();
                    // Build context BEFORE spawning the thread (self is not Send).
                    let parent_context = self.build_delegate_context();

                    std::thread::spawn(move || {
                        let rt = match tokio::runtime::Runtime::new() {
                            Ok(r) => r,
                            Err(e) => {
                                let _ = tx.send(format!("Internal error creating runtime: {e}"));
                                return;
                            }
                        };
                        let result = rt.block_on(run_delegate_task(
                            subagent_name,
                            arguments,
                            subagent_configs,
                            shared,
                            mode,
                            parent_id,
                            parent_name,
                            event_tx,
                            parent_context,
                        ));
                        let _ = tx.send(result);
                    });

                    parallel_futures.push(Box::pin(async move {
                        let result = match rx.await {
                            Ok(r) => r,
                            Err(_) => "Delegate task failed (channel closed).".to_string(),
                        };
                        (tool_call_id, result)
                    }));
                } else {
                    // Handle get_tool_result specially — reads from ToolOutputStore
                    if tc.function.name == "get_tool_result" {
                        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                        let lookup_id = args["tool_call_id"].as_str().unwrap_or("");
                        let stored = self
                            .tool_store
                            .get(lookup_id)
                            .cloned()
                            .unwrap_or_else(|| format!("[Tool result not found: {lookup_id}]"));
                        self.conversation.push(LlmMessage {
                            role: MessageRole::Tool,
                            content: stored,
                            tool_calls: None,
                            tool_call_id: Some(tc.id.clone()),
                        });
                        continue;
                    }

                    // Execute non-delegate calls sequentially (they mutate self)
                    tracing::debug!(
                        target: "anacleto::agent::session",
                        agent = %self.agent.name,
                        step = %self.steps_used,
                        tool = %tc.function.name,
                        arguments = %tc.function.arguments,
                        "Executing tool call"
                    );
                    let result = match self.execute_tool_call(tc).await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::error!(
                                agent = %self.agent.name,
                                tool = %tc.function.name,
                                error = %e,
                                "Tool execution failed"
                            );
                            // Do NOT return early — the error is added to the
                            // conversation as a tool result so the LLM can
                            // decide how to recover, AND parallel results
                            // collected below are not lost.
                            format!("[Tool error] {e}")
                        }
                    };
                    self.tool_store.insert(tc.id.clone(), result.clone());

                    tracing::debug!(
                        target: "anacleto::agent::session",
                        agent = %self.agent.name,
                        step = %self.steps_used,
                        tool_call_id = %tc.id,
                        result_len = %result.len(),
                        "Tool result stored"
                    );

                    let llm_result = summarize_tool_result(&result, &tc.id);
                    self.conversation.push(LlmMessage {
                        role: MessageRole::Tool,
                        content: llm_result,
                        tool_calls: None,
                        tool_call_id: Some(tc.id.clone()),
                    });
                }
            }

            // Collect parallel results in completion order
            while let Some((tool_call_id, result)) = parallel_futures.next().await {
                self.tool_store.insert(tool_call_id.clone(), result.clone());
                let llm_result = summarize_tool_result(&result, &tool_call_id);
                self.conversation.push(LlmMessage {
                    role: MessageRole::Tool,
                    content: llm_result,
                    tool_calls: None,
                    tool_call_id: Some(tool_call_id),
                });
            }

            self.compact_conversation().await;
        }
    }

    fn resolve_provider(&self) -> Result<Arc<dyn LlmProvider>> {
        let model = &self.agent.model;
        let provider_name = if model.contains('/') {
            "openrouter"
        } else if model.starts_with("claude") {
            "anthropic"
        } else if model.starts_with("gpt") || model.starts_with("o1") || model.starts_with("o3") {
            "openai"
        } else {
            "ollama"
        };
        self.shared.llm_registry.get(provider_name).ok_or_else(|| {
            Error::Llm(format!(
                "No LLM provider found for model '{}' (looked up as provider '{}')",
                self.agent.model, provider_name
            ))
        })
    }

    async fn compact_conversation(&mut self) {
        if let Ok(provider) = self.resolve_provider() {
            let compacted = summarize_conversation(
                &mut self.conversation,
                self.max_history_tokens,
                Some(&*provider),
                &self.agent.model,
                false,
                Some(&self.tool_store),
                Some(&self.shared.retry_config),
            )
            .await;
            if compacted {
                let tokens = conversation_tokens(&self.conversation) as u32;
                self.emit_event(EngineEvent::ConversationCompacted {
                    agent_id: self.agent.id.clone(),
                    agent_name: self.agent.name.clone(),
                    tokens,
                })
                .await;
                self.emit_event(EngineEvent::LocalTokenEstimate {
                    tokens: tokens as usize,
                })
                .await;
            }
        }
    }

    async fn execute_tool_call(&mut self, tc: &ToolCall) -> Result<String> {
        let tool_name = &tc.function.name;

        tracing::debug!(
            target: "anacleto::agent::session",
            agent = %self.agent.name,
            tool = %tool_name,
            arguments = %tc.function.arguments,
            "execute_tool_call"
        );

        tracing::info!(
            agent = %self.agent.name,
            tool = %tool_name,
            "Tool execution started"
        );

        // Skill tool
        let is_skill = {
            let reg = self.shared.skill_registry.read().await;
            reg.get(tool_name).is_some()
        };
        if is_skill {
            return self.execute_skill(tc).await;
        }

        // MCP tool
        if self.mcp_tool_map.contains_key(tool_name) {
            return self.execute_mcp_tool(tc).await;
        }

        // Built-in tool
        self.execute_builtin_tool(tc).await
    }

    /// Build a context block from the parent session for a subagent.
    /// Includes the last user message and the most recent tool outputs.
    fn build_delegate_context(&self) -> String {
        let mut parts = Vec::new();

        // Last user message
        if let Some(last_user) = self
            .conversation
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
        {
            parts.push(format!("[Last user message]\n{}", last_user.content));
        }

        // Last tool outputs (up to 5 most recent)
        let tool_outputs = self.tool_store.last_n(5);
        if !tool_outputs.is_empty() {
            let mut tool_block = String::from("[Executed tool results]\n");
            for (i, (id, output)) in tool_outputs.iter().enumerate() {
                tool_block.push_str(&format!(
                    "--- Tool output {} (id: {}) ---\n{}\n",
                    i + 1,
                    id,
                    output
                ));
            }
            parts.push(tool_block);
        }

        parts.join("\n\n")
    }

    async fn ask_user(&self, question: &str) -> Result<String> {
        if let Some(ref pq) = self.shared.pending_questions {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let key = format!("{}-{}", self.agent.id, self.steps_used);
            {
                let mut map = pq.lock().await;
                map.insert(key.clone(), tx);
            }
            self.emit_event(EngineEvent::AgentOutput {
                agent_id: self.agent.id.clone(),
                agent_name: self.agent.name.clone(),
                content: format!("[NeedsAnswer] {question}"),
            })
            .await;
            match rx.await {
                Ok(answer) => Ok(answer),
                Err(_) => Err(Error::ChannelClosed("User response channel closed".into())),
            }
        } else {
            Ok("yes".to_string())
        }
    }

    async fn execute_skill(&mut self, tc: &ToolCall) -> Result<String> {
        let skill_name = tc.function.name.clone();

        tracing::debug!(
            target: "anacleto::agent::session",
            agent = %self.agent.name,
            skill = %tc.function.name,
            arguments = %tc.function.arguments,
            "execute_skill"
        );

        let already_loaded = self.loaded_skills.contains(&skill_name);
        let reg = self.shared.skill_registry.read().await;
        let result = crate::agent::tools::execute_skill_tool(
            &reg,
            &self.agent.name,
            tc,
            &self.shared.event_tx,
            &self.agent.id,
            Some(&self.shared.hook_registry),
            false,
            "",
            already_loaded,
        )
        .await
        .map_err(|e| Error::Skill(format!("Skill '{}' failed: {e}", tc.function.name)))?;
        self.loaded_skills.insert(skill_name);
        Ok(result)
    }

    async fn execute_mcp_tool(&self, tc: &ToolCall) -> Result<String> {
        let tool_name = &tc.function.name;

        tracing::debug!(
            target: "anacleto::agent::session",
            agent = %self.agent.name,
            tool = %tool_name,
            arguments = %tc.function.arguments,
            "execute_mcp_tool"
        );

        let (server_name, original_name) = self
            .mcp_tool_map
            .get(tool_name)
            .ok_or_else(|| Error::Agent(format!("MCP tool '{tool_name}' not found in tool map")))?;

        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        match &self.mcp_registry {
            Some(registry) => {
                let reg = registry.lock().await;
                reg.call_tool(server_name, original_name, args)
                    .await
                    .map_err(|e| Error::Agent(format!("MCP tool '{tool_name}' failed: {e}")))
            }
            None => Ok("MCP registry not available.".to_string()),
        }
    }

    async fn execute_spawn_background(&self, tc: &ToolCall) -> std::result::Result<String, String> {
        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("bg-task");
        let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");

        let task_id = crate::agent::types::BackgroundTaskId::new();
        let task_id_str = task_id.to_string();

        let agent = Agent::create_subagent(
            name.to_string(),
            format!("Background agent for: {task}"),
            self.agent.model.clone(),
            Vec::new(),
            Vec::new(),
            self.max_steps,
            self.agent.id.clone(),
            Vec::new(),
            Vec::new(),
            self.agent.temperature,
            self.agent.max_tokens,
            self.agent.top_p,
        );

        // Emit SubagentCreated so the TUI shows the background task
        self.emit_event(EngineEvent::SubagentCreated {
            parent_id: self.agent.id.clone(),
            subagent_id: agent.id.clone(),
            subagent_name: agent.name.clone(),
            skills: Vec::new(),
            mcps: Vec::new(),
            agent_type: Some(format!("bg:{name}")),
        })
        .await;

        let shared = self.shared.clone();
        let mode = self.mode.clone();
        let task_owned = task.to_string();
        let task_name = name.to_string();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag_clone = cancel_flag.clone();
        let result_arc: Arc<tokio::sync::Mutex<Option<anyhow::Result<String>>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let result_arc_clone = result_arc.clone();
        let subagent_id = agent.id.clone();
        let subagent_name = agent.name.clone();
        let event_tx = self.shared.event_tx.clone();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(r) => r,
                Err(e) => {
                    let mut guard = result_arc_clone.blocking_lock();
                    *guard = Some(Err(anyhow::anyhow!("Runtime error: {e}")));
                    let _ = event_tx.blocking_send(EngineEvent::SubagentCompleted {
                        subagent_id: subagent_id.clone(),
                        subagent_name: subagent_name.clone(),
                        result: "error".to_string(),
                    });
                    return;
                }
            };
            rt.block_on(async {
                let mut sub_session = AgentSession::new(
                    agent,
                    shared.clone(),
                    Vec::new(),
                    Vec::new(),
                    Some(cancel_flag_clone),
                    mode,
                );
                match sub_session.resolve_provider() {
                    Ok(provider) => {
                        sub_session
                            .initialize(
                                &provider,
                                None,
                                None,
                                shared.plugins.as_ref(),
                                &shared.workspace,
                            )
                            .await
                            .ok();
                        let outcome = sub_session.process(&task_owned).await;
                        let mut guard = result_arc_clone.lock().await;
                        let (result_str, completed_result) = match outcome {
                            Ok(crate::agent::session::AgentOutcome::Completed(o)) => {
                                ("completed".to_string(), Ok(o))
                            }
                            Ok(crate::agent::session::AgentOutcome::NeedsAnswer { question }) => {
                                ("completed".to_string(), Ok(question))
                            }
                            Ok(crate::agent::session::AgentOutcome::OutOfSteps { partial }) => {
                                ("out_of_steps".to_string(), Ok(partial))
                            }
                            Ok(crate::agent::session::AgentOutcome::Cancelled) => {
                                ("error".to_string(), Ok("[Cancelled]".into()))
                            }
                            Err(e) => ("error".to_string(), Err(e.into())),
                        };
                        *guard = Some(completed_result);
                        let _ = event_tx
                            .send(EngineEvent::SubagentCompleted {
                                subagent_id: subagent_id.clone(),
                                subagent_name: subagent_name.clone(),
                                result: result_str,
                            })
                            .await;
                    }
                    Err(e) => {
                        let mut guard = result_arc_clone.lock().await;
                        *guard = Some(Err(e.into()));
                        let _ = event_tx
                            .send(EngineEvent::SubagentCompleted {
                                subagent_id: subagent_id.clone(),
                                subagent_name: subagent_name.clone(),
                                result: "error".to_string(),
                            })
                            .await;
                    }
                }
            });
        });

        // Store the cancel flag in the BackgroundTask so /stop can cancel it
        let bg_cancel_flag = cancel_flag.clone();

        // We don't have a JoinHandle available here since we use std::thread::spawn.
        // Store a placeholder handle so the BackgroundTask struct is satisfied.
        let handle = tokio::spawn(std::future::ready(()));

        let bg_task = crate::agent::types::BackgroundTask {
            task_id: task_id.clone(),
            agent_name: task_name,
            started_at: std::time::Instant::now(),
            handle,
            result: result_arc,
            cancel_flag: bg_cancel_flag,
        };
        {
            let mut mgr = self.shared.background_tasks.lock().await;
            mgr.insert(bg_task);
            mgr.cleanup();
        }
        Ok(format!(
            r#"{{"task_id": "{}", "status": "spawned"}}"#,
            task_id_str
        ))
    }

    async fn execute_check_task(&self, tc: &ToolCall) -> std::result::Result<String, String> {
        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");

        let mgr = self.shared.background_tasks.lock().await;
        match mgr.get(task_id) {
            Some(task) => {
                let result_guard = task.result.lock().await;
                match &*result_guard {
                    Some(Ok(output)) => Ok(format!(
                        r#"{{"status": "completed", "result": {}}}"#,
                        serde_json::to_string(output).unwrap_or_default()
                    )),
                    Some(Err(e)) => Ok(format!(r#"{{"status": "failed", "error": "{}"}}"#, e)),
                    None => Ok(r#"{"status": "running"}"#.to_string()),
                }
            }
            None => Ok(format!(
                r#"{{"status": "not_found", "task_id": "{}"}}"#,
                task_id
            )),
        }
    }

    /// Execute the `todo` tool: manage session tasks (add/update/delete/list).
    async fn execute_todo_tool(&self, tc: &ToolCall) -> std::result::Result<String, String> {
        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        let db = self
            .shared
            .db
            .as_ref()
            .ok_or_else(|| "No database available for todo tool".to_string())?;
        let session_id = self
            .shared
            .session_id
            .ok_or_else(|| "No active session for todo tool".to_string())?;

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
                serde_json::json!({
                    "action": "added",
                    "id": todo.id.to_string(),
                    "content": todo.content,
                    "status": todo.status,
                })
                .to_string()
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
                serde_json::json!({
                    "action": "updated",
                    "id": id.to_string(),
                })
                .to_string()
            }
            "delete" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "todo delete requires 'id'".to_string())?;
                let id = Uuid::parse_str(id).map_err(|e| format!("Invalid todo id: {e}"))?;
                db.delete_todo(id).await.map_err(|e| e.to_string())?;
                serde_json::json!({
                    "action": "deleted",
                    "id": id.to_string(),
                })
                .to_string()
            }
            _ => {
                // "list" (default) — list all todos
                let todos = db.list_todos(session_id).await.map_err(|e| e.to_string())?;
                let items: Vec<serde_json::Value> = todos
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "id": t.id.to_string(),
                            "content": t.content,
                            "status": t.status,
                            "priority": t.priority,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "action": "list",
                    "todos": items,
                })
                .to_string()
            }
        };

        // Emit the updated todo list so the TUI can refresh its sidebar.
        if let Ok(todos) = db.list_todos(session_id).await {
            let _ = self
                .shared
                .event_tx
                .send(EngineEvent::TodosUpdated(todos))
                .await;
        }

        Ok(result)
    }

    /// Execute the `question` tool: ask the user a question mid-turn.
    async fn execute_question_tool(&self, tc: &ToolCall) -> std::result::Result<String, String> {
        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "question requires 'question' field".to_string())?;
        let options: Vec<String> = args
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let recommended = args
            .get("recommended")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Build the full question text with options appended
        let mut full_question = question.to_string();
        if !options.is_empty() {
            full_question.push_str("\n\nOptions:");
            for opt in &options {
                full_question.push_str(&format!("\n  - {opt}"));
            }
        }
        if let Some(ref rec) = recommended {
            full_question.push_str(&format!("\n\nRecommended: {rec}"));
        }

        // Emit Question event so the TUI shows the question dialog
        let key = format!("{}-{}", self.agent.id, self.steps_used);
        self.emit_event(EngineEvent::Question {
            id: key,
            question: question.to_string(),
            options: options.clone(),
            recommended: recommended.clone(),
        })
        .await;

        let answer = self
            .ask_user(&full_question)
            .await
            .map_err(|e| e.to_string())?;

        let response = serde_json::json!({
            "question": question,
            "answer": answer,
        });

        Ok(response.to_string())
    }

    /// Execute the `apply_patch` tool: batch file add/update/delete operations.
    async fn execute_apply_patch_tool(&self, tc: &ToolCall) -> std::result::Result<String, String> {
        // Reject in Plan mode (read-only)
        if self.mode == AgentMode::Plan {
            return Err("apply_patch is not available in Plan mode (read-only)".to_string());
        }

        let batch = crate::engine::apply_patch::parse_patch_batch(&tc.function.arguments)
            .map_err(|e| format!("Invalid patch: {e}"))?;

        let results =
            crate::engine::apply_patch::apply_patch_batch(&self.shared.workspace, &batch, false)
                .map_err(|e| format!("Patch failed: {e}"))?;

        let response = serde_json::json!({
            "applied": true,
            "operations": results,
        });

        Ok(response.to_string())
    }

    async fn execute_builtin_tool(&self, tc: &ToolCall) -> Result<String> {
        let tool_name = &tc.function.name;

        tracing::debug!(
            target: "anacleto::agent::session",
            agent = %self.agent.name,
            tool = %tool_name,
            arguments = %tc.function.arguments,
            "execute_builtin_tool"
        );

        let _args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        let hook_ctx = HookContext {
            agent_name: Some(self.agent.name.clone()),
            tool_name: Some(tool_name.clone()),
            ..Default::default()
        };

        let _ = self
            .shared
            .hook_registry
            .run(HookPoint::BeforeTool, &hook_ctx)
            .await;

        let result = match tool_name.as_str() {
            "execute" => execute_execute_tool(&self.shared.workspace, tc).await,
            "read" => execute_read_tool(&self.shared.workspace, tc).await,
            "write" => execute_write_tool(&self.shared.workspace, tc).await,
            "insert" => execute_insert_tool(&self.shared.workspace, tc).await,
            "replace" => execute_replace_tool(&self.shared.workspace, tc).await,
            "delete" => execute_delete_tool(&self.shared.workspace, tc).await,
            "list" => execute_list_tool(&self.shared.workspace, tc).await,
            "grep" => execute_grep_tool(&self.shared.workspace, tc).await,
            "glob" => execute_glob_tool(&self.shared.workspace, tc).await,
            "websearch" => execute_websearch_tool(tc).await,
            "webfetch" => execute_webfetch_tool(tc).await,
            "format_document" => execute_format_document_tool(tc).await,
            "lsp_query" => execute_lsp_query_tool(tc).await,
            "mcp_list_resources" => match &self.mcp_registry {
                Some(reg) => execute_mcp_list_resources_tool(reg, tc).await,
                None => Ok("MCP registry not available".to_string()),
            },
            "mcp_list_resource_templates" => match &self.mcp_registry {
                Some(reg) => execute_mcp_list_resource_templates_tool(reg, tc).await,
                None => Ok("MCP registry not available".to_string()),
            },
            "mcp_read_resource" => match &self.mcp_registry {
                Some(reg) => execute_mcp_read_resource_tool(reg, tc).await,
                None => Ok("MCP registry not available".to_string()),
            },
            "spawn_background" => self.execute_spawn_background(tc).await,
            "check_task" => self.execute_check_task(tc).await,
            "todo" => self.execute_todo_tool(tc).await,
            "question" => self.execute_question_tool(tc).await,
            "apply_patch" => self.execute_apply_patch_tool(tc).await,
            "get_tool_result" => Ok(
                "[Error] get_tool_result should be handled directly in process() and never \
                 reach execute_builtin_tool. This is a bug."
                    .to_string(),
            ),
            // Safety net: delegate_to_* tools should never reach here because
            // they are intercepted in process() before execute_tool_call.
            // If they do, it's a bug — report it clearly.
            _ if tool_name.starts_with("delegate_to_") => Err(format!(
                "Delegate tool '{tool_name}' reached execute_builtin_tool — \
                 this is a bug. Delegate tools should be dispatched in process()."
            )),
            _ => Ok(format!("Unknown tool: {tool_name}")),
        };

        let _result_str = match &result {
            Ok(s) => s.clone(),
            Err(e) => e.to_string(),
        };
        let hook_ctx = HookContext {
            agent_name: Some(self.agent.name.clone()),
            tool_name: Some(tool_name.clone()),
            ..Default::default()
        };
        let _ = self
            .shared
            .hook_registry
            .run(HookPoint::AfterTool, &hook_ctx)
            .await;

        match &result {
            Ok(s) => tracing::info!(
                agent = %self.agent.name,
                tool = %tool_name,
                result_len = %s.len(),
                "Tool execution succeeded"
            ),
            Err(e) => tracing::warn!(
                agent = %self.agent.name,
                tool = %tool_name,
                error = %e,
                "Tool execution failed"
            ),
        }

        result.map_err(|e| Error::Agent(format!("Tool '{tool_name}' failed: {e}")))
    }

    async fn emit_event(&self, event: EngineEvent) {
        let _ = self.shared.event_tx.send(event).await;
    }
}

/// Run a delegate/task tool call asynchronously in a spawned task.
///
/// This is the sole implementation of subagent delegation. It creates a
/// fresh [`AgentSession`] for the subagent, runs it with a timeout, and
/// returns the subagent's output (or a timeout/error message) as a `String`.
/// All errors are captured as `String` return values so the spawned task
/// never panics.
#[allow(clippy::too_many_arguments)]
async fn run_delegate_task(
    subagent_name: String,
    arguments: String,
    subagent_configs: Vec<AgentConfig>,
    shared: Arc<AgentSharedState>,
    mode: AgentMode,
    parent_id: AgentId,
    parent_name: String,
    event_tx: tokio::sync::mpsc::Sender<EngineEvent>,
    parent_context: String,
) -> String {
    let args: serde_json::Value = serde_json::from_str(&arguments)
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");

    tracing::debug!(
        target: "anacleto::agent::session",
        subagent = %subagent_name,
        task = %task,
        "run_delegate_task started"
    );

    tracing::info!(
        subagent = %subagent_name,
        task_len = %task.len(),
        "Delegate task spawned"
    );

    let config = match subagent_configs
        .iter()
        .find(|c| c.name == subagent_name)
        .cloned()
    {
        Some(c) => c,
        None => return format!("Error: Subagent '{subagent_name}' not found in config."),
    };

    let subagent = Agent::create_subagent(
        config.name.clone(),
        config.system_prompt.clone(),
        config.model.clone(),
        config.skills.clone(),
        config.mcps.clone(),
        config.max_steps,
        parent_id.clone(),
        config.tools.clone(),
        config.writable_paths.clone(),
        config.temperature.map(|t| t as f32),
        config.max_tokens,
        config.top_p.map(|t| t as f32),
    );

    let _ = event_tx
        .send(EngineEvent::SubagentCreated {
            parent_id: parent_id.clone(),
            subagent_id: subagent.id.clone(),
            subagent_name: subagent.name.clone(),
            skills: config
                .skills
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            mcps: config.mcps.clone(),
            agent_type: Some(config.name.clone()),
        })
        .await;

    let skill_names: Vec<String> = config
        .skills
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let cancel_flag = Arc::new(AtomicBool::new(false));

    let mut sub_session = AgentSession::new(
        subagent,
        shared.clone(),
        Vec::new(),
        skill_names,
        Some(cancel_flag),
        mode,
    );

    let provider = match sub_session.resolve_provider() {
        Ok(p) => p,
        Err(e) => return format!("Subagent '{subagent_name}' provider error: {e}"),
    };

    if let Err(e) = sub_session
        .initialize(
            &provider,
            None,
            None,
            shared.plugins.as_ref(),
            &shared.workspace,
        )
        .await
    {
        return format!("Subagent '{subagent_name}' init error: {e}");
    }

    // Pass context from parent session to the subagent: last user message
    // and most recent tool outputs, so the subagent knows what has been
    // said and done before the delegation.
    if !parent_context.is_empty() {
        sub_session.conversation.push(LlmMessage {
            role: MessageRole::System,
            content: format!("[Parent session context]\n{parent_context}"),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    let _ = event_tx
        .send(EngineEvent::AgentStatusChanged {
            agent_id: parent_id.clone(),
            agent_name: parent_name.clone(),
            status: AgentStatus::WaitingForSubAgent,
        })
        .await;

    let result = sub_session.process(task).await;

    match result {
        Ok(outcome) => match outcome {
            AgentOutcome::Completed(output) => {
                tracing::info!(
                    subagent = %sub_session.agent.name,
                    result_len = %output.len(),
                    "Delegate subagent completed successfully"
                );
                let _ = event_tx
                    .send(EngineEvent::SubagentCompleted {
                        subagent_id: sub_session.agent.id.clone(),
                        subagent_name: sub_session.agent.name.clone(),
                        result: output.clone(),
                    })
                    .await;
                tracing::debug!(
                    target: "anacleto::agent::session",
                    subagent = %sub_session.agent.name,
                    result_len = %output.len(),
                    "run_delegate_task completed"
                );
                output
            }
            AgentOutcome::NeedsAnswer { question } => {
                // Ask user via shared pending_questions
                let answer = if let Some(ref pq) = shared.pending_questions {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let key = format!("{}-delegate-{}", parent_id, sub_session.agent.id);
                    {
                        let mut map = pq.lock().await;
                        map.insert(key.clone(), tx);
                    }
                    let _ = event_tx
                        .send(EngineEvent::AgentOutput {
                            agent_id: parent_id.clone(),
                            agent_name: parent_name.clone(),
                            content: format!("[NeedsAnswer] {question}"),
                        })
                        .await;
                    match rx.await {
                        Ok(answer) => answer,
                        Err(_) => return "Error: User response channel closed.".to_string(),
                    }
                } else {
                    "yes".to_string()
                };

                let resume = sub_session.process(&answer).await;

                match resume {
                    Ok(AgentOutcome::Completed(output)) => {
                        tracing::info!(
                            subagent = %sub_session.agent.name,
                            result_len = %output.len(),
                            "Delegate subagent completed successfully after resumption"
                        );
                        let _ = event_tx
                            .send(EngineEvent::SubagentCompleted {
                                subagent_id: sub_session.agent.id.clone(),
                                subagent_name: sub_session.agent.name.clone(),
                                result: output.clone(),
                            })
                            .await;
                        tracing::debug!(
                            target: "anacleto::agent::session",
                            subagent = %sub_session.agent.name,
                            result_len = %output.len(),
                            "run_delegate_task completed"
                        );
                        output
                    }
                    Ok(other) => {
                        let _ = event_tx
                            .send(EngineEvent::SubagentCompleted {
                                subagent_id: sub_session.agent.id.clone(),
                                subagent_name: sub_session.agent.name.clone(),
                                result: "error".to_string(),
                            })
                            .await;
                        format!(
                            "Subagent {} resumption: {:?}",
                            sub_session.agent.name, other
                        )
                    }
                    Err(e) => {
                        let _ = event_tx
                            .send(EngineEvent::SubagentCompleted {
                                subagent_id: sub_session.agent.id.clone(),
                                subagent_name: sub_session.agent.name.clone(),
                                result: "error".to_string(),
                            })
                            .await;
                        format!(
                            "Subagent {} failed on resumption: {e}",
                            sub_session.agent.name
                        )
                    }
                }
            }
            AgentOutcome::OutOfSteps { partial } => {
                tracing::warn!(
                    subagent = %sub_session.agent.name,
                    "Delegate subagent ran out of steps"
                );
                let _ = event_tx
                    .send(EngineEvent::SubagentCompleted {
                        subagent_id: sub_session.agent.id.clone(),
                        subagent_name: sub_session.agent.name.clone(),
                        result: "out_of_steps".to_string(),
                    })
                    .await;
                let _ = event_tx
                    .send(EngineEvent::AgentStatusChanged {
                        agent_id: parent_id,
                        agent_name: parent_name,
                        status: AgentStatus::Idle,
                    })
                    .await;
                format!(
                    "[Partial] Subagent {} out of steps.\n{partial}",
                    sub_session.agent.name
                )
            }
            AgentOutcome::Cancelled => {
                tracing::warn!(
                    subagent = %sub_session.agent.name,
                    "Delegate subagent cancelled"
                );
                let _ = event_tx
                    .send(EngineEvent::SubagentCompleted {
                        subagent_id: sub_session.agent.id.clone(),
                        subagent_name: sub_session.agent.name.clone(),
                        result: "error".to_string(),
                    })
                    .await;
                let _ = event_tx
                    .send(EngineEvent::AgentStatusChanged {
                        agent_id: parent_id,
                        agent_name: parent_name,
                        status: AgentStatus::Idle,
                    })
                    .await;
                format!("Subagent {} cancelled.", sub_session.agent.name)
            }
        },
        Err(e) => {
            tracing::error!(
                subagent = %sub_session.agent.name,
                error = %e,
                "Delegate subagent failed"
            );
            let _ = event_tx
                .send(EngineEvent::SubagentCompleted {
                    subagent_id: sub_session.agent.id.clone(),
                    subagent_name: sub_session.agent.name.clone(),
                    result: "error".to_string(),
                })
                .await;
            format!("Subagent {} failed: {e}", sub_session.agent.name)
        }
    }
}
