use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::agent::types::AgentRole;

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

    /// Shell tool inventory overrides.
    #[serde(default)]
    pub shell: ShellConfig,

    /// Agent definitions.
    ///
    /// Agents are no longer defined in `config.yaml`. They are loaded from
    /// Markdown files with YAML frontmatter by `crate::agent::loader`
    /// (global `~/.config/anacleto/agents/` + project `.anacleto/agents/`).
    /// This field is skipped during (de)serialization and populated by
    /// `load_config()` after the YAML merge.
    #[serde(skip)]
    pub agents: Vec<AgentConfig>,
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
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            anthropic: None,
            openai: None,
            openrouter: None,
            ollama: Some(OllamaConfig::default()),
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
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: default_ollama_url(),
            model: default_ollama_model(),
            context_window: default_context_window(),
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
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            history_limit_percent: default_history_limit(),
            database_path: default_db_path(),
            retry: RetryConfig::default(),
            max_steps: default_max_steps(),
            debug: false,
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

    /// Permission configuration.
    #[serde(default)]
    pub permissions: PermissionConfig,

    /// Subagent names (only for agents, not subagents).
    #[serde(default)]
    pub subagents: Vec<String>,

    /// The agent's system prompt (body of the Markdown file).
    #[serde(default)]
    pub system_prompt: String,

    /// Maximum number of turns (LLM+tool iterations) per task before the agent
    /// is forced to stop and mark the task as incomplete.
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
}

fn default_role() -> AgentRole {
    AgentRole::SubAgent
}

fn default_max_steps() -> u32 {
    90
}

/// Permission configuration for an agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionConfig {
    /// Explicitly denied permissions.
    #[serde(default)]
    pub deny: Vec<String>,

    /// Explicitly allowed permissions (if empty, all not denied are allowed).
    #[serde(default)]
    pub allow: Vec<String>,
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
