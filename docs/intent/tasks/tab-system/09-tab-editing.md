# Task 09: Tab Note Editing

## Goal
Implement inline tab editing from spec sections 4.4–4.5. Notes are always editable — no separate edit mode.

## Deliverables
1. Edit operations in `tab_view.rs`:
   - Click note to select → highlight, show detail in status bar
   - Type number → replace fret (two-digit: second digit within 500ms)
   - Delete/Backspace → delete selected note
   - Tab/Shift+Tab → select next/previous note
   - Click empty position → seek; type number → insert new note
   - Right-click → context menu: change technique, delete, view source/confidence
   - Drag note vertically → move to different string
   - Drag note horizontally → adjust timing (snap to beat grid, Alt for free)
   - Multi-select: Shift+click extend, Ctrl+click toggle
2. Non-destructive editing:
   - Original notes preserved (with original source/confidence)
   - User edits create new TabNote with `source: UserEdit`, `confidence: 1.0`
   - Diff layer: `TabDocument.user_edits: Vec<TabEdit>` tracking add/modify/delete
   - Right-click "Revert to original" on any edited note
3. Undo/Redo:
   - `UndoStack` with edit history
   - Ctrl+Z undo, Ctrl+Shift+Z redo
4. Editing works during playback (edits apply immediately)

## Dependencies
- Task 01 (data model)
- Task 07 (scrolling renderer)

## Files
- Modify `crates/rifflab-tab/src/model.rs` (add TabEdit, user_edits field)
- Modify `crates/rifflab-app/src/tab_view.rs` (editing interactions)

## Verification
- Select note, type new fret → updates
- Delete note → removed from view
- Undo restores deleted note
- Revert to original works on edited notes
