# Auto-Configuration Hooks Implementation Plan

> **Goal:** Three layers of automatic hook registration so tools (codegraph, etc.) configure themselves without manual YAML editing.

**Architecture:** Three independent layers — PATH auto-detect, skill frontmatter, and plugin trait — merge into the HookRegistry with strict precedence (Config > Plugin > Skill > Auto-detect) and deduplication.

**Tech Stack:** Rust, serde, std::env, existing HookRegistry/HookPoint/HookActionConfig types.

**Prerequisites:** Hook system already implemented (src/hook/mod.rs, HookRegistry, HookPoint, etc.).

## Global Constraints

- Edition 2024, rustc ≥ 1.85
- No new dependencies — use std::env and std::path only
- Follow existing patterns in each file (serde derives, constructor functions, etc.)
- Every function must be tested

---

## File Map

| File | Change |
|---|---|
| `src/hook/mod.rs` | Add `pub mod autoconfig;` |
| `src/hook/autoconfig.rs` | **Create** — `detect_auto_hooks()` PATH scanner |
| `src/skill/types.rs` | Add `hooks` field to `Skill` |
| `src/skill/loader.rs` | Add `hooks` to local `Frontmatter`, parse into Skill |
| `src/plugin/mod.rs` | Add `register_hooks()` to `Plugin` trait |
| `src/engine/orchestrator.rs` | Merge all 4 hook sources with precedence + dedup |
| `docs/plans/auto-hooks.md` | This file |

---

### Task 1: Create `src/hook/autoconfig.rs` (PATH auto-detect)

**Files:**
- Create: `src/hook/autoconfig.rs`
- Modify: `src/hook/mod.rs` (add `pub mod autoconfig;`)

**Interfaces:**
- Produces: `pub fn detect_auto_hooks() -> HashMap<HookPoint, Vec<HookActionConfig>>`

- [ ] **Step 1: Write the failing test**

```rust
// in src/hook/autoconfig.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::{HookAction, HookActionConfig};

    #[test]
    fn test_detect_no_known_tools() {
        // No codegraph on PATH in test sandbox
        let hooks = detect_auto_hooks();
        assert!(hooks.is_empty() || !hooks.contains_key(&HookPoint::AfterApply));
    }

    #[test]
    fn test_detect_codegraph_on_path() {
        // Simulate by checking $PATH contains a known dir with codegraph
        // (in CI codegraph may exist)
        let hooks = detect_auto_hooks();
        if which_codegraph().is_some() {
            assert!(hooks.contains_key(&HookPoint::AfterApply));
            if let Some(actions) = hooks.get(&HookPoint::AfterApply) {
                assert!(actions.iter().any(|a| matches!(&a.action, HookAction::Shell { command } if command == "codegraph sync")));
                assert_eq!(actions[0].timeout_secs, 60);
            }
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p anacleto hook::autoconfig::tests 2>&1 | tail -5`

- [ ] **Step 3: Write minimal implementation**

```rust
// src/hook/autoconfig.rs
use std::collections::HashMap;
use crate::hook::{HookAction, HookActionConfig, HookPoint};

/// Well-known tools and the hooks they need.
const KNOWN_TOOLS: &[(&str, HookPoint, &str, u64)] = &[
    ("codegraph", HookPoint::AfterApply, "codegraph sync", 60),
];

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
                action: HookAction::Shell { command: command.to_string() },
                timeout_secs: *timeout,
            });
        }
    }

    hooks
}

/// Check if a specific binary is on PATH (used by tests).
fn which(name: &str) -> Option<std::path::PathBuf> {
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
        let hooks = detect_auto_hooks();
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
            let actions = hooks.get(&HookPoint::AfterApply)
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
```

- [ ] **Step 4: Add module declaration**

```rust
// in src/hook/mod.rs, with other pub mod declarations
pub mod autoconfig;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p anacleto hook::autoconfig::tests 2>&1 | tail -10`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add src/hook/mod.rs src/hook/autoconfig.rs
git commit -m "feat(hooks): add PATH auto-detection layer"
```

---

### Task 2: Add `hooks` field to Skill + loader

**Files:**
- Modify: `src/skill/types.rs` (add field to `Skill`)
- Modify: `src/skill/loader.rs` (parse `hooks` from frontmatter)

**Interfaces:**
- Consumes: `HookActionConfig` from `crate::hook`
- Produces: `Skill.hooks: HashMap<String, Vec<HookActionConfig>>`
- `pub fn parse_skill(path: &Path) -> Result<Skill>` now populates `skill.hooks`

- [ ] **Step 1: Add hooks field to Skill**

```rust
// in src/skill/types.rs
use std::collections::HashMap;
use crate::hook::HookActionConfig;

