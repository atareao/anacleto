//! AWS Bedrock provider.
//!
//! Bedrock exposes an OpenAI-compatible chat completions endpoint, so this
//! provider reuses the OpenAI-compatible request/response handling with an
//! appropriate `base_url`. Note: production Bedrock deployments typically
//! require AWS SigV4 signing rather than a plain bearer token; that signing
//! layer is out of scope here and can be supplied via a proxy or the
//! `base_url`/`api_key` configuration.

use async_trait::async_trait;

use crate::error::Result;
use crate::llm::provider::{LlmProvider, OpenAIProvider};
use crate::llm::types::{LlmProviderConfig, LlmRequest, LlmResponse, LlmStreamChunk};

/// AWS Bedrock LLM provider (OpenAI-compatible).
pub struct BedrockProvider {
    inner: OpenAIProvider,
}

impl BedrockProvider {
    /// Creates a new Bedrock provider from the given configuration.
    pub fn new(config: &LlmProviderConfig) -> Self {
        let mut cfg = config.clone();
        if cfg.base_url.is_none() {
            cfg.base_url = Some("https://bedrock-runtime.us-east-1.amazonaws.com".to_string());
        }
        Self {
            inner: OpenAIProvider::new(&cfg),
        }
    }
}

#[async_trait]
impl LlmProvider for BedrockProvider {
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
    use super::*;
    use crate::llm::types::CacheControl;

    #[test]
    fn test_bedrock_provider_construction() {
        let config = LlmProviderConfig {
            provider_type: crate::llm::types::LlmProviderType::Bedrock,
            api_key: Some("bedrock-key".into()),
            model: "anthropic.claude-sonnet-4".into(),
            base_url: None,
            context_window: 200_000,
            input_price_per_million: 3.0,
            output_price_per_million: 15.0,
            cache_control: CacheControl::Auto,
            thinking_budget_tokens: None,
        };
        let provider = BedrockProvider::new(&config);
        assert_eq!(provider.context_window(), 200_000);
        assert_eq!(provider.input_price_per_million(), 3.0);
    }

    #[test]
    fn test_bedrock_default_base_url() {
        let config = LlmProviderConfig {
            provider_type: crate::llm::types::LlmProviderType::Bedrock,
            api_key: Some("bedrock-key".into()),
            model: "anthropic.claude-sonnet-4".into(),
            base_url: None,
            context_window: 200_000,
            input_price_per_million: 3.0,
            output_price_per_million: 15.0,
            cache_control: CacheControl::Auto,
            thinking_budget_tokens: None,
        };
        let provider = BedrockProvider::new(&config);
        // The inner OpenAI-compatible provider should have a Bedrock base URL.
        assert!(provider.inner.base_url().contains("bedrock"));
    }
}
