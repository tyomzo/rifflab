# Task 07: Scrolling Tablature Renderer (Primary UI)

## Goal
Implement the scrolling tablature view from spec section 4.2.1 using egui. This is the main practice view.

## Deliverables
1. `crates/rifflab-app/src/tab_view.rs` — tab rendering module
2. Rendering:
   - 4 horizontal string lines (G top, E bottom), labeled on left edge
   - Fret numbers rendered at (time, string) coordinates, monospace font
   - Playhead: vertical line at fixed position (25% from left)
   - Measure dividers: vertical lines at measure boundaries
   - Beat markers: small ticks on top edge
   - Technique annotations: H/P arcs, slide lines, ghost note parens, mute X, harmonic diamonds
   - Active note highlight (at playhead): color change + size increase
   - Past notes: 50% opacity fade
   - Section labels above staff
3. Scrolling:
   - During playback: auto-scroll, playhead stays fixed
   - When paused: mouse wheel horizontal scroll
   - Zoom: Ctrl+scroll changes how many measures visible (1-16, default 4)
4. Interaction:
   - Click on tab position → seek playback to that time
   - Click on note → select (show detail in status bar)
5. Color scheme from spec section 4.3 (dark theme)
6. New bottom tab `BottomTab::Tab` added to the tab bar

## Dependencies
- Task 01 (data model — TabDocument, TabNote)

## Files
- `crates/rifflab-app/src/tab_view.rs`
- Modify `crates/rifflab-app/src/main.rs` (add Tab bottom tab, wire rendering)

## Notes
- TabDocument is passed to the renderer, not loaded inside it
- Playback position comes from the transport (same as arrangement view)
- No editing in this task — that's Task 09

## Verification
- Create a test TabDocument programmatically, verify it renders
- Playhead tracks transport position
- Scroll and zoom work when paused