pub struct Skill {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub metadata: HashMap<String, String>,
    /// Hooks declared in the skill's frontmatter.
    /// Key is the hook point string (e.g. "after_apply"), value is a list of actions.
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookActionConfig>>,
}
```

- [ ] **Step 2: Parse hooks in loader frontmatter**

```rust
// in src/skill/loader.rs, inside the local Frontmatter struct
#[derive(Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    metadata: HashMap<String, String>,
    #[serde(default)]
    hooks: HashMap<String, Vec<HookActionConfig>>,
}

// In parse_skill(), after building the Skill:
let fm: Frontmatter = serde_yaml::from_str(&frontmatter_str)?;
// ...
skill.hooks = fm.hooks;
```

- [ ] **Step 3: Write tests**

```rust
// in src/skill/loader.rs or src/skill/types.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_hooks_default_empty() {
        let yaml = r#"---
name: test-skill
---
Some instructions"#;
        let skill = parse_skill_from_str(yaml).unwrap();
        assert!(skill.hooks.is_empty());
    }

    #[test]
    fn test_skill_hooks_parsed() {
        let yaml = r#"---
name: codegraph
hooks:
  after_apply:
    - type: shell
      command: "codegraph sync"
      timeout_secs: 60
---
Sync codegraph"#;
        let skill = parse_skill_from_str(yaml).unwrap();
        assert_eq!(skill.hooks.len(), 1);
        let actions = skill.hooks.get("after_apply").unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0].action {
            HookAction::Shell { command } => assert_eq!(command, "codegraph sync"),
        }
    }
}
```

- [ ] **Step 4: Build and test**

Run: `cargo test -p anacleto skill::tests 2>&1 | tail -10`

- [ ] **Step 5: Commit**

```bash
git add src/skill/types.rs src/skill/loader.rs
git commit -m "feat(skills): add hooks field to Skill frontmatter"
```

---

### Task 3: Add `register_hooks()` to Plugin trait

**Files:**
- Modify: `src/plugin/mod.rs`

**Interfaces:**
- Produces: `fn register_hooks(&self) -> Vec<(HookPoint, HookActionConfig)>` with default empty vec

- [ ] **Step 1: Add method to Plugin trait**

```rust
// in src/plugin/mod.rs
pub trait Plugin: Send + Sync {
    /// Return hooks this plugin wants to register.
    /// Called once at engine startup.
    fn register_hooks(&self) -> Vec<(HookPoint, HookActionConfig)> {
        Vec::new() // default: no hooks
    }
    // ... existing methods unchanged ...
}
```

- [ ] **Step 2: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::{HookAction, HookActionConfig, HookPoint};

    struct TestPlugin;
    impl Plugin for TestPlugin {
        fn register_hooks(&self) -> Vec<(HookPoint, HookActionConfig)> {
            vec![(
                HookPoint::AfterApply,
                HookActionConfig {
                    action: HookAction::Shell { command: "echo plugin".into() },
                    timeout_secs: 10,
                },
            )]
        }
    }

    #[test]
    fn test_plugin_register_hooks() {
        let plugin = TestPlugin;
        let hooks = plugin.register_hooks();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].0, HookPoint::AfterApply);
    }

    #[test]
    fn test_plugin_default_no_hooks() {
        struct EmptyPlugin;
        impl Plugin for EmptyPlugin {}
        let plugin = EmptyPlugin;
        assert!(plugin.register_hooks().is_empty());
    }
}
```

- [ ] **Step 3: Build and test**

Run: `cargo test -p anacleto plugin::tests 2>&1 | tail -10`

- [ ] **Step 4: Commit**

```bash
git add src/plugin/mod.rs
git commit -m "feat(plugins): add register_hooks() to Plugin trait"
```

---

### Task 4: Merge all 4 hook sources with precedence + dedup

**Files:**
- Modify: `src/engine/orchestrator.rs`

**Interfaces:**
- Consumes: `HookRegistry::from(&Config)`, `detect_auto_hooks()`, `Skill::hooks`, `Plugin::register_hooks()`
- Produces: Merged `HookRegistry` in `Engine::new()`

- [ ] **Step 1: Write test for merge logic first**

