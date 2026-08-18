use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::agent::tool_store::truncate_output;
use crate::config::types::AgentConfig;
use crate::engine::orchestrator::EngineEvent;
use crate::hook::HookRegistry;
use crate::llm::types::{ToolCall, ToolDefinition};
use crate::skill::types::Skill;
use crate::tools::delete::delete_tool_definition;
use crate::tools::execute::execute_tool_definition;
use crate::tools::format::format_document_tool_definition;
use crate::tools::glob::glob_tool_definition;
use crate::tools::grep::grep_tool_definition;
use crate::tools::insert::insert_tool_definition;
use crate::tools::list::list_tool_definition;
use crate::tools::lsp::lsp_query_tool_definition;
use crate::tools::mcp::{
    mcp_list_resource_templates_tool_definition, mcp_list_resources_tool_definition,
    mcp_read_resource_tool_definition,
};
use crate::tools::read::read_tool_definition;
use crate::tools::replace::replace_tool_definition;
use crate::tools::search_symbol::search_symbol_tool_definition;
use crate::tools::web::{webfetch_tool_definition, websearch_tool_definition};
use crate::tools::write::write_tool_definition;

// ---------------------------------------------------------------------------
// Subagent outcome type
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Skill-to-tool conversion and execution
// ---------------------------------------------------------------------------

/// Convert a `Skill` to a `ToolDefinition` that can be passed to an LLM.
pub(crate) fn skill_to_tool_definition(skill: &Skill) -> ToolDefinition {
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

    tool
}

/// Built-in `todo` tool definition: lets the model manage a persisted task list.
pub(crate) fn todo_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "todo".to_string(),
        description: "Manage session tasks: add, update (status/priority/content), delete, list. \
                       Status: pending, in_progress, completed, cancelled."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "update", "delete", "list"]
                },
                "content": {
                    "type": "string"
                },
                "id": {
                    "type": "string"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "cancelled"]
                },
                "priority": {
                    "type": "string"
                }
            },
            "required": ["action"]
        }),
    }
}

/// Execute a `todo` tool call against the database.
/// Execute a tool call against a matching skill and return the result as a string.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_skill_tool(
    registry: &crate::skill::registry::SkillRegistry,
    agent_name: &str,
    tool_call: &ToolCall,
    event_tx: &mpsc::Sender<EngineEvent>,
    agent_id: &crate::agent::types::AgentId,
    _hook_registry: Option<&HookRegistry>,
    show: bool,
    task_preview: &str,
    already_loaded: bool,
) -> std::result::Result<String, String> {
    tracing::info!(
        agent = %agent_name,
        skill = %tool_call.function.name,
        "Skill tool execution started"
    );

    // Find the skill by name
    let skill = registry.get(&tool_call.function.name).ok_or_else(|| {
        format!(
            "Skill '{}' not found in agent's skills",
            tool_call.function.name
        )
    })?;

    // Extract the task argument from the JSON string
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse tool call arguments: {e}"))?;
    let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");

    tracing::debug!(
        target: "anacleto::tools",
        agent = %agent_name,
        skill = %tool_call.function.name,
        task = %task,
        "Skill tool execution"
    );

    // Emit tool execution tracing event
    if show {
        let _ = event_tx
            .send(EngineEvent::ToolExecution {
                agent_id: agent_id.clone(),
                agent_name: agent_name.to_string(),
                tool_name: skill.name.clone(),
                task: task_preview.to_string(),
            })
            .await;
    }

    // Execute the tool and capture result
    let skill_name_lower = skill.name.to_lowercase();
    let result = if skill_name_lower.contains("web") || skill_name_lower.contains("research") {
        // Dynamic skills (web fetch/search) must run on every call: each
        // invocation produces new data, so there is nothing to deduplicate.
        execute_web_fetch(task).await
    } else if already_loaded {
        // The skill instructions were already delivered to the LLM on a
        // previous call in this session. Re-sending them would duplicate
        // tokens in the conversation history, so just acknowledge and pass
        // the new task through.
        Ok(format!(
            r#"✅ Skill "{}" ya cargada previamente. Nueva tarea: {}.

Ejecuta la tarea siguiendo las instrucciones de la skill ya presentes en la conversación."#,
            skill.name, task
        ))
    } else {
        Ok(format!(
            r#"📋 Loaded skill "{}".

{}

Original task: {}"#,
            skill.name, skill.instructions, task
        ))
    };

    // Emit tool result tracing event
    let summary = match &result {
        Ok(r) => truncate_output(r, 5000),
        Err(e) => e.clone(),
    };
    if show {
        let _ = event_tx
            .send(EngineEvent::ToolResult {
                agent_id: agent_id.clone(),
                agent_name: agent_name.to_string(),
                tool_name: skill.name.clone(),
                success: result.is_ok(),
                summary,
            })
            .await;
    }

    match &result {
        Ok(_) => tracing::info!(
            agent = %agent_name,
            skill = %tool_call.function.name,
            "Skill tool execution succeeded"
        ),
        Err(e) => tracing::warn!(
            agent = %agent_name,
            skill = %tool_call.function.name,
            error = %e,
            "Skill tool execution failed"
        ),
    }

    tracing::debug!(
        target: "anacleto::tools",
        agent = %agent_name,
        skill = %tool_call.function.name,
        success = %result.is_ok(),
        "Skill tool result"
    );

    result
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
        // No explicit URL in the task: fall back to a web search so that
        // general research tasks (e.g. "what will the weather be tomorrow")
        // still return useful results instead of failing.
        return crate::tools::web::web_search(trimmed).await;
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

/// Build one `ToolDefinition` per subagent, named `delegate_to_<name>`.
///
/// Each subagent gets its own tool with a `task`-only schema (no `name` enum),
/// so the LLM sees each subagent as an independent capability.
pub(crate) fn subagent_tool_definitions(subagent_configs: &[AgentConfig]) -> Vec<ToolDefinition> {
    subagent_configs
        .iter()
        .map(|sc| {
            let tool_name = format!("delegate_to_{}", sc.name);
            let mut description = format!(
                "Delegate a task to **{}** — {}",
                sc.name, sc.description
            );
            if !sc.when_to_use.is_empty() {
                description.push_str(&format!(" ({})", sc.when_to_use));
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
            description.push_str(&format!(
                "\n  - Tools: {tools_list}\n  - Skills: {skills_list}\n  - MCPs: {mcps_list}\n  - Max steps: {}\n  - Model: {}",
                sc.max_steps, sc.model
            ));

            ToolDefinition {
                name: tool_name,
                description,
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": format!("Task to delegate to {}", sc.name)
                        }
                    },
                    "required": ["task"]
                }),
            }
        })
        .collect()
}

