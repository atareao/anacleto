//! Hook system for Anacleto.
//!
//! Hooks are configurable shell commands that fire at specific points in the
//! agent lifecycle. They are defined in the YAML config and executed by the
//! engine at each hook point.
//!
//! # Hook Points
//!
//! | Hook Point | When it fires |
//! |---|---|
//! | `BeforeTool` | Before any tool execution |
//! | `AfterTool` | After any tool execution (success only) |
//! | `BeforeApply` | Before `apply_patch` batch operations |
//! | `AfterApply` | After `apply_patch` batch operations (success only) |
//! | `BeforeShell` | Before shell command execution |
//! | `AfterShell` | After shell command execution (success only) |
//! | `BeforeFsWrite` | Before filesystem write/edit/delete |
//! | `AfterFsWrite` | After filesystem write/edit/delete (success only) |
//! | `OnStartup` | Engine startup |
//! | `OnShutdown` | Engine shutdown |

pub mod autoconfig;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Config;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Points in the agent lifecycle where hooks can fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    /// Before any tool execution.
    BeforeTool,
    /// After any tool execution (success only).
    AfterTool,
    /// Before `apply_patch` batch operations.
    BeforeApply,
    /// After `apply_patch` batch operations (success only).
    AfterApply,
    /// Before shell command execution.
    BeforeShell,
    /// After shell command execution (success only).
    AfterShell,
    /// Before filesystem write/edit/delete.
    BeforeFsWrite,
    /// After filesystem write/edit/delete (success only).
    AfterFsWrite,
    /// Engine startup.
    OnStartup,
    /// Engine shutdown.
    OnShutdown,
}

/// The action a hook performs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookAction {
    /// Run a shell command.
    Shell { command: String },
}

/// A hook action with its configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookActionConfig {
    /// The action to perform.
    #[serde(flatten)]
    pub action: HookAction,
    /// Optional timeout in seconds (default: 30).
    #[serde(default = "default_hook_timeout")]
    pub timeout_secs: u64,
}

const fn default_hook_timeout() -> u64 {
    30
}

/// Contextual information passed to hook actions.
///
/// Fields are populated at runtime based on the hook point where they fire.
/// Template variables in hook commands (e.g. `{{tool_name}}`) are substituted
/// from this context before execution.
#[derive(Debug, Default)]
pub struct HookContext {
    /// Name of the tool being executed (for BeforeTool/AfterTool).
    pub tool_name: Option<String>,
    /// File path being operated on (for BeforeFsWrite/AfterFsWrite).
    pub file_path: Option<String>,
    /// The shell command being executed (for BeforeShell/AfterShell).
    pub shell_command: Option<String>,
    /// Name of the agent executing the hook.
    pub agent_name: Option<String>,
}

impl HookContext {
    /// Substitute `{{key}}` template variables in a command string.
    fn substitute(&self, command: &str) -> String {
        let mut result = command.to_string();
        if let Some(ref v) = self.tool_name {
            result = result.replace("{{tool_name}}", v);
        }
        if let Some(ref v) = self.file_path {
            result = result.replace("{{file_path}}", v);
        }
        if let Some(ref v) = self.shell_command {
            result = result.replace("{{shell_command}}", v);
        }
        if let Some(ref v) = self.agent_name {
            result = result.replace("{{agent_name}}", v);
        }
        result
    }
}

/// The result of executing a single hook action.
#[derive(Debug)]
pub struct HookResult {
    /// The command that was executed.
    pub command: String,
    /// Captured stdout (truncated to 4 KB).
    pub stdout: String,
    /// Captured stderr (truncated to 4 KB).
    pub stderr: String,
    /// Exit code, or `None` on timeout.
    pub exit_code: Option<i32>,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Registry of all configured hooks, keyed by [`HookPoint`].
///
/// Created once at engine startup from the YAML config and cloned into each
/// agent task. Hooks are fire-and-forget: their output is logged but does not
/// block or affect the agent's execution flow.
#[derive(Default, Clone)]
pub struct HookRegistry {
    hooks: Arc<HashMap<HookPoint, Vec<HookActionConfig>>>,
}

impl HookRegistry {
    /// Create a new registry from a map of hook points to actions.
    pub fn new(hooks: HashMap<HookPoint, Vec<HookActionConfig>>) -> Self {
        Self {
            hooks: Arc::new(hooks),
        }
    }