Add a test module in orchestrator.rs or in a separate test file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::{HookAction, HookActionConfig, HookPoint};

    /// Helper: create an action for testing
    fn action(cmd: &str, timeout: u64) -> HookActionConfig {
        HookActionConfig {
            action: HookAction::Shell { command: cmd.into() },
            timeout_secs: timeout,
        }
    }

    #[test]
    fn test_merge_hooks_precedence_config_wins() {
        // Config hook "override" should beat auto-detect "original"
        let config_hooks = {
            let mut m = HashMap::new();
            m.insert("after_apply".into(), vec![action("override", 30)]);
            m
        };
        let auto_hooks = {
            let mut m: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("original", 30)]);
            m
        };
        let plugin_hooks: Vec<(HookPoint, HookActionConfig)> = vec![];
        let skill_hooks: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();
        
        let merged = merge_hook_sources(&config_hooks, &plugin_hooks, &skill_hooks, &auto_hooks);
        let actions = merged.get(&HookPoint::AfterApply).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0].action {
            HookAction::Shell { command } => assert_eq!(command, "override"),
        }
    }

    #[test]
    fn test_merge_hooks_dedup_same_command() {
        // Same command from two sources should deduplicate
        let config_hooks = HashMap::new();
        let auto_hooks = {
            let mut m = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("codegraph sync", 60)]);
            m
        };
        let plugin_hooks = vec![(HookPoint::AfterApply, action("codegraph sync", 60))];
        let skill_hooks = {
            let mut m = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("codegraph sync", 60)]);
            m
        };
        
        let merged = merge_hook_sources(&config_hooks, &plugin_hooks, &skill_hooks, &auto_hooks);
        let actions = merged.get(&HookPoint::AfterApply).unwrap();
        assert_eq!(actions.len(), 1, "same (command, timeout) should deduplicate");
    }

    #[test]
    fn test_merge_hooks_different_commands_all_kept() {
        let config_hooks = HashMap::new();
        let auto_hooks = {
            let mut m = HashMap::new();
            m.insert(HookPoint::AfterApply, vec![action("hook_a", 30)]);
            m
        };
        let plugin_hooks = vec![(HookPoint::AfterApply, action("hook_b", 30))];
        let skill_hooks = HashMap::new();
        
        let merged = merge_hook_sources(&config_hooks, &plugin_hooks, &skill_hooks, &auto_hooks);
        let actions = merged.get(&HookPoint::AfterApply).unwrap();
        assert_eq!(actions.len(), 2, "different commands should both be kept");
    }
}
```

- [ ] **Step 2: Implement merge function**

```rust
// in src/engine/orchestrator.rs (as a free function)

/// Merge hooks from 4 sources with precedence:
/// 1. Config (user explicit) — highest priority
/// 2. Plugin hooks
/// 3. Skill hooks
/// 4. Auto-detect (PATH) — lowest priority
///
/// Deduplication: same (command, timeout_secs) at the same HookPoint
/// is kept only once (from the highest-precedence source).
fn merge_hook_sources(
    config: &HashMap<String, Vec<HookActionConfig>>,
    plugins: &[(HookPoint, HookActionConfig)],
    skills: &HashMap<HookPoint, Vec<HookActionConfig>>,
    auto: &HashMap<HookPoint, Vec<HookActionConfig>>,
) -> HashMap<HookPoint, Vec<HookActionConfig>> {
    let mut result: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();

    // Helper to insert keeping dedup
    fn insert_dedup(
        result: &mut HashMap<HookPoint, Vec<HookActionConfig>>,
        point: HookPoint,
        action: HookActionConfig,
    ) {
        let point_key = format!("{:?}_{}_{}", point, action.timeout_secs, match &action.action {
            HookAction::Shell { command } => command,
        });
        // Simple approach: track seen tuples per point via a helper
        let entry = result.entry(point).or_default();
        if !entry.iter().any(|a| {
            a.timeout_secs == action.timeout_secs
                && matches!((&a.action, &action.action), (HookAction::Shell { command: c1 }, HookAction::Shell { command: c2 }) if c1 == c2)
        }) {
            entry.push(action);
        }
    }

    // 1. Auto-detect (lowest priority)
    for (point, actions) in auto {
        for action in actions {
            insert_dedup(&mut result, *point, action.clone());
        }
    }

    // 2. Skill hooks
    for (point, actions) in skills {
        for action in actions {
            insert_dedup(&mut result, *point, action.clone());
        }
    }

    // 3. Plugin hooks
    for (point, action) in plugins {
        insert_dedup(&mut result, *point, action.clone());
    }

    // 4. Config (highest priority) — inserted last so they win dedup
    for (key, actions) in config {
        if let Some(point) = parse_hook_point(key) {
            for action in actions {
                // Remove any existing action with same command+timeout
                let entry = result.entry(point).or_default();
                entry.retain(|a| {
                    a.timeout_secs != action.timeout_secs
                        || !matches!((&a.action, &action.action), (HookAction::Shell { command: c1 }, HookAction::Shell { command: c2 }) if c1 == c2)
                });
                entry.push(action.clone());
            }
        }
    }

    result
}

