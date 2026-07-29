//! Background worker that runs the multi-turn Anthropic conversation loop,
//! including client-side tool execution.
//!
//! Phase C is still non-streaming: each model turn is a single non-streaming
//! POST. The loop alternates between sending the conversation, receiving any
//! `tool_use` blocks, executing them against a *clone* of the user's
//! `preset_nav`, appending `tool_result` blocks, and re-posting — until the
//! model returns `stop_reason: end_turn`. At that point we ship the final
//! reply plus the (possibly mutated) preset graph back to the UI thread,
//! which atomically swaps it in.

use super::tools;
use super::types::{ChatMessage, Role};
use crate::preset_graph::PresetGraph;
use rifflab_llm::{
    AnthropicClient, ContentBlock, LlmError, Message as LlmMessage, Role as LlmRole, Tool,
};
use serde_json::{json, Value};

/// Maximum tool-use turns per user message. Hard cap to bound API spend.
const MAX_TURNS: u32 = 8;

/// Events the worker sends back to the UI thread. The UI translates each
/// into chat-log entries / final state mutations.
#[derive(Debug)]
pub enum WorkerEvent {
    /// Inline activity line — "Searching the web…", "Edited preset 'Korn Blind'".
    Activity(String),
    /// Tool call completed; recorded as a `System`-role message in the chat.
    ToolCalled { name: String, summary: String, is_error: bool },
    /// Final assistant text reply.
    Reply(String),
    /// The mutated preset graph (Some) if any preset tool ran successfully.
    /// Sent right before `Done`.
    PresetMutation(Box<PresetGraph>),
    /// Failure surfaced to the user as a system message in the chat.
    Error(String),
    /// Marker that the request is over (success or failure). UI clears `pending`.
    Done,
}

/// Inputs the worker needs from the UI thread.
pub struct RequestInput {
    pub transcript: Vec<ChatMessage>,
    pub prompt: String,
    pub system_prompt: String,
    pub preset_nav: PresetGraph,
    pub tools: Vec<Tool>,
}

/// Spawn the background request. Returns the receiver the UI polls each frame.
pub fn spawn(input: RequestInput) -> std::sync::mpsc::Receiver<WorkerEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || run(input, tx));
    rx
}

fn run(input: RequestInput, tx: std::sync::mpsc::Sender<WorkerEvent>) {
    let client = match AnthropicClient::from_env() {
        Ok(c) => c,
        Err(LlmError::MissingKey) => {
            let _ = tx.send(WorkerEvent::Error(
                "ANTH_API_KEY is not set — chat is disabled.\n\
                 Source your key file (e.g. `source ~/.rystad_api_keys`) and restart."
                .into()));
            let _ = tx.send(WorkerEvent::Done);
            return;
        }
        Err(e) => {
            let _ = tx.send(WorkerEvent::Error(format!("LLM setup error: {e}")));
            let _ = tx.send(WorkerEvent::Done);
            return;
        }
    };

    // Translate the chat transcript (user/assistant only) into LLM messages.
    let mut messages: Vec<LlmMessage> = input.transcript.iter()
        .filter(|m| matches!(m.role, Role::User | Role::Assistant))
        .map(|m| LlmMessage {
            role: match m.role {
                Role::User => LlmRole::User,
                _ => LlmRole::Assistant,
            },
            content: Value::String(m.text.clone()),
        })
        .collect();
    messages.push(LlmMessage::user_text(input.prompt));

    let mut preset_nav = input.preset_nav;
    let mut preset_mutated = false;
    let mut final_text = String::new();

    for turn in 0..MAX_TURNS {
        let response = match client.post_messages(&input.system_prompt, &messages, &input.tools) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(WorkerEvent::Error(format!("Claude API error: {e}")));
                let _ = tx.send(WorkerEvent::Done);
                return;
            }
        };

        // Collect text + tool_use blocks from this turn.
        let mut turn_text = String::new();
        let mut tool_calls: Vec<(String, String, Value)> = Vec::new(); // (id, name, input)
        let mut assistant_blocks: Vec<Value> = Vec::new();
        for block in &response.content {
            match block {
                ContentBlock::Text(t) => {
                    if !t.is_empty() {
                        turn_text.push_str(t);
                        assistant_blocks.push(json!({ "type": "text", "text": t }));
                    }
                }
                ContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push((id.clone(), name.clone(), input.clone()));
                    assistant_blocks.push(json!({
                        "type": "tool_use", "id": id, "name": name, "input": input
                    }));
                }
                ContentBlock::Other(v) => {
                    // Pass through (e.g. server_tool_use, web_search_tool_result) so the
                    // model sees its own prior context on the next turn.
                    if let Some(ty) = v.get("type").and_then(|t| t.as_str()) {
                        if ty == "server_tool_use" {
                            let _ = tx.send(WorkerEvent::Activity("🔎 Web search…".into()));
                        }
                    }
                    assistant_blocks.push(v.clone());
                }
            }
        }

        // No tools requested → we're done. Stop_reason should be end_turn.
        if tool_calls.is_empty() {
            final_text = turn_text;
            break;
        }

        // Append the assistant's mixed text+tool_use turn to history.
        messages.push(LlmMessage {
            role: LlmRole::Assistant,
            content: Value::Array(assistant_blocks),
        });

        // Execute each tool, collect tool_result blocks.
        let mut user_blocks: Vec<Value> = Vec::new();
        for (id, name, tool_input) in &tool_calls {
            let outcome = tools::execute(name, tool_input, &mut preset_nav);
            let _ = tx.send(WorkerEvent::ToolCalled {
                name: name.clone(),
                summary: outcome.summary.clone(),
                is_error: outcome.is_error,
            });
            if !outcome.is_error && matches!(name.as_str(), "add_preset" | "edit_preset") {
                preset_mutated = true;
            }
            user_blocks.push(json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": outcome.result_json.to_string(),
                "is_error": outcome.is_error,
            }));
        }

        messages.push(LlmMessage {
            role: LlmRole::User,
            content: Value::Array(user_blocks),
        });

        if turn + 1 == MAX_TURNS {
            let _ = tx.send(WorkerEvent::Error(
                format!("Reached tool-call limit ({MAX_TURNS}); aborting this turn.")));
            break;
        }
    }

    if preset_mutated {
        let _ = tx.send(WorkerEvent::PresetMutation(Box::new(preset_nav)));
    }
    if !final_text.is_empty() {
        let _ = tx.send(WorkerEvent::Reply(final_text));
    }
    let _ = tx.send(WorkerEvent::Done);
}

/// Phase C system prompt builder. Wraps [`crate::chat::system_prompt::build`]
/// for callers that don't want to depend on the registry module directly.
pub fn build_system_prompt(
    registry: &rifflab_fx::registry::EffectRegistry,
    current_song: Option<&str>,
) -> String {
    crate::chat::system_prompt::build(registry, current_song)
}
