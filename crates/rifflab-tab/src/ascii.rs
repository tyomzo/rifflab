//! Rule-based ASCII bass tab parser (fallback for when LLM is unavailable).

use crate::model::{NoteSource, TabNote, Technique, STANDARD_TUNING, DROP_D_TUNING};
use std::collections::HashMap;

/// Strip markdown formatting (code fences, headers, bold) from tab text.
fn strip_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") { continue; }
        let line = if trimmed.starts_with('#') {
            trimmed.trim_start_matches('#').trim()
        } else {
            trimmed
        };
        let line = line.replace("**", "").replace("__", "");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Detect bass tuning from text content.
/// Scans for keywords like "Drop D", "Tuning: DADG", "D standard", etc.
/// Returns the appropriate tuning array.
pub fn detect_tuning(text: &str) -> [u8; 4] {
    let lower = text.to_lowercase();

    // Drop D patterns
    if lower.contains("drop d")
        || lower.contains("drop-d")
        || lower.contains("tuning: d")
        || lower.contains("tuning: dadg")
        || lower.contains("tuning:d")
        || lower.contains("d-a-d-g")
    {
        return DROP_D_TUNING;
    }

    // Check string labels in tab lines — if lowest string is labeled "D" instead of "E"
    for line in text.lines() {
        let trimmed = line.trim();
        // If first line of a tab group starts with "D|" and there are 4 tab lines,
        // the lowest string might be D (Drop D)
        if trimmed.len() >= 2 {
            let first = trimmed.as_bytes()[0];
            let second = trimmed.as_bytes()[1];
            if (first == b'D' || first == b'd') && (second == b'|' || second == b'-') {
                // Check if this is the LAST (lowest) string in a 4-line group
                // by looking at surrounding lines
                // Simple heuristic: if we see D as a string label in a tab,
                // and the text mentions "drop" anywhere, it's Drop D
                if lower.contains("drop") {
                    return DROP_D_TUNING;
                }
            }
        }
    }

    STANDARD_TUNING
}

/// Detect tempo from text content.
/// Scans for patterns like "120 BPM", "Tempo: 80", "~80 BPM".
pub fn detect_tempo(text: &str) -> Option<f64> {
    let lower = text.to_lowercase();
    for line in lower.lines() {
        let trimmed = line.trim();
        // "120 BPM" or "~120 BPM" or "Tempo: 120"
        if trimmed.contains("bpm") || trimmed.contains("tempo") {
            for word in trimmed.split(|c: char| !c.is_ascii_digit() && c != '.') {
                if let Ok(bpm) = word.parse::<f64>() {
                    if (30.0..=300.0).contains(&bpm) {
                        return Some(bpm);
                    }
                }
            }
        }
    }
    None
}

