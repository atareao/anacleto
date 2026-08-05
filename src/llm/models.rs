use serde::Deserialize;
use std::collections::HashMap;

/// Response shape for OpenAI's `GET /models/{model}` endpoint.
#[derive(Deserialize)]
pub(crate) struct OpenAiModelInfo {
    #[serde(default)]
    pub(crate) context_window: Option<usize>,
}

/// Response shape for OpenRouter's `GET /models` endpoint.
#[derive(Deserialize)]
pub(crate) struct OpenRouterModelList {
    pub(crate) data: Vec<OpenRouterModelData>,
}

#[derive(Deserialize)]
pub(crate) struct OpenRouterModelData {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) context_length: Option<usize>,
}

/// Response shape for Ollama's `POST /api/show` endpoint.
#[derive(Deserialize)]
pub(crate) struct OllamaShowResponse {
    #[serde(default)]
    pub(crate) model_info: HashMap<String, serde_json::Value>,
}
