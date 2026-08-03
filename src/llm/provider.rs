use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::error::{Error, Result};

use super::types::*;

/// Trait for LLM provider implementations.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a request and get a complete response.
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>;

    /// Send a request and get a streaming response.
    async fn complete_stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<Result<LlmStreamChunk>>>;

    /// Return the context window (max tokens) for this provider.
    fn context_window(&self) -> usize;

    /// Fetch the model's context window from the provider API.
    async fn fetch_context_window(&self) -> Result<usize>;

    /// Override the context window (used after fetching from the API).
    fn set_context_window(&self, value: usize);

    /// Input price in USD per million tokens.
    fn input_price_per_million(&self) -> f64;

    /// Output price in USD per million tokens.
    fn output_price_per_million(&self) -> f64;
}

/// Factory for creating LLM providers.
pub fn create_provider(config: &LlmProviderConfig) -> Box<dyn LlmProvider> {
    match config.provider_type {
        LlmProviderType::Anthropic => Box::new(AnthropicProvider::new(config)),
        LlmProviderType::OpenAI => Box::new(OpenAIProvider::new(config)),
        LlmProviderType::OpenRouter => Box::new(OpenRouterProvider::new(config)),
        LlmProviderType::Ollama => Box::new(OllamaProvider::new(config)),
    }
}

// ---------------------------------------------------------------------------
// OpenAI-compatible chat request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    type_: String,
    function: OpenAiFunction,
}

#[derive(Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    type_: String,
    function: OpenAiFunctionCall,
}

#[derive(Serialize, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    finish_reason: String,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Clone, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Deserialize)]
struct OpenAiStreamChunk {
    choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiStreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

/// Partial tool call in a streaming delta (fields come in separate SSE events).
#[derive(Deserialize)]
struct OpenAiStreamToolCall {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    type_: Option<String>,
    #[serde(default)]
    function: Option<OpenAiStreamFunction>,
}

#[derive(Deserialize)]
struct OpenAiStreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

// ---------------------------------------------------------------------------
// Anthropic-specific request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    type_: String,
    text: Option<String>,
    #[serde(default)]
    tool_use: Option<AnthropicToolUse>,
}

#[derive(Deserialize)]
struct AnthropicToolUse {
    id: String,
    name: String,
    input: serde_json::Value,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

// ---------------------------------------------------------------------------
// Ollama-specific request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: Option<OllamaOptions>,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
    done: bool,
    #[serde(default)]
    eval_count: Option<u32>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Create a shared reqwest Client with sensible defaults.
fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| Error::Provider(format!("Failed to create HTTP client: {e}")))
}

/// Convert a `Vec<LlmMessage>` into OpenAI-compatible message list.
fn into_openai_messages(messages: Vec<LlmMessage>) -> Vec<OpenAiMessage> {
    messages
        .into_iter()
        .map(|m| OpenAiMessage {
            role: match m.role {
                MessageRole::User => "user".into(),
                MessageRole::Assistant => "assistant".into(),
                MessageRole::System => "system".into(),
                MessageRole::Tool => "tool".into(),
            },
            content: m.content,
            tool_calls: m.tool_calls.map(|calls| {
                calls
                    .into_iter()
                    .map(|tc| OpenAiToolCall {
                        id: tc.id,
                        type_: tc.call_type,
                        function: OpenAiFunctionCall {
                            name: tc.function.name,
                            arguments: tc.function.arguments,
                        },
                    })
                    .collect()
            }),
            tool_call_id: m.tool_call_id,
        })
        .collect()
}

/// Convert a `Vec<LlmMessage>` into Anthropic-compatible message list,
/// extracting the system message separately.
fn into_anthropic_messages(messages: Vec<LlmMessage>) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system = None;
    let mut msgs = Vec::new();
    for m in messages {
        match m.role {
            MessageRole::System => system = Some(m.content),
            _ => msgs.push(AnthropicMessage {
                role: match m.role {
                    MessageRole::User => "user".into(),
                    MessageRole::Assistant => "assistant".into(),
                    MessageRole::Tool => "user".into(),
                    _ => "user".into(),
                },
                content: m.content,
            }),
        }
    }
    (system, msgs)
}

/// Convert a `Vec<LlmMessage>` into Ollama-compatible message list.
fn into_ollama_messages(messages: Vec<LlmMessage>) -> Vec<OllamaMessage> {
    messages
        .into_iter()
        .map(|m| OllamaMessage {
            role: match m.role {
                MessageRole::User => "user".into(),
                MessageRole::Assistant => "assistant".into(),
                MessageRole::System => "system".into(),
                MessageRole::Tool => "tool".into(),
            },
            content: m.content,
            tool_calls: None,
        })
        .collect()
}

