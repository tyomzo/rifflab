//! Rule-based ASCII bass tab parser (fallback for when LLM is unavailable).

use crate::model::{NoteSource, TabNote, Technique};
use std::collections::HashMap;

/// Parse raw ASCII bass tab text into a sequence of TabNote events.
/// Notes have string, fret, technique, and sequential position — but no timing.
pub fn parse_ascii_tab(text: &str) -> Vec<TabNote> {
    let lines: Vec<&str> = text.lines().collect();
    let mut all_notes = Vec::new();
    let mut position_counter = 0u32;
    let mut current_section: Option<String> = None;

    let mut i = 0;
    while i < lines.len() {
        // Look for section labels (standalone text lines before tab groups)
        let trimmed = lines[i].trim();
        if !trimmed.is_empty() && !is_tab_line(trimmed) && !trimmed.starts_with('|') {
            // Could be a section label like "Intro", "Verse", etc.
            if is_section_label(trimmed) {
                current_section = Some(trimmed.trim_end_matches(':').to_string());
            }
            i += 1;
            continue;
        }

        // Try to find a 4-line tab group starting at line i
        if let Some((group, consumed)) = find_tab_group(&lines, i) {
            let notes = parse_tab_group(&group, &mut position_counter, &mut current_section);
            all_notes.extend(notes);
            i += consumed;
        } else {
            i += 1;
        }
    }

    all_notes
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
}