/// Build the map of built-in tool definitions (name → ToolDefinition).
/// These are the tools available to every agent regardless of skills.
/// Tool definition for `get_tool_result`: retrieves a full tool result from ToolOutputStore.
pub fn get_tool_result_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "get_tool_result".to_string(),
        description: "Retrieve the full stored output of a previous tool call by its ID.\
                       Use this when a tool result was too large and was summarized in \
                       the conversation. The tool_call_id is shown in the summary (e.g., \
                       'call_abc123' or similar). Call this tool with that exact ID to \
                       retrieve the complete content."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "tool_call_id": {
                    "type": "string",
                    "description": "The tool_call_id shown in the summarized result. Pass it exactly as displayed."
                }
            },
            "required": ["tool_call_id"]
        }),
    }
}
pub(crate) fn question_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "question".to_string(),
        description: "Ask the user a question and wait for their answer. Use this when you need \
             clarification, confirmation, or additional information from the user."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional list of answer options"
                },
                "recommended": {
                    "type": "string",
                    "description": "Optional recommended answer"
                }
            },
            "required": ["question"]
        }),
    }
}

/// Tool definition for `apply_patch`: applies file patches.
pub(crate) fn apply_patch_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "apply_patch".to_string(),
        description: "Apply a batch of file operations (create, update, rename, delete) \
                       to the workspace. Each operation specifies a file path and the \
                       content or changes to apply."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "operations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {"type": "string", "enum": ["create", "update", "rename", "delete"]},
                            "path": {"type": "string"},
                            "content": {"type": "string"},
                            "old_path": {"type": "string"},
                            "start": {"type": "integer"},
                            "end": {"type": "integer"}
                        },
                        "required": ["op", "path"]
                    }
                }
            },
            "required": ["operations"]
        }),
    }
}

/// Tool definition for `spawn_background`: launches a subagent in the background.
pub(crate) fn spawn_background_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "spawn_background".to_string(),
        description: "Launch a subagent task that runs in the background. \
                       Returns a task_id that can be queried with check_task. \
                       The task continues even while the calling agent does other work."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "A descriptive name for the background task"
                },
                "task": {
                    "type": "string",
                    "description": "The task description for the background agent"
                }
            },
            "required": ["name", "task"]
        }),
    }
}

