//! LLM-based ASCII tab parser using the Anthropic Claude API.
//!
//! Sends raw ASCII tab text to Claude and parses the structured JSON response.
//! Falls back to the rule-based parser on failure.

use crate::ascii;
use crate::model::{NoteSource, TabNote, Technique};
use uuid::Uuid;

const SYSTEM_PROMPT: &str = r#"You are a bass guitar tablature parser. You receive ASCII bass tab and output a JSON array of note events.

Rules:
- Bass tab has 4 lines labeled G, D, A, E (top to bottom = highest to lowest string). Sometimes labels are lowercase or missing — infer from context (4 parallel lines of dashes and numbers).
- Each number on a line is a fret number. Multi-digit frets (10-24) are two adjacent digits.
- Dashes (-) are rests/sustain.
- Technique notation:
  h = hammer-on (between two fret numbers)
  p = pull-off (between two fret numbers)
  / = slide up
  \ = slide down
  x = muted note (dead note)
  () = ghost note
  * = harmonic
  ~ = vibrato
  b = bend
  t = tap
  s = slap (sometimes)
- Notes at the same horizontal position across strings are simultaneous (chord/double stop).
- Preserve the sequential order of notes as they appear left-to-right.
- Assign each note a sequential `position` integer (0-indexed) representing its left-to-right order. Notes at the same horizontal position share the same `position` value.
- If the tab contains section labels (Intro, Verse, Chorus, etc.), include them as `section` on the first note of that section.
- IMPORTANT: If a section says "play 3x", "play 4 times", "x7", or similar repeat markers, you MUST output the notes for that section that many times (with incrementing position values). Expand ALL repeats fully.
- If you encounter notation you cannot parse, skip it and continue.
- The input may contain markdown formatting (##, ```, **bold**) — ignore the formatting and parse the tab content inside.

Output ONLY a JSON array with no markdown fencing, no explanation:
[
  {
    "string": 0,
    "fret": 5,
    "position": 0,
    "technique": null,
    "section": null
  }
]"#;

/// Intermediate struct for JSON deserialization from the LLM response.
#[derive(serde::Deserialize)]
struct AsciiNote {
    string: u8,
    fret: u8,
    #[allow(dead_code)]
    position: u32,
    technique: Option<String>,
    section: Option<String>,
}

/// Get the Anthropic API key from environment.
fn get_api_key() -> Option<String> {
    std::env::var("ANTH_API_KEY").ok()
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
}

/// Parse ASCII tab text using the Anthropic Claude API.
///
/// Falls back to the rule-based parser if:
/// - API key is not set
/// - API call fails
/// - Response cannot be parsed
/// Parse ASCII tab using LLM (blocking, for use on background thread).
/// Sends progress messages via `log_tx` if provided.
pub fn parse_ascii_tab_llm(text: &str, log_tx: Option<&std::sync::mpsc::Sender<String>>) -> Vec<TabNote> {
    let send = |msg: String| {
        if let Some(tx) = log_tx { let _ = tx.send(msg); }
    };

    let api_key = match get_api_key() {
        Some(k) if !k.is_empty() => k,
        _ => {
            send("No API key (ANTH_API_KEY), using rule-based parser".into());
            return ascii::parse_ascii_tab(text);
        }
    };

    // Strip markdown formatting for cleaner parsing
    let cleaned = strip_markdown(text);
    send("Sending tab to Claude for parsing...".into());

    // Truncate input to 50KB
    let input = if cleaned.len() > 51200 { &cleaned[..51200] } else { &cleaned };

    match call_claude(&api_key, input) {
        Ok(notes) if !notes.is_empty() => {
            send(format!("LLM parsed {} notes", notes.len()));
            notes
        }
        Ok(_) => {
            send("LLM returned no notes, falling back to rule-based parser".into());
            ascii::parse_ascii_tab(text)
        }
        Err(e) => {
            send(format!("LLM error: {}, falling back to rule-based parser", e));
            ascii::parse_ascii_tab(text)
        }
    }
}

/// Strip markdown formatting to produce clean tab text.
fn strip_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue; // skip the fence line itself
        }
        // Strip markdown headers (## → plain text)
        let line = if trimmed.starts_with('#') {
            trimmed.trim_start_matches('#').trim()
        } else {
            trimmed
        };
        // Strip bold/italic markers
        let line = line.replace("**", "").replace("__", "");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn call_claude(api_key: &str, tab_text: &str) -> Result<Vec<TabNote>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let body = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 16384,
        "system": SYSTEM_PROMPT,
        "messages": [
            { "role": "user", "content": tab_text }
        ]
    });

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("HTTP error: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().unwrap_or_default();
        return Err(format!("API error {}: {}", status, text));
    }

    let resp: serde_json::Value = response
        .json()
        .map_err(|e| format!("JSON decode error: {e}"))?;

    // Extract text content from Claude's response
    let text_content = resp["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|block| block["text"].as_str())
        .ok_or("No text content in response")?;

    // Strip markdown code fences if present
    let json_str = text_content
        .trim()
        .strip_prefix("```json")
        .or_else(|| text_content.trim().strip_prefix("```"))
        .unwrap_or(text_content)
        .strip_suffix("```")
        .unwrap_or(text_content)
        .trim();

    // Parse as array of AsciiNote
    let ascii_notes: Vec<AsciiNote> = serde_json::from_str(json_str)
        .map_err(|e| format!("JSON parse error: {e}"))?;

    // Convert to TabNote
    let tab_notes = ascii_notes.into_iter().map(|an| {
        let technique = an.technique.as_deref().and_then(parse_technique);
        TabNote {
            id: Uuid::new_v4(),
            string: an.string.min(3),
            fret: an.fret.min(24),
            time_secs: 0.0,
            duration_secs: 0.0,
            beat: None,
            measure: None,
            technique,
            confidence: 0.9, // Higher confidence from LLM than rule-based
            source: NoteSource::AsciiParse,
            section: an.section,
        }
    }).collect();

    Ok(tab_notes)
}

