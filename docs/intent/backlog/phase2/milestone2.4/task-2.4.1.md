---
id: "2.4.1"
title: "Persist SessionResult to SQLite"
status: pending
crate: "rifflab-practice"
requirement: "FR-M5-08"
---

On session end, insert the `SessionResult` and its `Vec<NoteResult>` into SQLite. Use the rusqlite crate for database access. Store session metadata (song_id, timestamp, section, tempo, overall accuracy) in the sessions table and individual note results (pitch deviation, timing deviation, matched/missed) in the note_results table with a foreign key to the session.
