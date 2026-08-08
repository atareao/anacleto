//! Plugin system for Anacleto.
//!
//! Plugins extend the engine with hooks and transforms. A plugin can:
//!
//! - Observe and modify agent spawns (`on_agent_spawn`).
//! - Intercept tool calls (`on_tool_call`).
//! - Handle custom slash commands (`on_command`).
//! - React to engine events (`on_event`).
//! - Register custom tools and their handlers (`register_tool`).
//!
//! Plugins are loaded declaratively from `~/.config/anacleto/plugins/`
//! (see [`global_plugins_dir`](crate::config::paths::global_plugins_dir)).
//! Each plugin is a directory containing a `plugin.yaml` manifest plus
//! optional scripts.

use std::collections::HashMap;
use std::path::Path;

use crate::hook::{HookActionConfig, HookPoint};
use crate::llm::types::{ToolCall, ToolDefinition};

/// A plugin hook result. Hooks may return a replacement value or `None` to
/// leave the original unchanged.
pub type HookResult<T> = Option<T>;

/// A custom tool handler. Receives the raw [`ToolCall`] and returns the
/// tool result string synchronously.
pub type ToolHandler = fn(&ToolCall) -> String;

/// A plugin manifest loaded from disk.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginManifest {
    /// Unique plugin name.
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Version of the plugin.
    #[serde(default)]
    pub version: String,
}

/// Trait implemented by plugins.
///
/// All hooks have no-op defaults so a plugin only overrides what it needs.
pub trait Plugin: Send + Sync {
    /// The plugin's unique name.
    fn name(&self) -> &str;

    /// Return hooks this plugin wants to register.
    /// Called once at engine startup.
    fn register_hooks(&self) -> Vec<(HookPoint, HookActionConfig)> {
        Vec::new() // default: no hooks
    }

    /// Called when an agent is spawned. May return a modified system prompt.
    fn on_agent_spawn(&self, _agent_name: &str, _system_prompt: &str) -> HookResult<String> {
        None
    }

    /// Called before a tool call is executed. May return a replacement result
    /// to short-circuit the built-in handler.
    fn on_tool_call(&self, _tool_call: &ToolCall) -> HookResult<String> {
        None
    }

    /// Called when a slash command is issued. May return a replacement
    /// response to short-circuit the built-in handler.
    fn on_command(&self, _command: &str, _args: &str) -> HookResult<String> {
        None
    }

    /// Called on engine events. The event is provided as a debug string.
    fn on_event(&self, _event: &str) {
        let _ = _event;
    }
}

/// A registry of loaded plugins and their custom tools.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    custom_tools: Vec<ToolDefinition>,
    custom_tool_handlers: HashMap<String, ToolHandler>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a plugin instance.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
    }

    /// Register a custom tool definition and its handler. If a tool with the
    /// same name is already registered, its definition is replaced (no
    /// duplicates are accumulated).
    pub fn register_tool(&mut self, definition: ToolDefinition, handler: ToolHandler) {
        self.custom_tool_handlers
            .insert(definition.name.clone(), handler);
        if let Some(existing) = self
            .custom_tools
            .iter_mut()
            .find(|t| t.name == definition.name)
        {
            *existing = definition;
        } else {
            self.custom_tools.push(definition);
        }
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Return a reference to all registered plugins.
    pub fn list(&self) -> &[Box<dyn Plugin>] {
        &self.plugins
    }

    /// Custom tool definitions registered by plugins.
    pub fn custom_tools(&self) -> &[ToolDefinition] {
        &self.custom_tools
    }

    /// Look up a custom tool handler by name.
    pub fn custom_tool_handler(&self, name: &str) -> Option<ToolHandler> {
        self.custom_tool_handlers.get(name).copied()
    }

    /// Invoke `on_agent_spawn` across all plugins, threading the system prompt
    /// through each hook in registration order.
    pub fn on_agent_spawn(&self, agent_name: &str, system_prompt: &str) -> String {
        let mut prompt = system_prompt.to_string();
        for plugin in &self.plugins {
            if let Some(replacement) = plugin.on_agent_spawn(agent_name, &prompt) {
                prompt = replacement;
            }
        }
        prompt
    }

    /// Invoke `on_tool_call` across all plugins. Returns the first non-`None`
    /// replacement result, or `None` if no plugin handled the call.
    pub fn on_tool_call(&self, tool_call: &ToolCall) -> Option<String> {
        for plugin in &self.plugins {
            if let Some(result) = plugin.on_tool_call(tool_call) {
                return Some(result);
            }
        }
        None
    }

    /// Invoke `on_command` across all plugins. Returns the first non-`None`
    /// replacement response, or `None` if no plugin handled the command.
    pub fn on_command(&self, command: &str, args: &str) -> Option<String> {
        for plugin in &self.plugins {
            if let Some(result) = plugin.on_command(command, args) {
                return Some(result);
            }
        }
        None
    }

    /// Invoke `on_event` across all plugins.
    pub fn on_event(&self, event: &str) {
        for plugin in &self.plugins {
            plugin.on_event(event);
        }
    }

    /// Load plugins from a directory. Each subdirectory containing a
    /// `plugin.yaml` manifest is registered as a declarative plugin.
    pub fn load_from_dir(&mut self, dir: &Path) -> std::io::Result<usize> {
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut loaded = 0;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.yaml");
            if !manifest_path.exists() {
                continue;
            }
            let manifest: PluginManifest =
                serde_yaml::from_str(&std::fs::read_to_string(&manifest_path)?).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
            self.register(Box::new(DeclarativePlugin::new(manifest)));
            loaded += 1;
        }
        Ok(loaded)
    }
}

