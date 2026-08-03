//! Shell detection and modern tool inventory.
//!
//! This module detects the user's shell (from `$SHELL`, falling back to `sh`)
//! and inventories which modern Rust CLI tools are available on the system.
//! The inventory is computed once and cached so the agent never has to ask
//! repeatedly about tool availability.
//!
//! The built-in catalog (`default_tools`) can be overridden or extended at
//! startup via [`init`], typically from the `shell.tools` section of the
//! configuration. Overrides replace a built-in tool by name, and new tools are
//! appended.

use std::path::Path;
use std::sync::OnceLock;

/// Information about the detected user shell.
#[derive(Debug, Clone)]
pub struct ShellInfo {
    /// Shell name, e.g. "bash", "zsh", "fish", or "sh" for the fallback.
    pub name: String,
    /// Full path to the shell binary, e.g. "/bin/bash" or "/bin/sh".
    pub path: String,
}

/// Metadata describing a modern CLI tool and its classic GNU counterpart.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    /// Modern tool name, e.g. "bat".
    pub name: String,
    /// Classic counterpart, e.g. "cat". Empty string if none.
    pub classic: String,
    /// Short description of the modern tool.
    pub description: String,
}

impl ToolInfo {
    /// Build a `ToolInfo` from owned strings.
    pub fn new(
        name: impl Into<String>,
        classic: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            classic: classic.into(),
            description: description.into(),
        }
    }
}

/// The built-in catalog of modern Rust CLI tools and the classic tools they
/// replace. Used as the default when no configuration overrides are provided.
pub fn default_tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo::new("bat", "cat", "view files with syntax highlighting"),
        ToolInfo::new(
            "lsd",
            "ls",
            "modern directory listing with icons and git status",
        ),
        ToolInfo::new("fd", "find", "fast and user-friendly file search"),
        ToolInfo::new("rg", "grep", "recursively search file contents"),
        ToolInfo::new("sd", "sed", "intuitive find-and-replace"),
        ToolInfo::new("procs", "ps", "modern process viewer"),
        ToolInfo::new("duf", "df", "disk usage and free space viewer"),
        ToolInfo::new("dust", "du", "intuitive disk usage viewer"),
        ToolInfo::new("jq", "", "process JSON on the command line"),
        ToolInfo::new("yq", "", "process YAML on the command line"),
        ToolInfo::new("fzf", "", "fuzzy finder for interactive selection"),
        ToolInfo::new("hyperfine", "", "benchmark commands"),
        ToolInfo::new("watchexec", "", "re-run commands on file changes"),
        ToolInfo::new("tldr", "man", "simplified community-driven man pages"),
    ]
}

/// Merge configuration overrides into the default catalog.
///
/// An override replaces the built-in tool with the same `name`; otherwise it is
/// appended as a new tool.
pub fn merge_tools(defaults: Vec<ToolInfo>, overrides: &[ToolInfo]) -> Vec<ToolInfo> {
    let mut result = defaults;
    for o in overrides {
        if let Some(pos) = result.iter().position(|t| t.name == o.name) {
            result[pos] = o.clone();
        } else {
            result.push(o.clone());
        }
    }
    result
}

/// The detected shell and the inventory of available/missing modern tools.
#[derive(Debug, Clone)]
pub struct ToolInventory {
    /// Detected user shell.
    pub shell: ShellInfo,
    /// The full (merged) catalog of modern tools.
    pub tools: Vec<ToolInfo>,
    /// Names of modern tools that are available on the system.
    pub available: Vec<String>,
    /// Names of modern tools that are missing from the system.
    pub missing: Vec<String>,
}

impl ToolInventory {
    /// Detect the shell and which of the given tools are available.
    pub fn detect(tools: Vec<ToolInfo>) -> Self {
        let shell = detect_shell();
        let mut available = Vec::new();
        let mut missing = Vec::new();

        for tool in &tools {
            if tool_is_available(&tool.name) {
                available.push(tool.name.clone());
            } else {
                missing.push(tool.name.clone());
            }
        }

        Self {
            shell,
            tools,
            available,
            missing,
        }
    }

