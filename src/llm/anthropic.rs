use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{Error, Result};

use super::provider::LlmProvider;
use super::provider::{anthropic_tools, http_client, into_anthropic_messages, strip_model_prefix};
use super::types::*;

#[derive(Serialize)]
pub(crate) struct AnthropicRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<AnthropicMessage>,
    pub(crate) system: Option<String>,
    pub(crate) max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    pub(crate) stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) thinking: Option<AnthropicThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<AnthropicCacheControl>,
}

/// Anthropic extended thinking configuration.
#[derive(Serialize)]
pub(crate) struct AnthropicThinking {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) budget_tokens: u32,
}

impl AnthropicThinking {
    pub(crate) fn enabled(budget_tokens: u32) -> Self {
        Self {
            type_: "enabled".into(),
            budget_tokens,
        }
    }
}

/// Anthropic prompt-caching breakpoint marker.
#[derive(Serialize)]
pub(crate) struct AnthropicCacheControl {
    #[serde(rename = "type")]
    pub(crate) type_: String,
}

impl AnthropicCacheControl {
    pub(crate) fn ephemeral() -> Self {
        Self {
            type_: "ephemeral".into(),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct AnthropicMessage {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Serialize)]
pub(crate) struct AnthropicTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: serde_json::Value,
}

#[derive(Deserialize)]
pub(crate) struct AnthropicResponse {
    pub(crate) content: Vec<AnthropicContentBlock>,
    pub(crate) stop_reason: Option<String>,
    pub(crate) usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
pub(crate) struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) text: Option<String>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) tool_use: Option<AnthropicToolUse>,
}

#[derive(Deserialize)]
pub(crate) struct AnthropicToolUse {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) input: serde_json::Value,
}

#[derive(Deserialize)]
pub(crate) struct AnthropicUsage {
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
}

// ===========================================================================
// Anthropic Provider
// ===========================================================================

/// Anthropic LLM provider (Claude API).
pub(crate) struct AnthropicProvider {
    config: LlmProviderConfig,
    client: Client,
    context_window: AtomicUsize,
}

impl AnthropicProvider {
    /// Creates a new Anthropic provider from the given configuration.
    pub(crate) fn new(config: &LlmProviderConfig) -> Self {
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
        let cache = request.cache_control.unwrap_or(self.config.cache_control);
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
            thinking: self
                .config
                .thinking_budget_tokens
                .map(AnthropicThinking::enabled),
            cache_control: if cache == CacheControl::Auto {
                Some(AnthropicCacheControl::ephemeral())
            } else {
                None
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
        let mut thinking = String::new();

        for block in &data.content {
            match block.type_.as_str() {
                "text" => {
                    if let Some(text) = &block.text {
                        content.push_str(text);
                    }
                }
                "thinking" => {
                    // Extended thinking output. Kept separate from the main
                    // content so consumers can surface or drop it.
                    if let Some(t) = &block.thinking {
                        thinking.push_str(t);
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
            thinking: if thinking.is_empty() {
                None
            } else {
                Some(thinking)
            },
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
