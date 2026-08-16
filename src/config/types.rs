use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::agent::types::AgentRole;
use crate::hook::HookActionConfig;

/// Top-level configuration for Anacleto.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// LLM model definitions.
    #[serde(default)]
    pub models: ModelsConfig,

    /// Global MCP server definitions.
    #[serde(default)]
    pub mcps: HashMap<String, McpDefinition>,

    /// Session settings.
    #[serde(default)]
    pub session: SessionConfig,

    /// Known workspaces (directories) for the `/workspaces` command.
    #[serde(default)]
    pub workspaces: Vec<PathBuf>,

    /// Shell tool inventory overrides.
    #[serde(default)]
    pub shell: ShellConfig,

    /// Keybinding overrides for the TUI keymap.
    #[serde(default)]
    pub keymap: Option<KeymapConfig>,

    /// External editor command (overrides `$EDITOR`/`$VISUAL`).
    #[serde(default)]
    pub editor: Option<String>,

    /// Model picker dialog configuration.
    #[serde(default)]
    pub model_picker: ModelPickerConfig,

    /// Agent definitions.
    ///
    /// Agents are no longer defined in `config.yaml`. They are loaded from
    /// Markdown files with YAML frontmatter by `crate::agent::loader`
    /// (global `~/.config/anacleto/agents/` + project `.agents/agents/`).
    /// This field is skipped during (de)serialization and populated by
    /// `load_config()` after the YAML merge.
    #[serde(skip)]
    pub agents: Vec<AgentConfig>,

    /// Custom slash commands defined in config.
    #[serde(default)]
    pub commands: Vec<CustomCommand>,

    /// Hook system configuration: maps hook point names to lists of actions.
    ///
    /// Example:
    /// ```yaml
    /// hooks:
    ///   after_apply:
    ///     - type: shell
    ///       command: "codegraph sync"
    ///       timeout_secs: 60
    ///   on_startup:
    ///     - type: shell
    ///       command: "echo 'engine started'"
    /// ```
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookActionConfig>>,

    /// Default tool definitions and display properties.
    /// Each key is a built-in tool name, value is its default display config.
    /// Agents can override these in their frontmatter `tools:` section.
    #[serde(default)]
    pub tools: HashMap<String, ToolDefaults>,
}

/// LLM provider configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    /// Anthropic configuration.
    #[serde(default)]
    pub anthropic: Option<ProviderConfig>,

    /// OpenAI configuration.
    #[serde(default)]
    pub openai: Option<ProviderConfig>,

    /// OpenRouter configuration (OpenAI-compatible).
    #[serde(default)]
    pub openrouter: Option<ProviderConfig>,

    /// Ollama configuration.
    #[serde(default)]
    pub ollama: Option<OllamaConfig>,

    /// AWS Bedrock configuration (OpenAI-compatible).
    #[serde(default)]
    pub bedrock: Option<ProviderConfig>,

    /// Azure OpenAI configuration (OpenAI-compatible).
    #[serde(default)]
    pub azure: Option<ProviderConfig>,

    /// Google Gemini configuration (OpenAI-compatible).
    #[serde(default)]
    pub google: Option<ProviderConfig>,

    /// Prompt caching policy.
    #[serde(default)]
    pub cache: CacheConfig,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            anthropic: None,
            openai: None,
            openrouter: None,
            ollama: Some(OllamaConfig::default()),
            bedrock: None,
            azure: None,
            google: None,
            cache: CacheConfig::default(),
        }
    }
}

/// Prompt caching mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CacheMode {
    /// Enable prompt caching (inject provider-aware breakpoints).
    #[default]
    Auto,
    /// Disable prompt caching.
    Off,
}

/// Prompt caching configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Caching mode.
    #[serde(default)]
    pub mode: CacheMode,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            mode: CacheMode::Auto,
        }
    }
}

