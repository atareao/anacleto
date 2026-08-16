use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::lifecycle::PendingQuestions;
use crate::agent::tool_store::truncate_output;
use crate::config::types::AgentConfig;
use crate::engine::orchestrator::EngineEvent;
use crate::hook::HookRegistry;
use crate::llm::types::{ToolCall, ToolDefinition};
use crate::skill::types::Skill;

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
pub(crate) async fn execute_todo_tool(
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
pub(crate) fn question_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "question".to_string(),
        description: "Ask the user a question mid-turn to resolve ambiguity. \
                       Optionally provide options and a recommended default."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string"
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "recommended": {
                    "type": "string"
                }
            },
            "required": ["question"]
        }),
    }
}

/// Execute a `question` tool call: register a pending question, emit an event
/// for the TUI to display, and await the user's answer.
pub(crate) async fn execute_question_tool(
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
pub(crate) fn apply_patch_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "apply_patch".to_string(),
        description: "Apply a batch of file changes (add/update/delete) with a single approval."
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
                                "enum": ["add", "update", "delete"]
                            },
                            "path": {
                                "type": "string"
                            },
                            "content": {
                                "type": "string"
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

/// Check if a path is allowed for write operations.
/// The workspace is always writable. Additional paths can be declared
/// in the agent's `writable_paths`.
pub fn is_write_allowed(path: &Path, workspace: &Path, writable_paths: &[PathBuf]) -> bool {
    if path.starts_with(workspace) {
        return true;
    }
    writable_paths.iter().any(|p| path.starts_with(p))
}

/// Execute an `apply_patch` tool call.
///
/// Parses the batch, validates every path (rejecting traversal), checks
/// write permissions against workspace and writable_paths, and applies
/// the changes.
pub(crate) async fn execute_apply_patch_tool(
    workspace: &Path,
    writable_paths: &[PathBuf],
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

    // Validate every path: must be within workspace or writable_paths
    for op in &batch.operations {
        let resolved = crate::engine::apply_patch::resolve_within_workspace(workspace, &op.path)
            .map_err(|e| format!("Path validation failed: {e}"))?;
        if !is_write_allowed(&resolved, workspace, writable_paths) {
            return Err(format!(
                "Write not allowed for path: {} (not in workspace or writable_paths)",
                op.path
            ));
        }
    }

    let results = crate::engine::apply_patch::apply_patch_batch(workspace, &batch, false)?;

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
) -> std::result::Result<String, String> {
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
        execute_web_fetch(task).await
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

/// Convert an `AgentConfig` to a `ToolDefinition` so the LLM can invoke a subagent.
pub(crate) fn subagent_config_to_tool_definition(config: &AgentConfig) -> ToolDefinition {
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

    let mut desc = format!(
        "Delegate a task to the '{}' subagent. What it does: {}",
        config.name, config.description
    );
    if !config.when_to_use.is_empty() {
        desc.push_str(&format!(" When to use: {}", config.when_to_use));
    }

    ToolDefinition {
        name: config.name.clone(),
        description: desc,
        input_schema,
    }
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
        let result =
            execute_skill_tool(&registry, "agent", &tool_call, &tx, &id, None, true, "").await;
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
        let result =
            execute_skill_tool(&registry, "agent", &tool_call, &tx, &id, None, true, "").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("nonexistent"));
    }

    #[test]
    fn test_subagent_config_to_tool_definition_with_when_to_use() {
        let config = AgentConfig {
            name: "documenter".into(),
            description: "Documenta acciones".into(),
            when_to_use: "Tras cada tool call".into(),
            role: AgentRole::SubAgent,
            model: "m".into(),
            skills: vec![],
            mcps: vec![],
            subagents: vec![],
            system_prompt: "".into(),
            max_steps: 60,
            tools: vec![],
            writable_paths: vec![],
        };
        let def = subagent_config_to_tool_definition(&config);
        assert!(def.description.contains("Documenta acciones"));
        assert!(def.description.contains("Tras cada tool call"));
        assert_eq!(def.name, "documenter");
    }

    #[test]
    fn test_subagent_config_to_tool_definition_without_when_to_use() {
        let config = AgentConfig {
            name: "reviewer".into(),
            description: "Revisa código".into(),
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
        };
        let def = subagent_config_to_tool_definition(&config);
        assert!(def.description.contains("Revisa código"));
        assert!(!def.description.contains("When to use"));
        assert_eq!(def.name, "reviewer");
    }
}