    /// Run all hooks configured for the given [`HookPoint`].
    ///
    /// Hooks run sequentially in the order they are defined in config.
    /// Each hook is fire-and-forget: errors and timeouts are logged via
    /// `tracing::warn!` but do not propagate.
    ///
    /// Returns a list of results for introspection/testing.
    pub async fn run(&self, point: HookPoint, ctx: &HookContext) -> Vec<HookResult> {
        let Some(actions) = self.hooks.get(&point) else {
            return Vec::new();
        };
        if actions.is_empty() {
            return Vec::new();
        }

        tracing::debug!("Running {} hook(s) for {:?}", actions.len(), point);

        let mut results = Vec::with_capacity(actions.len());
        for action_cfg in actions {
            let result = match &action_cfg.action {
                HookAction::Shell { command } => {
                    let substituted = ctx.substitute(command);
                    run_shell_hook(&substituted, action_cfg.timeout_secs).await
                }
            };
            if result.exit_code != Some(0) {
                tracing::warn!(
                    "Hook command '{:?}' at {:?} exited with {:?}: {}",
                    action_cfg.action,
                    point,
                    result.exit_code,
                    result.stderr,
                );
            } else {
                tracing::debug!(
                    "Hook command '{:?}' at {:?} completed (stdout: {} bytes)",
                    action_cfg.action,
                    point,
                    result.stdout.len(),
                );
            }
            results.push(result);
        }

        results
    }

    /// Returns true if no hooks are configured.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

impl From<&Config> for HookRegistry {
    fn from(config: &Config) -> Self {
        let mut hooks: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();

        for (key, actions) in &config.hooks {
            // Map config string keys to HookPoint variants
            let point = match key.as_str() {
                "before_tool" => HookPoint::BeforeTool,
                "after_tool" => HookPoint::AfterTool,
                "before_apply" => HookPoint::BeforeApply,
                "after_apply" => HookPoint::AfterApply,
                "before_shell" => HookPoint::BeforeShell,
                "after_shell" => HookPoint::AfterShell,
                "before_fs_write" => HookPoint::BeforeFsWrite,
                "after_fs_write" => HookPoint::AfterFsWrite,
                "on_startup" => HookPoint::OnStartup,
                "on_shutdown" => HookPoint::OnShutdown,
                other => {
                    tracing::warn!("Unknown hook point '{}' in config, ignoring", other);
                    continue;
                }
            };
            hooks.insert(point, actions.clone());
        }

        Self::new(hooks)
    }
}

// ---------------------------------------------------------------------------
// Hook execution helpers
// ---------------------------------------------------------------------------

/// Run a shell command and capture its output.
///
/// The command is executed via `/bin/sh -c` with a configurable timeout.
async fn run_shell_hook(command: &str, timeout_secs: u64) -> HookResult {
    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .spawn();

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return HookResult {
                command: command.to_string(),
                stdout: String::new(),
                stderr: format!("Failed to spawn hook command: {e}"),
                exit_code: None,
            };
        }
    };