/// Convert tool definitions to OpenAI format.
fn into_openai_tools(tools: Vec<ToolDefinition>) -> Vec<OpenAiTool> {
    tools
        .into_iter()
        .map(|t| OpenAiTool {
            type_: "function".into(),
            function: OpenAiFunction {
                name: t.name,
                description: t.description,
                parameters: t.input_schema,
            },
        })
        .collect()
}

/// Parse a tool definition's input schema for Anthropic format
fn anthropic_tools(tools: Vec<ToolDefinition>) -> Vec<AnthropicTool> {
    tools
        .into_iter()
        .map(|t| AnthropicTool {
            name: t.name,
            description: t.description,
            input_schema: t.input_schema,
        })
        .collect()
}

/// Parse SSE data lines from a byte stream, yielding parsed `OpenAiStreamChunk`s.
/// Returns `None` when the stream is done (e.g. `[DONE]` received).
fn parse_sse_line(line: &str) -> Option<OpenAiStreamChunk> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return None;
    }
    if line == "data: [DONE]" {
        // Signal end-of-stream
        return None;
    }
    if let Some(data) = line.strip_prefix("data: ") {
        serde_json::from_str::<OpenAiStreamChunk>(data)
            .map_err(|e| tracing::warn!("Failed to parse SSE chunk: {e}"))
            .ok()
    } else {
        None
    }
}

/// Strip the `~` routing prefix from a model name if present.
/// The `~` prefix is used internally by Anacleto to hint at provider routing
/// (e.g. `~deepseek/deepseek-v4-flash` → `deepseek/deepseek-v4-flash`).
fn strip_model_prefix(model: &str) -> String {
    model.strip_prefix('~').unwrap_or(model).to_string()
}

/// Response shape for OpenAI's `GET /models/{model}` endpoint.
#[derive(Deserialize)]
struct OpenAiModelInfo {
    #[serde(default)]
    context_window: Option<usize>,
}

/// Response shape for OpenRouter's `GET /models` endpoint.
#[derive(Deserialize)]
struct OpenRouterModelList {
    data: Vec<OpenRouterModelData>,
}

#[derive(Deserialize)]
struct OpenRouterModelData {
    id: String,
    #[serde(default)]
    context_length: Option<usize>,
}

/// Response shape for Ollama's `POST /api/show` endpoint.
#[derive(Deserialize)]
struct OllamaShowResponse {
    #[serde(default)]
    model_info: HashMap<String, serde_json::Value>,
}

// ===========================================================================
// OpenAI Provider
// ===========================================================================

/// OpenAI-compatible LLM provider.
pub struct OpenAIProvider {
    config: LlmProviderConfig,
    client: Client,
    context_window: AtomicUsize,
}

impl OpenAIProvider {
    /// Creates a new OpenAI provider from the given configuration.
    pub fn new(config: &LlmProviderConfig) -> Self {
        Self {
            config: config.clone(),
            client: http_client().expect("Failed to create HTTP client"),
            context_window: AtomicUsize::new(config.context_window),
        }
    }

    fn base_url(&self) -> &str {
        self.config
            .base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1")
    }

    fn api_key(&self) -> Result<&str> {
        self.config
            .api_key
            .as_deref()
            .ok_or_else(|| Error::Provider("OpenAI API key is required".into()))
    }
}

#[async_trait]
impl LlmProvider for OpenAIProvider {
    fn context_window(&self) -> usize {
        self.context_window.load(Ordering::Relaxed)
    }

    fn set_context_window(&self, value: usize) {
        self.context_window.store(value, Ordering::Relaxed);
    }

    fn input_price_per_million(&self) -> f64 {
        self.config.input_price_per_million
    }

    fn output_price_per_million(&self) -> f64 {
        self.config.output_price_per_million
    }

