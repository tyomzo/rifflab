# Task 10: App Integration — Import/Export, Session, UI Wiring

## Goal
Wire the tab system into the RiffLab app: import tabs (paste/file), run pipelines, persist in sessions, and connect to transport.

## Deliverables
1. Tab import UI in the Tab bottom tab:
   - "Paste Tab" button → text input dialog → run ASCII parser → show tab
   - "Load .rltab" button → file picker → load TabDocument
   - "Transcribe from Audio" button → run audio pipeline on current bass stem
   - "Fuse" button (when both ASCII and audio exist) → run fusion
   - Progress indicator during transcription
2. Tab export:
   - "Save .rltab" button → file picker → save TabDocument as JSON
   - Auto-save tab in session directory (alongside stems, effects, cues)
3. Session integration:
   - `session.rs`: save/load TabDocument in session directory as `tab.rltab`
   - Load tab when loading a session
4. Transport integration:
   - Tab view receives playback position from transport (same frame callback as arrangement)
   - Click-to-seek in tab sends seek command to transport
   - Loop region shown on tab
5. Add `rifflab-tab` to workspace `Cargo.toml` and `rifflab-app` dependencies

## Dependencies
- Tasks 01-08 (all pipeline and rendering tasks)

## Files
- Modify `Cargo.toml` (workspace members)
- Modify `crates/rifflab-app/Cargo.toml` (add rifflab-tab dep)
- Modify `crates/rifflab-app/src/main.rs` (import UI, session, transport wiring)
- Modify `crates/rifflab-app/src/session.rs` (tab persistence)

## Verification
- Paste an ASCII tab → renders in tab view
- Transcribe from audio → renders with timing
- Save session → tab included; load session → tab restored
- Playback position syncs between arrangement and tab views
