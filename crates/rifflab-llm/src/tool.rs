use serde_json::{json, Value};

/// Tool definition sent in the `tools` array of an Anthropic `messages` request.
///
/// `WebSearch` is Anthropic's server-side tool — the model performs the search
/// itself. `Function` is a client-side tool: the model emits a `tool_use` block,
/// the caller executes it, and the result is sent back as a `tool_result` block.
#[derive(Debug, Clone)]
pub enum Tool {
    WebSearch,
    Function {
        name: String,
        description: String,
        input_schema: Value,
    },
}

impl Tool {
    /// Serialize this tool definition into the JSON shape Anthropic expects.
    pub fn to_json(&self) -> Value {
        match self {
            // https://docs.anthropic.com/en/docs/agents-and-tools/tool-use/web-search-tool
            Self::WebSearch => json!({
                "type": "web_search_20250305",
                "name": "web_search",
            }),
            Self::Function { name, description, input_schema } => json!({
                "name": name,
                "description": description,
                "input_schema": input_schema,
            }),
        }
    }
}