    async fn fetch_context_window(&self) -> Result<usize> {
        let model = strip_model_prefix(&self.config.model);

        // First, try the API. Some OpenAI-compatible proxies expose a
        // `context_window` field on `GET /models/{model}`. If present, use it.
        let url = format!("{}/models/{}", self.base_url(), model);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()?))
            .send()
            .await
            .map_err(|e| Error::Provider(format!("OpenAI model info request failed: {e}")))?;

        if resp.status().is_success()
            && let Ok(info) = resp.json::<OpenAiModelInfo>().await
            && let Some(window) = info.context_window
        {
            return Ok(window);
        }

        // The real OpenAI `/v1/models/{model}` endpoint does not return a
        // `context_window` field, so fall back to a hardcoded mapping by model
        // name prefix. Order from most specific to least specific.
        let window = match model.as_str() {
            m if m.starts_with("gpt-4.1") => 1_047_576,
            m if m.starts_with("gpt-4o") => 128_000,
            m if m.starts_with("gpt-4-turbo") => 128_000,
            m if m.starts_with("gpt-4") => 8_192,
            m if m.starts_with("gpt-3.5-turbo") => 16_385,
            m if m.starts_with("o1") => 200_000,
            m if m.starts_with("o3") => 200_000,
            m if m.starts_with("o4") => 200_000,
            _ => {
                return Err(Error::Provider(
                    "Unknown OpenAI model context window".into(),
                ));
            }
        };
        Ok(window)
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let model = strip_model_prefix(&request.model);
        let body = OpenAiChatRequest {
            model: model.clone(),
            messages: into_openai_messages(request.messages),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stream: false,
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(into_openai_tools(request.tools.clone()))
            },
            tool_choice: if request.tools.is_empty() {
                None
            } else {
                Some(serde_json::json!("auto"))
            },
            stream_options: Some(serde_json::json!({"include_usage": true})),
        };

        let url = format!("{}/chat/completions", self.base_url());
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()?))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("OpenAI request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("OpenAI returned {status}: {text}")));
        }

        let data: OpenAiChatResponse = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("OpenAI parse failed: {e}")))?;

        let choice = data
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::Provider("OpenAI returned no choices".into()))?;

        Ok(LlmResponse {
            content: choice.message.content.unwrap_or_default(),
            tool_calls: choice
                .message
                .tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(|tc| ToolCall {
                    id: tc.id,
                    call_type: tc.type_,
                    function: ToolFunction {
                        name: tc.function.name,
                        arguments: tc.function.arguments,
                    },
                })
                .collect(),
            finish_reason: choice.finish_reason,
            usage: data.usage.map(|u| LlmUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }),
        })
    }

    async fn complete_stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<Result<LlmStreamChunk>>> {
        let model = strip_model_prefix(&request.model);
        let body = OpenAiChatRequest {
            model: model.clone(),
            messages: into_openai_messages(request.messages),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stream: true,
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(into_openai_tools(request.tools.clone()))
            },
            tool_choice: if request.tools.is_empty() {
                None
            } else {
                Some(serde_json::json!("auto"))
            },
            stream_options: Some(serde_json::json!({"include_usage": true})),
        };

        let url = format!("{}/chat/completions", self.base_url());
        let client = self.client.clone();
        let api_key = self.api_key()?.to_string();

        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            let resp = match client
                .post(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx
                        .send(Err(Error::Provider(format!(
                            "OpenAI stream request failed: {e}"
                        ))))
                        .await;
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(Error::Provider(format!(
                        "OpenAI stream returned {status}: {text}"
                    ))))
                    .await;
                return;
            }

            let mut stream = resp.bytes_stream();
            use futures::StreamExt;
            let mut buf = String::new();
            // Accumulator for streaming tool calls (OpenAI sends partial fields across SSE events)
            let mut tool_calls_acc: Vec<OpenAiToolCall> = Vec::new();
            // Tracks whether a Done chunk has already been emitted, so we never
            // send more than one (OpenAI/OpenRouter send usage in a SEPARATE SSE
            // event with choices=[] after the final choice).
            let mut done_sent = false;

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        // Process complete SSE lines
                        while let Some(line_end) = buf.find('\n') {
                            let line = buf[..line_end].to_string();
                            buf = buf[line_end + 1..].to_string();
                            if let Some(chunk) = parse_sse_line(&line) {
                                let usage = chunk.usage.clone();
                                for choice in chunk.choices {
                                    if let Some(content) = choice.delta.content
                                        && !content.is_empty()
                                    {
                                        let _ = tx.send(Ok(LlmStreamChunk::Content(content))).await;
                                    }

                                    // Merge streaming tool call deltas by index
                                    if let Some(tcs) = choice.delta.tool_calls {
                                        for tc in tcs {
                                            let idx = tc.index.unwrap_or(0);
                                            while tool_calls_acc.len() <= idx {
                                                tool_calls_acc.push(OpenAiToolCall {
                                                    id: String::new(),
                                                    type_: "function".into(),
                                                    function: OpenAiFunctionCall {
                                                        name: String::new(),
                                                        arguments: String::new(),
                                                    },
                                                });
                                            }
                                            let entry = &mut tool_calls_acc[idx];
                                            if let Some(id) = tc.id {
                                                entry.id = id;
                                            }
                                            if let Some(t) = tc.type_ {
                                                entry.type_ = t;
                                            }
                                            if let Some(f) = tc.function {
                                                if let Some(name) = f.name {
                                                    entry.function.name = name;
                                                }
                                                if let Some(args) = f.arguments {
                                                    entry.function.arguments.push_str(&args);
                                                }
                                            }
                                        }
                                    }

                                    if let Some(finish) = choice.finish_reason
                                        && finish != "null"
                                    {
                                        // Emit completed tool calls before Done. We do NOT
                                        // emit Done here: OpenAI/OpenRouter send usage in a
                                        // SEPARATE SSE event (choices=[]) after this chunk,
                                        // so emitting Done here would report 0 tokens.
                                        if finish == "tool_calls" && !tool_calls_acc.is_empty() {
                                            for tc in tool_calls_acc.drain(..) {
                                                let _ = tx
                                                    .send(Ok(LlmStreamChunk::ToolCall(ToolCall {
                                                        id: tc.id,
                                                        call_type: tc.type_,
                                                        function: ToolFunction {
                                                            name: tc.function.name,
                                                            arguments: tc.function.arguments,
                                                        },
                                                    })))
                                                    .await;
                                            }
                                        }
                                    }
                                }
                                // OpenAI/OpenRouter send usage in a SEPARATE SSE event after
                                // the final choice (choices=[]). Emit Done here with the real
                                // usage, but only once.
                                if let Some(u) = usage.as_ref().filter(|_| !done_sent) {
                                    done_sent = true;
                                    let _ = tx
                                        .send(Ok(LlmStreamChunk::Done(LlmUsage {
                                            prompt_tokens: u.prompt_tokens,
                                            completion_tokens: u.completion_tokens,
                                            total_tokens: u.total_tokens,
                                        })))
                                        .await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(Error::Provider(format!(
                                "OpenAI stream read error: {e}"
                            ))))
                            .await;
                        return;
                    }
                }
            }

            // Fallback: if the provider never sent usage, emit a Done with 0
            // tokens so the stream never hangs waiting for a terminal chunk.
            if !done_sent {
                let _ = tx
                    .send(Ok(LlmStreamChunk::Done(LlmUsage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                    })))
                    .await;
            }
        });

        Ok(rx)
    }
}

