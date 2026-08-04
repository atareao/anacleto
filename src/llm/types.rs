use serde::{Deserialize, Serialize};

/// Role of a message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// A tool call from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolFunction,
}

/// Function details in a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
}

/// A tool definition to provide to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
}

/// Prompt caching policy.
///
/// This is a provider-aware hint: Anthropic uses explicit `cache_control`
/// breakpoints, while OpenAI/OpenRouter cache automatically and ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheControl {
    /// Enable prompt caching (inject breakpoints where the provider supports it).
    Auto,
    /// Disable prompt caching.
    Off,
}

/// Request to the LLM.
#[derive(Debug, Clone, Serialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    /// Per-request prompt caching policy override. When `None`, the provider's
    /// configured default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// Response from the LLM.
#[derive(Debug, Clone, Serialize)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: String,
    pub usage: Option<LlmUsage>,
    /// Intermediate reasoning/thinking text (e.g. Anthropic extended thinking).
    /// Kept separate from `content` so consumers can choose to surface or drop it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

/// Token usage information.
#[derive(Debug, Clone, Serialize)]
pub struct LlmUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// A chunk of a streaming response.
#[derive(Debug, Clone)]
pub enum LlmStreamChunk {
    Content(String),
    ToolCall(ToolCall),
    Done(LlmUsage),
    Error(String),
}

/// LLM provider type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmProviderType {
    Anthropic,
    OpenAI,
    OpenRouter,
    Ollama,
    /// AWS Bedrock (OpenAI-compatible endpoint).
    Bedrock,
    /// Azure OpenAI (OpenAI-compatible endpoint).
    Azure,
    /// Google Gemini (OpenAI-compatible endpoint).
    Google,
}

/// Configuration for an LLM provider.
#[derive(Debug, Clone)]
pub struct LlmProviderConfig {
    pub provider_type: LlmProviderType,
    pub api_key: Option<String>,
    pub model: String,
    pub base_url: Option<String>,
    pub context_window: usize,
    pub input_price_per_million: f64,
    pub output_price_per_million: f64,
    /// Prompt caching policy for this provider.
    pub cache_control: CacheControl,
    /// Anthropic extended thinking budget (tokens). When `Some`, the request
    /// enables extended thinking with this budget. Ignored by other providers.
    pub thinking_budget_tokens: Option<u32>,
}
