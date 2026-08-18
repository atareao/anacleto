use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{Error, Result};

use super::models::OllamaShowResponse;
use super::provider::{LlmProvider, http_client, into_ollama_messages, strip_model_prefix};
use super::types::*;

#[derive(Serialize)]
pub(crate) struct OllamaChatRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<OllamaMessage>,
    pub(crate) stream: bool,
    pub(crate) options: Option<OllamaOptions>,
}

#[derive(Serialize)]
pub(crate) struct OllamaMessage {
    pub(crate) role: String,
    pub(crate) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Serialize)]
pub(crate) struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) top_p: Option<f32>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OllamaChatResponse {
    pub(crate) message: OllamaResponseMessage,
    pub(crate) done: bool,
    #[serde(default)]
    pub(crate) eval_count: Option<u32>,
    #[serde(default)]
    pub(crate) prompt_eval_count: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OllamaResponseMessage {
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<serde_json::Value>>,
}

// ===========================================================================
// Ollama Provider
// ===========================================================================

/// Ollama LLM provider (local inference).
pub(crate) struct OllamaProvider {
    config: LlmProviderConfig,
    client: Client,
    context_window: AtomicUsize,
}

impl OllamaProvider {
    /// Creates a new Ollama provider from the given configuration.
    pub(crate) fn new(config: &LlmProviderConfig) -> Self {
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
            options: Some(OllamaOptions {
                temperature: request.temperature,
                top_p: request.top_p,
            }),
        };

        tracing::debug!(
            target: "anacleto::llm::ollama",
            request_body = %serde_json::to_string_pretty(&body).unwrap_or_default(),
            "Ollama LLM request"
        );

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

        tracing::debug!(
            target: "anacleto::llm::ollama",
            response_body = %serde_json::to_string_pretty(&data).unwrap_or_default(),
            "Ollama LLM response"
        );

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
                cost: None,
            }),
            thinking: None,
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
            options: Some(OllamaOptions {
                temperature: request.temperature,
                top_p: request.top_p,
            }),
        };

        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url());
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            tracing::debug!(
                target: "anacleto::llm::ollama",
                request_body = %serde_json::to_string_pretty(&body).unwrap_or_default(),
                "Ollama LLM stream request"
            );

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
                                            cost: None,
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