// ===========================================================================
// OpenRouter Provider (OpenAI-compatible)
// ===========================================================================

/// OpenRouter LLM provider (OpenAI-compatible API).
pub struct OpenRouterProvider {
    config: LlmProviderConfig,
    client: Client,
    context_window: AtomicUsize,
}

impl OpenRouterProvider {
    /// Creates a new OpenRouter provider from the given configuration.
    pub fn new(config: &LlmProviderConfig) -> Self {
        Self {
            config: config.clone(),
            client: http_client().expect("Failed to create HTTP client"),
            context_window: AtomicUsize::new(config.context_window),
        }
    }

    fn base_url(&self) -> &str {
        self.config
            .base_url
            .as_deref()
            .unwrap_or("https://openrouter.ai/api/v1")
    }

    fn api_key(&self) -> Result<&str> {
        self.config
            .api_key
            .as_deref()
            .ok_or_else(|| Error::Provider("OpenRouter API key is required".into()))
    }
}

#[async_trait]
impl LlmProvider for OpenRouterProvider {
    fn context_window(&self) -> usize {
        self.context_window.load(Ordering::Relaxed)
    }

    fn set_context_window(&self, value: usize) {
        self.context_window.store(value, Ordering::Relaxed);
    }

    fn input_price_per_million(&self) -> f64 {
        self.config.input_price_per_million
    }

    fn output_price_per_million(&self) -> f64 {
        self.config.output_price_per_million
    }