    /// Format the inventory as a prompt for the agent's context.
    pub fn to_prompt(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Shell: {} ({})\n\n",
            self.shell.name, self.shell.path
        ));

        out.push_str("Available modern tools (prefer these over classic GNU tools):\n");
        for tool in &self.tools {
            if !self.available.contains(&tool.name) {
                continue;
            }
            if tool.classic.is_empty() {
                out.push_str(&format!("- {} — {}\n", tool.name, tool.description));
            } else {
                out.push_str(&format!(
                    "- {} (replaces {}) — {}\n",
                    tool.name, tool.classic, tool.description
                ));
            }
        }

        if self.missing.is_empty() {
            out.push_str("\nAll modern tools available.\n");
        } else {
            out.push_str("\nNot available (use classic equivalents): ");
            out.push_str(&self.missing.join(", "));
            out.push('\n');
        }

        out
    }
}

/// Detect the user's shell from `$SHELL`, falling back to `sh`.
pub fn detect_shell() -> ShellInfo {
    match std::env::var("SHELL") {
        Ok(path) if !path.is_empty() => {
            let name = Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "sh".to_string());
            ShellInfo { name, path }
        }
        _ => ShellInfo {
            name: "sh".to_string(),
            path: "/bin/sh".to_string(),
        },
    }
}

/// Check whether a tool is available on the system via `command -v`.
fn tool_is_available(name: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {name} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Lazily-initialized, cached tool inventory.
static INVENTORY: OnceLock<ToolInventory> = OnceLock::new();

/// Initialize the cached inventory with configuration overrides.
///
/// Should be called once at startup, after the configuration is loaded. If
/// never called, [`inventory`] falls back to the built-in catalog.
pub fn init(overrides: &[ToolInfo]) {
    let tools = merge_tools(default_tools(), overrides);
    let _ = INVENTORY.set(ToolInventory::detect(tools));
}

/// Return the cached tool inventory, detecting it on first use.
pub fn inventory() -> &'static ToolInventory {
    INVENTORY.get_or_init(|| ToolInventory::detect(default_tools()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_shell_fallback() {
        let original = std::env::var("SHELL").ok();
        unsafe { std::env::remove_var("SHELL") };
        let shell = detect_shell();
        assert_eq!(shell.name, "sh");
        assert_eq!(shell.path, "/bin/sh");
        // Restore the environment.
        match original {
            Some(v) => unsafe { std::env::set_var("SHELL", v) },
            None => unsafe { std::env::remove_var("SHELL") },
        }
    }

    #[test]
    fn test_detect_shell_from_env() {
        let original = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", "/usr/bin/zsh") };
        let shell = detect_shell();
        assert_eq!(shell.name, "zsh");
        assert_eq!(shell.path, "/usr/bin/zsh");
        // Restore the environment.
        match original {
            Some(v) => unsafe { std::env::set_var("SHELL", v) },
            None => unsafe { std::env::remove_var("SHELL") },
        }
    }

    #[test]
    fn test_to_prompt_contains_shell() {
        let inv = ToolInventory::detect(default_tools());
        assert!(inv.to_prompt().contains("Shell:"));
    }

    #[test]
    fn test_to_prompt_lists_tools() {
        let inv = ToolInventory::detect(default_tools());
        let prompt = inv.to_prompt();
        assert!(prompt.contains("bat"));
        assert!(prompt.contains("rg"));
    }

    #[test]
    fn test_merge_tools_overrides_by_name() {
        let defaults = default_tools();
        let override_tool = ToolInfo::new("lsd", "ls", "custom lsd description");
        let merged = merge_tools(defaults, &[override_tool]);
        let lsd = merged.iter().find(|t| t.name == "lsd").unwrap();
        assert_eq!(lsd.description, "custom lsd description");
        // No duplicates.
        assert_eq!(merged.iter().filter(|t| t.name == "lsd").count(), 1);
    }

    #[test]
    fn test_merge_tools_appends_new() {
        let defaults = default_tools();
        let new_tool = ToolInfo::new("delta", "diff", "modern diff viewer");
        let merged = merge_tools(defaults, &[new_tool]);
        assert!(merged.iter().any(|t| t.name == "delta"));
    }
}
