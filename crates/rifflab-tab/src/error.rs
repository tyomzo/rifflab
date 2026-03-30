/// Errors from tab parsing, serialization, and LLM interaction.
#[derive(Debug, thiserror::Error)]
pub enum TabError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("parse failed: {0}")]
    Parse(String),

    #[error("LLM API error: {0}")]
    LlmApi(String),

    #[error("HTTP error: {0}")]
    Http(String),
}