fn parse_technique(s: &str) -> Option<Technique> {
    match s {
        "hammer_on" => Some(Technique::HammerOn),
        "pull_off" => Some(Technique::PullOff),
        "slide_up" => Some(Technique::SlideUp),
        "slide_down" => Some(Technique::SlideDown),
        "mute" => Some(Technique::Mute),
        "ghost" => Some(Technique::Ghost),
        "harmonic" => Some(Technique::Harmonic),
        "bend" => Some(Technique::Bend),
        "vibrato" => Some(Technique::Vibrato),
        "tap" => Some(Technique::TapOn),
        "slap" => Some(Technique::Slap),
        "pop" => Some(Technique::Pop),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_technique_strings() {
        assert_eq!(parse_technique("hammer_on"), Some(Technique::HammerOn));
        assert_eq!(parse_technique("pull_off"), Some(Technique::PullOff));
        assert_eq!(parse_technique("slide_up"), Some(Technique::SlideUp));
        assert_eq!(parse_technique("mute"), Some(Technique::Mute));
        assert_eq!(parse_technique("unknown"), None);
    }

    #[test]
    fn test_fallback_on_missing_api_key() {
        // With no API key set, should fall back to rule-based parser
        let tab = "G|--5--|\nD|--0--|\nA|-----|\nE|-----|";
        let notes = parse_ascii_tab_llm(tab, None);
        assert!(!notes.is_empty(), "Should fall back to rule-based parser");
    }

    #[test]
    fn test_strip_markdown_fences() {
        let json = "```json\n[{\"string\":0,\"fret\":5,\"position\":0,\"technique\":null,\"section\":null}]\n```";
        let stripped = json
            .trim()
            .strip_prefix("```json")
            .or_else(|| json.trim().strip_prefix("```"))
            .unwrap_or(json)
            .strip_suffix("```")
            .unwrap_or(json)
            .trim();
        let notes: Vec<AsciiNote> = serde_json::from_str(stripped).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].string, 0);
        assert_eq!(notes[0].fret, 5);
    }
}
