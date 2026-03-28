//! Convert TabDocument to ASCII text representation.

use crate::model::{TabDocument, TabNote, Technique};

/// Render a TabDocument as classic ASCII bass tab text.
///
/// `chars_per_beat`: horizontal resolution (default 4).
pub fn tab_to_ascii(tab: &TabDocument, chars_per_beat: usize) -> String {
    if tab.notes.is_empty() {
        return String::new();
    }

    let beat_duration = 60.0 / tab.tempo.initial_bpm;
    let total_duration = tab.duration_secs() + beat_duration;
    let total_chars = (total_duration / beat_duration * chars_per_beat as f64).ceil() as usize;

    // 4 string lines: G(3), D(2), A(1), E(0) — top to bottom
    let labels = ["G", "D", "A", "E"];
    let string_order = [3u8, 2, 1, 0];

    let mut lines: [Vec<u8>; 4] = [
        vec![b'-'; total_chars],
        vec![b'-'; total_chars],
        vec![b'-'; total_chars],
        vec![b'-'; total_chars],
    ];

    // Insert measure bars
    let measure_interval = tab.time_signature.beats_per_measure as f64 * beat_duration;
    let mut t = 0.0;
    while t <= total_duration {
        let col = (t / beat_duration * chars_per_beat as f64).round() as usize;
        if col < total_chars {
            for line in &mut lines {
                line[col] = b'|';
            }
        }
        t += measure_interval;
    }

    // Place notes
    let mut sections: Vec<(usize, String)> = Vec::new();
    for note in &tab.notes {
        let col = (note.time_secs / beat_duration * chars_per_beat as f64).round() as usize;
        let line_idx = match string_order.iter().position(|&s| s == note.string) {
            Some(i) => i,
            None => continue,
        };
        if col >= total_chars { continue; }

        // Write fret number
        let fret_str = format!("{}", note.fret);
        for (j, ch) in fret_str.bytes().enumerate() {
            if col + j < total_chars {
                lines[line_idx][col + j] = ch;
            }
        }

        // Write technique suffix
        if let Some(tech) = note.technique {
            let tech_col = col + fret_str.len();
            if tech_col < total_chars {
                lines[line_idx][tech_col] = match tech {
                    Technique::HammerOn => b'h',
                    Technique::PullOff => b'p',
                    Technique::SlideUp => b'/',
                    Technique::SlideDown => b'\\',
                    Technique::Mute => b'x',
                    Technique::Vibrato => b'~',
                    Technique::Bend => b'b',
                    Technique::TapOn => b't',
                    Technique::Ghost => b'(',
                    Technique::Harmonic => b'*',
                    _ => b'-',
                };
            }
        }

        // Track section labels
        if let Some(ref section) = note.section {
            sections.push((col, section.clone()));
        }
    }

    // Build output with labels and optional section headers
    let mut output = String::new();

    // Split into groups of ~80 chars for readability
    let group_width = 60.max(chars_per_beat * tab.time_signature.beats_per_measure as usize * 2);
    let mut offset = 0;
    while offset < total_chars {
        let end = (offset + group_width).min(total_chars);

        // Section label for this range
        for (col, label) in &sections {
            if *col >= offset && *col < end {
                output.push_str(label);
                output.push('\n');
                break;
            }
        }

        // String lines
        for (i, label) in labels.iter().enumerate() {
            output.push_str(label);
            output.push('|');
            let slice = &lines[i][offset..end];
            output.push_str(&String::from_utf8_lossy(slice));
            output.push('\n');
        }
        output.push('\n');
        offset = end;
    }

    output
}

/// Map a playback time to a character column position.
pub fn position_to_column(time_secs: f64, tab: &TabDocument, chars_per_beat: usize) -> usize {
    let beat_duration = 60.0 / tab.tempo.initial_bpm;
    (time_secs / beat_duration * chars_per_beat as f64).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn render_simple_tab() {
        let mut tab = TabDocument::new("Test");
        tab.tempo = TempoMap::constant(120.0);

        let mut n1 = TabNote::new(0, 0, NoteSource::AsciiParse);
        n1.time_secs = 0.0;
        n1.duration_secs = 0.5;

        let mut n2 = TabNote::new(1, 5, NoteSource::AsciiParse);
        n2.time_secs = 0.5;
        n2.duration_secs = 0.5;
        n2.technique = Some(Technique::HammerOn);

        tab.notes.push(n1);
        tab.notes.push(n2);

        let ascii = tab_to_ascii(&tab, 4);
        assert!(!ascii.is_empty());
        assert!(ascii.contains("G|"));
        assert!(ascii.contains("E|"));
        // Fret 0 on E string
        assert!(ascii.contains('0'));
        // Fret 5 with hammer-on on A string
        assert!(ascii.contains("5h"));
    }
}
