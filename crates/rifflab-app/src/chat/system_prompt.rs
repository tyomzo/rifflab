//! Generates the chat system prompt at runtime by walking the live
//! [`EffectRegistry`]. This guarantees the catalog the LLM sees always
//! matches the effects actually compiled into the binary.

use rifflab_core::audio::{ParamKind, ParamDescriptor};
use rifflab_fx::registry::EffectRegistry;
use std::fmt::Write;

const INTRO: &str = r#"You are RiffLab's preset assistant.

RiffLab is a guitar/bass practice workstation. The user authors **preset nodes** —
each preset is a directed audio graph of effects with serialized parameters.
You can read, create, and edit these presets via the tools below.

Workflow guidance:
- When asked to build a preset for a real song, use the `web_search` tool to look up
  the original artist's rig/tone (amp, pedals, EQ choices). Cite findings briefly.
- Then call `add_preset` with a valid FxGraph JSON pipeline.
- When asked to tweak a preset, prefer `edit_preset` with a `params` patch over
  replacing the whole pipeline.
- When comparing or balancing across presets, call `list_presets` first.
- Keep prose replies short — one or two sentences before/after tool calls.

"#;

const FXGRAPH_SCHEMA: &str = r#"
## FxGraph JSON schema

Every preset has a `pipeline` of this shape:

```json
{
  "nodes": [
    { "id": 1, "pos": [30.0, 150.0], "kind": "Input",  "label": "Input"  },
    { "id": 2, "pos": [1200.0, 150.0], "kind": "Output", "label": "Output" },
    {
      "id": 3, "pos": [200.0, 150.0],
      "kind": { "Effect": { "type_id": "<effect-id-from-catalog>" } },
      "label": "<human readable>",
      "params": [[<param_id>, <value>], ...]
    }
  ],
  "cables": [
    { "from": { "node_id": 1, "index": 0, "is_output": true  },
      "to":   { "node_id": 3, "index": 0, "is_output": false } }
  ],
  "next_id": 9,
  "pan": [0.0, 0.0]
}
```

Rules:
- Always include exactly one `"Input"` node and one `"Output"` node.
- Every effect node lives between Input and Output, wired in series via `cables`.
- `params` is an array of `[param_id, value]` pairs. Param IDs are listed in the
  effect catalog above. Omit unspecified params — they default sensibly.
- Position effects roughly left-to-right by signal flow; the user can rearrange.
- `next_id` must be greater than every node id you use.
"#;

const WORKED_EXAMPLE: &str = r#"
## Worked example — "Korn Blind, bass tone" (Fieldy)

```json
{
  "nodes": [
    { "id": 1, "pos": [30.0, 150.0],  "kind": "Input",  "label": "Input"  },
    { "id": 2, "pos": [1200.0, 150.0], "kind": "Output", "label": "Output" },
    { "id": 3, "pos": [200.0, 30.0],
      "kind": { "Effect": { "type_id": "builtin:noise_gate" } },
      "label": "Gate",
      "params": [[0, -40.0], [1, 1.0], [2, 50.0]] },
    { "id": 4, "pos": [200.0, 280.0],
      "kind": { "Effect": { "type_id": "builtin:compressor" } },
      "label": "Fieldy squash",
      "params": [[0, -24.0], [1, 6.0], [2, 5.0], [3, 80.0], [4, 4.0]] },
    { "id": 5, "pos": [450.0, 30.0],
      "kind": { "Effect": { "type_id": "builtin:eq" } },
      "label": "Scooped click",
      "params": [[0, 80.0], [1, 6.0], [2, 1.0],
                  [3, 500.0], [4, -6.0], [5, 1.5],
                  [6, 3500.0], [7, 5.0], [8, 2.0],
                  [9, 8000.0], [10, 2.0], [11, 1.0]] },
    { "id": 7, "pos": [720.0, 150.0],
      "kind": { "Effect": { "type_id": "builtin:cabinet" } },
      "label": "8x10 SVT",
      "params": [[0, 1.0], [1, 0.55], [2, 0.5], [3, 0.0]] }
  ],
  "cables": [
    { "from": { "node_id": 1, "index": 0, "is_output": true }, "to": { "node_id": 3, "index": 0, "is_output": false } },
    { "from": { "node_id": 3, "index": 0, "is_output": true }, "to": { "node_id": 4, "index": 0, "is_output": false } },
    { "from": { "node_id": 4, "index": 0, "is_output": true }, "to": { "node_id": 5, "index": 0, "is_output": false } },
    { "from": { "node_id": 5, "index": 0, "is_output": true }, "to": { "node_id": 7, "index": 0, "is_output": false } },
    { "from": { "node_id": 7, "index": 0, "is_output": true }, "to": { "node_id": 2, "index": 0, "is_output": false } }
  ],
  "next_id": 9, "pan": [0.0, 0.0]
}
```
"#;

/// Build the full system prompt: intro + dynamically-generated catalog + schema.
pub fn build(registry: &EffectRegistry, current_song: Option<&str>) -> String {
    let mut out = String::with_capacity(8192);
    out.push_str(INTRO);

    if let Some(song) = current_song {
        let _ = writeln!(out, "Currently loaded song: **{song}**");
        out.push('\n');
    }

    out.push_str("## Effect catalog\n\n");
    out.push_str("Each effect lists its `param_id`, name, unit, range, and default. ");
    out.push_str("Use the `type_id` verbatim when writing `\"kind\": { \"Effect\": { \"type_id\": ... } }`.\n\n");

    for (type_id, name, category) in registry.list_effects() {
        let _ = writeln!(out, "### {name} (`{type_id}`) — {category}");
        if let Some(effect) = registry.create_effect(&type_id) {
            for d in effect.param_descriptors() {
                append_param(&mut out, &d);
            }
        }
        out.push('\n');
    }

    out.push_str(FXGRAPH_SCHEMA);
    out.push_str(WORKED_EXAMPLE);

    out
}

fn append_param(out: &mut String, d: &ParamDescriptor) {
    let unit = if d.unit.is_empty() { String::new() } else { format!(" {}", d.unit) };
    let kind = match &d.kind {
        ParamKind::Float => "float".to_string(),
        ParamKind::Int => "int".to_string(),
        ParamKind::Bool => "bool (0/1)".to_string(),
        ParamKind::Enum(variants) => format!("enum [{}]", variants.iter().enumerate()
            .map(|(i, v)| format!("{i}={v}"))
            .collect::<Vec<_>>().join(", ")),
    };
    let _ = writeln!(
        out,
        "- `{id}` **{name}** ({kind}) — {min}{unit} … {max}{unit} (default {default}{unit})",
        id = d.id.0,
        name = d.name,
        min = d.min,
        max = d.max,
        default = d.default,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_every_registered_effect() {
        let reg = EffectRegistry::new();
        let prompt = build(&reg, None);
        for (type_id, name, _) in reg.list_effects() {
            assert!(prompt.contains(&type_id), "missing type_id {type_id}");
            assert!(prompt.contains(&name), "missing name {name}");
        }
        // Sanity: includes schema + example.
        assert!(prompt.contains("FxGraph JSON schema"));
        assert!(prompt.contains("Korn Blind"));
    }

    #[test]
    fn includes_song_when_provided() {
        let reg = EffectRegistry::new();
        let with_song = build(&reg, Some("blind.wav"));
        let without = build(&reg, None);
        assert!(with_song.contains("blind.wav"));
        assert!(!without.contains("blind.wav"));
    }
}
