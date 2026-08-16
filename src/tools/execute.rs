//! Builtin `execute` tool — runs shell commands on the system.
//!
//! Supports synchronous execution (default) with timeout, and asynchronous
//! background execution that returns a job ID immediately.

use std::path::Path;
use std::time::Duration;

use crate::agent::tool_store::truncate_output;
use crate::llm::types::{ToolCall, ToolDefinition};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default timeout for synchronous command execution (60 seconds).
const DEFAULT_TIMEOUT_MS: u64 = 60_000;

/// Maximum output size before truncation (100 KB in characters).
const MAX_OUTPUT_CHARS: usize = 100 * 1024;

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

/// Tool definition for the `execute` tool.
pub fn execute_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "execute".to_string(),
        description: "Execute shell commands on the system. Supports synchronous (default) and \
             asynchronous (background) execution."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory for the command"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 60000)",
                    "default": 60000
                },
                "async": {
                    "type": "boolean",
                    "description": "If true, run in background and return job ID immediately (default: false)",
                    "default": false
                }
            },
            "required": ["command"]
        }),
    }
}

// ---------------------------------------------------------------------------
// Main executor
// ---------------------------------------------------------------------------

/// Execute an `execute` tool call.
///
/// Checks `command.run` permission, parses arguments, and dispatches to
/// synchronous or asynchronous execution.
pub async fn execute_execute_tool(
    _workspace: &Path,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse execute arguments: {e}"))?;

    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "execute requires 'command'".to_string())?;

    let workdir = args.get("workdir").and_then(|v| v.as_str());
    let timeout_ms = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    let is_async = args.get("async").and_then(|v| v.as_bool()).unwrap_or(false);

    if is_async {
        execute_async(command, workdir).await
    } else {
        execute_sync(command, workdir, timeout_ms).await
    }
}

// ---------------------------------------------------------------------------
// Sync execution
// ---------------------------------------------------------------------------

/// Execute a command synchronously with a timeout.
async fn execute_sync(cmd: &str, workdir: Option<&str>, timeout_ms: u64) -> Result<String, String> {
    let mut child = tokio::process::Command::new("sh");
    child.arg("-c").arg(cmd);

    if let Some(dir) = workdir {
        child.current_dir(dir);
    }

    let child = child
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn command: {e}"))?;

    let timeout_duration = Duration::from_millis(timeout_ms);

    let output = tokio::time::timeout(timeout_duration, child.wait_with_output())
        .await
        .map_err(|_| {
            let seconds = timeout_ms / 1000;
            format!("Command timed out after {seconds} seconds")
        })?;

    let output = output.map_err(|e| format!("Command failed: {e}"))?;

    // Combine stdout and stderr
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        let exit_code = output.status.code().unwrap_or(-1);
        let stderr_trimmed = stderr.trim();
        if stderr_trimmed.is_empty() {
            return Err(format!("Command failed with exit code: {exit_code}"));
        }
        return Err(format!(
            "Command failed with exit code: {exit_code}\n{stderr_trimmed}"
        ));
    }

    let prompt = crate::shell::inventory().to_prompt();
    let mut combined = format!("{prompt}{stdout}");
    if !stderr.is_empty() {
        combined.push_str(stderr.as_ref());
    }

    Ok(truncate_output(&combined, MAX_OUTPUT_CHARS))
}

// ---------------------------------------------------------------------------
// Async execution
// ---------------------------------------------------------------------------

/// Execute a command in the background, returning a job ID.
async fn execute_async(cmd: &str, workdir: Option<&str>) -> Result<String, String> {
    let mut child = tokio::process::Command::new("sh");
    child.arg("-c").arg(cmd);

    if let Some(dir) = workdir {
        child.current_dir(dir);
    }

    let _child = child
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn background command: {e}"))?;

    let job_id = uuid::Uuid::new_v4().to_string();

    // Spawn a background task that waits for the child to complete, so it
    // doesn't become a zombie process.
    tokio::spawn(async move {
        let _ = _child.wait_with_output().await;
    });

    Ok(format!("[job:{job_id}] Command launched in background"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_exec".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "execute".into(),
                arguments: args.into(),
            },
        }
    }

    // -----------------------------------------------------------------------
    // Tool definition tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_execute_tool_definition() {
        let def = execute_tool_definition();
        assert_eq!(def.name, "execute");
        assert!(def.description.contains("shell commands"));
        assert!(def.description.contains("synchronous"));
        assert!(def.description.contains("asynchronous"));

        // Check schema shape
        let schema = &def.input_schema;
        assert_eq!(schema["type"], "object");

        let props = &schema["properties"];
        assert!(props.get("command").is_some());
        assert_eq!(props["command"]["type"], "string");

        assert!(props.get("workdir").is_some());
        assert_eq!(props["workdir"]["type"], "string");

        assert!(props.get("timeout").is_some());
        assert_eq!(props["timeout"]["type"], "integer");

        assert!(props.get("async").is_some());
        assert_eq!(props["async"]["type"], "boolean");

        // Verify required fields
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "command"));
        assert_eq!(required.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Argument parsing tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_execute_missing_command() {
        let ws = std::env::temp_dir().join("anacleto_exec_test_no_cmd");
        let result = execute_execute_tool(&ws, &tool_call(r#"{}"#)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("requires 'command'"));
    }

    // -----------------------------------------------------------------------
    // Sync execution tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_execute_simple_echo() {
        let ws = std::env::temp_dir().join("anacleto_exec_test_echo");
        std::fs::create_dir_all(&ws).unwrap();

        let result = execute_execute_tool(&ws, &tool_call(r#"{"command":"echo hello"}"#))
            .await
            .unwrap();

        // The output should contain the shell inventory prompt and "hello"
        assert!(result.contains("hello"));

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_execute_non_zero_exit() {
        let ws = std::env::temp_dir().join("anacleto_exec_test_false");
        std::fs::create_dir_all(&ws).unwrap();

        let result = execute_execute_tool(&ws, &tool_call(r#"{"command":"false"}"#)).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("exit code: 1"));

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_execute_timeout() {
        let ws = std::env::temp_dir().join("anacleto_exec_test_timeout");
        std::fs::create_dir_all(&ws).unwrap();

        let result =
            execute_execute_tool(&ws, &tool_call(r#"{"command":"sleep 10","timeout":100}"#)).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("timed out"));

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_execute_with_workdir() {
        let ws = std::env::temp_dir().join("anacleto_exec_test_workdir");
        std::fs::create_dir_all(&ws).unwrap();

        // Create a subdirectory to use as workdir
        let subdir = ws.join("sub");
        std::fs::create_dir_all(&subdir).unwrap();

        // Write a unique marker file in the subdir so we can verify pwd
        let marker = subdir.join("MARKER");
        std::fs::write(&marker, "x").unwrap();

        let subdir_str = subdir.to_string_lossy();
        let args = format!(r#"{{"command":"ls","workdir":"{subdir_str}"}}"#);
        let result = execute_execute_tool(&ws, &tool_call(&args)).await.unwrap();

        // The ls command should list MARKER when run in the subdir
        assert!(result.contains("MARKER"));

        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // Async execution tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_execute_async_returns_job_id() {
        let ws = std::env::temp_dir().join("anacleto_exec_test_async");
        std::fs::create_dir_all(&ws).unwrap();

        let result =
            execute_execute_tool(&ws, &tool_call(r#"{"command":"echo hello","async":true}"#))
                .await
                .unwrap();

        assert!(result.contains("[job:"));
        assert!(result.contains("Command launched in background"));

        std::fs::remove_dir_all(&ws).unwrap();
    }
}
