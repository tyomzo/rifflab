//! Shared Anthropic Claude API client for RiffLab.
//!
//! Used by the LLM-based tab parser (`rifflab-tab`) and the in-app chat panel.
//! Single client type, single API-key lookup, single set of error variants.

pub mod anthropic;
pub mod api_key;
pub mod tool;

pub use anthropic::{AnthropicClient, ClaudeResponse, ContentBlock, Message, Role, LlmError};
pub use tool::Tool;
pub use api_key::get_api_key;