/// Parse raw ASCII bass tab text into a sequence of TabNote events.
/// Notes have string, fret, technique, and sequential position — but no timing.
/// Handles repeat markers like "play 4 times", "x4", "repeat 6 times".
/// Strips markdown formatting (```, ##, **) before parsing.
pub fn parse_ascii_tab(text: &str) -> Vec<TabNote> {
    // Pre-process: strip markdown
    let cleaned = strip_markdown(text);
    let lines: Vec<&str> = cleaned.lines().collect();
    let mut all_notes = Vec::new();
    let mut position_counter = 0u32;
    let mut current_section: Option<String> = None;
    let mut pending_repeat: u32 = 1;

    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Check for repeat markers in non-tab lines
        if !trimmed.is_empty() && !is_tab_line(trimmed) && !trimmed.starts_with('|') {
            // Check for repeat count before or after a tab group
            if let Some(count) = parse_repeat_count(trimmed) {
                if !all_notes.is_empty() && pending_repeat == 1 {
                    // Repeat applies to the PREVIOUS tab group — find where it started
                    // by looking for the last section boundary or start
                    duplicate_last_group(&mut all_notes, count, &mut position_counter);
                } else {
                    // Repeat applies to the NEXT tab group
                    pending_repeat = count;
                }
                i += 1;
                continue;
            }
            // Section label — also check for embedded repeat like "Intro (x4):"
            if is_section_label(trimmed) {
                let label = trimmed.trim_end_matches(':').to_string();
                // Check for repeat in label like "(x4)" or "(4x)"
                if let Some(count) = parse_repeat_count(&label) {
                    pending_repeat = count;
                    // Strip the repeat part from section name
                    let clean = label
                        .replace(&format!("(x{})", count), "")
                        .replace(&format!("({}x)", count), "")
                        .replace(&format!("x{}", count), "")
                        .replace(&format!("{}x", count), "")
                        .trim().to_string();
                    current_section = Some(if clean.is_empty() { label } else { clean });
                } else {
                    current_section = Some(label);
                }
            }
            i += 1;
            continue;
        }

        // Try to find a 4-line tab group starting at line i
        if let Some((group, consumed)) = find_tab_group(&lines, i) {
            let notes = parse_tab_group(&group, &mut position_counter, &mut current_section);

            // Check if the line immediately after the tab group has a repeat marker
            let after_idx = i + consumed;
            let after_repeat = if after_idx < lines.len() {
                parse_repeat_count(lines[after_idx].trim())
            } else {
                None
            };

            let repeat_count = if let Some(count) = after_repeat {
                count
            } else {
                pending_repeat
            };
            pending_repeat = 1;

            // Add the original notes
            let group_start = all_notes.len();
            all_notes.extend(notes);

            // Duplicate for repeats
            if repeat_count > 1 {
                let group_notes: Vec<TabNote> = all_notes[group_start..].to_vec();
                for _ in 1..repeat_count {
                    for note in &group_notes {
                        let mut dup = TabNote::new(note.string, note.fret, NoteSource::AsciiParse);
                        dup.technique = note.technique;
                        dup.time_secs = position_counter as f64;
                        dup.section = None; // Don't repeat section label
                        all_notes.push(dup);
                    }
                    position_counter += group_notes.len() as u32;
                }
            }

            i += consumed;
            if after_repeat.is_some() { i += 1; } // skip the repeat line too
        } else {
            i += 1;
        }
    }

    all_notes
}

/// Parse repeat count from text like "play 4 times", "x4", "4x", "repeat 6 times", "(3x)".
fn parse_repeat_count(text: &str) -> Option<u32> {
    let lower = text.to_lowercase();

    // "play N times" or "repeat N times"
    if lower.contains("time") {
        for word in lower.split_whitespace() {
            if let Ok(n) = word.parse::<u32>() {
                if n >= 2 && n <= 32 { return Some(n); }
            }
        }
    }

    // Search for "xN" or "Nx" patterns anywhere in the text
    // Handles: "x4", "(x4)", "Intro (x4):", "3x", "(3x)"
    for token in lower.split(|c: char| !c.is_alphanumeric()) {
        let token = token.trim();
        if let Some(rest) = token.strip_prefix('x') {
            if let Ok(n) = rest.parse::<u32>() {
                if n >= 2 && n <= 32 { return Some(n); }
            }
        }
        if let Some(rest) = token.strip_suffix('x') {
            if let Ok(n) = rest.parse::<u32>() {
                if n >= 2 && n <= 32 { return Some(n); }
            }
        }
    }

    None
}

/// Duplicate the last group of notes (from the last section start or a heuristic boundary).
fn duplicate_last_group(notes: &mut Vec<TabNote>, total_times: u32, position: &mut u32) {
    if notes.is_empty() || total_times <= 1 { return; }

    // Find the start of the last "group" — look for the last section label or use all notes
    let group_start = notes.iter().rposition(|n| n.section.is_some()).unwrap_or(0);
    let group: Vec<TabNote> = notes[group_start..].to_vec();

    for _ in 1..total_times {
        for note in &group {
            let mut dup = TabNote::new(note.string, note.fret, NoteSource::AsciiParse);
            dup.technique = note.technique;
            dup.time_secs = *position as f64;
            all_notes_push_dup(notes, dup);
        }
        *position += group.len() as u32;
    }
}

fn all_notes_push_dup(notes: &mut Vec<TabNote>, note: TabNote) {
    notes.push(note);
}

/// A group of 4 tab lines (G, D, A, E from top to bottom).
struct TabGroup {
    lines: [String; 4], // index 0=G (string 3), 1=D (string 2), 2=A (string 1), 3=E (string 0)
}

