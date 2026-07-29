//! Tool definitions and execution for the chat panel.
//!
//! Tools run on the **worker thread**, against a *clone* of `preset_nav` that
//! the worker owns for the duration of a request. After the conversation
//! reaches `end_turn` the modified graph is shipped back to the UI thread,
//! which atomically swaps it into the live `RiffLabApp.preset_nav` (and pushes
//! a snapshot onto the undo stack first).
//!
//! Why clone-in-worker rather than RPC-back-to-UI for each tool call?
//! Single-threaded synchronous tool execution keeps the loop trivial: parse
//! tool_use → mutate the clone → produce tool_result → re-POST. Whatever the
//! LLM does across multiple tool calls in one turn lands as a single user-
//! visible mutation, which is also a natural undo boundary.

use crate::node_editor::FxGraph;
use crate::preset_graph::PresetGraph;
use rifflab_llm::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Build the list of tools advertised to the LLM. Server-side `web_search`
/// is always included; client-side preset tools have JSON schemas the LLM
/// must conform to.
pub fn tool_set() -> Vec<Tool> {
    vec![
        Tool::WebSearch,
        Tool::Function {
            name: "list_presets".to_string(),
            description: "List every preset node currently in the user's preset graph. \
                          Returns id, name, and the ordered list of effect type_ids in the \
                          pipeline. Call this first when balancing or comparing presets.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        Tool::Function {
            name: "add_preset".to_string(),
            description: "Add a new preset node to the graph. The pipeline is a full \
                          FxGraph JSON (nodes + cables) as documented in the system prompt. \
                          Returns the new node id.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short, human-readable preset name." },
                    "pipeline": {
                        "type": "object",
                        "description": "An FxGraph JSON object (see system prompt schema)."
                    }
                },
                "required": ["name", "pipeline"],
                "additionalProperties": false
            }),
        },
        Tool::Function {
            name: "edit_preset".to_string(),
            description: "Modify an existing preset. Identify the target by `id` or `name` \
                          (case-insensitive). Use `params` for surgical changes (preferred), \
                          or `replace_pipeline` to swap the entire FxGraph at once. \
                          Optionally `rename` it.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "object",
                        "properties": {
                            "id":   { "type": "integer" },
                            "name": { "type": "string"  }
                        }
                    },
                    "rename": { "type": "string" },
                    "params": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "node_label": { "type": "string", "description": "Match an effect node by its label substring (case-insensitive)." },
                                "node_id":    { "type": "integer", "description": "Or match by exact node id." },
                                "param_id":   { "type": "integer", "description": "Param id from the effect catalog." },
                                "value":      { "type": "number"  }
                            },
                            "required": ["param_id", "value"]
                        }
                    },
                    "replace_pipeline": {
                        "type": "object",
                        "description": "Full FxGraph JSON. Use sparingly."
                    }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        },
    ]
}

// ─── Execution ──────────────────────────────────────────────────────────────

/// Result of a tool call. `summary` is a short human-readable line for the
/// chat activity log; `result_json` is the JSON returned to the LLM.
pub struct ToolOutcome {
    pub summary: String,
    pub result_json: Value,
    pub is_error: bool,
}

/// Execute a tool call against the worker's local copy of the preset graph.
pub fn execute(name: &str, input: &Value, preset_nav: &mut PresetGraph) -> ToolOutcome {
    match name {
        "list_presets" => list_presets(preset_nav),
        "add_preset" => add_preset(input, preset_nav),
        "edit_preset" => edit_preset(input, preset_nav),
        other => ToolOutcome {
            summary: format!("Unknown tool: {other}"),
            result_json: json!({ "error": format!("unknown tool '{other}'") }),
            is_error: true,
        },
    }
}

fn list_presets(preset_nav: &PresetGraph) -> ToolOutcome {
    let presets: Vec<Value> = preset_nav.nodes.iter().map(|n| {
        let effects: Vec<String> = n.pipeline.nodes.iter()
            .filter_map(|fxn| match &fxn.kind {
                crate::node_editor::NodeKind::Effect { type_id } => Some(type_id.clone()),
                _ => None,
            }).collect();
        json!({
            "id": n.id,
            "name": n.name,
            "effects": effects,
        })
    }).collect();

    let summary = format!("Listed {} preset(s)", presets.len());
    ToolOutcome {
        summary,
        result_json: json!({ "presets": presets }),
        is_error: false,
    }
}

#[derive(Deserialize)]
struct AddPresetInput {
    name: String,
    pipeline: FxGraph,
}