    async fn fetch_context_window(&self) -> Result<usize> {
        let model = strip_model_prefix(&self.config.model);
        // OpenRouter's single-model endpoint `GET /models/{model}` returns 404
        // for model ids that contain a `/` (e.g. "deepseek/deepseek-v4-flash"),
        // so fetch the full list and look the model up by id instead.
        let url = format!("{}/models", self.base_url());
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()?))
            .send()
            .await
            .map_err(|e| Error::Provider(format!("OpenRouter model info request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "OpenRouter model info returned {status}: {text}"
            )));
        }

        let list: OpenRouterModelList = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("OpenRouter model info parse failed: {e}")))?;

        list.data
            .into_iter()
            .find(|m| m.id == model)
            .and_then(|m| m.context_length)
            .ok_or_else(|| {
                Error::Provider(format!(
                    "OpenRouter model '{model}' not found or has no context_length"
                ))
            })
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let model = strip_model_prefix(&request.model);
        let body = OpenAiChatRequest {
            model: model.clone(),
            messages: into_openai_messages(request.messages),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stream: false,
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(into_openai_tools(request.tools.clone()))
            },
            tool_choice: if request.tools.is_empty() {
                None
            } else {
                Some(serde_json::json!("auto"))
            },
            stream_options: Some(serde_json::json!({"include_usage": true})),
        };

        let url = format!("{}/chat/completions", self.base_url());
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()?))
            // OpenRouter-specific headers for app ranking
            .header("HTTP-Referer", "https://github.com/anacleto")
            .header("X-Title", "Anacleto")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("OpenRouter request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "OpenRouter returned {status}: {text}"
            )));
        }

        let data: OpenAiChatResponse = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("OpenRouter parse failed: {e}")))?;

        let choice = data
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::Provider("OpenRouter returned no choices".into()))?;

        Ok(LlmResponse {
            content: choice.message.content.unwrap_or_default(),
            tool_calls: choice
                .message
                .tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(|tc| ToolCall {
                    id: tc.id,
                    call_type: tc.type_,
                    function: ToolFunction {
                        name: tc.function.name,
                        arguments: tc.function.arguments,
                    },
                })
                .collect(),
            finish_reason: choice.finish_reason,
            usage: data.usage.map(|u| LlmUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }),
        })
    }

    async fn complete_stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<Result<LlmStreamChunk>>> {
        let model = strip_model_prefix(&request.model);
        let body = OpenAiChatRequest {
            model: model.clone(),
            messages: into_openai_messages(request.messages),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stream: true,
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(into_openai_tools(request.tools.clone()))
            },
            tool_choice: if request.tools.is_empty() {
                None
            } else {
                Some(serde_json::json!("auto"))
            },
            stream_options: Some(serde_json::json!({"include_usage": true})),
        };

        let url = format!("{}/chat/completions", self.base_url());
        let client = self.client.clone();
        let api_key = self.api_key()?.to_string();
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            let resp = match client
                .post(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .header("HTTP-Referer", "https://github.com/anacleto")
                .header("X-Title", "Anacleto")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx
                        .send(Err(Error::Provider(format!(
                            "OpenRouter stream request failed: {e}"
                        ))))
                        .await;
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(Error::Provider(format!(
                        "OpenRouter stream returned {status}: {text}"
                    ))))
                    .await;
                return;
            }

            let mut stream = resp.bytes_stream();
            use futures::StreamExt;
            let mut buf = String::new();
            // Accumulator for streaming tool calls (OpenRouter sends partial deltas across SSE events)
            let mut tool_calls_acc: Vec<OpenAiToolCall> = Vec::new();
            // Tracks whether a Done chunk has already been emitted, so we never
            // send more than one (OpenAI/OpenRouter send usage in a SEPARATE SSE
            // event with choices=[] after the final choice).
            let mut done_sent = false;

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(line_end) = buf.find('\n') {
                            let line = buf[..line_end].to_string();
                            buf = buf[line_end + 1..].to_string();
                            if let Some(chunk) = parse_sse_line(&line) {
                                let usage = chunk.usage.clone();
                                for choice in chunk.choices {
                                    if let Some(content) = choice.delta.content
                                        && !content.is_empty()
                                    {
                                        let _ = tx.send(Ok(LlmStreamChunk::Content(content))).await;
                                    }

                                    // Merge streaming tool call deltas by index
                                    if let Some(tcs) = choice.delta.tool_calls {
                                        for tc in tcs {
                                            let idx = tc.index.unwrap_or(0);
                                            while tool_calls_acc.len() <= idx {
                                                tool_calls_acc.push(OpenAiToolCall {
                                                    id: String::new(),
                                                    type_: "function".into(),
                                                    function: OpenAiFunctionCall {
                                                        name: String::new(),
                                                        arguments: String::new(),
                                                    },
                                                });
                                            }
                                            let entry = &mut tool_calls_acc[idx];
                                            if let Some(id) = tc.id {
                                                entry.id = id;
                                            }
                                            if let Some(t) = tc.type_ {
                                                entry.type_ = t;
                                            }
                                            if let Some(f) = tc.function {
                                                if let Some(name) = f.name {
                                                    entry.function.name = name;
                                                }
                                                if let Some(args) = f.arguments {
                                                    entry.function.arguments.push_str(&args);
                                                }
                                            }
                                        }
                                    }

                                    if let Some(finish) = choice.finish_reason
                                        && finish != "null"
                                    {
                                        // Emit completed tool calls before Done. We do NOT
                                        // emit Done here: OpenAI/OpenRouter send usage in a
                                        // SEPARATE SSE event (choices=[]) after this chunk,
                                        // so emitting Done here would report 0 tokens.
                                        if finish == "tool_calls" && !tool_calls_acc.is_empty() {
                                            for tc in tool_calls_acc.drain(..) {
                                                let _ = tx
                                                    .send(Ok(LlmStreamChunk::ToolCall(ToolCall {
                                                        id: tc.id,
                                                        call_type: tc.type_,
                                                        function: ToolFunction {
                                                            name: tc.function.name,
                                                            arguments: tc.function.arguments,
                                                        },
                                                    })))
                                                    .await;
                                            }
                                        }
                                    }
                                }
                                // OpenRouter sends usage in a SEPARATE SSE event
                                // (choices=[]). Emit Done here with the real usage,
                                // but only once.
                                if let Some(u) = usage.as_ref().filter(|_| !done_sent) {
                                    done_sent = true;
                                    let _ = tx
                                        .send(Ok(LlmStreamChunk::Done(LlmUsage {
                                            prompt_tokens: u.prompt_tokens,
                                            completion_tokens: u.completion_tokens,
                                            total_tokens: u.total_tokens,
                                        })))
                                        .await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(Error::Provider(format!(
                                "OpenRouter stream read error: {e}"
                            ))))
                            .await;
                        return;
                    }
                }
            }

            // Fallback: if the provider never sent usage, emit a Done with 0
            // tokens so the stream never hangs waiting for a terminal chunk.
            if !done_sent {
                let _ = tx
                    .send(Ok(LlmStreamChunk::Done(LlmUsage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                    })))
                    .await;
            }
        });

        Ok(rx)
    }
}

