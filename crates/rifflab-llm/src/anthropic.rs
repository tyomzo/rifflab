use crate::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
/// Default model — Sonnet 4.5 (training cutoff Jan 2025, knows most rigs).
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const DEFAULT_MAX_TOKENS: u32 = 4096;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("API error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("malformed response: {0}")]
    Malformed(String),
    #[error("missing API key (set ANTH_API_KEY or ANTHROPIC_API_KEY)")]
    MissingKey,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A single message in the conversation, in Anthropic's wire shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    /// `content` accepts either a plain string or an array of content blocks.
    /// We keep it as a raw JSON Value so callers can mix text, tool_use, and
    /// tool_result blocks without intermediate types.
    pub content: Value,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: Value::String(text.into()) }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: Value::String(text.into()) }
    }
}

/// One block of model output. Anthropic's API returns a `content` array of these.
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text(String),
    ToolUse { id: String, name: String, input: Value },
    /// Anything we don't model explicitly (e.g. web_search_tool_result).
    /// Preserved as raw JSON so callers can introspect.
    Other(Value),
}

#[derive(Debug, Clone)]
pub struct ClaudeResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub raw: Value,
}

impl ClaudeResponse {
    /// First text block concatenated, or empty string if there were none.
    pub fn text(&self) -> String {
        self.content.iter().filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join("")
    }
}

/// Synchronous client over `reqwest::blocking`. Spawn it on a background thread.
pub struct AnthropicClient {
    http: reqwest::blocking::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicClient {
    /// Build a client using the configured API key from env. Returns `MissingKey`
    /// if neither `ANTH_API_KEY` nor `ANTHROPIC_API_KEY` are set.
    pub fn from_env() -> Result<Self, LlmError> {
        let api_key = crate::api_key::get_api_key().ok_or(LlmError::MissingKey)?;
        Self::with_key(api_key)
    }

    pub fn with_key(api_key: String) -> Result<Self, LlmError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(|e| LlmError::Http(e.to_string()))?;
        Ok(Self {
            http,
            api_key,
            model: DEFAULT_MODEL.to_string(),
            max_tokens: DEFAULT_MAX_TOKENS,
        })
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Non-streaming `messages` POST. Returns the parsed content blocks plus
    /// the raw JSON value (for callers that want to inspect usage/cost).
    pub fn post_messages(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<ClaudeResponse, LlmError> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "system": system,
            "messages": messages,
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.iter().map(|t| t.to_json()).collect());
        }

        let resp = self.http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(LlmError::Api { status: status.as_u16(), body });
        }

        let raw: Value = resp.json().map_err(|e| LlmError::Http(e.to_string()))?;
        Ok(parse_response(raw))
    }
}

fn parse_response(raw: Value) -> ClaudeResponse {
    let content = raw.get("content")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(parse_block).collect())
        .unwrap_or_default();
    let stop_reason = raw.get("stop_reason").and_then(|v| v.as_str()).map(String::from);
    ClaudeResponse { content, stop_reason, raw }
}

fn parse_block(block: &Value) -> ContentBlock {
    match block.get("type").and_then(|v| v.as_str()) {
        Some("text") => ContentBlock::Text(
            block.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string()
        ),
        Some("tool_use") => ContentBlock::ToolUse {
            id: block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            name: block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            input: block.get("input").cloned().unwrap_or(Value::Null),
        },
        _ => ContentBlock::Other(block.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_block() {
        let raw = json!({
            "content": [{"type": "text", "text": "hello"}],
            "stop_reason": "end_turn"
        });
        let r = parse_response(raw);
        assert_eq!(r.text(), "hello");
        assert_eq!(r.stop_reason.as_deref(), Some("end_turn"));
        assert!(matches!(r.content[0], ContentBlock::Text(ref s) if s == "hello"));
    }

    #[test]
    fn parse_tool_use_block() {
        let raw = json!({
            "content": [{
                "type": "tool_use",
                "id": "toolu_abc",
                "name": "add_preset",
                "input": {"name": "Korn Blind"}
            }],
            "stop_reason": "tool_use"
        });
        let r = parse_response(raw);
        match &r.content[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_abc");
                assert_eq!(name, "add_preset");
                assert_eq!(input["name"], "Korn Blind");
            }
            _ => panic!("expected tool_use"),
        }
    }

    #[test]
    fn parse_unknown_block_preserved() {
        let raw = json!({
            "content": [{"type": "web_search_tool_result", "content": [{"url": "x"}]}],
        });
        let r = parse_response(raw);
        assert!(matches!(r.content[0], ContentBlock::Other(_)));
    }

    #[test]
    fn tool_json_websearch_shape() {
        let v = Tool::WebSearch.to_json();
        assert_eq!(v["type"], "web_search_20250305");
        assert_eq!(v["name"], "web_search");
    }

    #[test]
    fn message_user_text_serializes() {
        let m = Message::user_text("hi");
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["role"], "user");
        assert_eq!(v["content"], "hi");
    }
}