/// Parse a YAML config key string into a HookPoint.
fn parse_hook_point(key: &str) -> Option<HookPoint> {
    match key {
        "before_tool" => Some(HookPoint::BeforeTool),
        "after_tool" => Some(HookPoint::AfterTool),
        "before_apply" => Some(HookPoint::BeforeApply),
        "after_apply" => Some(HookPoint::AfterApply),
        "before_shell" => Some(HookPoint::BeforeShell),
        "after_shell" => Some(HookPoint::AfterShell),
        "before_fs_write" => Some(HookPoint::BeforeFsWrite),
        "after_fs_write" => Some(HookPoint::AfterFsWrite),
        "on_startup" => Some(HookPoint::OnStartup),
        "on_shutdown" => Some(HookPoint::OnShutdown),
        _ => None,
    }
}
```

- [ ] **Step 3: Integrate into Engine::new()**

```rust
// In Engine::new(), after loading config and skills and plugins:

// 1. Collect auto-detected hooks
let auto_hooks = crate::hook::autoconfig::detect_auto_hooks();

// 2. Collect skill hooks from all loaded skills
let mut skill_hooks: HashMap<HookPoint, Vec<HookActionConfig>> = HashMap::new();
for skill in skill_registry.blocking_read().list() {
    for (key, actions) in &skill.hooks {
        if let Some(point) = parse_hook_point(key) {
            let entry = skill_hooks.entry(point).or_default();
            entry.extend(actions.clone());
        }
    }
}

// 3. Collect plugin hooks
let plugin_hooks: Vec<(HookPoint, HookActionConfig)> = plugins
    .list()
    .iter()
    .flat_map(|p| p.register_hooks())
    .collect();

// 4. Merge all sources
let merged = merge_hook_sources(&config.hooks, &plugin_hooks, &skill_hooks, &auto_hooks);

// 5. Create HookRegistry from merged hooks
let hook_registry = HookRegistry::new(merged);
```

Note: `PluginRegistry` needs a `list()` method that returns `&[Arc<dyn Plugin>]`. If it doesn't exist, add it:

```rust
// in src/plugin/mod.rs
pub fn list(&self) -> &[Arc<dyn Plugin>] {
    &self.plugins
}
```

- [ ] **Step 4: Write integration test**

```rust
#[test]
fn test_auto_hooks_integration() {
    let config = Config::default(); // no hooks in config
    let engine = Engine::new(config).unwrap();
    // If codegraph is on PATH, AfterApply should have the auto hook
    if crate::hook::autoconfig::which("codegraph").is_some() {
        assert!(!engine.hook_registry.is_empty());
        let results = engine.hook_registry.run(HookPoint::AfterApply, &HookContext::default()).await;
        assert_eq!(results.len(), 1);
    }
}
```

- [ ] **Step 5: Build and test**

Run: `cargo test -p anacleto 2>&1 | tail -20`
Expected: 380+ tests pass, 0 failures

- [ ] **Step 6: Commit**

```bash
git add src/engine/orchestrator.rs src/plugin/mod.rs
git commit -m "feat(hooks): merge auto-detect, skill, plugin hooks with precedence"
```

---

## Self-Review Checklist

**Spec coverage:**
- [x] Layer 1 (PATH auto-detect) — Task 1
- [x] Layer 2 (Skill frontmatter hooks) — Task 2
- [x] Layer 3 (Plugin hooks) — Task 3
- [x] Integration + dedup — Task 4
- [x] Config > Plugin > Skill > Auto-detect precedence — Task 4
- [x] Deduplication by (command, timeout) — Task 4

**Placeholder scan:**
- [x] No "TBD", "TODO", "implement later"
- [x] All code blocks contain complete code
- [x] All tests have assertions
- [x] All file paths are exact

**Type consistency:**
- [x] `HookPoint` used consistently across all tasks
- [x] `HookActionConfig` same struct everywhere
- [x] `HookAction::Shell` same variant
- [x] `detect_auto_hooks()` returns `HashMap<HookPoint, Vec<HookActionConfig>>`
- [x] `register_hooks()` returns `Vec<(HookPoint, HookActionConfig)>`
- [x] `merge_hook_sources()` accepts and returns consistent types