// ===========================================================================
// Anthropic Provider
// ===========================================================================

/// Anthropic LLM provider (Claude API).
pub struct AnthropicProvider {
    config: LlmProviderConfig,
    client: Client,
    context_window: AtomicUsize,
}

impl AnthropicProvider {
    /// Creates a new Anthropic provider from the given configuration.
    pub fn new(config: &LlmProviderConfig) -> Self {
        Self {
            config: config.clone(),
            client: http_client().expect("Failed to create HTTP client"),
            context_window: AtomicUsize::new(config.context_window),
        }
    }

    fn base_url(&self) -> &str {
        self.config
            .base_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com/v1")
    }

    fn api_key(&self) -> Result<&str> {
        self.config
            .api_key
            .as_deref()
            .ok_or_else(|| Error::Provider("Anthropic API key is required".into()))
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn context_window(&self) -> usize {
        self.context_window.load(Ordering::Relaxed)
    }

    fn set_context_window(&self, value: usize) {
        self.context_window.store(value, Ordering::Relaxed);
    }

    fn input_price_per_million(&self) -> f64 {
        self.config.input_price_per_million
    }

    fn output_price_per_million(&self) -> f64 {
        self.config.output_price_per_million
    }

    async fn fetch_context_window(&self) -> Result<usize> {
        let model = strip_model_prefix(&self.config.model);
        let window = match model.as_str() {
            m if m.starts_with("claude-opus-4") => 1_000_000,
            m if m.starts_with("claude-sonnet-4") => 200_000,
            m if m.starts_with("claude-haiku-4") => 200_000,
            m if m.starts_with("claude-3-7-sonnet") => 200_000,
            m if m.starts_with("claude-3-5-sonnet") => 200_000,
            m if m.starts_with("claude-3-5-haiku") => 200_000,
            m if m.starts_with("claude-3-opus") => 200_000,
            m if m.starts_with("claude-3-sonnet") => 200_000,
            m if m.starts_with("claude-3-haiku") => 200_000,
            m if m.starts_with("claude-2.1") => 200_000,
            m if m.starts_with("claude-2") => 100_000,
            m if m.starts_with("claude-instant") => 100_000,
            _ => {
                return Err(Error::Provider(
                    "Unknown Anthropic model context window".into(),
                ));
            }
        };
        Ok(window)
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let (system, messages) = into_anthropic_messages(request.messages);
        let max_tokens = request.max_tokens.unwrap_or(4096);
        let model = strip_model_prefix(&request.model);

        let body = AnthropicRequest {
            model: model.clone(),
            messages,
            system,
            max_tokens,
            temperature: request.temperature,
            stream: false,
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(anthropic_tools(request.tools))
            },
        };

        let resp = self
            .client
            .post(format!("{}/messages", self.base_url()))
            .header("x-api-key", self.api_key()?)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("Anthropic request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "Anthropic returned {status}: {text}"
            )));
        }

        let data: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("Anthropic parse failed: {e}")))?;

        let mut content = String::new();
        let mut tool_calls = Vec::new();

        for block in &data.content {
            match block.type_.as_str() {
                "text" => {
                    if let Some(text) = &block.text {
                        content.push_str(text);
                    }
                }
                "tool_use" => {
                    if let Some(tool) = &block.tool_use {
                        tool_calls.push(ToolCall {
                            id: tool.id.clone(),
                            call_type: "function".into(),
                            function: ToolFunction {
                                name: tool.name.clone(),
                                arguments: serde_json::to_string(&tool.input).unwrap_or_default(),
                            },
                        });
                    }
                }
                _ => {}
            }
        }

        Ok(LlmResponse {
            content,
            tool_calls,
            finish_reason: data.stop_reason.unwrap_or_default(),
            usage: data.usage.map(|u| LlmUsage {
                prompt_tokens: u.input_tokens,
                completion_tokens: u.output_tokens,
                total_tokens: u.input_tokens + u.output_tokens,
            }),
        })
    }

    async fn complete_stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<Result<LlmStreamChunk>>> {
        // For now, fall back to non-streaming for Anthropic.
        // Anthropic uses a different SSE format (text/event-stream with
        // `content_block_delta`, `content_block_stop`, `message_stop` events).
        // A full SSE implementation is left as an enhancement.
        let response = self.complete(request).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        if !response.content.is_empty() {
            let _ = tx.send(Ok(LlmStreamChunk::Content(response.content))).await;
        }
        let _ = tx
            .send(Ok(LlmStreamChunk::Done(response.usage.unwrap_or(
                LlmUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                },
            ))))
            .await;
        Ok(rx)
    }
}

