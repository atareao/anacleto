use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{Error, Result};

use super::models::OpenAiModelInfo;
use super::provider::LlmProvider;
use super::provider::{
    http_client, into_openai_messages, into_openai_tools, parse_sse_line, strip_model_prefix,
};
use super::types::*;

#[derive(Serialize)]
pub(crate) struct OpenAiChatRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) top_p: Option<f32>,
    pub(crate) stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream_options: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub(crate) struct OpenAiMessage {
    pub(crate) role: String,
    pub(crate) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct OpenAiTool {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) function: OpenAiFunction,
}

#[derive(Serialize)]
pub(crate) struct OpenAiFunction {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OpenAiToolCall {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) function: OpenAiFunctionCall,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OpenAiFunctionCall {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OpenAiChatResponse {
    pub(crate) choices: Vec<OpenAiChoice>,
    #[serde(default)]
    pub(crate) usage: Option<OpenAiUsage>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OpenAiChoice {
    pub(crate) message: OpenAiResponseMessage,
    pub(crate) finish_reason: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OpenAiResponseMessage {
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<OpenAiToolCall>>,
    /// OpenRouter/OpenAI reasoning tokens (non-streaming).
    #[serde(default)]
    pub(crate) reasoning: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct OpenAiUsage {
    pub(crate) prompt_tokens: u32,
    pub(crate) completion_tokens: u32,
    pub(crate) total_tokens: u32,
    /// OpenRouter-specific: real cost in USD for prompt tokens.
    /// Can be a number or a string (OpenRouter returns strings).
    #[serde(default)]
    pub(crate) prompt_cost: Option<serde_json::Value>,
    /// OpenRouter-specific: real cost in USD for completion tokens.
    /// Can be a number or a string (OpenRouter returns strings).
    #[serde(default)]
    pub(crate) completion_cost: Option<serde_json::Value>,
}

/// Extract a cost value from OpenRouter's flexible field (string or number).
fn extract_cost(v: &Option<serde_json::Value>) -> Option<f64> {
    v.as_ref().and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        serde_json::Value::Null => None,
        _ => None,
    })
}

#[derive(Deserialize)]
pub(crate) struct OpenAiStreamChunk {
    pub(crate) choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    pub(crate) usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiStreamChoice {
    pub(crate) delta: OpenAiStreamDelta,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiStreamDelta {
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<OpenAiStreamToolCall>>,
    /// OpenRouter/OpenAI reasoning tokens (streaming).
    #[serde(default)]
    pub(crate) reasoning: Option<String>,
}

/// Partial tool call in a streaming delta (fields come in separate SSE events).
#[derive(Deserialize)]
pub(crate) struct OpenAiStreamToolCall {
    #[serde(default)]
    pub(crate) index: Option<usize>,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    pub(crate) type_: Option<String>,
    #[serde(default)]
    pub(crate) function: Option<OpenAiStreamFunction>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiStreamFunction {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) arguments: Option<String>,
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

    pub(crate) fn base_url(&self) -> &str {
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
            top_p: request.top_p,
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

        tracing::debug!(
            target: "anacleto::llm::openai",
            request_body = %serde_json::to_string_pretty(&body).unwrap_or_default(),
            "OpenAI LLM request"
        );

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

        tracing::debug!(
            target: "anacleto::llm::openai",
            response_body = %serde_json::to_string_pretty(&data).unwrap_or_default(),
            "OpenAI LLM response"
        );

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
            usage: data.usage.map(|u| {
                let prompt_cost = extract_cost(&u.prompt_cost);
                let completion_cost = extract_cost(&u.completion_cost);
                let cost = prompt_cost.zip(completion_cost).map(|(p, c)| p + c);
                LlmUsage {
                    prompt_tokens: u.prompt_tokens,
                    completion_tokens: u.completion_tokens,
                    total_tokens: u.total_tokens,
                    cost,
                }
            }),
            thinking: choice.message.reasoning,
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
            top_p: request.top_p,
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
            tracing::debug!(
                target: "anacleto::llm::openai",
                request_body = %serde_json::to_string_pretty(&body).unwrap_or_default(),
                "OpenAI LLM stream request"
            );

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

                                    // Emit reasoning tokens if present (OpenAI-compatible)
                                    if let Some(ref reasoning) = choice.delta.reasoning
                                        && !reasoning.is_empty()
                                    {
                                        let _ = tx
                                            .send(Ok(LlmStreamChunk::Thinking(reasoning.clone())))
                                            .await;
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
                                    let prompt_cost = extract_cost(&u.prompt_cost);
                                    let completion_cost = extract_cost(&u.completion_cost);
                                    let cost = prompt_cost.zip(completion_cost).map(|(p, c)| p + c);
                                    let _ = tx
                                        .send(Ok(LlmStreamChunk::Done(LlmUsage {
                                            prompt_tokens: u.prompt_tokens,
                                            completion_tokens: u.completion_tokens,
                                            total_tokens: u.total_tokens,
                                            cost,
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
                        cost: None,
                    })))
                    .await;
            }
        });

        Ok(rx)
    }
}
