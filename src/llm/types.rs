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

/// Request to the LLM.
#[derive(Debug, Clone, Serialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
}

/// Response from the LLM.
#[derive(Debug, Clone, Serialize)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: String,
    pub usage: Option<LlmUsage>,
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
}
