/// Global error type for Anacleto.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Config error: {0}")]
    Config(String),

    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Skill error: {0}")]
    Skill(String),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("LSP error: {0}")]
    Lsp(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Channel closed: {0}")]
    ChannelClosed(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("{0}")]
    Other(String),
}

/// Result alias using our global error type.
/// Result alias using [`Error`] as the error type.
pub type Result<T> = std::result::Result<T, Error>;
