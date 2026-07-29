//! AI chat panel: conversational preset generation and tweaking.
//!
//! The panel runs an Anthropic Claude conversation on a background thread
//! ([`worker`]), renders a chat history in a right-side egui panel ([`panel`]),
//! and persists history across launches ([`persistence`]).
//!
//! Phase B scope: text-only, non-streaming. Phase C will add tool use,
//! streaming responses, and undo snapshots.

pub mod panel;
pub mod persistence;
pub mod system_prompt;
pub mod tools;
pub mod types;
pub mod worker;

pub use panel::{ChatAction, ChatPanel};
