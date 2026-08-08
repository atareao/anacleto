use async_trait::async_trait;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::error::{Error, Result};

use super::anthropic::{AnthropicMessage, AnthropicProvider, AnthropicTool};
use super::azure::AzureProvider;
use super::bedrock::BedrockProvider;
use super::google::GoogleProvider;
use super::models::OpenRouterModelList;
use super::ollama::{OllamaMessage, OllamaProvider};
use super::openai::{
    OpenAiChatRequest, OpenAiChatResponse, OpenAiFunction, OpenAiFunctionCall, OpenAiMessage,
    OpenAiStreamChunk, OpenAiTool, OpenAiToolCall,
};
use super::types::*;

pub use super::openai::OpenAIProvider;

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
        LlmProviderType::Bedrock => Box::new(BedrockProvider::new(config)),
        LlmProviderType::Azure => Box::new(AzureProvider::new(config)),
        LlmProviderType::Google => Box::new(GoogleProvider::new(config)),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Create a shared reqwest Client with sensible defaults.
pub(crate) fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| Error::Provider(format!("Failed to create HTTP client: {e}")))
}

/// Convert a `Vec<LlmMessage>` into OpenAI-compatible message list.
pub(crate) fn into_openai_messages(messages: Vec<LlmMessage>) -> Vec<OpenAiMessage> {
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
///
/// Prompt caching for Anthropic is handled via the top-level `cache_control`
/// field on the request (automatic caching), which is set in
/// [`AnthropicProvider::complete`]. Message-level `cache_control` breakpoints
/// are intentionally NOT injected here: they are only valid inside a content
/// block array, and the automatic top-level caching already covers the
/// prompt-prefix caching use case.
pub(crate) fn into_anthropic_messages(
    messages: Vec<LlmMessage>,
) -> (Option<String>, Vec<AnthropicMessage>) {
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
pub(crate) fn into_ollama_messages(messages: Vec<LlmMessage>) -> Vec<OllamaMessage> {
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
pub(crate) fn into_openai_tools(tools: Vec<ToolDefinition>) -> Vec<OpenAiTool> {
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
pub(crate) fn anthropic_tools(tools: Vec<ToolDefinition>) -> Vec<AnthropicTool> {
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
pub(crate) fn parse_sse_line(line: &str) -> Option<OpenAiStreamChunk> {
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
pub(crate) fn strip_model_prefix(model: &str) -> String {
    model.strip_prefix('~').unwrap_or(model).to_string()
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
            .header("HTTP-Referer", "https://github.com/atareao/anacleto")
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
            thinking: None,
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
                .header("HTTP-Referer", "https://github.com/atareao/anacleto")
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
    use crate::llm::anthropic::{AnthropicCacheControl, AnthropicRequest, AnthropicResponse};

    #[test]
    fn test_factory_creates_all_providers() {
        let configs = vec![
            (LlmProviderType::Anthropic, "Anthropic".to_string()),
            (LlmProviderType::OpenAI, "OpenAI".to_string()),
            (LlmProviderType::OpenRouter, "OpenRouter".to_string()),
            (LlmProviderType::Ollama, "Ollama".to_string()),
            (LlmProviderType::Bedrock, "Bedrock".to_string()),
            (LlmProviderType::Azure, "Azure".to_string()),
            (LlmProviderType::Google, "Google".to_string()),
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
                cache_control: CacheControl::Auto,
                thinking_budget_tokens: None,
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
            cache_control: CacheControl::Auto,
            thinking_budget_tokens: None,
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
        assert_eq!(msgs[0].content, "Hello");
    }

    #[test]
    fn test_anthropic_cache_control_injection() {
        // Prompt caching for Anthropic is applied at the top level of the
        // request (automatic caching), not per-message. Verify the marker
        // serializes correctly and that the request carries it when Auto.
        let marker = AnthropicCacheControl::ephemeral();
        let json = serde_json::to_value(&marker).unwrap();
        assert_eq!(json["type"], "ephemeral");

        let request = AnthropicRequest {
            model: "claude-sonnet-4".into(),
            messages: vec![],
            system: None,
            max_tokens: 4096,
            temperature: None,
            stream: false,
            tools: None,
            thinking: None,
            cache_control: Some(AnthropicCacheControl::ephemeral()),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["cache_control"]["type"], "ephemeral");

        // With caching off, the field is omitted entirely.
        let request = AnthropicRequest {
            cache_control: None,
            ..request
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("cache_control").is_none());
    }

    #[test]
    fn test_anthropic_thinking_block_parsing() {
        // A response with a `thinking` block must expose it separately from the
        // main text content.
        let json = serde_json::json!({
            "content": [
                {"type": "thinking", "thinking": "Let me reason about this..."},
                {"type": "text", "text": "Here is the answer."}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let data: AnthropicResponse = serde_json::from_value(json).unwrap();
        assert_eq!(data.content.len(), 2);
        assert_eq!(data.content[0].type_, "thinking");
        assert_eq!(
            data.content[0].thinking.as_deref(),
            Some("Let me reason about this...")
        );
        assert_eq!(data.content[1].type_, "text");
        assert_eq!(data.content[1].text.as_deref(), Some("Here is the answer."));
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
