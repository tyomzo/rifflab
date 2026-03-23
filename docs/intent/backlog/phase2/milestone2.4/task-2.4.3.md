---
id: "2.4.3"
title: "SQLite schema initialization"
status: pending
crate: "rifflab-app"
requirement: "---"
---

Create the SQLite database schema on first run. Initialize the songs, sessions, and note_results tables using the schema defined in SDD-001 section 8. Use CREATE TABLE IF NOT EXISTS for idempotent initialization. Run schema migrations if the database already exists but is at an older schema version.