/// Generic LLM provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// API key (may reference env var like ${ANTHROPIC_API_KEY}).
    pub api_key: String,

    /// Model identifier (e.g., "claude-sonnet-4-20250514").
    #[serde(default = "default_model")]
    pub model: String,

    /// Context window size in tokens.
    #[serde(default = "default_context_window")]
    pub context_window: usize,

    /// Base URL for API (optional, for self-hosted).
    pub base_url: Option<String>,

    /// Input price in USD per million tokens.
    #[serde(default = "default_input_price_per_million")]
    pub input_price_per_million: f64,

    /// Output price in USD per million tokens.
    #[serde(default = "default_output_price_per_million")]
    pub output_price_per_million: f64,

    /// Anthropic extended thinking budget in tokens. When set, the Anthropic
    /// provider enables extended thinking with this budget. Ignored by other
    /// providers.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,

    /// Optional per-provider retry configuration. When set, overrides the
    /// global `session.retry` for this provider.
    #[serde(default)]
    pub retry: Option<RetryConfig>,
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

fn default_context_window() -> usize {
    200_000
}

fn default_input_price_per_million() -> f64 {
    3.0
}

fn default_output_price_per_million() -> f64 {
    15.0
}

/// Ollama-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    /// Base URL for Ollama API.
    #[serde(default = "default_ollama_url")]
    pub base_url: String,

    /// Model name.
    #[serde(default = "default_ollama_model")]
    pub model: String,

    /// Context window size.
    #[serde(default = "default_context_window")]
    pub context_window: usize,

    /// Optional per-provider retry configuration. When set, overrides the
    /// global `session.retry` for this provider.
    #[serde(default)]
    pub retry: Option<RetryConfig>,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: default_ollama_url(),
            model: default_ollama_model(),
            context_window: default_context_window(),
            retry: None,
        }
    }
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "llama3.2".to_string()
}

/// MCP server definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDefinition {
    /// Transport type: "stdio" or "tcp".
    #[serde(default = "default_mcp_transport")]
    pub transport: String,

    /// Command for stdio transport.
    pub command: Option<String>,

    /// Arguments for stdio transport.
    #[serde(default)]
    pub args: Vec<String>,

    /// Host for TCP transport.
    pub host: Option<String>,

    /// Port for TCP transport.
    pub port: Option<u16>,
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

/// Retry configuration with exponential backoff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Base delay in milliseconds (first retry wait).
    #[serde(default = "default_base_delay_ms")]
    pub base_delay_ms: u64,

    /// Maximum delay in milliseconds (cap for exponential backoff).
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            base_delay_ms: default_base_delay_ms(),
            max_delay_ms: default_max_delay_ms(),
        }
    }
}

fn default_max_retries() -> u32 {
    3
}

fn default_base_delay_ms() -> u64 {
    1000
}

fn default_max_delay_ms() -> u64 {
    30000
}

/// Session configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Percentage of context window to use for history.
    #[serde(default = "default_history_limit")]
    pub history_limit_percent: f64,

    /// Path to SQLite database.
    #[serde(default = "default_db_path")]
    pub database_path: PathBuf,

    /// Retry configuration for LLM, MCP, and subagent calls.
    #[serde(default)]
    pub retry: RetryConfig,

    /// Default maximum number of turns (LLM + tool iterations) per task before
    /// an agent is forced to stop and mark the task as incomplete. Agents can
    /// override this per-agent in their Markdown frontmatter.
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,

    /// Enable debug mode (show LLM request/response payloads in TUI).
    #[serde(default)]
    pub debug: bool,

    /// Maximum number of concurrent subagent tasks. 0 means unlimited.
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            history_limit_percent: default_history_limit(),
            database_path: default_db_path(),
            retry: RetryConfig::default(),
            max_steps: default_max_steps(),
            debug: false,
            max_concurrency: default_max_concurrency(),
        }
    }
}

fn default_history_limit() -> f64 {
    50.0
}

fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anacleto")
        .join("sessions.db")
}

fn default_max_concurrency() -> u32 {
    4
}

