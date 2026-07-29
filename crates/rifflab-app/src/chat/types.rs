use serde::{Deserialize, Serialize};

/// Who authored a message in the chat transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    /// Background activity (e.g. "Searching the web…", "Calling add_preset…")
    /// rendered as muted log-style lines between user/assistant turns.
    System,
}

/// A single line in the chat transcript. Phase B keeps content as a plain
/// string; Phase C will introduce richer content blocks (tool calls, citations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
    /// Unix epoch seconds.
    pub created_at: i64,
}

impl ChatMessage {
    pub fn now(role: Role, text: impl Into<String>) -> Self {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self { role, text: text.into(), created_at }
    }
}

/// Serializable form of the persisted chat log.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatHistory {
    pub messages: Vec<ChatMessage>,
}