    let timeout_duration = std::time::Duration::from_secs(timeout_secs);
    let output = match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return HookResult {
                command: command.to_string(),
                stdout: String::new(),
                stderr: format!("Failed to wait for hook command: {e}"),
                exit_code: None,
            };
        }
        Err(_) => {
            return HookResult {
                command: command.to_string(),
                stdout: String::new(),
                stderr: format!("Hook command timed out after {timeout_secs}s"),
                exit_code: None,
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    HookResult {
        command: command.to_string(),
        stdout: truncate_output(&stdout, 4096),
        stderr: truncate_output(&stderr, 4096),
        exit_code: output.status.code(),
    }
}

/// Truncate a string to a maximum length, adding "... [truncated]" if needed.
fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}... [truncated {} bytes]", &s[..max], s.len() - max)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_hook_point_serde() {
        let cases = [
            (HookPoint::BeforeTool, "before_tool"),
            (HookPoint::AfterTool, "after_tool"),
            (HookPoint::BeforeApply, "before_apply"),
            (HookPoint::AfterApply, "after_apply"),
            (HookPoint::BeforeShell, "before_shell"),
            (HookPoint::AfterShell, "after_shell"),
            (HookPoint::BeforeFsWrite, "before_fs_write"),
            (HookPoint::AfterFsWrite, "after_fs_write"),
            (HookPoint::OnStartup, "on_startup"),
            (HookPoint::OnShutdown, "on_shutdown"),
        ];
        for (point, expected) in &cases {
            let serialized = serde_yaml::to_string(point).unwrap();
            assert_eq!(serialized.trim(), *expected);
            let deserialized: HookPoint = serde_yaml::from_str(expected).unwrap();
            assert_eq!(deserialized, *point);
        }
    }

    #[test]
    fn test_hook_action_shell_deser() {
        let yaml = r#"
type: shell
command: "echo hello"
"#;
        let action: HookAction = serde_yaml::from_str(yaml).unwrap();
        match action {
            HookAction::Shell { command } => assert_eq!(command, "echo hello"),
        }
    }

    #[test]
    fn test_hook_action_config_deser() {
        let yaml = r#"
type: shell
command: "codegraph sync"
timeout_secs: 60
"#;
        let cfg: HookActionConfig = serde_yaml::from_str(yaml).unwrap();
        match cfg.action {
            HookAction::Shell { ref command } => assert_eq!(command, "codegraph sync"),
        }
        assert_eq!(cfg.timeout_secs, 60);
    }

    #[test]
    fn test_hook_action_config_default_timeout() {
        let yaml = r#"
type: shell
command: "echo hi"
"#;
        let cfg: HookActionConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.timeout_secs, 30);
    }

    #[test]
    fn test_hook_context_substitution() {
        let ctx = HookContext {
            tool_name: Some("test_tool".into()),
            file_path: Some("/tmp/test.txt".into()),
            shell_command: Some("ls -la".into()),
            agent_name: Some("test_agent".into()),
        };
        assert_eq!(ctx.substitute("tool: {{tool_name}}"), "tool: test_tool");
        assert_eq!(ctx.substitute("path: {{file_path}}"), "path: /tmp/test.txt");
        assert_eq!(ctx.substitute("cmd: {{shell_command}}"), "cmd: ls -la");
        assert_eq!(ctx.substitute("agent: {{agent_name}}"), "agent: test_agent");
        // Unknown vars pass through unchanged
        assert_eq!(ctx.substitute("{{unknown}}"), "{{unknown}}");
        // Multiple vars in one string
        assert_eq!(
            ctx.substitute("{{tool_name}} on {{file_path}}"),
            "test_tool on /tmp/test.txt"
        );
    }

    #[tokio::test]
    async fn test_registry_empty_is_noop() {
        let registry = HookRegistry::new(HashMap::new());
        let results = registry
            .run(HookPoint::BeforeTool, &HookContext::default())
            .await;
        assert!(results.is_empty());
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn test_registry_with_shell_command() {
        let mut hooks = HashMap::new();
        hooks.insert(
            HookPoint::OnStartup,
            vec![HookActionConfig {
                action: HookAction::Shell {
                    command: "echo 'hello world'".into(),
                },
                timeout_secs: 10,
            }],
        );
        let registry = HookRegistry::new(hooks);
        let results = registry
            .run(HookPoint::OnStartup, &HookContext::default())
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].stdout.contains("hello world"));
        assert_eq!(results[0].exit_code, Some(0));
    }

    #[tokio::test]
    async fn test_registry_failing_command() {
        let mut hooks = HashMap::new();
        hooks.insert(
            HookPoint::BeforeTool,
            vec![HookActionConfig {
                action: HookAction::Shell {
                    command: "exit 42".into(),
                },
                timeout_secs: 10,
            }],
        );
        let registry = HookRegistry::new(hooks);
        let results = registry
            .run(HookPoint::BeforeTool, &HookContext::default())
            .await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].exit_code, Some(42));
    }

    #[tokio::test]
    async fn test_registry_timeout() {
        let mut hooks = HashMap::new();
        hooks.insert(
            HookPoint::OnShutdown,
            vec![HookActionConfig {
                action: HookAction::Shell {
                    command: "sleep 10".into(),
                },
                timeout_secs: 1,
            }],
        );
        let registry = HookRegistry::new(hooks);
        let results = registry
            .run(HookPoint::OnShutdown, &HookContext::default())
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].stderr.contains("timed out"));
        assert_eq!(results[0].exit_code, None);
    }

    #[tokio::test]
    async fn test_registry_substitution() {
        let mut hooks = HashMap::new();
        hooks.insert(
            HookPoint::AfterFsWrite,
            vec![HookActionConfig {
                action: HookAction::Shell {
                    command: "echo 'wrote {{file_path}}'".into(),
                },
                timeout_secs: 10,
            }],
        );
        let registry = HookRegistry::new(hooks);
        let ctx = HookContext {
            file_path: Some("/tmp/test.md".into()),
            ..Default::default()
        };
        let results = registry.run(HookPoint::AfterFsWrite, &ctx).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].stdout.contains("/tmp/test.md"));
    }

    #[test]
    fn test_from_config_empty() {
        let config = Config::default();
        let registry = HookRegistry::from(&config);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_hook_action_config_serde_roundtrip() {
        let yaml = r#"
type: shell
command: "codegraph sync"
timeout_secs: 45
"#;
        let cfg: HookActionConfig = serde_yaml::from_str(yaml).unwrap();
        let serialized = serde_yaml::to_string(&cfg).unwrap();
        assert!(serialized.contains("codegraph sync"));
        assert!(serialized.contains("45"));
    }
}
