# Task 08: ASCII Text View Renderer

## Goal
Implement the ASCII text view from spec section 4.2.2. Renders the tab in classic monospace ASCII format with playback cursor tracking.

## Deliverables
1. `crates/rifflab-tab/src/render_ascii.rs` — TabDocument → ASCII string conversion
2. Functions:
   - `tab_to_ascii(tab: &TabDocument, chars_per_beat: usize) -> String`
     - Quantize note positions to character grid
     - Fill dashes for empty positions
     - Insert measure bars `|` based on time signature
     - Add technique characters (h, p, /, \, x, etc.)
     - Multi-digit frets take 2 columns
   - `position_to_column(time_secs: f64, tab: &TabDocument, chars_per_beat: usize) -> usize`
     - For playback cursor tracking
3. In `tab_view.rs`: add ASCII view mode toggle
   - Render in monospace `egui::TextEdit` (read-only)
   - Highlight current column with background color
   - Auto-scroll to keep cursor visible
4. Section headers rendered as comments above each tab group

## Dependencies
- Task 01 (data model)
- Task 07 (tab view integration point)

## Files
- `crates/rifflab-tab/src/render_ascii.rs`
- Modify `crates/rifflab-app/src/tab_view.rs`

## Verification
- Convert the Tool 46&2 TabDocument to ASCII, verify it matches expected format
- Cursor tracks playback position