// ===========================================================================
// Ollama Provider
// ===========================================================================

/// Ollama LLM provider (local inference).
pub struct OllamaProvider {
    config: LlmProviderConfig,
    client: Client,
    context_window: AtomicUsize,
}

impl OllamaProvider {
    /// Creates a new Ollama provider from the given configuration.
    pub fn new(config: &LlmProviderConfig) -> Self {
        Self {
            config: config.clone(),
            client: http_client().expect("Failed to create HTTP client"),
            context_window: AtomicUsize::new(config.context_window),
        }
    }

    fn base_url(&self) -> String {
        self.config
            .base_url
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".into())
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn context_window(&self) -> usize {
        self.context_window.load(Ordering::Relaxed)
    }

    fn set_context_window(&self, value: usize) {
        self.context_window.store(value, Ordering::Relaxed);
    }

    fn input_price_per_million(&self) -> f64 {
        0.0
    }

    fn output_price_per_million(&self) -> f64 {
        0.0
    }

    async fn fetch_context_window(&self) -> Result<usize> {
        let model = strip_model_prefix(&self.config.model);
        let body = serde_json::json!({ "model": model });
        let resp = self
            .client
            .post(format!("{}/api/show", self.base_url()))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("Ollama show request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "Ollama show returned {status}: {text}"
            )));
        }

        let data: OllamaShowResponse = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("Ollama show parse failed: {e}")))?;

        data.model_info
            .iter()
            .find(|(key, _)| key.contains("context_length"))
            .and_then(|(_, value)| value.as_u64().map(|v| v as usize))
            .ok_or_else(|| Error::Provider("Ollama model info has no context_length".into()))
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let model = strip_model_prefix(&request.model);
        let body = OllamaChatRequest {
            model: model.clone(),
            messages: into_ollama_messages(request.messages),
            stream: false,
            options: request.temperature.map(|t| OllamaOptions {
                temperature: Some(t),
            }),
        };

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url()))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("Ollama request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("Ollama returned {status}: {text}")));
        }

        let data: OllamaChatResponse = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("Ollama parse failed: {e}")))?;

        Ok(LlmResponse {
            content: data.message.content,
            tool_calls: data
                .message
                .tool_calls
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| {
                    let name = v.get("function")?.get("name")?.as_str()?.to_string();
                    let arguments = v
                        .get("function")?
                        .get("arguments")
                        .map(|a| a.to_string())
                        .unwrap_or_default();
                    Some(ToolCall {
                        id: format!("ollama_{}", name),
                        call_type: "function".into(),
                        function: ToolFunction { name, arguments },
                    })
                })
                .collect(),
            finish_reason: if data.done { "stop" } else { "unknown" }.into(),
            usage: Some(LlmUsage {
                prompt_tokens: data.prompt_eval_count.unwrap_or(0),
                completion_tokens: data.eval_count.unwrap_or(0),
                total_tokens: data.prompt_eval_count.unwrap_or(0) + data.eval_count.unwrap_or(0),
            }),
        })
    }

    async fn complete_stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<Result<LlmStreamChunk>>> {
        let model = strip_model_prefix(&request.model);
        let body = OllamaChatRequest {
            model: model.clone(),
            messages: into_ollama_messages(request.messages),
            stream: true,
            options: request.temperature.map(|t| OllamaOptions {
                temperature: Some(t),
            }),
        };

        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url());
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            let resp = match client.post(&url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx
                        .send(Err(Error::Provider(format!(
                            "Ollama stream request failed: {e}"
                        ))))
                        .await;
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(Error::Provider(format!(
                        "Ollama stream returned {status}: {text}"
                    ))))
                    .await;
                return;
            }

            use futures::StreamExt;
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(line_end) = buf.find('\n') {
                            let line = buf[..line_end].trim().to_string();
                            buf = buf[line_end + 1..].to_string();
                            if line.is_empty() {
                                continue;
                            }
                            if let Ok(data) = serde_json::from_str::<OllamaChatResponse>(&line) {
                                if !data.message.content.is_empty() {
                                    let _ = tx
                                        .send(Ok(LlmStreamChunk::Content(data.message.content)))
                                        .await;
                                }
                                if data.done {
                                    let _ = tx
                                        .send(Ok(LlmStreamChunk::Done(LlmUsage {
                                            prompt_tokens: data.prompt_eval_count.unwrap_or(0),
                                            completion_tokens: data.eval_count.unwrap_or(0),
                                            total_tokens: data.prompt_eval_count.unwrap_or(0)
                                                + data.eval_count.unwrap_or(0),
                                        })))
                                        .await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(Error::Provider(format!(
                                "Ollama stream read error: {e}"
                            ))))
                            .await;
                        return;
                    }
                }
            }
        });

        Ok(rx)
    }
}

