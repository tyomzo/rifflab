# Task 02: Rule-Based ASCII Tab Parser (Fallback)

## Goal
Implement the rule-based ASCII tab parser from spec section 3.2.2. This is the offline/fallback parser that works without an API.

## Deliverables
1. `crates/rifflab-tab/src/ascii.rs` — rule-based parser module
2. Functions:
   - `parse_ascii_tab(text: &str) -> Vec<TabNote>` — main entry point
   - Identify 4-line groups matching bass tab pattern (G/D/A/E labels or inferred)
   - Scan left-to-right, extract fret numbers (including multi-digit like 10-24)
   - Detect techniques: h, p, /, \, x, (), *, ~, b, t between/around fret numbers
   - Assign sequential `position` values (same column = same position)
   - Notes get `source: NoteSource::AsciiParse`, `confidence: 0.8`, `time_secs: 0.0`
3. Section label detection (Intro, Verse, Chorus, etc. appearing before tab groups)
4. Tests with real-world tabs:
   - Tool "46 & 2" bass tab from `docs/bass-tabs/tool-46and2-bass.md`
   - Simple 4-line tab with various techniques
   - Edge cases: missing labels, inconsistent spacing, multi-digit frets

## Dependencies
- Task 01 (data model)

## Files
- `crates/rifflab-tab/src/ascii.rs`
- Update `crates/rifflab-tab/src/lib.rs` to include module

## Verification
- `cargo test -p rifflab-tab` — parser tests pass
- Parse the Tool 46&2 tab snippet and verify correct string/fret/technique extraction