fn add_preset(input: &Value, preset_nav: &mut PresetGraph) -> ToolOutcome {
    let parsed: AddPresetInput = match serde_json::from_value(input.clone()) {
        Ok(v) => v,
        Err(e) => return error_outcome(&format!("add_preset: invalid arguments — {e}")),
    };
    let name = parsed.name.clone();
    let id = preset_nav.add_node_with_pipeline(parsed.name, parsed.pipeline);
    ToolOutcome {
        summary: format!("Added preset '{name}' (id {id})"),
        result_json: json!({ "id": id, "name": name }),
        is_error: false,
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct PresetSelector {
    #[serde(default)] id: Option<u64>,
    #[serde(default)] name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ParamPatch {
    #[serde(default)] node_label: Option<String>,
    #[serde(default)] node_id: Option<u64>,
    param_id: u32,
    value: f32,
}

#[derive(Deserialize, Debug)]
struct EditPresetInput {
    target: PresetSelector,
    #[serde(default)] rename: Option<String>,
    #[serde(default)] params: Vec<ParamPatch>,
    #[serde(default)] replace_pipeline: Option<FxGraph>,
}

fn edit_preset(input: &Value, preset_nav: &mut PresetGraph) -> ToolOutcome {
    let parsed: EditPresetInput = match serde_json::from_value(input.clone()) {
        Ok(v) => v,
        Err(e) => return error_outcome(&format!("edit_preset: invalid arguments — {e}")),
    };

    // Resolve target node id.
    let target_id = match resolve_target(&parsed.target, preset_nav) {
        Ok(id) => id,
        Err(msg) => return error_outcome(&msg),
    };

    let mut changes: Vec<String> = Vec::new();
    let mut error: Option<String> = None;

    {
        let Some(node) = preset_nav.find_node_mut(target_id) else {
            return error_outcome(&format!("edit_preset: preset id {target_id} disappeared"));
        };

        if let Some(new_name) = parsed.rename.clone() {
            changes.push(format!("renamed to '{new_name}'"));
            node.name = new_name;
        }

        if let Some(new_pipeline) = parsed.replace_pipeline {
            node.pipeline = new_pipeline;
            changes.push("replaced pipeline".to_string());
        }

        for patch in parsed.params {
            match apply_param_patch(&mut node.pipeline, &patch) {
                Ok(label) => changes.push(format!("{label}.{} = {}", patch.param_id, patch.value)),
                Err(msg) => {
                    error = Some(msg);
                    break;
                }
            }
        }
    }

    if let Some(msg) = error {
        return error_outcome(&format!("edit_preset partial: {msg}"));
    }

    let summary = if changes.is_empty() {
        format!("Edit preset {target_id}: no-op")
    } else {
        format!("Edited preset {target_id}: {}", changes.join("; "))
    };
    ToolOutcome {
        summary,
        result_json: json!({ "id": target_id, "changes": changes }),
        is_error: false,
    }
}

fn resolve_target(sel: &PresetSelector, preset_nav: &PresetGraph) -> Result<u64, String> {
    if let Some(id) = sel.id {
        if preset_nav.find_node(id).is_some() {
            return Ok(id);
        }
        return Err(format!("no preset with id {id}"));
    }
    if let Some(name) = sel.name.as_deref() {
        let needle = name.to_lowercase();
        let mut matches: Vec<u64> = preset_nav.nodes.iter()
            .filter(|n| n.name.to_lowercase().contains(&needle))
            .map(|n| n.id)
            .collect();
        if matches.is_empty() {
            return Err(format!("no preset name matches '{name}'"));
        }
        if matches.len() > 1 {
            // Be deterministic: prefer exact (case-insensitive) match if present.
            if let Some(exact) = preset_nav.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(name)) {
                return Ok(exact.id);
            }
            matches.sort();
            return Err(format!("ambiguous name '{name}' matches ids {matches:?}"));
        }
        return Ok(matches[0]);
    }
    Err("target must include `id` or `name`".to_string())
}

fn apply_param_patch(pipeline: &mut FxGraph, patch: &ParamPatch) -> Result<String, String> {
    // Resolve target node within the pipeline by id or label substring.
    let node_id = if let Some(id) = patch.node_id {
        if pipeline.find_node(id).is_none() {
            return Err(format!("pipeline node id {id} not found"));
        }
        id
    } else if let Some(label) = patch.node_label.as_deref() {
        let needle = label.to_lowercase();
        let candidates: Vec<&crate::node_editor::FxNode> = pipeline.nodes.iter()
            .filter(|n| matches!(n.kind, crate::node_editor::NodeKind::Effect { .. })
                && n.label.to_lowercase().contains(&needle))
            .collect();
        match candidates.len() {
            0 => return Err(format!("no effect node labelled like '{label}'")),
            1 => candidates[0].id,
            _ => return Err(format!("label '{label}' is ambiguous ({} matches)", candidates.len())),
        }
    } else {
        return Err("param patch needs `node_id` or `node_label`".to_string());
    };

    let label_copy;
    {
        let Some(node) = pipeline.find_node_mut(node_id) else {
            return Err(format!("pipeline node {node_id} disappeared"));
        };
        // Replace existing param or append.
        let mut found = false;
        for (pid, val) in node.params.iter_mut() {
            if *pid == patch.param_id {
                *val = patch.value;
                found = true;
                break;
            }
        }
        if !found {
            node.params.push((patch.param_id, patch.value));
            node.params.sort_by_key(|(pid, _)| *pid);
        }
        label_copy = node.label.clone();
    }
    Ok(label_copy)
}

fn error_outcome(msg: &str) -> ToolOutcome {
    ToolOutcome {
        summary: msg.to_string(),
        result_json: json!({ "error": msg }),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_editor::FxGraph;
    use crate::preset_graph::PresetGraph;

    fn make_simple_preset_nav() -> PresetGraph {
        let mut nav = PresetGraph::default();
        nav.add_node_with_pipeline("Korn Blind".into(), FxGraph::new_default());
        nav.add_node_with_pipeline("Tool 46&2".into(), FxGraph::new_default());
        nav
    }

    #[test]
    fn list_presets_returns_all() {
        let nav = make_simple_preset_nav();
        let out = execute("list_presets", &json!({}), &mut nav.clone());
        assert!(!out.is_error);
        assert_eq!(out.result_json["presets"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn add_preset_appends_node() {
        let mut nav = make_simple_preset_nav();
        let input = json!({
            "name": "Test",
            "pipeline": FxGraph::new_default(),
        });
        let out = execute("add_preset", &input, &mut nav);
        assert!(!out.is_error);
        assert_eq!(nav.nodes.len(), 3);
        assert!(out.result_json["id"].as_u64().is_some());
    }

    #[test]
    fn edit_preset_by_name_substring() {
        let mut nav = make_simple_preset_nav();
        let target_id = nav.nodes.iter().find(|n| n.name == "Korn Blind").unwrap().id;
        let input = json!({
            "target": { "name": "korn" },
            "rename": "Korn Blind v2"
        });
        let out = execute("edit_preset", &input, &mut nav);
        assert!(!out.is_error, "{}", out.summary);
        assert_eq!(out.result_json["id"].as_u64(), Some(target_id));
        assert_eq!(nav.find_node(target_id).unwrap().name, "Korn Blind v2");
    }

    #[test]
    fn edit_preset_ambiguous_name_errors() {
        let mut nav = PresetGraph::default();
        nav.add_node_with_pipeline("Bass Tone A".into(), FxGraph::new_default());
        nav.add_node_with_pipeline("Bass Tone B".into(), FxGraph::new_default());
        let out = execute("edit_preset", &json!({"target": {"name": "bass"}}), &mut nav);
        assert!(out.is_error);
        assert!(out.summary.contains("ambiguous"));
    }

    #[test]
    fn edit_preset_param_patch_by_label() {
        use crate::node_editor::{FxNode, NodeKind};
        let mut nav = PresetGraph::default();
        let mut pipe = FxGraph::new_default();
        // Append an EQ-labelled effect node.
        pipe.nodes.push(FxNode {
            id: 3,
            pos: [200.0, 100.0],
            kind: NodeKind::Effect { type_id: "builtin:eq".into() },
            label: "EQ scoop".into(),
            params: vec![(4, 0.0)],
            midi_binding: None,
        });
        pipe.next_id = 4;
        nav.add_node_with_pipeline("Preset".into(), pipe);

        let input = json!({
            "target": { "name": "Preset" },
            "params": [{ "node_label": "EQ", "param_id": 4, "value": -8.0 }]
        });
        let out = execute("edit_preset", &input, &mut nav);
        assert!(!out.is_error, "{}", out.summary);

        let preset = &nav.nodes[0];
        let eq = preset.pipeline.find_node(3).unwrap();
        let v = eq.params.iter().find(|(pid, _)| *pid == 4).map(|(_, v)| *v);
        assert_eq!(v, Some(-8.0));
    }
}
