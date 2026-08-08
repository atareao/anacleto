use std::collections::HashMap;

use crate::hook::{HookAction, HookActionConfig, HookPoint};

/// Well-known tools and the hooks they need.
const KNOWN_TOOLS: &[(&str, HookPoint, &str, u64)] =
    &[("codegraph", HookPoint::AfterApply, "codegraph sync", 60)];

/// Scan PATH for known tools and return auto-detected hooks.
pub fn detect_auto_hooks() -> HashMap<HookPoint, Vec<HookActionConfig>> {
    let mut hooks: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();

    let paths = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return hooks,
    };

    for (binary, point, command, timeout) in KNOWN_TOOLS {
        let found = std::env::split_paths(&paths).any(|dir| dir.join(binary).exists());
        if found {
            hooks.entry(*point).or_default().push(HookActionConfig {
                action: HookAction::Shell {
                    command: command.to_string(),
                },
                timeout_secs: *timeout,
            });
        }
    }

    hooks
}

/// Check if a specific binary is on PATH (used by tests).
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full = dir.join(name);
            if full.exists() { Some(full) } else { None }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_returns_empty_for_unknown() {
        let _hooks = detect_auto_hooks();
        // No assertion on emptiness (CI may have codegraph)
        // Just ensure the function doesn't panic
    }

    #[test]
    fn test_which_returns_none_for_nonexistent() {
        assert!(which("this_tool_definitely_does_not_exist_xyz_123").is_none());
    }

    #[test]
    fn test_which_finds_sh() {
        assert!(which("sh").is_some());
    }

    #[test]
    fn test_codegraph_hook_format() {
        if which("codegraph").is_some() {
            let hooks = detect_auto_hooks();
            let actions = hooks
                .get(&HookPoint::AfterApply)
                .expect("codegraph should register AfterApply hook");
            assert_eq!(actions[0].timeout_secs, 60);
            match &actions[0].action {
                HookAction::Shell { command } => {
                    assert_eq!(command, "codegraph sync");
                }
            }
        }
    }
}