/// Agent/subagent configuration.
///
/// This struct is populated by `crate::agent::loader` from Markdown files
/// with YAML frontmatter, mirroring the skill format. The `system_prompt`
/// comes from the Markdown body; the remaining fields come from the
/// frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique name for this agent.
    pub name: String,

    /// Short human-readable summary of this agent (from frontmatter).
    pub description: String,

    /// Directrices de cuándo el agente padre debe invocar este subagente
    /// (texto libre, inyectado automáticamente en el system prompt del padre).
    #[serde(default)]
    pub when_to_use: String,

    /// Role in the hierarchy: "root" or "subagent" (default "subagent").
    #[serde(default = "default_role")]
    pub role: AgentRole,

    /// LLM model to use (references a model name from models config).
    #[serde(default = "default_model")]
    pub model: String,

    /// List of skill paths.
    #[serde(default)]
    pub skills: Vec<PathBuf>,

    /// List of MCP names (references global MCP definitions).
    #[serde(default)]
    pub mcps: Vec<String>,

    /// Subagent names (only for agents, not subagents).
    #[serde(default)]
    pub subagents: Vec<String>,

    /// The agent's system prompt (body of the Markdown file).
    ///
    /// May be a template containing `{var}` placeholders that are substituted
    /// at runtime. Supported variables: `{model}`, `{workspace}` and `{tools}`
    /// (a comma-separated list of the agent's tool names).
    #[serde(default)]
    pub system_prompt: String,

    /// Maximum number of turns (LLM+tool iterations) per task before the agent
    /// is forced to stop and mark the task as incomplete.
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,

    /// List of built-in tool names this agent can use.
    /// Only tools listed here are available to the agent.
    #[serde(default)]
    pub tools: Vec<String>,

    /// Additional paths where this agent can write (workspace is always writable).
    #[serde(default)]
    pub writable_paths: Vec<PathBuf>,
}

/// Default values for a built-in tool's display properties, defined in config.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefaults {
    /// Description sent to the LLM (overrides the hardcoded one).
    #[serde(default)]
    pub description: String,
    /// Whether executions are shown in the chat (default: true).
    #[serde(default = "default_tool_show")]
    pub show: bool,
    /// Custom display template with `{param}` placeholders (optional).
    /// E.g., "📖 reading: {path}" for the `read` tool.
    pub display: Option<String>,
    /// Custom color for the tool execution line in the TUI (optional).
    /// Supports: red, green, yellow, blue, magenta, cyan, white, gray,
    /// dark_gray, light_* variants, and hex colors like "#ff8800".
    pub color: Option<String>,
}

impl Default for ToolDefaults {
    fn default() -> Self {
        Self {
            description: String::new(),
            show: true,
            display: None,
            color: None,
        }
    }
}

fn default_tool_show() -> bool {
    true
}

fn default_role() -> AgentRole {
    AgentRole::SubAgent
}

fn default_max_steps() -> u32 {
    90
}

/// Shell tool inventory configuration.
///
/// Overrides or extends the built-in catalog of modern CLI tools. Each entry
/// replaces the built-in tool with the same `name`, or is appended as a new
/// tool if no built-in matches.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShellConfig {
    /// Custom/override modern tool definitions.
    #[serde(default)]
    pub tools: Vec<ShellToolConfig>,
}

/// A single modern tool definition in the shell inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellToolConfig {
    /// Modern tool name, e.g. "lsd".
    pub name: String,

    /// Classic GNU counterpart it replaces, e.g. "ls". Empty if none.
    #[serde(default)]
    pub classic: String,

    /// Short description of the tool.
    #[serde(default)]
    pub description: String,
}

/// Keybinding overrides for the TUI keymap.
///
/// Maps an action name (e.g. `ToggleSidebar`) to a list of key strings
/// (e.g. `ctrl+b`, `f2`, `enter`, `ctrl+shift+p`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeymapConfig {
    /// Action name → list of key strings.
    #[serde(default)]
    pub bindings: HashMap<String, Vec<String>>,
}

/// Configuration for the model picker dialog.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPickerConfig {
    /// Models shown in the "Favorites" tab.
    #[serde(default)]
    pub favorites: Vec<String>,
}

/// A custom slash command defined in config.
///
/// The `template` may contain `{env:VAR}` and `{file:path}` placeholders that
/// are expanded at dispatch time (see `crate::engine::template::expand_vars`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCommand {
    /// Command name including the leading slash, e.g. `/deploy`.
    pub name: String,
    /// Short description shown in the command palette.
    #[serde(default)]
    pub description: String,
    /// Template expanded and sent to the engine as user input.
    pub template: String,
}