/// A plugin backed by a declarative manifest (no executable code).
///
/// Declarative plugins currently only provide identity and metadata; their
/// hooks are no-ops. This is the foundation for future script-backed plugins.
struct DeclarativePlugin {
    manifest: PluginManifest,
}

impl DeclarativePlugin {
    fn new(manifest: PluginManifest) -> Self {
        Self { manifest }
    }
}

impl Plugin for DeclarativePlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlugin {
        name: String,
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            &self.name
        }

        fn on_agent_spawn(&self, _agent: &str, prompt: &str) -> HookResult<String> {
            Some(format!("{prompt}\n[plugin:{}]", self.name))
        }

        fn on_command(&self, command: &str, _args: &str) -> HookResult<String> {
            if command == "/ping" {
                Some("pong".to_string())
            } else {
                None
            }
        }
    }

    #[test]
    fn test_register_and_len() {
        let mut reg = PluginRegistry::new();
        assert!(reg.is_empty());
        reg.register(Box::new(TestPlugin {
            name: "a".to_string(),
        }));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_on_agent_spawn_threads_prompt() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin {
            name: "a".to_string(),
        }));
        reg.register(Box::new(TestPlugin {
            name: "b".to_string(),
        }));
        let out = reg.on_agent_spawn("root", "base");
        assert_eq!(out, "base\n[plugin:a]\n[plugin:b]");
    }

    #[test]
    fn test_on_command_short_circuit() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin {
            name: "a".to_string(),
        }));
        assert_eq!(reg.on_command("/ping", ""), Some("pong".to_string()));
        assert_eq!(reg.on_command("/other", ""), None);
    }

    #[test]
    fn test_register_tool_and_handler() {
        let mut reg = PluginRegistry::new();
        let def = ToolDefinition {
            name: "my_tool".to_string(),
            description: "A custom tool".to_string(),
            input_schema: serde_json::json!({}),
        };
        reg.register_tool(def, |_| "custom result".to_string());
        assert_eq!(reg.custom_tools().len(), 1);
        assert_eq!(reg.custom_tools()[0].name, "my_tool");
        let handler = reg.custom_tool_handler("my_tool").unwrap();
        let tc = ToolCall {
            id: "1".to_string(),
            call_type: "function".to_string(),
            function: crate::llm::types::ToolFunction {
                name: "my_tool".to_string(),
                arguments: "{}".to_string(),
            },
        };
        assert_eq!(handler(&tc), "custom result");
        assert!(reg.custom_tool_handler("missing").is_none());
    }

    #[test]
    fn test_load_from_dir_missing() {
        let mut reg = PluginRegistry::new();
        let n = reg
            .load_from_dir(Path::new("/nonexistent/plugins"))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn test_load_from_dir_yaml_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("myplugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.yaml"),
            "name: myplugin\ndescription: Test\nversion: 1.0.0\n",
        )
        .unwrap();
        let mut reg = PluginRegistry::new();
        let n = reg.load_from_dir(tmp.path()).unwrap();
        assert_eq!(n, 1);
        assert_eq!(reg.len(), 1);
    }

    // -- Plugin::register_hooks tests ---------------------------------------

    use crate::hook::{HookAction, HookActionConfig, HookPoint};

    struct HookTestPlugin;
    impl Plugin for HookTestPlugin {
        fn name(&self) -> &str {
            "hook-test"
        }

        fn register_hooks(&self) -> Vec<(HookPoint, HookActionConfig)> {
            vec![(
                HookPoint::AfterApply,
                HookActionConfig {
                    action: HookAction::Shell {
                        command: "echo plugin".into(),
                    },
                    timeout_secs: 10,
                },
            )]
        }
    }

    #[test]
    fn test_plugin_register_hooks() {
        let plugin = HookTestPlugin;
        let hooks = plugin.register_hooks();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].0, HookPoint::AfterApply);
    }

    #[test]
    fn test_plugin_default_no_hooks() {
        struct EmptyPlugin;
        impl Plugin for EmptyPlugin {
            fn name(&self) -> &str {
                "empty"
            }
        }
        let plugin = EmptyPlugin;
        assert!(plugin.register_hooks().is_empty());
    }

    #[test]
    fn test_plugin_registry_list() {
        let mut reg = PluginRegistry::new();
        assert!(reg.list().is_empty());
        reg.register(Box::new(HookTestPlugin));
        assert_eq!(reg.list().len(), 1);
    }
}
