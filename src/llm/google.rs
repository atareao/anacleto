//! Google Gemini provider.
//!
//! Google's Gemini API exposes an OpenAI-compatible chat completions endpoint
//! (`https://generativelanguage.googleapis.com/v1beta/openai`). The `api_key`
//! is the Google AI Studio API key.

use async_trait::async_trait;

use crate::error::Result;
use crate::llm::provider::{LlmProvider, OpenAIProvider};
use crate::llm::types::{LlmProviderConfig, LlmRequest, LlmResponse, LlmStreamChunk};

/// Google Gemini LLM provider (OpenAI-compatible).
pub struct GoogleProvider {
    inner: OpenAIProvider,
}

impl GoogleProvider {
    /// Creates a new Google provider from the given configuration.
    pub fn new(config: &LlmProviderConfig) -> Self {
        let mut cfg = config.clone();
        if cfg.base_url.is_none() {
            cfg.base_url =
                Some("https://generativelanguage.googleapis.com/v1beta/openai".to_string());
        }
        Self {
            inner: OpenAIProvider::new(&cfg),
        }
    }
}

#[async_trait]
impl LlmProvider for GoogleProvider {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        self.inner.complete(request).await
    }

    async fn complete_stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<Result<LlmStreamChunk>>> {
        self.inner.complete_stream(request).await
    }

    fn context_window(&self) -> usize {
        self.inner.context_window()
    }

    async fn fetch_context_window(&self) -> Result<usize> {
        self.inner.fetch_context_window().await
    }

    fn set_context_window(&self, value: usize) {
        self.inner.set_context_window(value);
    }

    fn input_price_per_million(&self) -> f64 {
        self.inner.input_price_per_million()
    }

    fn output_price_per_million(&self) -> f64 {
        self.inner.output_price_per_million()
    }
}

#[cfg(test)]
mod tests {
    use crate::config::types::RetryConfig;
    use super::*;
    use crate::llm::types::CacheControl;

    #[test]
    fn test_google_provider_construction() {
        let config = LlmProviderConfig {
            provider_type: crate::llm::types::LlmProviderType::Google,
            api_key: Some("google-key".into()),
            model: "gemini-2.0-flash".into(),
            base_url: None,
            context_window: 1_000_000,
            input_price_per_million: 1.25,
            output_price_per_million: 5.0,
            cache_control: CacheControl::Auto,
            thinking_budget_tokens: None,
            retry: RetryConfig::default(),
        };
        let provider = GoogleProvider::new(&config);
        assert_eq!(provider.context_window(), 1_000_000);
        assert_eq!(provider.input_price_per_million(), 1.25);
    }

    #[test]
    fn test_google_default_base_url() {
        let config = LlmProviderConfig {
            provider_type: crate::llm::types::LlmProviderType::Google,
            api_key: Some("google-key".into()),
            model: "gemini-2.0-flash".into(),
            base_url: None,
            context_window: 1_000_000,
            input_price_per_million: 1.25,
            output_price_per_million: 5.0,
            cache_control: CacheControl::Auto,
            thinking_budget_tokens: None,
            retry: RetryConfig::default(),
        };
        let provider = GoogleProvider::new(&config);
        assert!(provider.inner.base_url().contains("generativelanguage"));
    }
}
