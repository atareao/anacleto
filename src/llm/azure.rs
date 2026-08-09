//! Azure OpenAI provider.
//!
//! Azure OpenAI exposes an OpenAI-compatible chat completions endpoint. The
//! `base_url` should point at the deployment endpoint
//! (`https://<resource>.openai.azure.com/openai/deployments/<deployment>`)
//! and the `api_key` is the Azure resource key.

use async_trait::async_trait;

use crate::error::Result;
use crate::llm::provider::{LlmProvider, OpenAIProvider};
use crate::llm::types::{LlmProviderConfig, LlmRequest, LlmResponse, LlmStreamChunk};

/// Azure OpenAI LLM provider (OpenAI-compatible).
pub struct AzureProvider {
    inner: OpenAIProvider,
}

impl AzureProvider {
    /// Creates a new Azure provider from the given configuration.
    pub fn new(config: &LlmProviderConfig) -> Self {
        let mut cfg = config.clone();
        if cfg.base_url.is_none() {
            cfg.base_url = Some(
                "https://YOUR_RESOURCE.openai.azure.com/openai/deployments/YOUR_DEPLOYMENT"
                    .to_string(),
            );
        }
        Self {
            inner: OpenAIProvider::new(&cfg),
        }
    }
}

#[async_trait]
impl LlmProvider for AzureProvider {
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
    fn test_azure_provider_construction() {
        let config = LlmProviderConfig {
            provider_type: crate::llm::types::LlmProviderType::Azure,
            api_key: Some("azure-key".into()),
            model: "gpt-4o".into(),
            base_url: Some("https://my-resource.openai.azure.com/openai/deployments/gpt-4o".into()),
            context_window: 128_000,
            input_price_per_million: 3.0,
            output_price_per_million: 15.0,
            cache_control: CacheControl::Auto,
            thinking_budget_tokens: None,
            retry: RetryConfig::default(),
        };
        let provider = AzureProvider::new(&config);
        assert_eq!(provider.context_window(), 128_000);
        assert_eq!(provider.output_price_per_million(), 15.0);
    }

    #[test]
    fn test_azure_default_base_url() {
        let config = LlmProviderConfig {
            provider_type: crate::llm::types::LlmProviderType::Azure,
            api_key: Some("azure-key".into()),
            model: "gpt-4o".into(),
            base_url: None,
            context_window: 128_000,
            input_price_per_million: 3.0,
            output_price_per_million: 15.0,
            cache_control: CacheControl::Auto,
            thinking_budget_tokens: None,
            retry: RetryConfig::default(),
        };
        let provider = AzureProvider::new(&config);
        assert!(provider.inner.base_url().contains("openai.azure.com"));
    }
}