/// Tool definition for `check_task`: queries the status of a background task.
pub(crate) fn check_task_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "check_task".to_string(),
        description: "Check the status of a previously spawned background task. \
                       Returns 'running', 'completed: <result>', or 'failed: <error>'."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task_id returned by spawn_background"
                }
            },
            "required": ["task_id"]
        }),
    }
}

pub fn builtin_tool_definitions() -> HashMap<String, ToolDefinition> {
    let mut map = HashMap::new();
    for def in [
        todo_tool_definition(),
        question_tool_definition(),
        apply_patch_tool_definition(),
        spawn_background_tool_definition(),
        check_task_tool_definition(),
        get_tool_result_tool_definition(),
        read_tool_definition(),
        write_tool_definition(),
        insert_tool_definition(),
        replace_tool_definition(),
        delete_tool_definition(),
        list_tool_definition(),
        grep_tool_definition(),
        glob_tool_definition(),
        webfetch_tool_definition(),
        websearch_tool_definition(),
        mcp_list_resources_tool_definition(),
        mcp_read_resource_tool_definition(),
        mcp_list_resource_templates_tool_definition(),
        lsp_query_tool_definition(),
        format_document_tool_definition(),
        search_symbol_tool_definition(),
        execute_tool_definition(),
    ] {
        map.insert(def.name.clone(), def);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::AgentRole;

    #[test]
    fn test_skill_to_tool_definition() {
        let skill = Skill {
            name: "code-review".into(),
            description: "Reviews code for quality".into(),
            instructions: "Check for correctness, performance, style.".into(),
            metadata: Default::default(),
            hooks: Default::default(),
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
        let mut registry = crate::skill::registry::SkillRegistry::new();
        registry.insert(Skill {
            name: "test-skill".into(),
            description: "A test skill".into(),
            instructions: "Do the thing.".into(),
            metadata: Default::default(),
            hooks: Default::default(),
        });
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
        let result = execute_skill_tool(
            &registry, "agent", &tool_call, &tx, &id, None, true, "", false,
        )
        .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("test-skill"));
        assert!(output.contains("Do the thing."));
        assert!(output.contains("test the thing"));
    }

    #[tokio::test]
    async fn test_execute_skill_tool_not_found() {
        let registry = crate::skill::registry::SkillRegistry::new();
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
        let result = execute_skill_tool(
            &registry, "agent", &tool_call, &tx, &id, None, true, "", false,
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("nonexistent"));
    }

    #[test]
    fn test_delegate_tool_definition_lists_subagents() {
        let configs = vec![
            AgentConfig {
                name: "reviewer".into(),
                description: "Revisa código".into(),
                when_to_use: "Después de escribir".into(),
                role: AgentRole::SubAgent,
                model: "m".into(),
                skills: vec![],
                mcps: vec![],
                subagents: vec![],
                system_prompt: "".into(),
                max_steps: 60,
                tools: vec![],
                writable_paths: vec![],
                temperature: None,
                max_tokens: None,
                top_p: None,
            },
            AgentConfig {
                name: "writer".into(),
                description: "Escribe código".into(),
                when_to_use: "".into(),
                role: AgentRole::SubAgent,
                model: "m".into(),
                skills: vec![],
                mcps: vec![],
                subagents: vec![],
                system_prompt: "".into(),
                max_steps: 60,
                tools: vec![],
                writable_paths: vec![],
                temperature: None,
                max_tokens: None,
                top_p: None,
            },
        ];
        let defs = subagent_tool_definitions(&configs);
        assert_eq!(defs.len(), 2);

        // Verify each tool definition
        for def in &defs {
            assert!(def.name.starts_with("delegate_to_"));
            assert!(def.input_schema["properties"]["task"].is_object());
            // No "name" property — the subagent is identified by the tool name
            assert!(def.input_schema["properties"]["name"].is_null());
        }

        // Check specific subagent tools
        let reviewer_def = defs
            .iter()
            .find(|d| d.name == "delegate_to_reviewer")
            .unwrap();
        assert!(reviewer_def.description.contains("Revisa código"));
        assert!(reviewer_def.description.contains("Después de escribir"));

        let writer_def = defs
            .iter()
            .find(|d| d.name == "delegate_to_writer")
            .unwrap();
        assert!(writer_def.description.contains("Escribe código"));
    }
}