fn is_tab_line(line: &str) -> bool {
    let stripped = line.trim();
    // Must contain dashes and possibly numbers/technique chars
    if stripped.len() < 3 { return false; }
    // Allow optional string label prefix (G|, D|, A|, E|, etc.)
    let content = if stripped.len() >= 2 && stripped.as_bytes()[1] == b'|' {
        &stripped[2..]
    } else if stripped.starts_with('|') {
        &stripped[1..]
    } else {
        stripped
    };
    let tab_chars = content.chars().filter(|c| {
        matches!(c, '-' | '0'..='9' | 'h' | 'p' | '/' | '\\' | 'x' | 'X'
            | '(' | ')' | '*' | '~' | 'b' | 't' | 's' | '|' | ' ')
    }).count();
    tab_chars as f32 / content.len().max(1) as f32 > 0.7
}

fn is_section_label(s: &str) -> bool {
    let lower = s.to_lowercase();
    let labels = ["intro", "verse", "chorus", "bridge", "outro", "solo",
                  "pre-chorus", "pre chorus", "interlude", "riff", "break",
                  "section", "bass", "main"];
    labels.iter().any(|l| lower.contains(l))
        || (s.len() < 30 && !s.contains('-') && s.chars().all(|c| c.is_alphanumeric() || c.is_whitespace() || c == ':' || c == '(' || c == ')'))
}

fn find_tab_group(lines: &[&str], start: usize) -> Option<(TabGroup, usize)> {
    if start + 3 >= lines.len() { return None; }

    // Find 4 consecutive tab-like lines
    let mut tab_lines = Vec::new();
    let mut idx = start;
    while idx < lines.len() && tab_lines.len() < 4 {
        let trimmed = lines[idx].trim();
        if is_tab_line(trimmed) {
            tab_lines.push(idx);
        } else if !tab_lines.is_empty() {
            break; // Non-tab line after tab lines started
        } else if !trimmed.is_empty() {
            break; // Non-empty, non-tab line before any tab lines — stop (could be a section label)
        }
        idx += 1;
    }

    if tab_lines.len() < 4 { return None; }

    // Determine string assignment from labels
    let mut string_map: HashMap<usize, u8> = HashMap::new(); // line_index → string_index
    for &line_idx in &tab_lines {
        let line = lines[line_idx].trim();
        if line.len() >= 2 {
            match line.as_bytes()[0].to_ascii_uppercase() {
                b'G' if line.as_bytes()[1] == b'|' || line.as_bytes()[1] == b'-' => { string_map.insert(line_idx, 3); }
                b'D' if line.as_bytes()[1] == b'|' || line.as_bytes()[1] == b'-' => { string_map.insert(line_idx, 2); }
                b'A' if line.as_bytes()[1] == b'|' || line.as_bytes()[1] == b'-' => { string_map.insert(line_idx, 1); }
                b'E' if line.as_bytes()[1] == b'|' || line.as_bytes()[1] == b'-' => { string_map.insert(line_idx, 0); }
                _ => {}
            }
        }
    }

    // If no labels found, assume standard order: G, D, A, E (top to bottom)
    if string_map.len() < 4 {
        for (i, &line_idx) in tab_lines.iter().take(4).enumerate() {
            string_map.insert(line_idx, [3, 2, 1, 0][i]);
        }
    }

    // Build the group with lines ordered by string index
    let mut ordered: Vec<(u8, String)> = tab_lines.iter().take(4)
        .map(|&li| {
            let s = string_map.get(&li).copied().unwrap_or(0);
            let content = strip_label(lines[li]);
            (s, content)
        })
        .collect();
    ordered.sort_by_key(|(s, _)| std::cmp::Reverse(*s)); // G(3) first

    let group = TabGroup {
        lines: [
            ordered.get(0).map(|(_, l)| l.clone()).unwrap_or_default(), // G (string 3)
            ordered.get(1).map(|(_, l)| l.clone()).unwrap_or_default(), // D (string 2)
            ordered.get(2).map(|(_, l)| l.clone()).unwrap_or_default(), // A (string 1)
            ordered.get(3).map(|(_, l)| l.clone()).unwrap_or_default(), // E (string 0)
        ],
    };

    let consumed = tab_lines.last().unwrap() - start + 1;
    Some((group, consumed))
}