/// Registry of LLM providers, stored as Arc for sharing across agents.
pub struct LlmProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
}

impl Clone for LlmProviderRegistry {
    fn clone(&self) -> Self {
        Self {
            providers: self.providers.clone(),
        }
    }
}

impl LlmProviderRegistry {
    /// Creates an empty provider registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Registers a named LLM provider in the registry.
    pub fn register(&mut self, name: String, provider: Arc<dyn LlmProvider>) {
        self.providers.insert(name, provider);
    }

    /// Returns a reference to the provider registered under the given name, if any.
    pub fn get(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.providers.get(name).cloned()
    }
}

impl Default for LlmProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_factory_creates_all_providers() {
        let configs = vec![
            (LlmProviderType::Anthropic, "Anthropic".to_string()),
            (LlmProviderType::OpenAI, "OpenAI".to_string()),
            (LlmProviderType::OpenRouter, "OpenRouter".to_string()),
            (LlmProviderType::Ollama, "Ollama".to_string()),
        ];

        for (ptype, _name) in configs {
            let config = LlmProviderConfig {
                provider_type: ptype.clone(),
                api_key: if ptype == LlmProviderType::Ollama {
                    None
                } else {
                    Some("test-key".into())
                },
                model: "test-model".into(),
                base_url: None,
                context_window: 100_000,
                input_price_per_million: 3.0,
                output_price_per_million: 15.0,
            };
            let provider = create_provider(&config);
            assert_eq!(
                std::mem::discriminant(&ptype),
                std::mem::discriminant(&ptype)
            );
            // Just verify it doesn't panic
            let _ = provider;
        }
    }

    #[test]
    fn test_registry() {
        let mut registry = LlmProviderRegistry::new();
        let config = LlmProviderConfig {
            provider_type: LlmProviderType::OpenAI,
            api_key: Some("key".into()),
            model: "gpt-4o".into(),
            base_url: None,
            context_window: 100_000,
            input_price_per_million: 3.0,
            output_price_per_million: 15.0,
        };
        registry.register("primary".into(), Arc::from(create_provider(&config)));
        assert!(registry.get("primary").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_message_conversion_openai() {
        let messages = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "You are helpful.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "Hello".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let openai_msgs = into_openai_messages(messages);
        assert_eq!(openai_msgs.len(), 2);
        assert_eq!(openai_msgs[0].role, "system");
        assert_eq!(openai_msgs[1].role, "user");
    }

    #[test]
    fn test_message_conversion_anthropic() {
        let messages = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "You are helpful.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "Hello".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let (system, msgs) = into_anthropic_messages(messages);
        assert_eq!(system, Some("You are helpful.".into()));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn test_sse_parsing() {
        let line = r#"data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunk = parse_sse_line(line);
        assert!(chunk.is_some());

        let done = "data: [DONE]";
        assert!(parse_sse_line(done).is_none());

        let empty = "";
        assert!(parse_sse_line(empty).is_none());

        let comment = ": keep-alive";
        assert!(parse_sse_line(comment).is_none());
    }

    #[test]
    fn test_openai_tool_conversion() {
        let tool = ToolDefinition {
            name: "get_weather".into(),
            description: "Get weather".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                }
            }),
        };
        let openai_tools = into_openai_tools(vec![tool]);
        assert_eq!(openai_tools.len(), 1);
        assert_eq!(openai_tools[0].function.name, "get_weather");
    }

    #[test]
    fn test_ollama_message_conversion() {
        let messages = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "Be concise.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "Hi".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let ollama_msgs = into_ollama_messages(messages);
        assert_eq!(ollama_msgs[0].role, "system");
        assert_eq!(ollama_msgs[1].role, "user");
    }
}