fn strip_label(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.len() >= 2 && trimmed.as_bytes()[1] == b'|' {
        trimmed[2..].to_string()
    } else if trimmed.starts_with('|') {
        trimmed[1..].to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_tab_group(group: &TabGroup, position: &mut u32, section: &mut Option<String>) -> Vec<TabNote> {
    let mut notes = Vec::new();
    let max_len = group.lines.iter().map(|l| l.len()).max().unwrap_or(0);

    // String indices: group.lines[0]=G(3), [1]=D(2), [2]=A(1), [3]=E(0)
    let string_indices = [3u8, 2, 1, 0];

    let mut col = 0;
    while col < max_len {
        let mut column_notes = Vec::new();

        for (line_idx, line) in group.lines.iter().enumerate() {
            let string = string_indices[line_idx];
            let bytes = line.as_bytes();

            if col >= bytes.len() { continue; }
            let ch = bytes[col];

            if ch.is_ascii_digit() {
                // Check for multi-digit fret
                let mut fret_str = String::new();
                fret_str.push(ch as char);
                if col + 1 < bytes.len() && bytes[col + 1].is_ascii_digit() {
                    fret_str.push(bytes[col + 1] as char);
                }
                if let Ok(fret) = fret_str.parse::<u8>() {
                    let technique = detect_technique(bytes, col, fret_str.len());
                    let mut note = TabNote::new(string, fret, NoteSource::AsciiParse);
                    note.technique = technique;
                    column_notes.push(note);
                }
                // Skip extra digit if multi-digit
                if fret_str.len() > 1 { col += 1; }
            } else if ch == b'x' || ch == b'X' {
                let mut note = TabNote::new(string, 0, NoteSource::AsciiParse);
                note.technique = Some(Technique::Mute);
                column_notes.push(note);
            }
        }

        if !column_notes.is_empty() {
            let is_first = notes.is_empty();
            for mut note in column_notes {
                note.time_secs = *position as f64; // Use position as placeholder
                if is_first {
                    if let Some(s) = section.take() {
                        note.section = Some(s);
                    }
                }
                notes.push(note);
            }
            *position += 1;
        }

        col += 1;
    }

    notes
}

fn detect_technique(bytes: &[u8], col: usize, fret_len: usize) -> Option<Technique> {
    // Check character after the fret number
    let after = col + fret_len;
    if after < bytes.len() {
        match bytes[after] {
            b'h' => return Some(Technique::HammerOn),
            b'p' => return Some(Technique::PullOff),
            b'/' => return Some(Technique::SlideUp),
            b'\\' => return Some(Technique::SlideDown),
            b'~' => return Some(Technique::Vibrato),
            b'b' => return Some(Technique::Bend),
            b't' => return Some(Technique::TapOn),
            _ => {}
        }
    }
    // Check character before the fret number
    if col > 0 {
        match bytes[col - 1] {
            b'h' => return Some(Technique::HammerOn),
            b'p' => return Some(Technique::PullOff),
            b'/' => return Some(Technique::SlideUp),
            b'\\' => return Some(Technique::SlideDown),
            b'(' => return Some(Technique::Ghost),
            b'*' => return Some(Technique::Harmonic),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_tab() {
        let tab = "\
G|-----5----7----|
D|---3-----------|
A|---------------|
E|-0-------------|";

        let notes = parse_ascii_tab(tab);
        assert!(notes.len() >= 3, "Expected at least 3 notes, got {}", notes.len());

        // Check E string open
        assert!(notes.iter().any(|n| n.string == 0 && n.fret == 0));
        // Check D string fret 3
        assert!(notes.iter().any(|n| n.string == 2 && n.fret == 3));
        // Check G string fret 5
        assert!(notes.iter().any(|n| n.string == 3 && n.fret == 5));
    }

    #[test]
    fn parse_techniques() {
        let tab = "\
G|--5h7---8p7---|
D|--0-----------|
A|--------------|
E|--------------|";

        let notes = parse_ascii_tab(tab);
        // Should detect hammer-on on the 5
        let has_hammer = notes.iter().any(|n| n.technique == Some(Technique::HammerOn));
        assert!(has_hammer, "Should detect hammer-on");
        // Should detect pull-off on the 8
        let has_pulloff = notes.iter().any(|n| n.technique == Some(Technique::PullOff));
        assert!(has_pulloff, "Should detect pull-off");
    }

    #[test]
    fn parse_multi_digit_frets() {
        let tab = "\
G|--12---10--|
D|-----------|
A|-----------|
E|-----------|";

        let notes = parse_ascii_tab(tab);
        assert!(notes.iter().any(|n| n.fret == 12), "Should parse fret 12");
        assert!(notes.iter().any(|n| n.fret == 10), "Should parse fret 10");
    }

    #[test]
    fn parse_muted_notes() {
        let tab = "\
G|----------|
D|----------|
A|--x-------|
E|----------|";

        let notes = parse_ascii_tab(tab);
        assert!(notes.iter().any(|n| n.technique == Some(Technique::Mute)));
    }

    #[test]
    fn parse_section_labels() {
        let tab = "\
Intro:
G|--5--|
D|--0--|
A|-----|
E|-----|

Verse:
G|--7--|
D|-----|
A|--3--|
E|-----|";

        let notes = parse_ascii_tab(tab);
        let intro_note = notes.iter().find(|n| n.section.as_deref() == Some("Intro"));
        assert!(intro_note.is_some(), "Should detect 'Intro' section label");
        let verse_note = notes.iter().find(|n| n.section.as_deref() == Some("Verse"));
        assert!(verse_note.is_some(), "Should detect 'Verse' section label");
    }

    #[test]
    fn parse_tool_46and2_riff() {
        let tab = "\
G|-----7----5h7---8p7---5h7---3h5-|
D|--0-----0-----0-----0-----0----|
A|--------------------------------|
D|--------------------------------|";

        let notes = parse_ascii_tab(tab);
        assert!(notes.len() >= 8, "Expected at least 8 notes from 46&2 riff, got {}", notes.len());
    }

    #[test]
    fn parse_repeat_play_n_times() {
        let tab = "\
G|--5--|
D|--0--|
A|-----|
E|-----|
play 3 times";

        let notes = parse_ascii_tab(tab);
        // 2 notes per group × 3 repeats = 6
        assert!(notes.len() >= 6, "Expected 6 notes (2 × 3 repeats), got {}", notes.len());
    }

    #[test]
    fn parse_repeat_x4() {
        let tab = "\
Intro (x4):
G|--5--|
D|--0--|
A|-----|
E|-----|";

        let notes = parse_ascii_tab(tab);
        // "x4" in section label → 4 repeats of 2 notes = 8
        assert!(notes.len() >= 8, "Expected 8 notes (2 × 4 repeats), got {}", notes.len());
    }

    #[test]
    fn detect_drop_d_tuning() {
        assert_eq!(detect_tuning("Tuning: Drop D\nG|--5--|"), DROP_D_TUNING);
        assert_eq!(detect_tuning("drop d tuning"), DROP_D_TUNING);
        assert_eq!(detect_tuning("Tuning: D-A-D-G"), DROP_D_TUNING);
        assert_eq!(detect_tuning("Standard tuning"), STANDARD_TUNING);
        assert_eq!(detect_tuning("G|--5--|"), STANDARD_TUNING);
    }

    #[test]
    fn detect_tempo_from_text() {
        assert_eq!(detect_tempo("Tempo: 80 BPM"), Some(80.0));
        assert_eq!(detect_tempo("~120 BPM"), Some(120.0));
        assert_eq!(detect_tempo("no tempo here"), None);
    }

    #[test]
    fn parse_repeat_count_detection() {
        assert_eq!(parse_repeat_count("play 4 times"), Some(4));
        assert_eq!(parse_repeat_count("Play 6 Times"), Some(6));
        assert_eq!(parse_repeat_count("repeat 3 times"), Some(3));
        assert_eq!(parse_repeat_count("x4"), Some(4));
        assert_eq!(parse_repeat_count("(3x)"), Some(3));
        assert_eq!(parse_repeat_count("8x"), Some(8));
        assert_eq!(parse_repeat_count("just text"), None);
        assert_eq!(parse_repeat_count("x1"), None); // 1 doesn't count as repeat
    }
}
